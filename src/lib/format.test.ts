import { describe, expect, it } from "vitest";
import { argsBrief, humanBytes, humanMs, looksBinary, readableStamp, splitSession } from "./format";

describe("humanBytes", () => {
  it("三档单位都对", () => {
    expect(humanBytes(0)).toBe("0B");
    expect(humanBytes(1023)).toBe("1023B");
    expect(humanBytes(1024)).toBe("1.0KB");
    expect(humanBytes(1536)).toBe("1.5KB");
    expect(humanBytes(1048576)).toBe("1.0MB");
  });
});

describe("humanMs", () => {
  it("毫秒 / 秒 / 分", () => {
    expect(humanMs(500)).toBe("500ms");
    expect(humanMs(1500)).toBe("1.5s");
    expect(humanMs(90000)).toBe("1分30秒");
    expect(humanMs(3661000)).toBe("1时1分");
  });
});

describe("splitSession", () => {
  it("标准形如 2026-10-12T17-50-03_甲_vs_乙", () => {
    const s = splitSession("2026-10-12T17-50-03_甲_vs_乙");
    expect(s.stamp).toBe("2026-10-12T17-50-03");
    expect(s.a).toBe("甲");
    expect(s.b).toBe("乙");
  });
  it("没有 _vs_ 时不崩", () => {
    const s = splitSession("2026-10-12T17-50-03_only");
    expect(s.a).toBe("only");
    expect(s.b).toBe("?");
  });
  it("完全不成形也返回可渲染的东西", () => {
    expect(splitSession("garbage").stamp).toBe("garbage");
  });
});

describe("readableStamp", () => {
  it("转成斜杠形式", () => {
    expect(readableStamp("2026-10-12T17-50-03")).toBe("2026/10/12 17:50:03");
  });
  it("不符合就原样返回", () => {
    expect(readableStamp("nope")).toBe("nope");
  });
});

describe("argsBrief", () => {
  it("优先取 path", () => {
    expect(argsBrief({ path: "arena/x.md", content: "很长" })).toBe("arena/x.md");
  });
  it("没有 path 就看 dir / to / query", () => {
    expect(argsBrief({ query: "rps" })).toBe("rps");
    expect(argsBrief({ to: "arena/y.md" })).toBe("arena/y.md");
  });
  it("都没有就列前几个键", () => {
    expect(argsBrief({ a: 1, b: 2, c: 3, d: 4 })).toBe("a, b, c");
  });
  it("空对象与非对象都安全", () => {
    expect(argsBrief({})).toBe("");
    expect(argsBrief(null)).toBe("");
    expect(argsBrief("直接字符串")).toBe("直接字符串");
  });
});

describe("looksBinary", () => {
  it("常见二进制扩展名认得出来", () => {
    expect(looksBinary("arena/a.png")).toBe(true);
    expect(looksBinary("arena/a.exe")).toBe(true);
    expect(looksBinary("arena/a.md")).toBe(false);
    expect(looksBinary("arena/noext")).toBe(false);
  });
});
