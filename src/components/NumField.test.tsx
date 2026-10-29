// @vitest-environment jsdom
//
// 数字输入框的回归测试。
//
// 这是"模型探测后可选择"那条链路上实测到的第三个坑:
// 在真机 WebView 上往「单轮超时(ms)」逐字键入 60000,最终落到输入框里的是
// 300060000 —— 因为旧实现每按一键就把钳制过的值写回受控 input。
// 后果不只是难看:单轮硬超时从 60 秒变成 83 小时,实质失效。

import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import NumField from "./NumField";

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
});

/** React 受控组件认的是原生 setter,直接改 .value 不会触发 onChange */
function setNativeValue(el: HTMLInputElement, v: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(el, v);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

/**
 * 忠实模拟 Setup 的用法:父组件持有 value,**收到 onChange 后回写**。
 * 这一点很关键 —— 第一版测试桩把 value 钉成常量、不回写,于是 NumField 在失焦时
 * 会把显示收敛回"真正生效的旧值"(这其实是对的),测试却判成失败。
 * 桩不忠实,测试就是在测一个不存在的场景。
 */
function mount(initial: number, min: number) {
  const seen: number[] = [];
  let setExternal: ((v: number) => void) | null = null;
  function Host() {
    const [v, setV] = useState(initial);
    setExternal = setV;
    return (
      <NumField
        label="单轮超时(ms)"
        value={v}
        step={5000}
        min={min}
        hint=""
        onChange={(nv) => {
          seen.push(nv);
          setV(nv);
        }}
      />
    );
  }
  act(() => {
    root.render(<Host />);
  });
  return {
    input: container.querySelector("input")!,
    seen,
    /** 模拟"重新载入配置"把外部值推进来(同一实例收到新 props) */
    pushExternal: (v: number) => act(() => setExternal?.(v)),
  };
}

describe("NumField", () => {
  it("逐字键入 60000 不会变成 300060000", () => {
    const { input, seen } = mount(120000, 3000);

    act(() => input.focus());
    act(() => setNativeValue(input, ""));

    // 真实按键:每一击都在当前显示值后面追加一个字符
    for (const ch of "60000") {
      act(() => setNativeValue(input, input.value + ch));
    }

    // 显示的是用户打的东西,不是被钳过的中间值
    expect(input.value).toBe("60000");
    // 但草稿里一直是合法值(≥ min),配置不会被打成非法
    expect(seen.every((v) => v >= 3000)).toBe(true);
    expect(seen.at(-1)).toBe(60000);

    act(() => input.blur());
    expect(input.value).toBe("60000");
  });

  it("中间态不会把配置写成 0 或非法值", () => {
    const { input, seen } = mount(3000, 3000);
    act(() => input.focus());
    act(() => setNativeValue(input, ""));
    // 清空:不提交
    expect(seen).toEqual([]);
    act(() => input.blur());
    // 失焦回到当前生效值
    expect(input.value).toBe("3000");
  });

  it("小于下限的值在失焦时被收敛,并且显示的就是收敛后的值", () => {
    const { input, seen } = mount(120000, 3000);
    act(() => input.focus());
    act(() => setNativeValue(input, ""));
    act(() => setNativeValue(input, "5"));
    expect(input.value).toBe("5"); // 打字期间不打断用户
    act(() => input.blur());
    expect(input.value).toBe("3000"); // 失焦后显示真正生效的值
    expect(seen.at(-1)).toBe(3000);
  });

  it("外部值变化(重新载入配置)会同步到显示", () => {
    const { input, pushExternal } = mount(120000, 3000);
    expect(input.value).toBe("120000");
    pushExternal(9000);
    expect(input.value).toBe("9000");
  });

  it("正在编辑时外部值不打断用户(不能把光标里的数字顶掉)", () => {
    const { input, pushExternal } = mount(120000, 3000);
    act(() => input.focus());
    act(() => setNativeValue(input, ""));
    act(() => setNativeValue(input, "55"));
    expect(input.value).toBe("55");
    pushExternal(777);
    expect(input.value).toBe("55"); // 编辑中不被外部覆盖
    act(() => input.blur());
    expect(input.value).toBe("3000"); // 失焦后按自己打的收敛(55 < 3000 → 3000)
  });
});
