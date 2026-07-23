# M5GA

M5GA 是一个面向 Windows 的本地批量视频整理工具，使用 Vue、TypeScript、Rust 和 Tauri 构建。

它会递归扫描指定目录，通过 FFprobe 识别视频编码、码率、文件大小和封装格式，再调用 FFmpeg 转码或更换封装。处理完成后安全地原位替换源文件，适合在视频归档到 NAS 前统一压缩和整理。

## 主要功能

- 递归扫描目录及其子目录，不跟随目录链接。
- 将符合阈值的 H.264 视频转为 H.265。
- 将 MPEG-4 Part 2、MPEG-2、VC-1 等选中的旧编码无条件转为 H.265。
- 将选中的 MOV、AVI、WMV、FLV 等封装转换为 MP4。
- H.265 视频命中封装规则时只更换封装，不重新编码视频流。
- 支持 NVIDIA NVENC 和 CPU `libx265` 编码。
- 显示完整 FFmpeg 命令模板、逐文件进度和本次节省的总空间。
- 可取消任务、清空队列，并在队列结束后保存运行日志。
- 可保留源文件及其所在文件夹的修改时间，避免 NAS 按目录修改时间排序时顺序被打乱。

## 默认规则

- H.264 转 H.265：开启。
- H.264 最低码率：`0 Mbps`，即不限制码率。
- H.264 文件大小阈值：`20 MB`，小于该大小时跳过。
- 旧编码：默认选择 MPEG-4 Part 2、Microsoft MPEG-4 v3、MPEG-1/2、WMV3 和 VC-1，无视码率及文件大小直接转码。
- 不需要的封装：默认选择 MOV、AVI、WMV 和 FLV，无视 H.264 阈值进行处理。
- 编码参数：`hevc_nvenc`、preset `p5`、CQ `28`、10-bit `p010le`。
- 输出大于或等于源文件时不替换。
- 保持文件及文件夹修改时间：开启。
- 保存运行日志：开启，日志写入 `%APPDATA%\m5ga`。

## 安全替换策略

1. FFmpeg 先在源文件所在目录生成 `<文件名>.m5ga-transcoding.mp4` 临时文件。
2. FFmpeg 成功后，再用 FFprobe 校验临时输出是否可读取。
3. 转码任务默认检查输出是否小于源文件；未节省空间时删除临时文件并保留源文件。
4. 校验通过后才替换源文件。原文件为 MP4 时使用临时备份完成原位替换；其他封装会生成同名 MP4 并删除源文件。
5. 非 MP4 源文件旁已有同名 MP4 时停止处理，不覆盖已有文件。
6. 开启时间戳保留后，恢复最终视频和直接包含该视频的文件夹的修改时间。

任务按队列逐个处理文件，适合单 GPU，也能降低同时读写多个大文件带来的磁盘压力。

## 运行要求

- Windows 10/11。
- Microsoft Edge WebView2 Runtime。Windows 10/11 通常已自带。
- FFmpeg 和 FFprobe，且 `ffmpeg`、`ffprobe` 命令可从 `PATH` 直接运行。
- 使用 NVENC 时，需要支持 HEVC 编码的 NVIDIA 显卡及可用驱动。

在 PowerShell 中可以先检查：

```powershell
ffmpeg -version
ffprobe -version
```

## 开发环境

需要安装：

- Node.js
- Rust MSVC 工具链
- Visual Studio Build Tools（包含“使用 C++ 的桌面开发”）
- FFmpeg

安装依赖并启动开发模式：

```powershell
npm install
npm run tauri dev
```

项目内的 `.cargo/config.toml` 使用中科大 crates.io 稀疏索引，`.npmrc` 使用 npmmirror；这些配置只对当前项目生效。

## 构建单文件 EXE

```powershell
npm run tauri -- build --no-bundle
```

输出文件：

```text
src-tauri\target\release\m5ga.exe
```

`--no-bundle` 只生成应用 EXE，不生成 MSI 或 NSIS 安装包。可以将 `m5ga.exe` 复制到转码机运行，但 FFmpeg/FFprobe 目前没有打包进 EXE，转码机仍需安装 FFmpeg 并配置 `PATH`。

如需生成安装包：

```powershell
npm run tauri build
```

## 验证

```powershell
npm run build
cd src-tauri
cargo fmt --check
cargo test --locked
cargo check --locked
```

## NAS 注意事项

- SMB/NAS 共享必须允许当前用户创建、重命名、删除文件和修改文件属性。
- 若 NAS 不允许修改目录属性，视频仍可能完成替换，但应用会报告无法恢复文件夹修改时间。
- 建议先在少量测试视频或备份目录中验证编码参数、权限和时间戳行为，再处理正式归档目录。
