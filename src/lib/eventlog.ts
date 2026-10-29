// 事件流 -> 台面条目。**纯函数**,全部逻辑都在这里,便于 vitest 直接测。
//
// 为什么不用 zustand 直接往 store 里塞:事件会重复到达(直播 + 回放读同一份日志),
// 需要一条明确的"归并规则"。规则集中在 `applyEvent`,界面只负责画。

import type { AgentBrief, Event } from "./types";

export type Item =
  | { kind: "say"; id: string; turn: number; agent: string; text: string; streaming: boolean; ms?: number; finish?: string }
  | { kind: "tool"; id: string; turn: number; agent: string; name: string; args: unknown; ok: boolean; preview: string; full: string; ms: number }
  | { kind: "pass"; id: string; turn: number; agent: string }
  | { kind: "system"; id: string; turn: number; text: string }
  | { kind: "violation"; id: string; turn: number; agent: string; path: string; detail: string }
  | { kind: "error"; id: string; turn: number; agent: string; attempt: number; error: string; retryable: boolean }
  | { kind: "compress"; id: string; turn: number; agent: string; via_model: string; tokens_in: number; tokens_out: number }
  | { kind: "open"; id: string; session: string; agents: AgentBrief[]; mode: string; note: string; ts: string }
  | { kind: "end"; id: string; reason: string; turns: number; ms_total: number };

export interface LogState {
  /** 按到达顺序的台面条目 */
  items: Item[];
  /** 已处理事件总数(含 delta) */
  total: number;
  /** 解析/归并失败的原始行,不静默吞 */
  bad: string[];
  /** 是否见过 session_end */
  ended: boolean;
}

export const emptyLog = (): LogState => ({ items: [], total: 0, bad: [], ended: false });

/**
 * 发言的聚合键:**一轮 + 一方 + 轮内序号**。
 *
 * 为什么必须有 seq:一轮之内可以有多条发言 —— 模型先说一句、调个工具、
 * 再说一句,或者同一轮里连调两次 say(后端注释写明了"同一轮里重复 say
 * 视为多条发言,逐条落盘")。这些 SayEnd 的 turn 与 agent 完全一样,
 * 以前只靠这两个字段做键,后一条就把前一条**覆盖**掉了,
 * 台面上整整少一段发言,而且看不出少了。
 *
 * seq 缺省为 0,所以旧日志(没有这个字段)读起来与以前完全一致。
 */
const sayKey = (turn: number, agent: string, seq: number) =>
  "say:" + turn + ":" + agent + ":" + seq;

/**
 * 把一条事件并进状态。**返回新对象**(不在原对象上改),
 * 这样 React 的依赖比较和测试的断言都可靠。
 */
export function applyEvent(s: LogState, ev: Event): LogState {
  switch (ev.type) {
    case "say_delta": {
      const key = sayKey(ev.turn, ev.agent, ev.seq ?? 0);
      const items = s.items.slice();
      const i = items.findIndex((x) => x.kind === "say" && x.id === key);
      if (i >= 0) {
        const cur = items[i] as Extract<Item, { kind: "say" }>;
        items[i] = { ...cur, text: cur.text + ev.text, streaming: true };
      } else {
        items.push({ kind: "say", id: key, turn: ev.turn, agent: ev.agent, text: ev.text, streaming: true });
      }
      return { ...s, items, total: s.total + 1 };
    }
    case "say_end": {
      const key = sayKey(ev.turn, ev.agent, ev.seq ?? 0);
      const items = s.items.slice();
      const i = items.findIndex((x) => x.kind === "say" && x.id === key);
      const done: Item = {
        kind: "say", id: key, turn: ev.turn, agent: ev.agent,
        text: ev.text, streaming: false, ms: ev.ms, finish: ev.finish_reason,
      };
      if (i >= 0) items[i] = done;
      else items.push(done);
      return { ...s, items, total: s.total + 1 };
    }
    case "tool_call":
      return {
        ...s,
        total: s.total + 1,
        items: [...s.items, {
          kind: "tool", id: "tool:" + ev.call_id, turn: ev.turn, agent: ev.agent,
          name: ev.name, args: ev.args, ok: ev.ok, preview: ev.preview, full: ev.result_full, ms: ev.ms,
        }],
      };
    case "pass":
      return { ...s, total: s.total + 1, items: [...s.items, { kind: "pass", id: "pass:" + ev.turn + ":" + ev.agent, turn: ev.turn, agent: ev.agent }] };
    case "system_note":
      return { ...s, total: s.total + 1, items: [...s.items, { kind: "system", id: "sys:" + s.total, turn: ev.turn, text: ev.text }] };
    case "violation":
      return { ...s, total: s.total + 1, items: [...s.items, { kind: "violation", id: "vio:" + s.total, turn: ev.turn, agent: ev.agent, path: ev.path, detail: ev.detail }] };
    case "turn_error":
      return { ...s, total: s.total + 1, items: [...s.items, { kind: "error", id: "err:" + ev.turn + ":" + ev.agent + ":" + ev.attempt, turn: ev.turn, agent: ev.agent, attempt: ev.attempt, error: ev.error, retryable: ev.retryable }] };
    case "compress":
      return { ...s, total: s.total + 1, items: [...s.items, { kind: "compress", id: "cmp:" + ev.turn + ":" + ev.agent, turn: ev.turn, agent: ev.agent, via_model: ev.via_model, tokens_in: ev.tokens_in, tokens_out: ev.tokens_out }] };
    case "session_open":
      return { ...s, total: s.total + 1, items: [{ kind: "open", id: "open", session: ev.session_id, agents: ev.agents, mode: ev.mode, note: ev.opening_note, ts: ev.ts }, ...s.items] };
    case "session_end":
      return { ...s, ended: true, total: s.total + 1, items: [...s.items, { kind: "end", id: "end", reason: ev.reason, turns: ev.turns, ms_total: ev.ms_total }] };
    // 这三类不进台面(轮次分隔由界面按 turn 变化画;agent_unreachable/context_snapshot 走状态栏)
    case "turn_begin":
    case "agent_unreachable":
    case "context_snapshot":
    default:
      return { ...s, total: s.total + 1 };
  }
}

/** 解析并归并一行 JSONL。坏行不丢,记进 `bad`。 */
export function applyLine(s: LogState, line: string): LogState {
  const t = line.trim();
  if (!t) return s;
  let ev: Event;
  try {
    ev = JSON.parse(t) as Event;
  } catch {
    return { ...s, bad: [...s.bad, line] };
  }
  if (!ev || typeof ev !== "object" || typeof (ev as { type?: unknown }).type !== "string") {
    return { ...s, bad: [...s.bad, line] };
  }
  return applyEvent(s, ev);
}

/** 一次性重建(回放用)。 */
export function buildLog(lines: string[]): LogState {
  let s = emptyLog();
  for (const l of lines) s = applyLine(s, l);
  return s;
}

/** 把 session_end 的 reason 代码翻成人话(与 Rust `EndReason::human` 对齐)。 */
export const END_TEXT: Record<string, string> = {
  agent_close: "AI 自己散会了",
  cold_field: "连续沉默,自然冷场",
  max_turns: "到达轮次上限",
  budget: "token 预算用尽",
  agent_unreachable: "一方接口连续失败,沉默退场",
  user_stop: "人类按下结束",
  config_invalid: "两个 API 不够独立或没填全,未能开局",
};

export const endText = (code: string): string => END_TEXT[code] ?? code;
