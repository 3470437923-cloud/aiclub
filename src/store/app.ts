// 全局状态。刻意保持**扁平**:只有"界面需要什么",没有业务规则
// (业务规则都在 Rust 与 lib/eventlog.ts 里)。

import { create } from "zustand";
import * as api from "../lib/bridge";
import { applyLine, buildLog, emptyLog, type LogState } from "../lib/eventlog";
import type { AppConfig, AppInfo, MemoryView, RunStatus, SessionMeta, SkillView, TreeEntry, ViolationView, WorkspaceFile } from "../lib/types";

export type View = "stage" | "files" | "notes" | "runs" | "setup" | "about";

export interface Toast { id: number; text: string; kind: "info" | "error" }

interface AppState {
  // 启动
  booted: boolean;
  bootError: string | null;
  info: AppInfo | null;
  config: AppConfig | null;

  // 视图
  view: View;
  sidebarOpen: boolean;

  // 运行
  st: RunStatus | null;
  log: LogState;
  live: boolean;

  // 观察数据
  violations: ViolationView[];
  treeEntries: TreeEntry[];
  currentFile: WorkspaceFile | null;
  currentPath: string | null;
  notes: MemoryView[];
  runs: SessionMeta[];
  skills: SkillView[];

  /**
   * 设置页草稿。
   *
   * 为什么必须放在 store 里而不是 Setup 组件的 useState:
   * App.tsx 用 `{view === "setup" && <Setup />}` 分发,切到别的页 Setup 就被卸载,
   * 局部 state 连同"刚探测出来的模型列表"一起消失 —— 用户探测完去看一眼台面
   * 再回来,下拉框没了、没保存的输入也回滚了。这是实测复现过的问题。
   *
   * 键按**下标**而不是 agent.id:两个 agent 的 id 相同(手写 config.toml 时
   * serde default 会给空 id)时,按 id 存会让两侧互相覆盖。
   */
  setupDraft: AppConfig | null;
  modelOptions: Record<string, string[]>;
  connectionResults: Record<string, { ok: boolean; model?: string; ms?: number; text: string }>;
  setSetupDraft: (c: AppConfig | null) => void;
  setModelOptions: (index: number, models: string[]) => void;
  setConnectionResult: (id: string, result: { ok: boolean; model?: string; ms?: number; text: string } | null) => void;

  // 回放
  replaying: string | null;

  toasts: Toast[];

  // ── 动作 ──
  boot: () => Promise<void>;
  reloadConfig: () => Promise<void>;
  setConfig: (c: AppConfig) => void;
  revalidate: () => Promise<void>;
  setView: (v: View) => void;
  toggleSidebar: () => void;

  start: (resume: boolean, sessionId?: string) => Promise<void>;
  togglePause: () => Promise<void>;
  stop: () => Promise<void>;
  refreshStatus: () => Promise<void>;
  pushEvent: (line: string) => void;
  setLive: (v: boolean) => void;

  loadTree: () => Promise<void>;
  openFile: (path: string) => Promise<void>;
  closeFile: () => void;
  loadNotes: () => Promise<void>;
  loadRuns: () => Promise<void>;
  loadSkills: () => Promise<void>;
  replay: (session: string | null) => Promise<void>;
  removeRun: (session: string) => Promise<void>;
  exportRun: (session: string) => Promise<void>;

  toast: (text: string, kind?: "info" | "error") => void;
  dropToast: (id: number) => void;
}

let toastSeq = 1;

