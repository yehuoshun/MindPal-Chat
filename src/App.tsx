import { useEffect, useRef, useState } from "react";
import type { AppConfig, ChatMessage, ConversationSummary, Persona } from "./types";
import * as api from "./lib/api";
import "./App.css";

const PROVIDER_DEFAULTS: Record<string, { model: string; base_url: string }> = {
  deepseek: { model: "deepseek-chat", base_url: "https://api.deepseek.com/v1" },
  claude: { model: "claude-sonnet-4-20250514", base_url: "https://api.anthropic.com/v1" },
};

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

  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

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
      const full = await api.chatStream(persona.system_prompt, ctx, (t) =>
        setStreamText((prev) => prev + t),
      );
      await api.saveMessage(convId, "assistant", full);
      setMessages([...ctx, { role: "assistant", content: full }]);
      refreshConversations();
    } catch (e) {
      setError(String(e));
    } finally {
      setStreaming(false);
      setStreamText("");
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
