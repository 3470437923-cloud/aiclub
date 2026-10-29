// @vitest-environment jsdom
//
// 窗口拖动的回归测试。
//
// 背景(2026-10-14 真机实测):本机 WebView 完全不支持 -webkit-app-region
//   · CSS.supports("-webkit-app-region","drag") === false
//   · 往元素上写一遍再读回来是空字符串(声明被解析器丢弃)
//   · 扫描 222 条 CSS 规则,含 app-region 的 0 条
// 所以拖动**只能**靠 Tauri 注入的 drag.js + data-tauri-drag-region。
//
// 判定函数逐行照搬 tauri-2.12.3/src/window/scripts/drag.js 的 isDragRegion,
// 在真实渲染出来的 DOM 上跑 —— 这样"标题栏哪些地方能拖"是可测的,
// 而不是靠肉眼看。

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import TitleBar from "./TitleBar";
import { useApp } from "../store/app";

// ── Tauri drag.js 的判定逻辑(照搬) ──
const ATTR = "data-tauri-drag-region";
const CLICKABLE_TAGS = new Set(["A", "BUTTON", "INPUT", "SELECT", "TEXTAREA", "LABEL", "SUMMARY"]);
const INTERACTIVE_ROLES = new Set(["button", "link", "menuitem", "tab", "checkbox", "radio", "switch", "option"]);

function isClickable(el: Element): boolean {
  return (
    CLICKABLE_TAGS.has(el.tagName) ||
    (el.hasAttribute("contenteditable") && el.getAttribute("contenteditable") !== "false") ||
    (el.hasAttribute("tabindex") && el.getAttribute("tabindex") !== "-1") ||
    INTERACTIVE_ROLES.has(el.getAttribute("role") ?? "")
  );
}

/** composedPath 的祖先链等价物:target → … → root */
function pathOf(el: Element): Element[] {
  const out: Element[] = [];
  let cur: Element | null = el;
  while (cur) {
    out.push(cur);
    cur = cur.parentElement;
  }
  return out;
}

function isDragRegion(path: Element[]): boolean {
  for (const el of path) {
    const attr = el.getAttribute(ATTR);
    if (isClickable(el) && attr === null) return false;
    if (attr === null) continue;
    if (attr === "false") return false;
    if (attr === "deep") return true;
    if (attr === "" || attr === "true") return el === path[0];
  }
  return false;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useApp.setState({ info: null, st: null, config: null, replaying: null });
});

function render(overrides: Record<string, unknown> = {}) {
  useApp.setState({ info: { os: "macos", version: "0.1.0" } as never, ...overrides });
  act(() => {
    root.render(<TitleBar />);
  });
}

/** 标题栏上用户会去拖 / 会去点的位置 */
function targets() {
  const q = (s: string) => container.querySelector(s);
  return {
    titlebar: q(".titlebar")!,
    drag: q(".drag")!,
    brand: q(".brand")!,
    brandText: q(".brand b")!,
    brandIcon: q(".brand img")!,
    version: q(".brand .ver")!,
    sidebarBtn: q(".btn.icon.ghost")!,
  };
}

describe("标题栏拖动区", () => {
  it("data-os 首帧就是 macos(不能等 useEffect 晚一拍)", () => {
    render();
    // 旧写法用 useState + useEffect 设 os,首帧是空串,
    // 于是交通灯安全区(84px)和隐藏窗控都会闪一下。
    expect(container.querySelector(".titlebar")!.getAttribute("data-os")).toBe("macos");
  });

  it("整条标题栏都是拖动区,包括图标、标题文字、版本号", () => {
    render();
    const t = targets();
    // 这些正是旧版全部拖不动的地方(只有中间那条空 .drag 能拖)
    expect(isDragRegion(pathOf(t.brandIcon))).toBe(true);
    expect(isDragRegion(pathOf(t.brandText))).toBe(true);
    expect(isDragRegion(pathOf(t.version))).toBe(true);
    expect(isDragRegion(pathOf(t.brand))).toBe(true);
    expect(isDragRegion(pathOf(t.drag))).toBe(true);
    expect(isDragRegion(pathOf(t.titlebar))).toBe(true);
  });

  it("按钮仍然是按钮:可点击元素阻断拖动,不需要逐个加 data-tauri-drag-region=\"false\"", () => {
    render();
    const t = targets();
    expect(isDragRegion(pathOf(t.sidebarBtn))).toBe(false);
  });

  it("窗控按钮同样阻断拖动(点关闭不会变成拖窗口)", () => {
    render();
    const winBtn = container.querySelector(".win-btn")!;
    expect(isDragRegion(pathOf(winBtn))).toBe(false);
  });

  it("对照组:旧版布局(裸属性挂在一个空 .drag 上)标题文字拖不动", () => {
    // 手工搭出旧结构,证明这个测试确实能判红 ——
    // 否则"能拖"这个断言是空的。
    const legacy = document.createElement("header");
    legacy.className = "titlebar";
    legacy.innerHTML =
      '<div class="brand"><img alt=""><b>AiClub</b><span class="ver">v0.1.0</span></div>' +
      '<div class="drag" data-tauri-drag-region></div>' +
      '<div class="win-btns"><button class="win-btn"></button></div>';
    document.body.appendChild(legacy);

    const brandText = legacy.querySelector(".brand b")!;
    const drag = legacy.querySelector(".drag")!;
    expect(isDragRegion(pathOf(brandText))).toBe(false); // ← 旧版的真实行为
    expect(isDragRegion(pathOf(drag))).toBe(true);

    legacy.remove();
  });

  it("没有任何 -webkit-app-region 依赖(该属性在本机 WebView 是死声明)", () => {
    render();
    const bar = container.querySelector(".titlebar") as HTMLElement;
    // jsdom 里本来就不支持;这里断言的是"代码不再把拖动寄托在它身上":
    // 拖动能力必须来自 data-tauri-drag-region。
    expect(bar.getAttribute(ATTR)).toBe("deep");
    expect(bar.style.getPropertyValue("-webkit-app-region")).toBe("");
  });
});
