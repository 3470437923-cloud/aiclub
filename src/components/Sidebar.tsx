import type { ReactElement } from "react";
import { useApp, type View } from "../store/app";
import { endText } from "../lib/eventlog";
import { humanMs } from "../lib/format";
import * as api from "../lib/bridge";
import {
  IconStage, IconFolder, IconNote, IconFilm, IconGear, IconInfo,
  IconPlay, IconPause, IconStop, IconSkill,
} from "./Icons";

const NAV: { v: View; label: string; icon: () => ReactElement }[] = [
  { v: "stage", label: "台面", icon: () => <IconStage /> },
  { v: "files", label: "文件", icon: () => <IconFolder /> },
  { v: "notes", label: "记忆", icon: () => <IconNote /> },
  { v: "runs", label: "录像", icon: () => <IconFilm /> },
];

export default function Sidebar() {
  const view = useApp((s) => s.view);
  const setView = useApp((s) => s.setView);
  const st = useApp((s) => s.st);
  const violations = useApp((s) => s.violations);
  const open = useApp((s) => s.sidebarOpen);
  const togglePause = useApp((s) => s.togglePause);
  const stop = useApp((s) => s.stop);
  const start = useApp((s) => s.start);
  const skills = useApp((s) => s.skills);
  const toast = useApp((s) => s.toast);

  if (!open) return null;

  const running = st?.running ?? false;
  const paused = st?.paused ?? false;
  const blocked = violations.length > 0;
  const agents = st?.agents ?? [];

  return (
    <nav className="rail">
      <div className="rail-head">
        <span className="section-title" style={{ padding: 0 }}>对话台</span>
      </div>

      <div className="nav">
        {NAV.map((n) => (
          <button
            key={n.v}
            className="nav-item"
            aria-current={view === n.v ? "page" : undefined}
            onClick={() => setView(n.v)}
          >
            {n.icon()}
            <span>{n.label}</span>
          </button>
        ))}
      </div>

      <div className="nav-sep" />

      {/* ── 人类仅有的四个动作 ── */}
      <div className="controls">
        {!running ? (
          <button
            className="btn primary grow"
            disabled={blocked}
            onClick={() => void start(false)}
            title={blocked ? "先在设置中完成参与者配置" : "开局"}
          >
            <IconPlay size={14} /> 开局
          </button>
        ) : (
          <>
            <button className="btn grow" onClick={() => void togglePause()} title={paused ? "继续" : "暂停"}>
              {paused ? <IconPlay size={14} /> : <IconPause size={14} />}
              {paused ? "继续" : "暂停"}
            </button>
            <button className="btn danger" onClick={() => void stop()} title="结束">
              <IconStop size={14} /> 结束
            </button>
          </>
        )}
      </div>

      {/* ── 参与者 ── */}
      <div className="rail-section">
        <span className="section-title">参与者</span>
        {agents.map((ag, i) =>
          ag ? (
            <div className={"agent-card" + (i === 1 ? " b" : "") + (st?.current === ag.id ? " active" : "")} key={ag.id}>
              <span className={"sigil " + (i === 0 ? "a" : "b")}>{ag.name.slice(0, 1)}</span>
              <div className="grow col" style={{ gap: 2 }}>
                <span className="who">{ag.name}</span>
                <span className="meta">{ag.company} · {ag.model}</span>
                <span className="meta">{ag.host}</span>
                <span className="meta tnum">
                  {ag.tokens.toLocaleString()} tok
                  {ag.failures > 0 ? " · 失败 " + ag.failures : ""}
                  {st?.current === ag.id ? " · 正在发言" : ""}
                </span>
              </div>
            </div>
          ) : (
            <div className="agent-card" key={i}>
              <span className={"sigil " + (i === 0 ? "a" : "b")}>?</span>
              <div className="grow col" style={{ gap: 2 }}>
                <span className="who">未配置</span>
                <span className="meta">去设置里完成角色接入</span>
              </div>
            </div>
          ),
        )}
      </div>

      {/* ── 技能库 ── */}
      {skills.length > 0 && (
        <>
          <div className="nav-sep" />
          <div className="rail-section">
            <span className="section-title">共享技能({skills.length})</span>
            {skills.map((s) => (
              <div className="row gap-6" key={s.slug} title={s.description} style={{ padding: "0 2px" }}>
                <IconSkill size={13} />
                <span className="xs ellipsis t3">{s.name}</span>
              </div>
            ))}
          </div>
        </>
      )}

      <div className="grow" />

      {/* ── 账目 ── */}
      <div className="rail-section" style={{ paddingBottom: 10 }}>
        <span className="section-title">本场账目</span>
        <div className="kv-list">
          <div className="row"><span className="dim">轮次</span><span className="tnum t2">{st?.turn ?? 0}{st?.max_turns ? " / " + st.max_turns : ""}</span></div>
          <div className="row"><span className="dim">工具调用</span><span className="tnum t2">{st?.tool_calls ?? 0}</span></div>
          <div className="row">
            <span className="dim">越界</span>
            <span className="tnum t2">{st?.violations ?? 0}</span>
          </div>
          <div className="row"><span className="dim">时长</span><span className="tnum t2">{humanMs(st?.ms_total ?? 0)}</span></div>
        </div>
        {st?.end_reason && (
          <div className="xs dim" style={{ marginTop: 6 }}>结局:{endText(st.end_reason)}</div>
        )}
      </div>

      <div className="nav-sep" style={{ margin: "0 9px 8px" }} />
      <div className="nav" style={{ paddingBottom: 10 }}>
        <button className="nav-item" aria-current={view === "setup" ? "page" : undefined} onClick={() => setView("setup")}>
          <IconGear /> <span>设置</span>
          {blocked && <span className="k">{violations.length} 项待修</span>}
        </button>
        <button className="nav-item" aria-current={view === "about" ? "page" : undefined} onClick={() => setView("about")}>
          <IconInfo /> <span>关于</span>
        </button>
        <button
          className="nav-item"
          onClick={() =>
            void api.windowControl("close").catch((e) => {
              // 同 TitleBar:ACL 拒绝时必须让人看见,不能静默变成"点了没反应"
              toast(`退出失败:${e instanceof Error ? e.message : String(e)}`, "error");
            })
          }
          title="关闭 AiClub"
        >
          <IconStop size={15} /> <span>退出</span>
        </button>
      </div>
    </nav>
  );
}
