use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Emitter, State};
use walkdir::WalkDir;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "avi", "mkv", "m4v", "wmv", "flv", "webm", "mts", "m2ts", "ts",
];

#[derive(Default)]
struct AppState {
    running: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScanRequest {
    root: String,
    min_bitrate_mbps: f64,
    min_file_size_mb: f64,
    transcode_h264: bool,
    unwanted_extensions: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VideoFile {
    path: String,
    relative_path: String,
    name: String,
    extension: String,
    codec: String,
    resolution: String,
    duration_secs: f64,
    bitrate_bps: u64,
    size_bytes: u64,
    needs_processing: bool,
    action: String,
    reason: String,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct QueueItem {
    path: String,
    action: String,
    duration_secs: f64,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EncodeSettings {
    encoder: String,
    preset: String,
    cq: u8,
    pixel_format: String,
    replace_if_larger: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscodeRequest {
    files: Vec<QueueItem>,
    settings: EncodeSettings,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct JobEvent {
    kind: String,
    path: Option<String>,
    index: usize,
    total: usize,
    progress: f64,
    message: String,
    original_bytes: Option<u64>,
    output_bytes: Option<u64>,
}

struct ProbeInfo {
    codec: String,
    audio_codec: String,
    width: u64,
    height: u64,
    duration_secs: f64,
    bitrate_bps: u64,
}

fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);
    command
}

fn should_transcode_h264(
    codec: &str,
    bitrate_bps: u64,
    size_bytes: u64,
    bitrate_threshold_bps: u64,
    size_threshold_bytes: u64,
    enabled: bool,
) -> bool {
    enabled
        && codec == "h264"
        && (bitrate_threshold_bps == 0 || bitrate_bps >= bitrate_threshold_bps)
        && (size_threshold_bytes == 0 || size_bytes >= size_threshold_bytes)
}

fn probe_video(path: &Path) -> Result<ProbeInfo, String> {
    let output = hidden_command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration,bit_rate:stream=codec_type,codec_name,width,height,bit_rate",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("无法启动 ffprobe：{error}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("无法解析 ffprobe 输出：{error}"))?;
    let stream = value["streams"]
        .as_array()
        .and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream["codec_type"].as_str() == Some("video"))
        })
        .ok_or_else(|| "文件中没有视频流".to_string())?;
    let audio_codec = value["streams"]
        .as_array()
        .and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream["codec_type"].as_str() == Some("audio"))
        })
        .and_then(|stream| stream["codec_name"].as_str())
        .unwrap_or("")
        .to_string();
    let format = &value["format"];
    let parse_u64 = |value: &serde_json::Value| {
        value
            .as_str()
            .and_then(|text| text.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let duration_secs = format["duration"]
        .as_str()
        .and_then(|text| text.parse::<f64>().ok())
        .unwrap_or(0.0);
    let stream_bitrate = parse_u64(&stream["bit_rate"]);

    Ok(ProbeInfo {
        codec: stream["codec_name"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        audio_codec,
        width: stream["width"].as_u64().unwrap_or(0),
        height: stream["height"].as_u64().unwrap_or(0),
        duration_secs,
        bitrate_bps: if stream_bitrate > 0 {
            stream_bitrate
        } else {
            parse_u64(&format["bit_rate"])
        },
    })
}

#[tauri::command]
fn check_tools() -> Result<String, String> {
    let ffmpeg = hidden_command("ffmpeg")
        .arg("-version")
        .output()
        .map_err(|_| "未找到 FFmpeg，请先安装并将 ffmpeg/ffprobe 加入 PATH".to_string())?;
    let ffprobe = hidden_command("ffprobe")
        .arg("-version")
        .output()
        .map_err(|_| "未找到 ffprobe，请先安装并将 ffmpeg/ffprobe 加入 PATH".to_string())?;
    if !ffmpeg.status.success() || !ffprobe.status.success() {
        return Err("FFmpeg 或 ffprobe 无法正常运行".to_string());
    }
    Ok(String::from_utf8_lossy(&ffmpeg.stdout)
        .lines()
        .next()
        .unwrap_or("FFmpeg 可用")
        .to_string())
}

fn scan_videos_blocking(request: ScanRequest) -> Result<Vec<VideoFile>, String> {
    let root = PathBuf::from(&request.root);
    if !root.is_dir() {
        return Err("请选择有效目录".to_string());
    }
    let unwanted: Vec<String> = request
        .unwanted_extensions
        .iter()
        .map(|item| item.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .collect();
    let bitrate_threshold = (request.min_bitrate_mbps.max(0.0) * 1_000_000.0) as u64;
    let size_threshold = (request.min_file_size_mb.max(0.0) * 1_048_576.0) as u64;
    let mut files = Vec::new();

    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !VIDEO_EXTENSIONS.contains(&extension.as_str())
            || path.to_string_lossy().contains(".m5ga-transcoding.")
        {
            continue;
        }
        let metadata = match fs::metadata(path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let probe = match probe_video(path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let bad_container = unwanted.contains(&extension);
        let is_hevc = matches!(probe.codec.as_str(), "hevc" | "h265");
        let h264_rule = should_transcode_h264(
            &probe.codec,
            probe.bitrate_bps,
            metadata.len(),
            bitrate_threshold,
            size_threshold,
            request.transcode_h264,
        );
        let (action, reason) = if bad_container && is_hevc {
            (
                "remux",
                format!(".{extension} 封装将转换为 .mp4（视频流不重编码）"),
            )
        } else if bad_container {
            ("transcode", format!(".{extension} 封装不在保留列表"))
        } else if h264_rule {
            let reason = if bitrate_threshold == 0 {
                format!(
                    "H.264 命中文件大小阈值（{:.1} MB），码率不限",
                    metadata.len() as f64 / 1_048_576.0
                )
            } else {
                format!(
                    "H.264 码率 {:.1} Mbps 超过阈值",
                    probe.bitrate_bps as f64 / 1_000_000.0
                )
            };
            ("transcode", reason)
        } else {
            ("skip", "无需处理".to_string())
        };

        files.push(VideoFile {
            path: path.to_string_lossy().to_string(),
            relative_path: path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string(),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            extension,
            codec: probe.codec,
            resolution: format!("{}x{}", probe.width, probe.height),
            duration_secs: probe.duration_secs,
            bitrate_bps: probe.bitrate_bps,
            size_bytes: metadata.len(),
            needs_processing: action != "skip",
            action: action.to_string(),
            reason,
        });
    }
    files.sort_by(|a, b| {
        a.relative_path
            .to_lowercase()
            .cmp(&b.relative_path.to_lowercase())
    });
    Ok(files)
}

#[tauri::command]
async fn scan_videos(request: ScanRequest) -> Result<Vec<VideoFile>, String> {
    tauri::async_runtime::spawn_blocking(move || scan_videos_blocking(request))
        .await
        .map_err(|error| format!("扫描任务异常退出：{error}"))?
}

fn emit(app: &AppHandle, event: JobEvent) {
    let _ = app.emit("transcode-event", event);
}

fn valid_log_filename(filename: &str) -> bool {
    filename.ends_with(".log")
        && !filename.is_empty()
        && filename.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

#[tauri::command]
fn save_run_log(filename: String, content: String) -> Result<String, String> {
    if !valid_log_filename(&filename) {
        return Err("日志文件名无效".to_string());
    }
    let appdata = env::var_os("APPDATA").ok_or_else(|| "无法读取 APPDATA 目录".to_string())?;
    let directory = PathBuf::from(appdata).join("m5ga");
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建日志目录：{error}"))?;
    let path = directory.join(filename);
    fs::write(&path, content).map_err(|error| format!("无法保存日志：{error}"))?;
    Ok(path.to_string_lossy().to_string())
}

fn output_path(source: &Path) -> PathBuf {
    if source
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
    {
        source.to_path_buf()
    } else {
        source.with_extension("mp4")
    }
}

fn temp_path(source: &Path) -> PathBuf {
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    source.with_file_name(format!("{stem}.m5ga-transcoding.mp4"))
}

fn replace_source(source: &Path, temp: &Path, final_path: &Path) -> Result<(), String> {
    if final_path != source {
        if final_path.exists() {
            return Err(format!("目标文件已存在：{}", final_path.display()));
        }
        fs::rename(temp, final_path).map_err(|error| format!("无法移动输出文件：{error}"))?;
        if let Err(error) = fs::remove_file(source) {
            let _ = fs::remove_file(final_path);
            return Err(format!("无法删除源文件：{error}"));
        }
        return Ok(());
    }

    let backup = source.with_extension("m5ga-backup");
    if backup.exists() {
        return Err(format!("发现未处理的备份文件：{}", backup.display()));
    }
    fs::rename(source, &backup).map_err(|error| format!("无法备份源文件：{error}"))?;
    if let Err(error) = fs::rename(temp, source) {
        let _ = fs::rename(&backup, source);
        return Err(format!("无法替换源文件：{error}"));
    }
    if let Err(error) = fs::remove_file(&backup) {
        return Err(format!("转码成功，但旧文件备份未能删除：{error}"));
    }
    Ok(())
}

fn run_one(
    app: &AppHandle,
    item: &QueueItem,
    settings: &EncodeSettings,
    cancel: &AtomicBool,
    index: usize,
    total: usize,
) -> Result<(u64, u64, PathBuf), String> {
    let source = PathBuf::from(&item.path);
    if !source.is_file() {
        return Err("源文件不存在".to_string());
    }
    let original_bytes = fs::metadata(&source)
        .map_err(|error| error.to_string())?
        .len();
    let final_path = output_path(&source);
    if final_path != source && final_path.exists() {
        return Err(format!("同名 MP4 已存在：{}", final_path.display()));
    }
    let temp = temp_path(&source);
    if temp.exists() {
        fs::remove_file(&temp).map_err(|error| format!("无法清理上次的临时文件：{error}"))?;
    }

    let mut command = hidden_command("ffmpeg");
    command
        .args(["-hide_banner", "-y", "-i"])
        .arg(&source)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a?",
            "-map_metadata",
            "0",
            "-map_chapters",
            "0",
        ]);
    if item.action == "remux" {
        command.args(["-c:v", "copy"]);
    } else {
        command.args(["-c:v", &settings.encoder]);
        if settings.encoder == "hevc_nvenc" {
            command.args([
                "-preset",
                &settings.preset,
                "-profile:v",
                "main10",
                "-rc",
                "vbr",
                "-cq",
                &settings.cq.to_string(),
                "-pix_fmt",
                &settings.pixel_format,
            ]);
        } else {
            command.args([
                "-preset",
                &settings.preset,
                "-crf",
                &settings.cq.to_string(),
            ]);
            if settings.pixel_format == "p010le" {
                command.args(["-pix_fmt", "yuv420p10le"]);
            } else {
                command.args(["-pix_fmt", &settings.pixel_format]);
            }
        }
    }
    let source_probe = probe_video(&source)?;
    if source_probe.audio_codec.starts_with("pcm_")
        || matches!(source_probe.audio_codec.as_str(), "vorbis" | "opus")
    {
        command.args(["-c:a", "aac", "-b:a", "192k"]);
    } else {
        command.args(["-c:a", "copy"]);
    }
    command
        .args(["-movflags", "+faststart", "-progress", "pipe:1", "-nostats"])
        .arg(&temp)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 FFmpeg：{error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法读取 FFmpeg 进度".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法读取 FFmpeg 日志".to_string())?;
    let stderr_thread = std::thread::spawn(move || {
        BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
            .collect::<Vec<_>>()
            .join("\n")
    });

    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(&temp);
            let _ = stderr_thread.join();
            return Err("任务已取消".to_string());
        }
        if let Some(value) = line.strip_prefix("out_time_us=") {
            if let Ok(micros) = value.parse::<f64>() {
                let progress = if item.duration_secs > 0.0 {
                    (micros / 1_000_000.0 / item.duration_secs * 100.0).clamp(0.0, 99.9)
                } else {
                    0.0
                };
                emit(
                    app,
                    JobEvent {
                        kind: "progress".to_string(),
                        path: Some(item.path.clone()),
                        index,
                        total,
                        progress,
                        message: format!("正在处理 {:.1}%", progress),
                        original_bytes: Some(original_bytes),
                        output_bytes: None,
                    },
                );
            }
        }
    }

    let status = child.wait().map_err(|error| error.to_string())?;
    let stderr_text = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        let _ = fs::remove_file(&temp);
        let detail = stderr_text
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("FFmpeg 执行失败\n{detail}"));
    }
    probe_video(&temp).map_err(|error| {
        let _ = fs::remove_file(&temp);
        format!("输出文件校验失败：{error}")
    })?;
    let output_bytes = fs::metadata(&temp)
        .map_err(|error| error.to_string())?
        .len();
    if item.action != "remux" && !settings.replace_if_larger && output_bytes >= original_bytes {
        let _ = fs::remove_file(&temp);
        return Err(format!(
            "输出未节省空间（原 {:.2} GB，输出 {:.2} GB），已保留源文件",
            original_bytes as f64 / 1_073_741_824.0,
            output_bytes as f64 / 1_073_741_824.0
        ));
    }
    replace_source(&source, &temp, &final_path)?;
    Ok((original_bytes, output_bytes, final_path))
}

