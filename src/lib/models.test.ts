import { describe, expect, it } from "vitest";
import { mergeModelOptions, preferChatModel, selectableModel } from "./models";

describe("mergeModelOptions", () => {
  it("保留当前自定义模型并合并去重探测结果", () => {
    expect(mergeModelOptions(" custom-model ", ["b", "a", "b", " "])).toEqual([
      "a",
      "b",
      "custom-model",
    ]);
  });

  it("空当前值时只返回有效探测模型", () => {
    expect(mergeModelOptions("", [" model-a ", "", "model-a"])).toEqual(["model-a"]);
  });
});

describe("preferChatModel", () => {
  it("跳过 embedding / tts / 图像这类非对话模型", () => {
    // 字母序第一个是 bge-embedding,直接取 options[0] 会静默写进一个不能对话的模型
    expect(
      preferChatModel(["bge-embedding-v3", "tts-1", "dall-e-3", "deepseek-chat"]),
    ).toBe("deepseek-chat");
  });

  it("多个候选时优先含 chat 的", () => {
    expect(preferChatModel(["glm-4-plus", "glm-4-chat"])).toBe("glm-4-chat");
  });

  it("同优先级取最短的(通常是没有日期后缀的稳定别名)", () => {
    expect(
      preferChatModel(["gpt-4o-2024-08-06", "gpt-4o-2024-05-13", "gpt-4o"]),
    ).toBe("gpt-4o");
  });

  it("全是被排除的模型时退回原始列表,绝不返回空", () => {
    expect(preferChatModel(["text-embedding-3-small", "whisper-1"])).toBe(
      "whisper-1",
    );
  });

  it("空输入返回空串", () => {
    expect(preferChatModel([])).toBe("");
    expect(preferChatModel(["", "   "])).toBe("");
  });

  it("去掉首尾空白再比较", () => {
    expect(preferChatModel(["  deepseek-chat  "])).toBe("deepseek-chat");
  });
});

describe("selectableModel", () => {
  it("受控 select 的 value 必须逐字命中某个 option", () => {
    expect(selectableModel("deepseek-chat", ["deepseek-chat", "glm-4"])).toBe(
      "deepseek-chat",
    );
  });

  it("用户手填带空格时仍能命中(旧写法 includes 未 trim 的值会漏判,下拉框显示占位符)", () => {
    expect(selectableModel("  deepseek-chat ", ["deepseek-chat"])).toBe(
      "deepseek-chat",
    );
  });

  it("自定义模型不在探测结果里时返回空串(显示占位项)", () => {
    expect(selectableModel("my-private-model", ["deepseek-chat"])).toBe("");
  });

  it("空当前值返回空串", () => {
    expect(selectableModel("", ["deepseek-chat"])).toBe("");
  });
});
