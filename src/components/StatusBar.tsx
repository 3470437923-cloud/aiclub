import { useApp } from "../store/app";
import { endText } from "../lib/eventlog";
import { humanMs } from "../lib/format";

/** 状态栏:引擎现在到底在干什么,一行说清。 */
export default function StatusBar() {
  const st = useApp((s) => s.st);
  const log = useApp((s) => s.log);
  const live = useApp((s) => s.live);
  const replaying = useApp((s) => s.replaying);
  const config = useApp((s) => s.config);
  const violations = useApp((s) => s.violations);

  const running = st?.running ?? false;
  const paused = st?.paused ?? false;
  const cur = st?.agents.find((a) => a.id === st.current);
  const totalTokens = (st?.agents ?? []).reduce((sum, a) => sum + a.tokens, 0) || (st?.usage_a?.input ?? 0) + (st?.usage_a?.output ?? 0) + (st?.usage_b?.input ?? 0) + (st?.usage_b?.output ?? 0);

  const dotCls = !running ? "dot" : paused ? "dot paused" : "dot on";
  const stateText = !running
    ? st?.end_reason
      ? "已结束 · " + endText(st.end_reason)
      : "待机"
    : paused
      ? "已暂停"
      : "运行中 · 轮到 " + (cur?.name ?? "…");

  return (
    <footer className="statusbar">
      <span className={dotCls} aria-hidden />
      <span>{stateText}</span>

      {replaying && (
        <>
          <span className="sep" />
          <span>回放 {replaying}(直播已暂停显示)</span>
        </>
      )}

      <span className="sep" />
      <span className="tnum">第 {st?.turn ?? 0} / {st?.max_turns ?? "—"} 轮</span>

      <span className="sep" />
      <span className="tnum">
        {totalTokens.toLocaleString()} {" tok"}
      </span>

      <span className="sep" />
      <span className="tnum">{humanMs(st?.ms_total ?? 0)}</span>

      <span className="sep" />
      <span className="tnum">工具 {st?.tool_calls ?? 0}</span>

      {violations.length > 0 && (
        <>
          <span className="sep" />
          <span>{violations.length} 项配置待修</span>
        </>
      )}

      {log.bad.length > 0 && (
        <>
          <span className="sep" />
          <span title={log.bad.slice(0, 3).join("\n")}>{log.bad.length} 行事件无法解析</span>
        </>
      )}

      <span className="grow" />
      <span>{config?.globals.max_turns ? "上限 " + config.globals.max_turns + " 轮" : "不限轮次"}</span>
      <span className="sep" />
      <span>{config?.shell.enabled ? "shell 开" : "shell 关"}</span>
      <span className="sep" />
      <span>{live ? "直播" : "回放"}</span>
    </footer>
  );
}
