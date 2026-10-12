// AiClub 对比度门禁 —— 见 docs/specs/2026-10-12-aiclub-design.md §10.2
//
// 规矩:色值不可跨项目照抄。每个文字令牌必须在**它可能落到的每一档表面**上
// 实测,而不是只测 --sheet。任一格低于门槛即退出码 1。
//
//   node tools/contrast.mjs                     跑门禁(读 src/styles/tokens.css)
//   node tools/contrast.mjs --tokens <path>      改用指定的 tokens 文件(自检/CI 用)
//   node tools/contrast.mjs --json               只输出 JSON(供 CI 采集)
//
// 门槛:正文与小字 4.5:1;大字/非文字图形 3:1。本表按 4.5 一刀切(严)。

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
function argTokensPath() {
  const i = process.argv.indexOf("--tokens");
  if (i !== -1 && process.argv[i + 1]) return process.argv[i + 1];
  return join(HERE, "..", "src", "styles", "tokens.css");
}
const TOKENS_CSS = argTokensPath();
const THRESHOLD = 4.5;

/** 文字令牌 → 它允许落到的表面令牌集合。新增表面时必须同步这里。 */
const SURFACES = {
  dark: ["bg", "bg2", "rail", "sheet", "raised", "well"],
  light: ["bg", "bg2", "rail", "sheet", "raised", "well"],
};
const TEXT_TOKENS = ["tx1", "tx2", "tx3", "txdim"];
/** 反色填充上的字(主操作按钮) */
const ON_INK = { dark: ["onink", "ink"], light: ["onink", "ink"] };

function srgbToLin(v) {
  return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
}
function relLum(hex) {
  const h = hex.replace("#", "");
  if (!/^([0-9a-f]{6})$/i.test(h)) throw new Error(`需要 6 位十六进制,得到 ${hex}`);
  const c = h.match(/../g).map((x) => srgbToLin(parseInt(x, 16) / 255));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
}
function ratio(a, b) {
  const l1 = relLum(a);
  const l2 = relLum(b);
  return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
}

/** 从 tokens.css 里抠出某个 data-theme 块内的令牌值(含 :root 默认块) */
function readTokens(css, theme) {
  const sel = theme === "dark" ? /html\[data-theme="dark"\]/ : /html\[data-theme="light"\]/;
  const blocks = [];
  const re = /([^\n{}]+)\{([^{}]*)\}/g;
  let m;
  while ((m = re.exec(css)) !== null) {
    if (sel.test(m[1])) blocks.push(m[2]);
  }
  if (!blocks.length) throw new Error(`tokens.css 里找不到 ${theme} 主题块`);
  const out = {};
  for (const b of blocks) {
    for (const [, name, value] of b.matchAll(/--([\w-]+)\s*:\s*([^;]+);/g)) {
      const v = value.trim();
      if (/^#[0-9a-fA-F]{6}$/.test(v)) out[name.replace(/-/g, "")] = v;
    }
  }
  return out;
}

const jsonOnly = process.argv.includes("--json");
const failures = [];
const report = {};

let css;
try {
  css = readFileSync(TOKENS_CSS, "utf8");
} catch (e) {
  console.error(`读不到 ${TOKENS_CSS}\n请先建立 src/styles/tokens.css(本脚本是它的门禁,不是可选检查)。`);
  process.exit(2);
}

for (const theme of ["dark", "light"]) {
  const t = readTokens(css, theme);
  report[theme] = {};
  const missing = [...TEXT_TOKENS, ...SURFACES[theme], ...ON_INK[theme]].filter((k) => !(k in t));
  if (missing.length) {
    failures.push(`${theme}: tokens.css 缺少十六进制令牌 ${missing.join(", ")}(半透明表面不能当文字底,文字档必须是实色)`);
    continue;
  }
  for (const tk of TEXT_TOKENS) {
    report[theme][tk] = {};
    for (const sk of SURFACES[theme]) {
      const r = ratio(t[tk], t[sk]);
      report[theme][tk][sk] = +r.toFixed(2);
      if (r < THRESHOLD) {
        failures.push(`${theme}  --${tk.replace(/([a-z])(dim)/, "$1-$2")} on --${sk.replace(/^(bg2)$/, "bg-2")} = ${r.toFixed(2)} (< ${THRESHOLD})`);
      }
    }
  }
  const [onTok, fillTok] = ON_INK[theme];
  const ri = ratio(t[onTok], t[fillTok]);
  report[theme].onink_ink = ri.toFixed(2);
  if (ri < THRESHOLD) failures.push(`${theme}  --on-${onTok} on --${fillTok} = ${ri.toFixed(2)} (< ${THRESHOLD})`);
}

if (jsonOnly) {
  console.log(JSON.stringify(report, null, 2));
} else {
  for (const theme of ["dark", "light"]) {
    console.log(`\n══ ${theme === "dark" ? "曜灰 dark" : "雾灰 light"} ══`);
    const r = report[theme];
    if (!r) {
      console.log("  (主题块缺失)");
      continue;
    }
    const cols = SURFACES[theme].map((s) => s.padStart(8)).join("");
    console.log(`  token${cols}`);
    for (const tk of TEXT_TOKENS) {
      const cells = SURFACES[theme]
        .map((s) => r[tk][s].toFixed(2).padStart(8) + (r[tk][s] < THRESHOLD ? "!" : " "))
        .join("");
      console.log(`  ${tk.padEnd(6)}${cells}`);
    }
    console.log(`  on-ink/ink ${r.onink_ink}`);
  }
}

if (failures.length) {
  console.error(`\n✕ 对比度门禁未通过(${failures.length} 处):`);
  for (const f of failures) console.error("  · " + f);
  process.exit(1);
}
console.log(`\n✓ 全表面矩阵通过(门槛 ${THRESHOLD}:1)`);
