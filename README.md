# MindPal Chat 🦞

跨平台 AI 伴侣应用 —— 心灵伙伴。一套代码，四端覆盖（Windows / macOS / Linux / Android / iOS）。

## 技术栈

| 层 | 选型 |
|----|------|
| 跨平台框架 | Tauri 2.0 |
| 前端 UI | React 19 + TypeScript + Vite |
| 后端逻辑 | Rust（LLM 流式路由、SQLite 存储） |
| LLM | DeepSeek 主力 + Claude 备用（OpenAI 兼容协议） |
| 记忆 | SQLite 全本地（一期） |

## 当前进度（一期：原生 App 骨架 ✅ 已搭建）

- ✅ Tauri 2.0 项目骨架（React + TS）
- ✅ 基础聊天 UI：会话列表 / 新对话 / 删除 / 流式输出
- ✅ LLM 路由（Rust）：DeepSeek + Claude，SSE 流式逐字输出
- ✅ 本地 SQLite 存储：会话 + 消息（自动用首条消息当标题）
- ✅ 人格系统：`src/personas/*.json`，随包内置（北极熊 / 小鹿），前端可切换
- ✅ 设置面板：provider / API Key / 模型 / Base URL / 温度，存本地 `config.json`
- ✅ 长期记忆：对话归档 + 用户画像提取 + 检索注入，设置可查看/删除
- ✅ 语音朗读（Edge TTS，**默认关闭**，用户可开启；6 个中文音色，CI 真实联网验证协议）
- ✅ 语音输入 STT（**默认关闭**，桌面端；whisper.cpp 本地识别，开启后按需在本机下载模型，下载前二次确认，录音不上传）
- ✅ Android 工程：`src-tauri/gen/android`（CI 的 android job 自动生成维护，包名 com.mindpal.chat）
- ⬜ Android 实际构建（需本机 Android SDK + `npm run tauri android build`）
- ⬜ STT 移动端（whisper.cpp 交叉编译）
- ⬜ 应用图标换 MindPal 品牌（当前是 Tauri 默认图标）

## 多端产物

### 已发布（GitHub Releases）

| 端 | 产物 | 体积 | 备注 |
|----|------|------|------|
| Windows | NSIS 安装包 | ~3.6 MB | exe 本体 ~12 MB；分发需签名 |
| Linux | .deb | ~5.6 MB | 二进制 ~13 MB |
| macOS | .dmg (arm64) | ~4.5 MB | 最低系统版本 10.15 |
| Android | .apk ×4（按 ABI 拆分） | 各 8-12 MB | debug 签名：手机开「允许未知来源」即可安装；正式分发（Play 商店等）需 release 签名 |

### 工程就绪（未出可安装包）

| 端 | 状态 |
|----|------|
| iOS | 工程已生成（`src-tauri/gen/apple`），真机包需 Apple 开发者账号 + 签名 |
| Web | 二期，需另写轻量 API Server（当前后端走 Tauri IPC，浏览器不可用） |

> 构建方式：`npm run tauri build -- --bundles <deb|nsis|dmg>`；CI 见 `.github/workflows/platforms.yml`（手动 + 每日定时）。
> 语音能力默认关闭、模型由用户自行下载，**不占包体**。

## 发版（GitHub Releases）

- **推荐**：push 一个 `v*` tag（如 `git tag v0.2.0 && git push origin v0.2.0`）→ Release workflow 自动构建 Linux(.deb) / Windows(.exe) / macOS(.dmg) / Android(.apk，debug 签名) 并发布到 Releases，产物版本号取 tag，changelog 自动生成
- **手动**：Actions → Release → Run workflow，version 留空则用当前版本号（自动创建 `v<版本>` tag 发版）
- iOS 需签名 + 开发者账号，不在此流程内

详细架构见 [`docs/architecture-overview.md`](docs/architecture-overview.md)，测试规划见 [`docs/testing-plan.md`](docs/testing-plan.md)。

## 开发

前置条件（Linux 需要 webkit2gtk 等系统依赖，见 [Tauri 官方文档](https://tauri.app/start/prerequisites/)）：

- **cmake + C++ 工具链**：语音识别（whisper.cpp）编译需要（Windows 装 cmake + VS Build Tools；Linux `apt install cmake g++`；macOS `brew install cmake`）
- **libclang**（Windows/macOS 需要）：whisper 绑定生成用——Windows 装 LLVM（并设 `LIBCLANG_PATH`），macOS 自带 Xcode 的 libclang；Linux 用 crate 自带预生成绑定，无需额外装
- **macOS**：最低系统版本 10.15（whisper.cpp 用了 `std::filesystem`）

```bash
npm install
npm run tauri dev     # 桌面开发调试
npm run tauri build   # 打包
npm run tauri android init   # Android（需 Android SDK）
```

纯前端构建验证（不需要 Rust 环境）：

```bash
npm run build         # tsc 类型检查 + vite 打包
```

## 目录结构

```
MindPal-Chat/
├── docs/                  # 架构与测试方案
├── src/                   # 前端（React + TS）
│   ├── personas/          # 人格 JSON（新增文件即自动加载）
│   ├── lib/api.ts         # Tauri invoke / 事件封装
│   ├── App.tsx            # 聊天主界面
│   └── types.ts
└── src-tauri/             # Rust 后端
    ├── src/
    │   ├── lib.rs         # 命令注册 / 全局状态
    │   ├── llm.rs         # LLM 路由（DeepSeek / Claude SSE 流式）
    │   ├── db.rs          # SQLite（会话 + 消息）
    │   ├── config.rs      # 本地配置读写
    │   └── types.rs
    └── tauri.conf.json
```

## 数据隐私

- 所有数据（对话、配置）只存本机：`app_data_dir/mindpal.db` + `app_data_dir/config.json`
- 聊天记录不上传任何服务器，API Key 明文存本地配置文件（后续版本做加密）
