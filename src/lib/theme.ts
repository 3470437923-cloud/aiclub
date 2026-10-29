/**
 * 主题切换。
 *
 * 为什么是 localStorage,而不是 config.globals.theme:
 * `index.html` 里那段内联脚本必须在 **CSS 之前**决定 data-theme,
 * 否则会先闪一下深色底再切成浅色。那一刻同步可读的只有 localStorage,
 * 拿不到 Tauri 的 config(要等 IPC 往返)。所以唯一真相源是 localStorage。
 *
 * config.globals.theme 是历史上留下的另一个字段(Rust 侧默认 "dark"),
 * 前端从没读过它,也永远不会 —— 两套真相源只会打架,这里不接它。
 *
 * 背景:在加这个文件之前,index.html 会读 `aiclub.theme`,但**没有任何代码写它**,
 * 于是 light("雾灰")主题永远不可达 —— 它的 token 与对比度矩阵都是完整的
 * (tools/contrast.mjs 会验它),用户却看不到。
 */

export type Theme = "dark" | "light";

const KEY = "aiclub.theme";

/** 读当前主题。默认深色 —— 和 index.html 里那段脚本的兜底保持一致。 */
export function getTheme(): Theme {
  try {
    const t = localStorage.getItem(KEY);
    return t === "light" ? "light" : "dark";
  } catch {
    // 隐私模式等场景下 localStorage 可能直接抛错
    return "dark";
  }
}

/** 应用主题并持久化。同时更新 <html data-theme> 与 localStorage。 */
export function setTheme(t: Theme): void {
  try {
    localStorage.setItem(KEY, t);
  } catch {
    /* 存不下就只在本次会话生效,不该因此崩掉界面 */
  }
  document.documentElement.setAttribute("data-theme", t);
}

export function toggleTheme(): Theme {
  const next: Theme = getTheme() === "dark" ? "light" : "dark";
  setTheme(next);
  return next;
}
