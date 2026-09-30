import type { Persona, ChatMessage, AppConfig, ConversationSummary, MemoryItem } from "../types";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// ---------- 配置 ----------
export const getConfig = () => invoke<AppConfig>("get_config");

export const saveConfig = (config: AppConfig) => invoke<void>("save_config", { config });

// ---------- 会话 ----------
export const newConversation = (title?: string) => invoke<number>("new_conversation", { title });

export const listConversations = () => invoke<ConversationSummary[]>("list_conversations");

export const loadConversation = (id: number) => invoke<ChatMessage[]>("load_conversation", { id });

export const deleteConversation = (id: number) => invoke<void>("delete_conversation", { id });

export const renameConversation = (id: number, title: string) =>
  invoke<void>("rename_conversation", { id, title });

export const saveMessage = (conversationId: number, role: string, content: string) =>
  invoke<number>("save_message", { conversationId, role, content });

// ---------- 记忆 ----------
export const listMemories = () => invoke<MemoryItem[]>("list_memories");

export const deleteMemory = (id: number) => invoke<void>("delete_memory", { id });

export const clearMemories = () => invoke<void>("clear_memories");

// ---------- 聊天（流式） ----------
/**
 * 发起流式聊天：订阅 llm-token 事件逐段回调，Promise resolve 时返回完整文本
 */
export async function chatStream(
  systemPrompt: string,
  messages: ChatMessage[],
  onToken: (token: string) => void,
  memoryEnabled: boolean,
): Promise<string> {
  let unlisten: (() => void) | null = null;
  try {
    const un = await listen<string>("llm-token", (e) => onToken(e.payload));
    unlisten = un;
    return await invoke<string>("chat_stream", { systemPrompt, messages, memoryEnabled });
  } finally {
    unlisten?.();
  }
}

// ---------- 人格 ----------
/** 通过 Vite glob 加载 src/personas/ 下所有人格 JSON（新增文件即生效） */
const personaModules = import.meta.glob<{ default: Persona }>("./personas/*.json", {
  eager: true,
});

export const personas: Persona[] = Object.values(personaModules).map((m) => m.default);