fn run_queue(
    app: AppHandle,
    request: TranscodeRequest,
    running: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) {
    let total = request.files.len();
    for (offset, item) in request.files.iter().enumerate() {
        let index = offset + 1;
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        emit(
            &app,
            JobEvent {
                kind: "started".to_string(),
                path: Some(item.path.clone()),
                index,
                total,
                progress: 0.0,
                message: if item.action == "remux" {
                    "正在转换封装"
                } else {
                    "正在转码"
                }
                .to_string(),
                original_bytes: None,
                output_bytes: None,
            },
        );
        match run_one(&app, item, &request.settings, &cancel, index, total) {
            Ok((original, output, final_path)) => emit(
                &app,
                JobEvent {
                    kind: "completed".to_string(),
                    path: Some(item.path.clone()),
                    index,
                    total,
                    progress: 100.0,
                    message: format!("完成：{}", final_path.display()),
                    original_bytes: Some(original),
                    output_bytes: Some(output),
                },
            ),
            Err(error) => {
                let cancelled = cancel.load(Ordering::SeqCst);
                emit(
                    &app,
                    JobEvent {
                        kind: if cancelled { "cancelled" } else { "failed" }.to_string(),
                        path: Some(item.path.clone()),
                        index,
                        total,
                        progress: 0.0,
                        message: error,
                        original_bytes: None,
                        output_bytes: None,
                    },
                );
                if cancelled {
                    break;
                }
            }
        }
    }
    running.store(false, Ordering::SeqCst);
    emit(
        &app,
        JobEvent {
            kind: "batch-finished".to_string(),
            path: None,
            index: total,
            total,
            progress: 100.0,
            message: if cancel.load(Ordering::SeqCst) {
                "队列已取消"
            } else {
                "队列处理完成"
            }
            .to_string(),
            original_bytes: None,
            output_bytes: None,
        },
    );
}

