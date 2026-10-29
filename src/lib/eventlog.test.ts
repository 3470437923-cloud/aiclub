// eventlog 是前端唯一的业务逻辑,所以它必须被单测覆盖到边界。
// 这些用例是照着 Rust 侧事件模型写的 —— 两边不一致时它们会先红。

import { describe, expect, it } from "vitest";
import { applyEvent, applyLine, buildLog, emptyLog, endText, type LogState } from "./eventlog";
import type { Event } from "./types";

const feed = (...evs: Event[]): LogState => evs.reduce(applyEvent, emptyLog());

describe("say 的流式聚合", () => {
  it("多条 delta 拼成一条,收到 say_end 后定稿", () => {
    const s = feed(
      { type: "say_delta", turn: 1, agent: "A", text: "你" },
      { type: "say_delta", turn: 1, agent: "A", text: "好" },
      { type: "say_delta", turn: 1, agent: "A", text: "。" },
    );
    expect(s.items).toHaveLength(1);
    expect(s.items[0]).toMatchObject({ kind: "say", text: "你好。", streaming: true });

    const done = applyEvent(s, { type: "say_end", turn: 1, agent: "A", text: "你好。", input: 3, output: 2, ms: 42, finish_reason: "stop" });
    expect(done.items).toHaveLength(1);
    expect(done.items[0]).toMatchObject({ kind: "say", text: "你好。", streaming: false, ms: 42 });
  });

  it("没有 delta 直接来 say_end 也能落地(非流式端点)", () => {
    const s = feed({ type: "say_end", turn: 2, agent: "B", text: "直接一句", input: 1, output: 1, ms: 5 });
    expect(s.items[0]).toMatchObject({ kind: "say", text: "直接一句", agent: "B" });
  });

  it("同一轮里两个 agent 的 delta 不会串到一起", () => {
    const s = feed(
      { type: "say_delta", turn: 1, agent: "A", text: "甲说" },
      { type: "say_delta", turn: 1, agent: "B", text: "乙说" },
    );
    expect(s.items).toHaveLength(2);
    expect((s.items[0] as { text: string }).text).toBe("甲说");
    expect((s.items[1] as { text: string }).text).toBe("乙说");
  });

  it("say_end 与已聚合的 delta 对不上时以 say_end 为准(权威值是磁盘上的那条)", () => {
    const s = feed(
      { type: "say_delta", turn: 1, agent: "A", text: "半句" },
      { type: "say_end", turn: 1, agent: "A", text: "完整的一整句话", input: 9, output: 9, ms: 1 },
    );
    expect(s.items).toHaveLength(1);
    expect((s.items[0] as { text: string }).text).toBe("完整的一整句话");
  });
});

