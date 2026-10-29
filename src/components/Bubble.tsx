import { useState } from "react";
import type { Item } from "../lib/eventlog";
import { argsBrief, humanMs } from "../lib/format";

interface NameMap { [id: string]: { name: string; side: "a" | "b" } }

/**
 * 台面上的一个条目。
 * 身份编码是**三重冗余**:左右位置 + 气泡形状(实底/描边)+ 文字名字。
 * 所以即便完全看不到颜色,也能分清谁在说话 —— 这是 KL-03 的补偿手段。
 */
export default function Bubble({ it, names }: { it: Item; names: NameMap }) {
  switch (it.kind) {
    case "say": {
      const n = names[it.agent];
      const side = n?.side ?? "a";
      return (
        <div className={"bubble-wrap " + side}>
          <span className={"sigil sm " + side} title={n?.name ?? it.agent}>
            {(n?.name ?? it.agent).slice(0, 1)}
          </span>
          <div className={"bubble " + side}>
            <div className="bubble-head">
              <span>{n?.name ?? it.agent}</span>
              <span>· 第 {it.turn} 轮</span>
              {it.streaming && <span>· 正在说</span>}
              {!it.streaming && it.ms ? <span>· {humanMs(it.ms)}</span> : null}
            </div>
            <div className="bubble-body selectable">
              {it.text}
              {it.streaming && <span className="caret" />}
            </div>
          </div>
        </div>
      );
    }

    case "tool":
      return <ToolCard it={it} names={names} />;

    case "pass": {
      const n = names[it.agent];
      return (
        <div className="pass-note" title={n?.name}>
          {n?.name ?? it.agent} 第 {it.turn} 轮保持沉默
        </div>
      );
    }

    case "system":
      return <div className="sysnote">{it.text}</div>;

    case "violation": {
      const n = names[it.agent];
      return (
        <div className="violation">
          <span className="tag">越界</span>
          <div className="grow">
            <div style={{ color: "var(--tx-1)" }}>
              {n?.name ?? it.agent} 试图访问 {it.path}
            </div>
            <div className="dim" style={{ marginTop: 2, fontSize: "var(--fs-2xs)" }}>{it.detail}</div>
            <div className="dim" style={{ marginTop: 2, fontSize: "var(--fs-2xs)" }}>
              这条是公开的 —— 台面上所有人都看见了。
            </div>
          </div>
        </div>
      );
    }

    case "error": {
      const n = names[it.agent];
      return (
        <div className="sysnote" style={{ borderColor: "var(--line-strong)", color: "var(--tx-3)" }}>
          {n?.name ?? it.agent} 第 {it.turn} 轮第 {it.attempt} 次调用失败:{it.error}
          {it.retryable ? "(会自动重试)" : ""}
        </div>
      );
    }

    case "compress": {
      const n = names[it.agent];
      return (
        <div className="sysnote">
          {n?.name ?? it.agent} 用自身端点({it.via_model})压缩了早期上下文 · 入 {it.tokens_in} / 出 {it.tokens_out} tok
        </div>
      );
    }

    case "end":
      return (
        <div className="session-end">
          <div className="r">{endHuman(it.reason)}</div>
          <div className="s">共 {it.turns} 轮 · 历时 {humanMs(it.ms_total)}</div>
        </div>
      );

    case "open":
      return (
        <div className="session-end" style={{ maxWidth: "86%" }}>
          <div className="s" style={{ marginBottom: 6 }}>
            开局字条 · {it.agents.map((x) => x.name + "@" + x.host).join("  vs  ")}
          </div>
          <div className="bubble-body selectable" style={{ fontSize: "var(--fs-xs)", color: "var(--tx-3)", textAlign: "left" }}>
            {it.note}
          </div>
        </div>
      );
  }
}

function endHuman(code: string): string {
  const m: Record<string, string> = {
    agent_close: "AI 自己散会了",
    cold_field: "连续沉默,自然冷场",
    max_turns: "到达轮次上限",
    budget: "token 预算用尽",
    agent_unreachable: "一方接口连续失败,沉默了",
    user_stop: "人类按下结束",
    config_invalid: "未能开局",
  };
  return m[code] ?? code;
}

/** 工具卡:默认折叠成一行,点开看全文。这一行摘要就是对面 AI 能看到的东西。 */
function ToolCard({ it, names }: { it: Extract<Item, { kind: "tool" }>; names: NameMap }) {
  const [open, setOpen] = useState(false);
  const n = names[it.agent];
  const side = n?.side ?? "a";
  return (
    <div className={"toolcard " + side}>
      <button className="toolcard-head" onClick={() => setOpen(!open)} aria-expanded={open}>
        <span className={"sigil sm " + side}>{(n?.name ?? it.agent).slice(0, 1)}</span>
        <span className="name">{it.name}</span>
        <span className="prev">{argsBrief(it.args) || it.preview}</span>
        <span className="grow" />
        <span className="dim">{it.ok ? "" : "失败 · "}{humanMs(it.ms)}</span>
        <span className="dim">{open ? "收起" : "展开"}</span>
      </button>
      {open && <div className="toolcard-body selectable">{it.full || "(空)"}</div>}
    </div>
  );
}
