// Tauri IPC 的唯一出入口。
//
// 为什么单独一层:界面不该到处 import `@tauri-apps/api` —— 那样既难测,
// 也无法处理"不在 Tauri 里跑"(纯浏览器打开 vite 时)的情况。
// 所有命令在这里声明一次,参数与返回值的类型都来自 types.ts。

import type {
  AppConfig, AppInfo, MemoryView, RunStatus, SessionMeta, SkillView,
  StartAck, TreeEntry, WorkspaceFile, AgentProfile, ConnectionResult,
} from "./types";

/** 事件推送的频道名(与 commands.rs 的 `aiclub://event` 一致)。 */
export const EVENT_CHANNEL = "aiclub://event";

type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
type ListenFn = <T>(ev: string, cb: (e: { payload: T }) => void) => Promise<() => void>;

export interface Host {
  invoke: InvokeFn;
  listen: ListenFn;
}

let host: Host | null = null;

/** 是否运行在 Tauri 壳里。浏览器里预览时为 false。 */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** 惰性装载宿主。浏览器里会抛,由调用方给出可读提示。 */
export async function getHost(): Promise<Host> {
  if (host) return host;
  if (!inTauri()) {
    throw new Error("当前不在 AiClub 桌面壳里 —— 请通过桌面应用打开,而不是直接用浏览器访问。");
  }
  const [{ invoke }, { listen }] = await Promise.all([
    import("@tauri-apps/api/core"),
    import("@tauri-apps/api/event"),
  ]);
  host = { invoke: invoke as InvokeFn, listen: listen as unknown as ListenFn };
  return host;
}

/** 仅供测试:注入一个假的宿主。 */
export function __setHost(h: Host | null): void {
  host = h;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const h = await getHost();
  return h.invoke<T>(cmd, args);
}

// ─────────────── 应用信息 / 配置 ───────────────
//
// 契约:Rust 命令的参数名一律按 camelCase 传入(Tauri v2 会把 `base_url`
// 解析成 JS 侧的 `baseUrl`)。这里的键名必须与 commands.rs 的函数签名一一对应。

export const appInfo = () => call<AppInfo>("app_info");
export const getConfig = () => call<AppConfig>("get_config");
export const saveConfig = (cfg: AppConfig) => call<string>("save_config", { cfg });
export const checkPair = (cfg: AppConfig) => call<string[]>("check_pair", { cfg });
export const touchWorkspace = (cfg: AppConfig) => call<string>("touch_workspace", { cfg });
export const detect = (baseUrl: string, apiKey: string) =>
  call<string[]>("detect", { baseUrl, apiKey });
/** Minimal completion check. Keep the exact camelCase payload for Tauri. */
export const testConnection = (profile: AgentProfile) =>
  call<ConnectionResult>("test_connection", { profile });

/** 选目录。返回 null 表示用户取消。 */
export async function pickDirectory(title = "选择工作区目录"): Promise<string | null> {
  await getHost(); // 借用它的"是不是在 Tauri 里"检查
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: true, multiple: false, title });
  return typeof r === "string" ? r : null;
}

// ─────────────── 运行控制(人类仅有的四个动作) ───────────────

export const startSession = (cfg: AppConfig, resumeOf: string | null = null) =>
  call<StartAck>("start_session", { cfg, resumeOf });
export const status = () => call<RunStatus>("status");
export const pause = (on: boolean) => call<void>("pause", { on });
export const stopSession = () => call<void>("stop_session");

// ─────────────── 观察 ───────────────

/** 返回的是原始 JSONL 行(与 events.jsonl 同构),由 eventlog.ts 的 buildLog 解析。 */
export const eventsOf = (session: string) => call<string[]>("events_of", { session });
export const sessions = () => call<SessionMeta[]>("sessions");
export const deleteSession = (session: string) => call<void>("delete_session", { session });
export const exportSession = (session: string) => call<string>("export_session", { session });
export const tree = (cfg: AppConfig) => call<TreeEntry[]>("tree", { cfg });
export const readWorkspace = (cfg: AppConfig, path: string) =>
  call<WorkspaceFile>("read_workspace", { cfg, path });
export const memories = (cfg: AppConfig) => call<MemoryView[]>("memories", { cfg });
export const skillsList = (cfg: AppConfig) => call<SkillView[]>("skills_list", { cfg });

// ─────────────── 事件订阅 ───────────────

/**
 * 订阅引擎事件。返回取消订阅函数。
 * 回调收到的是**原始行**(JSON 字符串),由上层 reducer 负责解析 —— 这样
 * "解析失败"也能被 reducer 明确处理并计入日志,而不是在桥接层静默吞掉。
 */
export async function onEventLine(cb: (line: string) => void): Promise<() => void> {
  const h = await getHost();
  return h.listen<string>(EVENT_CHANNEL, (e) => cb(e.payload));
}

/** 窗口控制(自绘标题栏要用)。 */
export async function windowControl(action: "minimize" | "toggleMaximize" | "close"): Promise<void> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const w = getCurrentWindow();
  if (action === "minimize") await w.minimize();
  else if (action === "toggleMaximize") await w.toggleMaximize();
  else await w.close();
}

/** Start dragging the frameless window from the custom title bar. */
export async function startDragging(): Promise<void> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().startDragging();
}
