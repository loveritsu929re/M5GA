# M5GA 开发指南

## 项目概览

- 这是一个 Windows 本地视频转码工具，使用 Tauri 2、Vue 3、TypeScript 和 Rust。
- 前端入口是 `src/App.vue`，可复用的前端组件位于 `src/components/`。
- Rust 后端入口和 Tauri 命令位于 `src-tauri/src/lib.rs`。
- 视频分析依赖 `ffprobe`，转码和换封装依赖 `ffmpeg`；两者必须能通过 `PATH` 调用。

## 功能约束

- 处理流程必须先生成并校验临时 MP4，再替换源文件，不能直接覆盖源文件。
- 默认不覆盖已有的同名 MP4，也不替换输出更大的转码结果，除非用户显式开启对应选项。
- H.264 规则受码率和文件大小阈值约束；选中的旧编码和不需要的封装规则不受这些阈值约束。
- MP4 输出命令需要包含 `-movflags +faststart`，并在界面命令模板中保持同步。
- 保持时间戳功能需要保留源视频、所选扫描根目录以及相关子目录的修改时间。
- 日志保存位置为 `%APPDATA%\m5ga`，日志文件名必须经过后端校验。

## 代码风格

- 修改文件时使用 `apply_patch`，不要使用脚本重写整个文件。
- 保持现有的简洁 Vue 单文件组件结构和 Rust 标准库优先的实现方式。
- 不要提交构建产物、`node_modules/`、`dist/`、`src-tauri/target/` 或 `.vscode/`。
- 不要提交密钥、个人路径或本地工具配置。
- 不要为了无关问题修改现有行为。

## 依赖镜像

- Cargo 镜像配置位于 `.cargo/config.toml`，使用中科大 crates.io 稀疏索引。
- npm 镜像配置位于 `.npmrc`，使用 npmmirror。

## 验证命令

在提交前按改动范围执行，完整验证命令为：

```powershell
npm run build
cd src-tauri
cargo fmt --check
cargo test --locked
cargo check --locked
```

不要在没有明确请求时创建分支、提交或推送；不要使用破坏性的 Git 操作覆盖用户已有改动。
