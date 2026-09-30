# MindPal Chat — 架构方案

## 一句话

跨平台 AI 伴侣应用。一套代码，四端覆盖。

## 技术栈

| 层 | 选型 | 理由 |
|----|------|------|
| 跨平台框架 | **Tauri 2.0** | 一人搞定 Win + Linux + Android + iOS，原生性能，打包 ~10MB |
| 前端 UI | **React / Vue** | 四端共用一套 UI 代码 |
| 后端逻辑 | **Rust** | LLM stream 解析、本地向量、语音处理、加密——性能拉满 |
| LLM | **DeepSeek 主力 + Claude 备用** | 中文好 + 便宜 + 效果好 |
| 记忆 | **SQLite + LanceDB / vec0 全本地** | 隐私卖点，用户数据不出设备 |
| 语音 TTS | **Edge TTS** | 免费、效果好、延迟低 |
| 语音 STT | **Whisper.cpp** | 本地推理，私密，免费 |

> 一期落地：TTS 已实现（Rust 直接实现 Edge TTS WebSocket 协议，见 `src-tauri/src/tts.rs`，算法与常量对齐 edge-tts Python 库，2026-10 实测可用）；
> 音频按「音色+文本」哈希缓存到 app_data/tts/，重复文本不重复合成；设置里可开关朗读 + 换音色；STT（Whisper.cpp）二期。
> ⚠️ 隐私：朗读文本会发送给微软 Edge TTS 服务，对话内容本身仍只存本地。
| 人格系统 | **自定义 JSON / 兼容酒馆角色卡** | 自己定义格式，兼容社区资源 |

## 核心架构

```
src/ (共享 UI 代码)
├── 编译到 Tauri → Windows / macOS / Linux 原生桌面
├── 编译到 Tauri Mobile → Android / iOS
└── (可选) 独立部署 → Web 浏览器
```

**应用内部结构：**

```
Tauri 2.0 App
├── WebView UI (React/Vue)
│   ├── 聊天界面
│   ├── 人格管理/切换
│   ├── 设置
│   └── 语音按钮
└── Rust 后端
    ├── LLM Router (DeepSeek / Claude)
    ├── Memory Engine (SQLite + 向量)
    ├── Voice Pipeline (TTS + STT)
    ├── Local DB (对话日志)
    └── Plugin Loader (预留 MCP 接口)
```

## 人格系统

用户可导入/切换人格，格式：

```json
{
  "name": "北极熊",
  "description": "高冷毒舌但可靠",
  "system_prompt": "你是...",
  "greeting": "又来了？",
  "avatar": "avatar.png",
  "voice": "zh-CN-XiaoxiaoNeural",
  "memory_enabled": true,
  "tools_enabled": false
}
```

兼容酒馆角色卡 PNG 导入（读取 EXIF JSON）。

## 记忆系统

| 层级 | 技术 | 用途 | 一期落地 |
|------|------|------|----------|
| 短期 | 滑动窗口 + 摘要 | 当前对话上下文 | ✅（对话窗口直传 LLM） |
| 中期 | SQLite 对话日志 | 历史回溯 | ✅（messages 表） |
| 长期 | LanceDB / vec0 向量索引 | 语义搜索记忆 | ⚠️ 一期用 SQLite 中文拆词 + LIKE 检索（数据量小全扫毫秒级），数据量上来再换真向量 |
| 用户画像 | JSON schema 增量更新 | 记住用户偏好/事实 | ✅ 一期用规则提取（我叫/我喜欢/我住在…）存 facts，二期升级 LLM 抽取 |

记忆写入时机：每次发消息时后端自动归档用户消息（kind=chat）+ 规则提取画像事实（kind=fact，前缀去重防堆积）；
记忆注入：发消息时用本条用户消息检索 top 6 条命中，拼进 system prompt。
隐私：设置面板可查看/删除单条/清空全部记忆，全本地。

## 四端覆盖

| 端 | Tauri 2.0 | 说明 |
|----|:---------:|------|
| Windows | ✅ | .exe / MSI 安装包 |
| macOS | ✅ | .dmg / .app |
| Linux | ✅ | .deb / AppImage |
| Android | ✅ | .apk / .aab（工程已生成：`src-tauri/gen/android`，CI 自动维护；构建需本机 Android SDK） |
| iOS | ✅ | .ipa (需 Mac 签名) |
| Web | ✅ (额外) | 加轻量 API Server，浏览器直接跑 |

## 三阶段路线

### 第一期：原生 App

- Tauri 2.0 项目骨架
- 基础聊天 UI + LLM API 对接
- 本地 SQLite 对话存储
- 1 套默认人格
- Windows / Android 先出

### 第二期：Web 版

- 轻量后端 API 代理
- 前端代码编译为独立 Web 部署
- 服务端数据库选型

### 第三期：MCP Server（可选）

- 让其他 AI 工具能读写 MindPal 的记忆/人格
- 可通过 MCP 客户端与伴侣交互

### 不做

- CLI 版本（对话伴侣在终端里敲命令体验不好）

## 隐私与数据

- 所有用户数据默认存本地
- API Key 本地加密存储
- 不上传聊天记录到任何服务器
- 可选云同步（后期）

## 品牌

- 名称：MindPal Chat
- 中文名：心灵伙伴
- 定位：AI 伴侣跨平台应用