// 展示层的小工具。全部是纯函数。

export function humanBytes(b: number): string {
  if (b < 1024) return b + "B";
  if (b < 1024 * 1024) return (b / 1024).toFixed(1) + "KB";
  return (b / 1048576).toFixed(1) + "MB";
}

export function humanMs(ms: number): string {
  if (ms < 1000) return ms + "ms";
  const s = ms / 1000;
  if (s < 60) return s.toFixed(1) + "s";
  const m = Math.floor(s / 60);
  const rest = Math.round(s % 60);
  if (m < 60) return m + "分" + rest + "秒";
  return Math.floor(m / 60) + "时" + (m % 60) + "分";
}

/** 会话 id 形如 2026-10-12T17-50-03_甲_vs_乙 —— 拆成可读的两段。 */
export function splitSession(id: string): { stamp: string; a: string; b: string } {
  const i = id.indexOf("_");
  if (i < 0) return { stamp: id, a: "?", b: "?" };
  const stamp = id.slice(0, i);
  const rest = id.slice(i + 1);
  const j = rest.indexOf("_vs_");
  if (j < 0) return { stamp, a: rest, b: "?" };
  return { stamp, a: rest.slice(0, j), b: rest.slice(j + 4) };
}

/** 时间戳 -> 本地可读。解析失败就原样返回,不抛。 */
export function readableStamp(stamp: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2})-(\d{2})-(\d{2})$/.exec(stamp);
  if (!m) return stamp;
  return m[1] + "/" + m[2] + "/" + m[3] + " " + m[4] + ":" + m[5] + ":" + m[6];
}

/** 工具参数一行摘要(给工具卡标题用)。 */
export function argsBrief(args: unknown): string {
  if (args === null || args === undefined) return "";
  if (typeof args !== "object") return String(args);
  const o = args as Record<string, unknown>;
  const path = o["path"] ?? o["dir"] ?? o["to"] ?? o["query"];
  if (typeof path === "string") return path;
  const keys = Object.keys(o);
  if (!keys.length) return "";
  return keys.slice(0, 3).join(", ");
}

export const ZONE_LABEL: Record<string, string> = {
  arena: "共享",
  skills: "技能",
  memory: "记忆",
  other: "其他",
};

/** 按扩展名给代码块一个语言提示(只用于换行策略,不做高亮)。 */
export function looksBinary(name: string): boolean {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  return ["png", "jpg", "jpeg", "gif", "webp", "ico", "icns", "pdf", "zip", "gz", "exe", "dll", "so", "dylib", "woff", "woff2", "ttf", "otf"].includes(ext);
}
