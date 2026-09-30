export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
}

export interface Persona {
  name: string;
  description: string;
  system_prompt: string;
  greeting: string;
  avatar?: string;
  voice?: string;
  memory_enabled?: boolean;
  tools_enabled?: boolean;
}

export interface AppConfig {
  provider: "deepseek" | "claude";
  api_key: string;
  model: string;
  base_url: string;
  temperature: number;
  voice_enabled: boolean;
  voice: string;
}

export interface ConversationSummary {
  id: number;
  title: string;
  updated_at: number;
}

export interface MemoryItem {
  id: number;
  content: string;
  kind: "chat" | "fact";
  created_at: number;
}
