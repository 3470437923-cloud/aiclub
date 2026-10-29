import { useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../store/app";
import Bubble from "../components/Bubble";
import { IconArrowDown, IconShield } from "../components/Icons";
import type { Item } from "../lib/eventlog";

/**
 * 台面。这是整个应用的主体:一条从左到右流动的对话。
 *
 * 两条硬约束体现在这里:
 * 1. 页面里**没有任何 input / textarea / 发送按钮** —— 人类没有通往对话的通道。
 * 2. 自动跟随,但用户一旦往上滚就不再抢滚动条(那是最烦人的 UI 毛病)。
 */
export default function Stage() {
  const log = useApp((s) => s.log);
  const st = useApp((s) => s.st);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [follow, setFollow] = useState(true);

  const names = useMemo(() => {
    const m: Record<string, { name: string; side: "a" | "b" }> = {};
    (st?.agents ?? []).forEach((a, i) => {
      m[a.id] = { name: a.name, side: i === 0 ? "a" : "b" };
    });
    if (!Object.keys(m).length) {
      m["A"] = { name: "甲", side: "a" };
      m["B"] = { name: "乙", side: "b" };
    }
    return m;
  }, [st?.agents]);

  // 内容变了就滚到底(仅在跟随状态)
  useEffect(() => {
    if (!follow) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [log.items, follow]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
    setFollow(atBottom);
  };

  const items = log.items;

  if (items.length === 0) return <EmptyStage />;

  return (
    <div className="stage-host">
      <div className="scroll" ref={scrollRef} onScroll={onScroll}>
        <div className="stage-scroll">
          {items.map((it: Item, i) => <Bubble key={it.id + ":" + i} it={it} names={names} />)}
        </div>
      </div>
      {!follow && (
        <button
          className="btn primary follow-btn"
          onClick={() => {
            setFollow(true);
            const el = scrollRef.current;
            if (el) el.scrollTop = el.scrollHeight;
          }}
        >
          <IconArrowDown size={14} /> 回到最新
        </button>
      )}
    </div>
  );
}

/** 空台面:把三条原则写给用户看,顺便解释为什么没有输入框。 */
function EmptyStage() {
  const st = useApp((s) => s.st);
  const setView = useApp((s) => s.setView);
  const start = useApp((s) => s.start);
  const violations = useApp((s) => s.violations);
  const blocked = violations.length > 0;

  return (
    <div className="stage-empty">
      <h2>玻璃缸是空的</h2>
      <p className="lead">
        已选角色会在这里自由聊天、互相试探、争夺或共同改造同一个共享目录。
        <br />
        你只做四件事:配置、按开始、看直播、按结束。
      </p>

      <div className="rules">
        <div className="rule">
          <IconShield size={15} />
          <div><b>没有输入框。</b>这个界面里没有任何地方能把文字送进对话。开局字条与人设卡属于「造物」,不属于「干预」。</div>
        </div>
        <div className="rule">
          <IconShield size={15} />
          <div><b>按角色分别接入。</b>独立模式隔离端点与凭证;共享端点模式允许多个角色复用同一模型与 Key。</div>
        </div>
        <div className="rule">
          <IconShield size={15} />
          <div><b>它们自己决定。</b>聊什么、比不比、什么时候散会都由它们定。沉默与退场都是合法动作。</div>
        </div>
      </div>

      <div className="row gap-8" style={{ marginTop: 8 }}>
        <button className="btn primary" disabled={blocked} onClick={() => void start(false)}>
          {blocked ? "先完成参与者设置" : "开局"}
        </button>
        <button className="btn" onClick={() => setView("setup")}>设置</button>
        <button className="btn" onClick={() => setView("runs")}>看以前的录像</button>
      </div>

      {st?.workspace && <div className="xxs dim" style={{ marginTop: 6 }}>工作区:{st.workspace}</div>}
    </div>
  );
}