describe("其他事件类型的归并", () => {
  it("工具调用各自成为一张卡", () => {
    const s = feed(
      { type: "tool_call", turn: 1, agent: "A", call_id: "c1", name: "write", args: { path: "arena/x.md" }, ok: true, preview: "写 arena/x.md", result_full: "ok", ms: 3 },
      { type: "tool_call", turn: 1, agent: "A", call_id: "c2", name: "read", args: { path: "arena/x.md" }, ok: true, preview: "读 arena/x.md", result_full: "内容", ms: 2 },
    );
    expect(s.items.filter((i) => i.kind === "tool")).toHaveLength(2);
  });

  it("同一 call_id 重复到达会变成两张卡(后端不会重复发,但前端也不去猜)", () => {
    const one: Event = { type: "tool_call", turn: 1, agent: "A", call_id: "c1", name: "read", args: {}, ok: true, preview: "p", result_full: "f", ms: 1 };
    const s = feed(one, one);
    expect(s.items).toHaveLength(2);
  });

  it("pass / system_note / violation / turn_error / compress 都进台面", () => {
    const s = feed(
      { type: "pass", turn: 1, agent: "A" },
      { type: "system_note", turn: 1, text: "文件被对方改过" },
      { type: "violation", turn: 2, agent: "B", kind: "sandbox", path: "memory/A/notes.jsonl", detail: "只读" },
      { type: "turn_error", turn: 2, agent: "B", attempt: 1, error: "超时", retryable: true },
      { type: "compress", turn: 3, agent: "A", via_model: "m", tokens_in: 10, tokens_out: 3 },
    );
    expect(s.items.map((i) => i.kind)).toEqual(["pass", "system", "violation", "error", "compress"]);
  });

  it("turn_begin / agent_unreachable / context_snapshot 不进台面,但计入 total", () => {
    const s = feed(
      { type: "turn_begin", turn: 1, agent: "A", ts: "t" },
      { type: "agent_unreachable", agent: "A", consecutive_failures: 3 },
      { type: "context_snapshot", turn: 1, agent: "A", bytes: 100, compressed: false },
    );
    expect(s.items).toHaveLength(0);
    expect(s.total).toBe(3);
  });

  it("session_open 插在最前面(回放时它就是观众看到的第一屏)", () => {
    const s = feed(
      { type: "say_end", turn: 1, agent: "A", text: "后到的", input: 1, output: 1, ms: 1 },
      { type: "session_open", session_id: "s1", agents: [], mode: "independent", opening_note: "字条", ts: "t" },
    );
    expect(s.items[0].kind).toBe("open");
    expect(s.items[1].kind).toBe("say");
  });

  it("session_end 把 ended 置真并留在末尾", () => {
    const s = feed({ type: "session_end", reason: "max_turns", turns: 6, by_agent: {}, ms_total: 1000, ts: "t" });
    expect(s.ended).toBe(true);
    expect(s.items.at(-1)).toMatchObject({ kind: "end", reason: "max_turns", turns: 6 });
  });
});

describe("不可变性(React 依赖比较靠它)", () => {
  it("applyEvent 不改原对象", () => {
    const a = emptyLog();
    const b = applyEvent(a, { type: "pass", turn: 1, agent: "A" });
    expect(a.items).toHaveLength(0);
    expect(b.items).toHaveLength(1);
    expect(a).not.toBe(b);
  });
});

describe("坏行处理", () => {
  it("空行直接跳过,不算坏", () => {
    const s = applyLine(emptyLog(), "   ");
    expect(s.bad).toHaveLength(0);
    expect(s.total).toBe(0);
  });

  it("半行 JSON 记进 bad,不炸也不丢", () => {
    const s = applyLine(emptyLog(), '{"type":"turn_begin","turn":9');
    expect(s.bad).toHaveLength(1);
    expect(s.items).toHaveLength(0);
  });

  it("合法 JSON 但缺 type 也算坏", () => {
    const s = applyLine(emptyLog(), '{"turn":1}');
    expect(s.bad).toHaveLength(1);
  });

  it("未知 type 不崩:计入 total 但不进台面(向前兼容)", () => {
    const s = applyLine(emptyLog(), '{"type":"some_future_event","x":1}');
    expect(s.bad).toHaveLength(0);
    expect(s.total).toBe(1);
  });

  it("坏行不影响后续好行的处理", () => {
    const s = buildLog([
      '{"type":"pass","turn":1,"agent":"A"}',
      "{ 这不是 JSON",
      '{"type":"pass","turn":2,"agent":"B"}',
    ]);
    expect(s.items).toHaveLength(2);
    expect(s.bad).toHaveLength(1);
  });
});

describe("buildLog 与直播归并必须等价(回放与直播看的是同一份事实)", () => {
  it("同样的事件序列,两条路径得到相同的台面", () => {
    const lines = [
      '{"type":"session_open","session_id":"s","agents":[],"mode":"independent","opening_note":"n","ts":"t"}',
      '{"type":"turn_begin","turn":1,"agent":"A","ts":"t"}',
      '{"type":"say_delta","turn":1,"agent":"A","text":"你"}',
      '{"type":"say_delta","turn":1,"agent":"A","text":"好"}',
      '{"type":"say_end","turn":1,"agent":"A","text":"你好","input":1,"output":1,"ms":9}',
      '{"type":"tool_call","turn":1,"agent":"A","call_id":"c","name":"write","args":{},"ok":true,"preview":"p","result_full":"f","ms":2}',
      '{"type":"pass","turn":2,"agent":"B"}',
      '{"type":"session_end","reason":"cold_field","turns":2,"by_agent":{},"ms_total":100,"ts":"t"}',
    ];
    const viaBuild = buildLog(lines);
    let viaLive = emptyLog();
    for (const l of lines) viaLive = applyLine(viaLive, l);
    expect(viaLive.items).toEqual(viaBuild.items);
    expect(viaLive.ended).toBe(true);
  });
});

