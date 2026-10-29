import { useEffect } from "react";
import { useApp } from "../store/app";
import { IconFilm, IconTrash, IconExport, IconPlay } from "../components/Icons";
import { endText } from "../lib/eventlog";
import { humanMs, readableStamp, splitSession } from "../lib/format";

/** 录像列表:events.jsonl 是唯一事实源,所以每一场都能完整重放。 */
export default function Runs() {
  const runs = useApp((s) => s.runs);
  const loadRuns = useApp((s) => s.loadRuns);
  const replay = useApp((s) => s.replay);
  const replaying = useApp((s) => s.replaying);
  const removeRun = useApp((s) => s.removeRun);
  const exportRun = useApp((s) => s.exportRun);
  const start = useApp((s) => s.start);

  useEffect(() => { void loadRuns(); }, [loadRuns]);

  return (
    <div className="pane">
      <div className="pane-head">
        <IconFilm size={15} />
        <span className="title">录像</span>
        <span className="chip">{runs.length} 场</span>
        <span className="grow" />
        <button className="btn sm ghost" onClick={() => void replay(null)}>回到直播</button>
        <button className="btn sm ghost" onClick={() => void loadRuns()}>刷新</button>
      </div>

      <div className="pane-body">
        {runs.length === 0 ? (
          <div className="empty">
            <IconFilm size={26} />
            <div className="big">还没有录像</div>
            <div className="xs" style={{ maxWidth: 340, lineHeight: 1.7 }}>
              每场对话都落成一份追加式的 events.jsonl。直播、回放、崩溃续跑读的是同一份,
              所以不存在「看起来是这样、实际不是」的情况。
            </div>
          </div>
        ) : (
          runs.map((r) => {
            const s = splitSession(r.id);
            return (
              <div
                key={r.id}
                className="run-row"
                role="button"
                tabIndex={0}
                aria-selected={replaying === r.id}
                onClick={() => void replay(r.id)}
                onKeyDown={(e) => { if (e.key === "Enter") void replay(r.id); }}
              >
                <div className="id" title={r.id}>{r.participants?.length ? r.participants.map((p) => p.name).join(" · ") : `${s.a} vs ${s.b}`}</div>
                {r.participants && r.participants.length > 0 && <div className="sub">参与者: {r.participants.map((p) => p.name).join("、")}</div>}
                <div className="row gap-6">
                  <button className="btn sm ghost" title="按本场参与者快照续跑" onClick={(e) => { e.stopPropagation(); void start(true, r.id); }}>续跑</button>
                  <button
                    className="btn icon sm ghost"
                    title="导出可读的 transcript.md"
                    onClick={(e) => { e.stopPropagation(); void exportRun(r.id); }}
                  >
                    <IconExport size={13} />
                  </button>
                  <button
                    className="btn icon sm ghost"
                    title="删除这一场"
                    onClick={(e) => {
                      e.stopPropagation();
                      if (confirm("删除这一场的全部事件记录?此操作不可撤销。")) void removeRun(r.id);
                    }}
                  >
                    <IconTrash size={13} />
                  </button>
                </div>
                <div className="sub">
                  <span>{readableStamp(s.stamp)}</span>
                  <span className="tnum">{r.turns} 轮</span>
                  <span className="tnum">{r.tokens.toLocaleString()} tok</span>
                  <span className="tnum">{humanMs(r.ms_total)}</span>
                  <span className="tnum">{r.events} 事件</span>
                  <span>{r.host_a} ↔ {r.host_b}</span>
                  {(r.mode === "shared_endpoint" || r.mode === "same_origin_dual_model") && <span className="chip">共享端点</span>}
                </div>
                <div className="row" style={{ gridRow: 1, gridColumn: 2 }}>
                  {r.end_reason ? <span className="end">{endText(r.end_reason)}</span> : <span className="end">未结束</span>}
                </div>
                {replaying === r.id && (
                  <div className="sub" style={{ gridColumn: "1 / -1" }}>
                    <span className="row gap-4"><IconPlay size={11} /> 正在回放</span>
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
