// Vite 原生支持 ?raw 后缀,把 CSS 当字符串读进来。
// 不用 node:fs —— 那要引入 @types/node,而这里只是读一个同目录的文本文件。
import surface from "./surface.css?raw";
import { describe, expect, it } from "vitest";

/**
 * 侧栏滚动契约。
 *
 * jsdom 不做布局计算,量不出 scrollHeight,所以这里测的是**样式声明本身** ——
 * 它守不住"看起来对不对",但能守住"这条声明不被删掉",而那正是出问题的地方。
 *
 * 背景(2026-10-14 真机实测):
 *   窗口缩到 minHeight=600 时,侧栏内容自然高 780px、可用高度 520px,
 *   被裁掉 260px;把 scrollTop 设成 100 再读回来仍然是 0 —— 完全滚不动。
 *   原因:.rail 是 overflow: visible,而父级 .app-body 是 overflow: hidden。
 * 布局证据留在 .evidence/ 里。
 */
const css: string = surface;

/** 取出某个选择器的规则块(够用的粗粒度解析) */
function block(css: string, selector: string): string {
  const i = css.indexOf(selector + " {");
  if (i < 0) throw new Error("找不到规则:" + selector);
  return css.slice(i, css.indexOf("}", i));
}

describe("侧栏(.rail)滚动契约", () => {
  it(".rail 自己可滚动(父级是 overflow:hidden,指望不上)", () => {
    const b = block(css, ".rail");
    expect(b).toMatch(/overflow-y:\s*auto/);
    expect(b).toMatch(/overflow-x:\s*hidden/);
  });

  it(".rail 保持 min-height:0,否则 flex 子项撑破容器、滚动失效", () => {
    expect(block(css, ".rail")).toMatch(/min-height:\s*0/);
  });

  it("分组与卡片不被压扁:空间不足时滚动,而不是把每张卡挤成一条", () => {
    // .grow 是唯一的弹性占位,必须排除在外,否则它无法吸收多余空间
    const b = block(css, ".rail > *:not(.grow)");
    expect(b).toMatch(/flex-shrink:\s*0/);
  });

  it("不再保留永不生效的 .rail.collapsed(Sidebar 收起时是 return null)", () => {
    expect(css).not.toMatch(/\.rail\.collapsed\s*\{/);
  });
});
