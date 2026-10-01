import { useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppConfig, ChatMessage, ConversationSummary, MemoryItem, Persona, SttStatus } from "./types";
import * as api from "./lib/api";
import { Recorder, pcmToBase64 } from "./lib/recorder";
import "./App.css";

const PROVIDER_DEFAULTS: Record<string, { model: string; base_url: string }> = {
  deepseek: { model: "deepseek-chat", base_url: "https://api.deepseek.com/v1" },
  claude: { model: "claude-sonnet-4-20250514", base_url: "https://api.anthropic.com/v1" },
};

/** Edge TTS 中文音色 */
const VOICES: { id: string; name: string }[] = [
  { id: "zh-CN-XiaoxiaoNeural", name: "晓晓 · 女（温暖）" },
  { id: "zh-CN-XiaoyiNeural", name: "晓伊 · 女（活泼）" },
  { id: "zh-CN-YunxiNeural", name: "云希 · 男（阳光）" },
  { id: "zh-CN-YunjianNeural", name: "云健 · 男（沉稳）" },
  { id: "zh-CN-YunyangNeural", name: "云扬 · 男（新闻）" },
  { id: "zh-CN-YunxiaNeural", name: "云夏 · 男（少年）" },
];

/** Whisper 本地识别模型 */
const STT_MODELS: { id: string; name: string }[] = [
  { id: "tiny", name: "tiny · 快（约 32MB）" },
  { id: "base", name: "base · 均衡（约 60MB）" },
  { id: "small", name: "small · 更准（约 190MB）" },
];

let currentAudio: HTMLAudioElement | null = null;

/** 合成并播放（新播放会打断旧的） */
async function speak(text: string, voice: string, enabled: boolean) {
  if (!enabled || !text.trim()) return;
  try {
    const path = await api.ttsSpeak(text, voice);
    const audio = new Audio(convertFileSrc(path));
    currentAudio?.pause();
    currentAudio = audio;
    audio.play().catch(() => {});
  } catch {
    // 语音失败不影响聊天
  }
}

