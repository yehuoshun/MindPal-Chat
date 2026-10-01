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

详细架构见 [`docs/architecture-overview.md`](docs/architecture-overview.md)，测试规划见 [`docs/testing-plan.md`](docs/testing-plan.md)。

## 开发

前置条件（Linux 需要 webkit2gtk 等系统依赖，见 [Tauri 官方文档](https://tauri.app/start/prerequisites/)）：

- **cmake + C++ 工具链**：语音识别（whisper.cpp）编译需要（Windows 装 cmake + VS Build Tools；Linux `apt install cmake g++`；macOS `brew install cmake`）

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
