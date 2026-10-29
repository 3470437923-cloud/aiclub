import { useEffect, useRef } from "react";
import { useApp } from "./store/app";
import * as api from "./lib/bridge";
import TitleBar from "./components/TitleBar";
import Sidebar from "./components/Sidebar";
import StatusBar from "./components/StatusBar";
import Toasts from "./components/Toasts";
import Stage from "./views/Stage";
import Files from "./views/Files";
import Notes from "./views/Notes";
import Runs from "./views/Runs";
import Setup from "./views/Setup";
import About from "./views/About";

/**
 * 应用外壳。
 * 职责只有三件:开场拉一次状态、订阅事件、按 view 分发。
 * 所有业务规则都不在这里(在 Rust 与 lib/eventlog.ts 里)。
 */
export default function App() {
  const booted = useApp((s) => s.booted);
  const bootError = useApp((s) => s.bootError);
  const boot = useApp((s) => s.boot);
  const view = useApp((s) => s.view);
  const pushEvent = useApp((s) => s.pushEvent);
  const refreshStatus = useApp((s) => s.refreshStatus);
  const loadRuns = useApp((s) => s.loadRuns);
  const loadSkills = useApp((s) => s.loadSkills);
  const toast = useApp((s) => s.toast);

  const bootedRef = useRef(false);
  const lastTurn = useRef(-1);

  useEffect(() => {
    if (bootedRef.current) return;
    bootedRef.current = true;
    void boot();
  }, [boot]);

  // 订阅引擎事件(只订阅一次)
  useEffect(() => {
    let un: (() => void) | null = null;
    let alive = true;
    void (async () => {
      try {
        un = await api.onEventLine((line) => pushEvent(line));
        if (!alive && un) un();
      } catch {
        /* 浏览器预览时没有 Tauri 事件系统,忽略 */
      }
    })();
    return () => {
      alive = false;
      if (un) un();
    };
  }, [pushEvent]);

  // 状态轮询:轮次变化或运行态变化时刷新,并顺手刷新文件树与录像
  useEffect(() => {
    const t = setInterval(() => {
      void (async () => {
        const before = useApp.getState().st;
        await refreshStatus();
        const after = useApp.getState().st;
        if (!after) return;
        if (after.turn !== lastTurn.current) {
          lastTurn.current = after.turn;
          void useApp.getState().loadTree();
        }
        if (before?.running && !after.running) {
          // 刚结束:刷新录像与记忆
          void loadRuns();
          void useApp.getState().loadNotes();
        }
      })();
    }, 900);
    return () => clearInterval(t);
  }, [refreshStatus, loadRuns, loadSkills, toast]);

  if (bootError) {
    return (
      <div className="boot-fail">
        <div style={{ fontSize: "var(--fs-2xl)" }}>AiClub 起不来</div>
        <p className="hint" style={{ maxWidth: 520, lineHeight: 1.7 }}>{bootError}</p>
        <button className="btn primary" onClick={() => location.reload()}>重试</button>
      </div>
    );
  }

  if (!booted) {
    return (
      <div className="boot-fail">
        <div style={{ fontSize: "var(--fs-xl)", color: "var(--tx-2)" }}>正在启动…</div>
      </div>
    );
  }

  return (
    <div className="app">
      <TitleBar />
      <div className="app-body">
        <Sidebar />
        <main className="pane main">
          {view === "stage" && <Stage />}
          {view === "files" && <Files />}
          {view === "notes" && <Notes />}
          {view === "runs" && <Runs />}
          {view === "setup" && <Setup />}
          {view === "about" && <About />}
        </main>
      </div>
      <StatusBar />
      <Toasts />
    </div>
  );
}