function fmtTime(ts: number): string {
  return new Date(ts * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function App() {
  const [personas] = useState<Persona[]>(api.personas);
  const [personaIdx, setPersonaIdx] = useState(0);
  const persona = personas[personaIdx] ?? personas[0];

  const [conversations, setConversations] = useState<ConversationSummary[]>([]);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [streaming, setStreaming] = useState(false);
  const [streamText, setStreamText] = useState("");
  const [input, setInput] = useState("");
  const [error, setError] = useState<string | null>(null);

  const [showSettings, setShowSettings] = useState(false);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [draft, setDraft] = useState<AppConfig | null>(null);
  const [memories, setMemories] = useState<MemoryItem[]>([]);
  const [stt, setStt] = useState<SttStatus | null>(null);
  const [sttProgress, setSttProgress] = useState<{ downloaded: number; total: number } | null>(null);
  const [downloadingModel, setDownloadingModel] = useState(false);
  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const recRef = useRef<Recorder | null>(null);

  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // 打开设置时加载记忆列表 + 语音输入状态
  useEffect(() => {
    if (showSettings) {
      api.listMemories().then(setMemories).catch((e) => setError(String(e)));
      api.sttStatus().then(setStt).catch(() => {});
    }
  }, [showSettings]);

  // 初始化时也取一次：决定麦克风按钮是否显示
  useEffect(() => {
    api.sttStatus().then(setStt).catch(() => {});
    let unlisten: (() => void) | undefined;
    listen<{ downloaded: number; total: number }>("stt-download", (e) => setSttProgress(e.payload)).then(
      (fn) => (unlisten = fn),
    );
    return () => unlisten?.();
  }, []);

  const refreshConversations = () => {
    api.listConversations().then(setConversations).catch((e) => setError(String(e)));
  };

  useEffect(() => {
    api
      .getConfig()
      .then((c) => {
        setConfig(c);
        setDraft(c);
      })
      .catch((e) => setError(String(e)));
    refreshConversations();
  }, []);

  // 流式输出时自动滚到底
  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [messages, streamText]);

  const openConversation = async (id: number) => {
    if (streaming) return;
    try {
      const msgs = await api.loadConversation(id);
      setMessages(msgs);
      setActiveId(id);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const newChat = () => {
    if (streaming) return;
    setActiveId(null);
    setMessages([]);
    setError(null);
    inputRef.current?.focus();
  };

  const switchPersona = (idx: number) => {
    setPersonaIdx(idx);
    newChat();
  };

  const removeConversation = async (id: number) => {
    try {
      await api.deleteConversation(id);
      if (activeId === id) newChat();
      refreshConversations();
    } catch (e) {
      setError(String(e));
    }
  };

  const send = async () => {
    const text = input.trim();
    if (!text || streaming || !persona) return;
    if (!config?.api_key) {
      setShowSettings(true);
      setError("先到设置里填写 API Key");
      return;
    }
    const userMsg: ChatMessage = { role: "user", content: text };
    setInput("");
    setError(null);
    setStreaming(true);
    setStreamText("");

    let convId = activeId;
    try {
      if (convId === null) {
        convId = await api.newConversation();
        setActiveId(convId);
      }
      await api.saveMessage(convId, "user", text);
      const ctx = [...messages, userMsg];
      setMessages(ctx);
      const full = await api.chatStream(
        persona.system_prompt,
        ctx,
        (t) => setStreamText((prev) => prev + t),
        persona.memory_enabled ?? true,
      );
      await api.saveMessage(convId, "assistant", full);
      setMessages([...ctx, { role: "assistant", content: full }]);
      if (config) speak(full, config.voice || persona.voice || "zh-CN-XiaoxiaoNeural", config.voice_enabled);
      refreshConversations();
    } catch (e) {
      setError(String(e));
    } finally {
      setStreaming(false);
      setStreamText("");
    }
  };

  const removeMemory = async (id: number) => {
    try {
      await api.deleteMemory(id);
      setMemories((prev) => prev.filter((m) => m.id !== id));
    } catch (e) {
      setError(String(e));
    }
  };

  const clearAllMemories = async () => {
    if (!window.confirm("确定清空全部记忆？此操作不可恢复。")) return;
    try {
      await api.clearMemories();
      setMemories([]);
    } catch (e) {
      setError(String(e));
    }
  };

  const downloadSttModel = async () => {
    if (!draft) return;
    setDownloadingModel(true);
    setSttProgress(null);
    try {
      await api.sttDownloadModel(draft.stt_model);
      // 后端识别读 config.stt_model，下载后立即落盘
      await api.saveConfig(draft);
      setConfig(draft);
      setStt(await api.sttStatus());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setDownloadingModel(false);
      setSttProgress(null);
    }
  };

  /** 麦克风：点一次开始录，再点一次停止并识别 */
  const toggleRecord = async () => {
    if (transcribing) return;
    if (!recording) {
      if (!config?.stt_enabled) {
        setShowSettings(true);
        setError("语音输入未开启");
        return;
      }
      if (stt && !stt.model_present) {
        setShowSettings(true);
        setError(`语音模型未下载（${stt.model}），请先下载`);
        return;
      }
      try {
        const rec = new Recorder();
        await rec.start();
        recRef.current = rec;
        setRecording(true);
        setError(null);
      } catch (e) {
        setError(`麦克风打开失败：${String(e)}`);
      }
      return;
    }
    setRecording(false);
    setTranscribing(true);
    try {
      const rec = recRef.current;
      recRef.current = null;
      if (!rec) return;
      const { pcm, sampleRate } = await rec.stop();
      if (pcm.length < sampleRate * 0.3) {
        setError("录音太短，请再说一次");
        return;
      }
      const text = await api.sttTranscribe(
        pcmToBase64(pcm),
        sampleRate,
        config?.stt_language || "zh",
      );
      if (text) {
        setInput((prev) => (prev ? `${prev} ${text}` : text));
        setError(null);
      } else {
        setError("没听清，再试一次");
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setTranscribing(false);
    }
  };

  const saveSettings = async () => {
    if (!draft) return;
    try {
      await api.saveConfig(draft);
      setConfig(draft);
      setShowSettings(false);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const changeProvider = (provider: string) => {
    if (!draft) return;
    const def = PROVIDER_DEFAULTS[provider] ?? PROVIDER_DEFAULTS.deepseek;
    setDraft({ ...draft, provider: provider as AppConfig["provider"], ...def });
  };

  const showGreeting = activeId === null && messages.length === 0 && !streaming;

  return (
    <div className="app">
      {/* ---------- 侧栏 ---------- */}
      <aside className="sidebar">
        <div className="sidebar-head">
          <span className="brand">🦞 MindPal Chat</span>
          <button className="icon-btn" title="设置" onClick={() => setShowSettings(true)}>
            ⚙️
          </button>
        </div>

        <label className="persona-label">当前人格</label>
        <select
          className="persona-select"
          value={personaIdx}
          onChange={(e) => switchPersona(Number(e.target.value))}
        >
          {personas.map((p, i) => (
            <option key={p.name} value={i}>
              {p.name} — {p.description}
            </option>
          ))}
        </select>

        <button className="new-btn" onClick={newChat}>
          ＋ 新对话
        </button>

        <div className="conv-list">
          {conversations.map((c) => (
            <div
              key={c.id}
              className={`conv-item ${c.id === activeId ? "active" : ""}`}
              onClick={() => openConversation(c.id)}
            >
              <div className="conv-title">{c.title}</div>
              <div className="conv-meta">
                <span>{fmtTime(c.updated_at)}</span>
                <button
                  className="conv-del"
                  title="删除会话"
                  onClick={(e) => {
                    e.stopPropagation();
                    removeConversation(c.id);
                  }}
                >
                  ✕
                </button>
              </div>
            </div>
          ))}
          {conversations.length === 0 && <div className="conv-empty">还没有对话</div>}
        </div>
      </aside>

      {/* ---------- 聊天区 ---------- */}
      <main className="chat">
        <div className="chat-head">
          <span className="chat-persona">
            {persona ? `${persona.name} · ${persona.description}` : "加载中…"}
          </span>
          {error && <span className="error-badge">⚠ {error}</span>}
        </div>

        <div className="msg-list" ref={listRef}>
          {showGreeting && persona && (
            <div className="msg assistant">
              <div className="bubble">{persona.greeting}</div>
            </div>
          )}
          {messages.map((m, i) => (
            <div key={i} className={`msg ${m.role}`}>
              <div className="bubble">{m.content}</div>
            </div>
          ))}
          {streaming && (
            <div className="msg assistant">
              <div className="bubble streaming">
                {streamText}
                <span className="cursor" />
              </div>
            </div>
          )}
        </div>

        <div className="composer">
          {stt?.supported && (
            <button
              className={`mic-btn ${recording ? "recording" : ""}`}
              title={recording ? "停止并识别" : "点击说话"}
              onClick={toggleRecord}
              disabled={transcribing}
            >
              {transcribing ? "⏳" : recording ? "⏹" : "🎤"}
            </button>
          )}
          <textarea
            ref={inputRef}
            value={input}
            placeholder="和你的伙伴聊聊…（Enter 发送，Shift+Enter 换行）"
            rows={1}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
          />
          <button className="send-btn" onClick={send} disabled={streaming || !input.trim()}>
            {streaming ? "…" : "发送"}
          </button>
        </div>
      </main>

      {/* ---------- 设置弹窗 ---------- */}
      {showSettings && draft && (
        <div className="modal-mask" onClick={() => setShowSettings(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h2>设置</h2>
            <label>
              LLM 服务商
              <select
                value={draft.provider}
                onChange={(e) => changeProvider(e.target.value)}
              >
                <option value="deepseek">DeepSeek</option>
                <option value="claude">Claude (Anthropic)</option>
              </select>
            </label>
            <label>
              API Key
              <input
                type="password"
                value={draft.api_key}
                placeholder="sk-..."
                onChange={(e) => setDraft({ ...draft, api_key: e.target.value })}
              />
            </label>
            <label>
              模型
              <input
                value={draft.model}
                onChange={(e) => setDraft({ ...draft, model: e.target.value })}
              />
            </label>
            <label>
              Base URL
              <input
                value={draft.base_url}
                onChange={(e) => setDraft({ ...draft, base_url: e.target.value })}
              />
            </label>
            <label className="row">
              <span>温度 {draft.temperature.toFixed(1)}</span>
              <input
                type="range"
                min={0}
                max={2}
                step={0.1}
                value={draft.temperature}
                onChange={(e) => setDraft({ ...draft, temperature: Number(e.target.value) })}
              />
            </label>
            <div className="voice-section">
              <h3>语音输入（本地识别）</h3>
              <label className="row">
                <span>启用语音输入</span>
                <input
                  type="checkbox"
                  checked={draft.stt_enabled}
                  onChange={(e) => setDraft({ ...draft, stt_enabled: e.target.checked })}
                />
              </label>
              <label>
                识别模型
                <select
                  value={draft.stt_model}
                  onChange={(e) => setDraft({ ...draft, stt_model: e.target.value })}
                >
                  {STT_MODELS.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name}
                    </option>
                  ))}
                </select>
              </label>
              <div className="stt-status">
                {stt === null
                  ? "…"
                  : !stt.supported
                    ? "当前平台暂不支持本地识别（桌面端可用）"
                    : stt.model_present
                      ? `✅ 模型已就绪：${stt.model}`
                      : `⚠️ 模型未下载：${stt.model}（约 ${stt.size_mb}MB）`}
              </div>
              <div className="modal-actions">
                <button
                  className="ghost"
                  type="button"
                  onClick={downloadSttModel}
                  disabled={downloadingModel || !stt?.supported}
                >
                  {downloadingModel
                    ? sttProgress && sttProgress.total
                      ? `下载中 ${Math.floor((sttProgress.downloaded / sttProgress.total) * 100)}%`
                      : "下载中…"
                    : "下载模型"}
                </button>
              </div>
              <p className="hint">
                识别全在本机完成（whisper.cpp），录音不上传；首次需下载模型（来自 HuggingFace）。
              </p>
            </div>
            <div className="voice-section">
              <h3>语音</h3>
              <label className="row">
                <span>朗读回复</span>
                <input
                  type="checkbox"
                  checked={draft.voice_enabled}
                  onChange={(e) => setDraft({ ...draft, voice_enabled: e.target.checked })}
                />
              </label>
              <label>
                音色
                <select
                  value={draft.voice}
                  onChange={(e) => setDraft({ ...draft, voice: e.target.value })}
                >
                  {VOICES.map((v) => (
                    <option key={v.id} value={v.id}>
                      {v.name}
                    </option>
                  ))}
                </select>
              </label>
              <div className="modal-actions">
                <button
                  className="ghost"
                  type="button"
                  onClick={() => speak("你好，我是你的心灵伙伴，很高兴认识你。", draft.voice, true)}
                >
                  试听音色
                </button>
              </div>
              <p className="hint">语音由微软 Edge TTS 在线合成，朗读文本会发送给微软服务，对话内容本身仍只存本地。</p>
            </div>
            <div className="memory-section">
              <h3>记忆（全本地存储）</h3>
              <div className="mem-list">
                {memories.map((m) => (
                  <div className="mem-item" key={m.id}>
                    <span className={`mem-kind ${m.kind}`}>{m.kind === "fact" ? "画像" : "对话"}</span>
                    <span className="mem-text">{m.content}</span>
                    <button
                      className="mem-del"
                      title="删除这条记忆"
                      onClick={() => removeMemory(m.id)}
                    >
                      ✕
                    </button>
                  </div>
                ))}
                {memories.length === 0 && (
                  <p className="hint">还没有记忆。聊天后你说过的话会被自动记住，用于后续对话。</p>
                )}
              </div>
              <div className="modal-actions">
                <button className="ghost danger" onClick={clearAllMemories}>
                  清空全部记忆
                </button>
              </div>
            </div>
            <p className="hint">配置只保存在本机（app data 目录），不会上传。</p>
            <div className="modal-actions">
              <button className="ghost" onClick={() => setShowSettings(false)}>
                取消
              </button>
              <button className="primary" onClick={saveSettings}>
                保存
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
