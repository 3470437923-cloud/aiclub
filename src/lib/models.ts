/** Keep detected models usable even when the current value is custom or blank. */
export function mergeModelOptions(current: string, detected: string[]): string[] {
  return [...new Set([current.trim(), ...detected.map((model) => model.trim())])]
    .filter(Boolean)
    .sort((a, b) => a.localeCompare(b));
}

/**
 * 从探测结果里挑一个"能聊天"的默认模型。
 *
 * 为什么不直接取排序后的第一个:`/v1/models` 通常把 embedding / tts / rerank /
 * 图像模型和对话模型混在一张表里,按字母序取第一个很可能悄悄写进一个根本
 * 不能对话的模型 —— 而用户看不出来,只会在开局后收到一串 400。
 *
 * 规则(可证伪,有单测):
 *   1. 先排除明显不是对话模型的 id(embedding / rerank / tts / 图像 / OCR…);
 *   2. 在剩下的里面优先含 "chat" 的;
 *   3. 最后取最短的(通常是没有日期后缀的稳定别名),同长按字典序。
 * 若全被排除,则退回原始列表,绝不返回空。
 */
const NON_CHAT =
  /(embed|rerank|whisper|tts|speech|audio|image|dall|stable-diffusion|moderation|ocr|transcri)/i;

export function preferChatModel(detected: string[]): string {
  const clean = detected.map((m) => m.trim()).filter(Boolean);
  if (clean.length === 0) return "";

  const chatty = clean.filter((m) => !NON_CHAT.test(m));
  const pool = chatty.length > 0 ? chatty : clean;

  const withChat = pool.filter((m) => /chat/i.test(m));
  const finalPool = withChat.length > 0 ? withChat : pool;

  return (
    [...finalPool].sort((a, b) => a.length - b.length || a.localeCompare(b))[0] ?? ""
  );
}

/**
 * 受控 `<select>` 的 value 必须**逐字**等于某个 option 的 value,否则浏览器会把
 * 它当成"没选中"并显示占位项。
 * 探测结果经过 trim,而用户手填的 model 可能带空格 —— 直接 includes 会漏判,
 * 于是出现"输入框里有模型、下拉框却显示『请选择』"的错位。
 */
export function selectableModel(current: string, options: string[]): string {
  const want = current.trim();
  return options.find((o) => o === want) ?? "";
}