export const useApp = create<AppState>((set, get) => ({
  booted: false,
  bootError: null,
  info: null,
  config: null,
  view: "stage",
  sidebarOpen: true,
  st: null,
  log: emptyLog(),
  live: true,
  violations: [],
  treeEntries: [],
  currentFile: null,
  currentPath: null,
  notes: [],
  runs: [],
  skills: [],
  replaying: null,
  toasts: [],
  setupDraft: null,
  modelOptions: {},
  connectionResults: {},

  setSetupDraft(c) {
    set({ setupDraft: c });
  },

  setModelOptions(index, models) {
    set({ modelOptions: { ...get().modelOptions, [String(index)]: models } });
  },

  setConnectionResult(id, result) {
    const next = { ...get().connectionResults };
    if (result) next[id] = result; else delete next[id];
    set({ connectionResults: next });
  },

  async boot() {
    // 关键路径:拿不到 info/config 就真的起不来,该进错误页。
    try {
      const [info, cfg] = await Promise.all([api.appInfo(), api.getConfig()]);
      const migrated = { ...cfg, characters: cfg.characters?.length ? cfg.characters : cfg.agents };
      set({ info, config: migrated, booted: true, bootError: null });
    } catch (e) {
      set({ bootError: e instanceof Error ? e.message : String(e) });
      return;
    }

    // 非关键路径:补状态、补历史。
    //
    // 这些以前和上面包在同一个 try 里,于是任何一步失败都会把整个应用
    // 换成"AiClub 起不来"错误页 —— 哪怕用户其实能正常用。
    // 最现实的一条:st.session 存在,但录像文件被删/损坏/没权限读,
    // 用户就再也进不去设置页去修它了。辅助数据失败只该 toast。
    try {
      const st = await api.status();
      set({ st });
      // 若引擎已经在跑(比如重新打开窗口),把历史事件补回来
      if (st.session) {
        try {
          const lines = await api.eventsOf(st.session);
          set({ log: buildLog(lines) });
        } catch (e) {
          get().toast(
            `补历史失败(不影响继续使用):${e instanceof Error ? e.message : String(e)}`,
            "error",
          );
        }
      }
    } catch (e) {
      get().toast(`读状态失败:${e instanceof Error ? e.message : String(e)}`, "error");
    }

    // 这四步各自内部已 catch,失败只会 toast,不会抛出。
    await get().loadRuns();
    await get().loadSkills();
    await get().loadTree();
    await get().loadNotes();
  },

  async reloadConfig() {
    try {
      const cfg = await api.getConfig();
      const migrated = { ...cfg, characters: cfg.characters?.length ? cfg.characters : cfg.agents };
      set({ config: migrated, setupDraft: JSON.parse(JSON.stringify(migrated)), connectionResults: {} });
      await get().revalidate();
    } catch (e) {
      get().toast(String(e), "error");
    }
  },

  setConfig(c) {
    set({ config: c });
  },

  async revalidate() {
    const cfg = get().config;
    if (!cfg) return;
    try {
      set({ violations: await api.checkPair(cfg) });
    } catch (e) {
      get().toast(String(e), "error");
    }
  },

  setView(v) { set({ view: v }); },
  toggleSidebar() { set({ sidebarOpen: !get().sidebarOpen }); },

  async start(resume, sessionId) {
    const cfg = get().setupDraft ?? get().config;
    if (!cfg) {
      get().toast("配置还没加载,稍等再试", "error");
      return;
    }
    if (!resume && cfg.agents.length < 2) {
      get().toast("至少选择两位参与者", "error");
      set({ view: "setup" });
      return;
    }
    try {
      // Persist the selected participant snapshot before launch.
      await api.saveConfig(cfg);
      set({ config: cfg });
      // resume 语义:续跑 = 沿用当前会话 id;全新一局 = null
      const resumeOf = resume ? (sessionId ?? get().st?.session ?? null) : null;
      if (resume && !resumeOf) throw new Error("请选择要续跑的录像");
      // Load the target history before starting so early live events cannot mix with another session.
      const log = resumeOf ? buildLog(await api.eventsOf(resumeOf)) : emptyLog();
      set({ log, live: true, replaying: null });
      await api.startSession(cfg, resumeOf);
      get().toast(resumeOf ? "已续跑" : "已开局");
      await get().refreshStatus();
      set({ view: "stage" });
    } catch (e) {
      get().toast(e instanceof Error ? e.message : String(e), "error");
    }
  },

  async togglePause() {
    const st = get().st;
    if (!st) return;
    try {
      await api.pause(!st.paused);
      await get().refreshStatus();
    } catch (e) {
      get().toast(String(e), "error");
    }
  },

  async stop() {
    try {
      await api.stopSession();
      get().toast("已请求结束");
      await get().refreshStatus();
    } catch (e) {
      get().toast(String(e), "error");
    }
  },

  async refreshStatus() {
    try {
      set({ st: await api.status() });
    } catch {
      /* 状态查询失败不打扰用户 */
    }
  },

  pushEvent(line) {
    // 回放模式下不掺入直播事件,否则会串台
    if (!get().live) return;
    set({ log: applyLine(get().log, line) });
  },

  setLive(v) {
    if (v) set({ live: true, replaying: null });
    else set({ live: false });
  },

  async loadTree() {
    const cfg = get().config;
    if (!cfg) return;
    try {
      set({ treeEntries: await api.tree(cfg) });
    } catch (e) {
      get().toast(String(e), "error");
    }
  },

  async openFile(path) {
    const cfg = get().config;
    if (!cfg) return;
    try {
      const f = await api.readWorkspace(cfg, path);
      set({ currentFile: f, currentPath: path, view: "files" });
    } catch (e) {
      get().toast(e instanceof Error ? e.message : String(e), "error");
    }
  },

  closeFile() { set({ currentFile: null, currentPath: null }); },

  async loadNotes() {
    const cfg = get().config;
    if (!cfg) return;
    try {
      const memoryCfg = { ...cfg, agents: cfg.characters?.length ? cfg.characters : cfg.agents };
      set({ notes: await api.memories(memoryCfg) });
    } catch (e) {
      get().toast(String(e), "error");
    }
  },

  async loadRuns() {
    try {
      set({ runs: await api.sessions() });
    } catch (e) {
      get().toast(String(e), "error");
    }
  },

  async loadSkills() {
    const cfg = get().config;
    if (!cfg) return;
    try {
      set({ skills: await api.skillsList(cfg) });
    } catch {
      /* 技能目录可能还没建,不是错误 */
    }
  },

  async replay(session) {
    if (session === null) {
      set({ replaying: null, live: true, log: emptyLog() });
      const st = get().st;
      if (st?.session) {
        const lines = await api.eventsOf(st.session);
        set({ log: buildLog(lines) });
      }
      return;
    }
    try {
      const lines = await api.eventsOf(session);
      set({ log: buildLog(lines), replaying: session, live: false, view: "stage" });
    } catch (e) {
      get().toast(String(e), "error");
    }
  },

  async removeRun(session) {
    try {
      await api.deleteSession(session);
      if (get().replaying === session) set({ replaying: null, live: true, log: emptyLog() });
      await get().loadRuns();
      get().toast("已删除该场记录");
    } catch (e) {
      get().toast(String(e), "error");
    }
  },

  async exportRun(session) {
    try {
      const p = await api.exportSession(session);
      get().toast("已导出到 " + p);
    } catch (e) {
      get().toast(String(e), "error");
    }
  },

  toast(text, kind = "info") {
    const id = toastSeq++;
    set({ toasts: [...get().toasts, { id, text, kind }] });
    // 错误留久一点,免得用户看不到
    const ms = kind === "error" ? 9000 : 4200;
    setTimeout(() => get().dropToast(id), ms);
  },

  dropToast(id) {
    set({ toasts: get().toasts.filter((t) => t.id !== id) });
  },
}));
