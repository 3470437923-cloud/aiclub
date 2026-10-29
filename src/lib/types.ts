// 与 Rust 侧 `events.rs` / `config.rs` / `engine.rs` 一一对应的类型。
// 改这里之前先改那边 —— 前端只是投影,不是事实源。

/** 13 种事件(serde tag = "type", snake_case)。 */
export type Event =
  | { type: "session_open"; session_id: string; agents: AgentBrief[]; mode: string; opening_note: string; ts: string }
  | { type: "turn_begin"; turn: number; agent: string; ts: string }
  // seq = 轮内发言序号。一轮里可以有多条发言(先说一句、调工具、再说一句),
  // 光靠 turn+agent 分不开,会把后一条覆盖掉前一条。缺省 0 以兼容旧日志。
  | { type: "say_delta"; turn: number; agent: string; seq?: number; text: string }
  | { type: "say_end"; turn: number; agent: string; seq?: number; text: string; input: number; output: number; cached?: number; ms: number; finish_reason?: string }
  | { type: "tool_call"; turn: number; agent: string; call_id: string; name: string; args: unknown; ok: boolean; preview: string; result_full: string; ms: number }
  | { type: "pass"; turn: number; agent: string }
  | { type: "violation"; turn: number; agent: string; kind: string; path: string; detail: string }
  | { type: "system_note"; turn: number; text: string }
  | { type: "turn_error"; turn: number; agent: string; attempt: number; error: string; retryable: boolean }
  | { type: "agent_unreachable"; agent: string; consecutive_failures: number }
  | { type: "context_snapshot"; turn: number; agent: string; bytes: number; compressed: boolean }
  | { type: "compress"; turn: number; agent: string; via_model: string; tokens_in: number; tokens_out: number }
  | { type: "session_end"; reason: string; turns: number; by_agent: unknown; ms_total: number; ts: string };

export type EventType = Event["type"];

export interface AgentBrief {
  id: string;
  name: string;
  company: string;
  model: string;
  host: string;
}

export interface UsagePair { input: number; output: number; cached?: number }

export interface AgentView extends AgentBrief {
  tokens: number;
  failures: number;
  active: boolean;
}

export interface RunStatus {
  running: boolean;
  paused: boolean;
  stop_requested: boolean;
  turn: number;
  max_turns: number;
  mode: string;
  session: string;
  usage_a: UsagePair;
  usage_b: UsagePair;
  violations: number;
  tool_calls: number;
  end_reason: string | null;
  end_text: string | null;
  ms_total: number;
  agents: AgentView[];
  current: string | null;
  shell_enabled: boolean;
  workspace: string;
}

export interface AgentProfile {
  id: string;
  name: string;
  company: string;
  base_url: string;
  api_key: string;
  model: string;
  temperament: string;
  temperature: number;
  max_output_tokens: number;
  context_window: number;
}

export interface Globals {
  theme: string;
  pause_between_turns_ms: number;
  max_turns: number;
  cold_field_rounds: number;
  budget_tokens: number;
  turn_timeout_ms: number;
  turn_retries: number;
}

export interface ShellConfig {
  enabled: boolean;
  timeout_ms: number;
  output_bytes: number;
  env_passthrough: string[];
}

export interface WorkspaceConfig { root: string; mode: string }

export interface AppConfig {
  version: number;
  workspace: WorkspaceConfig;
  globals: Globals;
  agents: AgentProfile[];
  /** All saved character profiles. agents is the selected session subset. */
  characters?: AgentProfile[];
  shell: ShellConfig;
  persona_template: string;
}

export interface AppInfo {
  version: string;
  os: string;
  config_path: string;
  workspace_default: string;
  sessions_dir: string;
  persona_default: string;
}

/** 配置违规的人话文案(Rust `check_pair` 返回的 `Vec<String>`)。 */
export type ViolationView = string;

export interface StartAck {
  session: string;
  workspace: string;
  host_a: string;
  host_b: string;
  mode: string;
  same_origin: boolean;
}

export interface SessionMeta {
  id: string;
  events: number;
  turns: number;
  end_reason: string | null;
  a: string;
  b: string;
  host_a: string;
  host_b: string;
  mode: string;
  tokens: number;
  ms_total: number;
  unix: number;
  participants?: AgentBrief[];
}

export interface ConnectionResult { model: string; ms: number; text: string }

export interface TreeEntry { path: string; dir: boolean; bytes: number; zone: string }

export interface WorkspaceFile { path: string; bytes: number; binary: boolean; text: string; truncated: boolean }

export interface MemoryNote { ts: string; session: string; turn: number; text: string; tags: string[] }

export interface MemoryView { agent: string; name: string; notes: MemoryNote[] }

export interface SkillView { slug: string; name: string; description: string }
