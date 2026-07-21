# M5GA

Windows 本地批量视频整理工具。递归扫描目录，通过 FFprobe 判断编码、码率和封装，使用 FFmpeg 将高码率 H.264 转为 H.265，或把不需要的容器转换为 MP4。

## 开发运行

1. 安装 Node.js、Rust、FFmpeg，确保 `ffmpeg` 和 `ffprobe` 在 PATH 中。
2. 执行 `npm install`。
3. 执行 `npm run tauri dev`。

项目内已配置国内依赖镜像：Cargo 使用中科大 crates.io 稀疏索引，npm 包使用 npmmirror。配置只对当前项目生效。中科大 Node 镜像提供的是 Node.js 安装包，不是 npm 包仓库；本项目已经有可用的 Node.js，因此无需在 npm 配置中使用该地址。

## 安全策略

- 输出先写入源文件同目录的 `.m5ga-transcoding.mp4` 临时文件。
- FFmpeg 成功且 FFprobe 能读取输出后才替换源文件。
- 默认输出不节省空间时不替换。
- MOV/AVI 等转成 MP4 时，如同名 MP4 已存在则不处理该文件。
- 每次只处理一个文件，适合单 GPU 并降低磁盘压力。
