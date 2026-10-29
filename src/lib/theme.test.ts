// @vitest-environment jsdom
//
// 这个模块直接操作 localStorage 与 document,必须在 jsdom 下跑。

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { getTheme, setTheme, toggleTheme } from "./theme";

/**
 * 主题可达性回归。
 *
 * 这个模块存在的理由就是"浅色主题曾经完全不可达":
 * index.html 会读 localStorage["aiclub.theme"],但全项目没有任何一处写它,
 * 于是 html[data-theme="light"] 那套 token(对比度已由 tools/contrast.mjs 验证)
 * 永远不会被应用。这里守住"写了就真的生效"。
 */

beforeEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
});

afterEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
});

describe("theme", () => {
  it("默认是深色(与 index.html 的兜底一致)", () => {
    expect(getTheme()).toBe("dark");
  });

  it("setTheme 同时写 localStorage 和 <html data-theme>", () => {
    setTheme("light");
    // 两处都必须写:前者让下次启动不闪白,后者让本次立即生效
    expect(localStorage.getItem("aiclub.theme")).toBe("light");
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("toggleTheme 在两个主题间来回切", () => {
    expect(toggleTheme()).toBe("light");
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    expect(toggleTheme()).toBe("dark");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  it("localStorage 里是脏值时退回深色,不把非法值写进 data-theme", () => {
    localStorage.setItem("aiclub.theme", "rainbow");
    expect(getTheme()).toBe("dark");
  });

  it("浅色主题的 token 真的存在于样式表里(不是只有开关没有样式)", async () => {
    const tokens: string = (await import("../styles/tokens.css?raw")).default;
    expect(tokens).toContain('html[data-theme="light"]');
    expect(tokens).toContain('html[data-theme="dark"]');
  });
});