#[tauri::command]
fn start_transcode(
    app: AppHandle,
    state: State<'_, AppState>,
    request: TranscodeRequest,
) -> Result<(), String> {
    if request.files.is_empty() {
        return Err("没有选中待处理文件".to_string());
    }
    if state
        .running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("已有转码任务正在运行".to_string());
    }
    state.cancel.store(false, Ordering::SeqCst);
    let running = state.running.clone();
    let cancel = state.cancel.clone();
    std::thread::spawn(move || run_queue(app, request, running, cancel));
    Ok(())
}

#[tauri::command]
fn cancel_transcode(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            check_tools,
            scan_videos,
            save_run_log,
            start_transcode,
            cancel_transcode
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("m5ga-{name}-{nonce}"));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn replaces_an_mp4_only_after_temp_exists() {
        let directory = test_directory("replace");
        let source = directory.join("video.mp4");
        let temp = directory.join("video.m5ga-transcoding.mp4");
        fs::write(&source, b"original").unwrap();
        fs::write(&temp, b"encoded").unwrap();

        replace_source(&source, &temp, &source).unwrap();

        assert_eq!(fs::read(&source).unwrap(), b"encoded");
        assert!(!temp.exists());
        assert!(!source.with_extension("m5ga-backup").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn preserves_existing_mp4_extension_case_for_in_place_replacement() {
        let source = PathBuf::from("video.MP4");

        assert_eq!(output_path(&source), source);
    }

    #[test]
    fn keeps_source_when_mp4_target_already_exists() {
        let directory = test_directory("collision");
        let source = directory.join("video.mov");
        let temp = directory.join("video.m5ga-transcoding.mp4");
        let target = directory.join("video.mp4");
        fs::write(&source, b"source").unwrap();
        fs::write(&temp, b"encoded").unwrap();
        fs::write(&target, b"existing").unwrap();

        assert!(replace_source(&source, &temp, &target).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(fs::read(&target).unwrap(), b"existing");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn zero_bitrate_threshold_transcodes_every_h264() {
        assert!(should_transcode_h264(
            "h264", 0, 30_000_000, 0, 20_000_000, true
        ));
        assert!(should_transcode_h264(
            "h264", 2_000_000, 30_000_000, 0, 20_000_000, true
        ));
        assert!(!should_transcode_h264(
            "hevc", 20_000_000, 30_000_000, 0, 20_000_000, true
        ));
        assert!(!should_transcode_h264(
            "h264", 20_000_000, 30_000_000, 0, 20_000_000, false
        ));
    }

    #[test]
    fn positive_bitrate_threshold_is_enforced() {
        assert!(!should_transcode_h264(
            "h264", 9_999_999, 30_000_000, 10_000_000, 20_000_000, true
        ));
        assert!(should_transcode_h264(
            "h264", 10_000_000, 30_000_000, 10_000_000, 20_000_000, true
        ));
    }

    #[test]
    fn h264_file_size_threshold_is_enforced() {
        assert!(!should_transcode_h264(
            "h264", 20_000_000, 19_999_999, 0, 20_000_000, true
        ));
        assert!(should_transcode_h264(
            "h264", 20_000_000, 20_000_000, 0, 20_000_000, true
        ));
        assert!(should_transcode_h264("h264", 20_000_000, 1, 0, 0, true));
    }

    #[test]
    fn log_filename_rejects_path_traversal() {
        assert!(valid_log_filename("m5ga-20260722-120000.log"));
        assert!(!valid_log_filename("../m5ga.log"));
        assert!(!valid_log_filename("m5ga.txt"));
        assert!(!valid_log_filename("m5ga 日志.log"));
    }
}