describe("同一轮的多条发言(seq)", () => {
  it("同轮同方的两条 say_end 是两条独立发言,不能互相覆盖", () => {
    // 真实场景:模型先说了句正文,又调了一次 say 工具 —— 后端会落两条 SayEnd,
    // turn 与 agent 完全相同。修复前它们共用一个聚合键,后一条覆盖前一条,
    // 台面上整整少一段发言,而且看不出少了。
    const s = feed(
      { type: "say_end", turn: 1, agent: "A", seq: 0, text: "先说一句", input: 1, output: 1, ms: 5 },
      { type: "say_end", turn: 1, agent: "A", seq: 1, text: "工具里又说一句", input: 1, output: 1, ms: 5 },
    );
    const says = s.items.filter((i) => i.kind === "say");
    expect(says).toHaveLength(2);
    expect(says.map((i) => (i.kind === "say" ? i.text : ""))).toEqual([
      "先说一句",
      "工具里又说一句",
    ]);
  });

  it("delta 与随后的 say_end 靠 seq 对齐,不会串到另一条发言上", () => {
    const s = feed(
      { type: "say_delta", turn: 1, agent: "A", seq: 0, text: "第一" },
      { type: "say_end", turn: 1, agent: "A", seq: 0, text: "第一句", input: 1, output: 1, ms: 5 },
      { type: "say_delta", turn: 1, agent: "A", seq: 1, text: "第二" },
      { type: "say_end", turn: 1, agent: "A", seq: 1, text: "第二句", input: 1, output: 1, ms: 5 },
    );
    const says = s.items.filter((i) => i.kind === "say");
    expect(says).toHaveLength(2);
    expect(says.every((i) => i.kind === "say" && !i.streaming)).toBe(true);
    expect(says.map((i) => (i.kind === "say" ? i.text : ""))).toEqual(["第一句", "第二句"]);
  });

  it("旧日志没有 seq 时,行为与以前完全一致(缺省 0)", () => {
    // 兼容性:升级前录下的 events.jsonl 里没有 seq 字段,不能因此错位。
    const s = feed(
      { type: "say_delta", turn: 1, agent: "A", text: "老" },
      { type: "say_delta", turn: 1, agent: "A", text: "日志" },
      { type: "say_end", turn: 1, agent: "A", text: "老日志", input: 1, output: 1, ms: 5 },
    );
    const says = s.items.filter((i) => i.kind === "say");
    expect(says).toHaveLength(1);
    expect(says[0]!.kind === "say" && says[0]!.text).toBe("老日志");
  });

  it("回放同一份日志两次得到相同结果(seq 不破坏幂等)", () => {
    const lines: Event[] = [
      { type: "say_end", turn: 1, agent: "A", seq: 0, text: "一", input: 1, output: 1, ms: 5 },
      { type: "say_end", turn: 1, agent: "A", seq: 1, text: "二", input: 1, output: 1, ms: 5 },
    ];
    const once = lines.reduce(applyEvent, emptyLog());
    const twice = lines.reduce(applyEvent, once);
    expect(twice.items).toEqual(once.items);
  });
});

describe("结局文案", () => {
  it("七个 code 都有中文", () => {
    for (const c of ["agent_close", "cold_field", "max_turns", "budget", "agent_unreachable", "user_stop", "config_invalid"]) {
      expect(endText(c)).not.toBe(c);
    }
  });
  it("未知 code 原样返回,不显示空白", () => {
    expect(endText("brand_new_reason")).toBe("brand_new_reason");
  });
});
