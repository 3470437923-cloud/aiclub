// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Setup from "./Setup";
import { useApp } from "../store/app";
import { __setHost } from "../lib/bridge";
import type { AgentProfile, AppConfig, ConnectionResult } from "../lib/types";

const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value));
function fixture(): AppConfig {
  const characters: AgentProfile[] = Array.from({ length: 16 }, (_, index) => ({
    id: "role-" + (index + 1), name: "角色" + (index + 1), company: "测试来源",
    base_url: "https://role-" + (index + 1) + ".example/v1", api_key: "fake-key-" + (index + 1),
    model: "chat-" + (index + 1), temperament: "", temperature: 0.7,
    max_output_tokens: 2048, context_window: 32768,
  }));
  return {
    version: 1, characters, agents: characters.slice(0, 2),
    workspace: { root: "/test-workspace", mode: "shared_endpoint" },
    globals: { theme: "dark", pause_between_turns_ms: 0, max_turns: 32,
      cold_field_rounds: 2, budget_tokens: 0, turn_timeout_ms: 30000, turn_retries: 0 },
    shell: { enabled: false, timeout_ms: 1000, output_bytes: 4096, env_passthrough: [] },
    persona_template: "你是{name}",
  };
}

let container: HTMLDivElement;
let root: Root;
let persisted: AppConfig;
let saved: AppConfig[];
let requested: AgentProfile[];
let detected: { baseUrl: string; apiKey: string }[];
let connection: (profile: AgentProfile) => Promise<ConnectionResult>;
const initialState = useApp.getState();

beforeEach(() => {
  vi.useFakeTimers();
  persisted = fixture(); saved = []; requested = []; detected = [];
  connection = async (profile) => ({ model: profile.model, ms: 42, text: "连接回复" });
  __setHost({
    listen: async () => () => {},
    invoke: async <T,>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
      let result: unknown;
      switch (cmd) {
        case "check_pair": case "skills_list": case "memories": result = []; break;
        case "get_config": result = clone(persisted); break;
        case "save_config": {
          persisted = clone(args!.cfg as AppConfig); saved.push(clone(persisted));
          result = "saved"; break;
        }
        case "test_connection": {
          const profile = clone(args!.profile as AgentProfile); requested.push(profile);
          result = await connection(profile); break;
        }
        case "detect":
          detected.push({ baseUrl: args!.baseUrl as string, apiKey: args!.apiKey as string });
          result = ["detected-chat"]; break;
        default: throw new Error("Unexpected IPC: " + cmd);
      }
      return result as T;
    },
  });
  useApp.setState({ ...initialState, config: clone(persisted), setupDraft: null,
    modelOptions: {}, connectionResults: {}, violations: [], toasts: [], view: "setup" }, true);
  container = document.createElement("div"); document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount()); container.remove();
  __setHost(null); useApp.setState(initialState, true);
  vi.clearAllTimers(); vi.useRealTimers();
});

async function render() { await act(async () => { root.render(<Setup />); }); }
async function click(button: HTMLButtonElement) { await act(async () => { button.click(); }); }
function button(text: string, scope: ParentNode = container): HTMLButtonElement {
  const found = Array.from(scope.querySelectorAll<HTMLButtonElement>("button"))
    .find((item) => item.textContent === text);
  if (!found) throw new Error("Missing button: " + text);
  return found;
}
async function choose(index: number) { await click(button("角色" + index + " · chat-" + index)); }
function field(label: string): HTMLInputElement {
  const found = Array.from(container.querySelectorAll(".profile .field"))
    .find((item) => item.querySelector("label")?.textContent === label)?.querySelector("input");
  if (!found) throw new Error("Missing field: " + label);
  return found;
}
async function input(label: string, value: string) {
  await act(async () => {
    const el = field(label);
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

// Real Setup + Zustand + bridge; only the IPC boundary is replaced, never a real API.
describe("Setup 角色库与连接", () => {
  it("初始只编辑一张卡，16张角色通过选择器访问", async () => {
    await render();
    expect(container.textContent).toContain("角色库 (16)");
    expect(container.querySelectorAll(".profile")).toHaveLength(1);
  });

  it("可按所选顺序从2人扩展为16人，再回到2人且角色库完整", async () => {
    await render();
    for (let index = 3; index <= 16; index++) {
      await choose(index); await click(button("加入开局"));
    }
    expect(useApp.getState().setupDraft!.agents.map((profile) => profile.id))
      .toEqual(Array.from({ length: 16 }, (_, index) => "role-" + (index + 1)));
    await click(button("保存"));
    expect(saved[0].agents).toHaveLength(16);
    expect(saved[0].characters).toHaveLength(16);
    for (let index = 3; index <= 16; index++) {
      await choose(index); await click(button("已选参与"));
    }
    await click(button("保存"));
    expect(saved[1].agents.map((profile) => profile.id)).toEqual(["role-1", "role-2"]);
    expect(saved[1].characters).toHaveLength(16);
  });

  it("编辑单卡并保存保留稳定ID、其他卡和参与者数据", async () => {
    await render(); await choose(2);
    expect(container.querySelectorAll(".profile")).toHaveLength(1);
    await input("名字(台面上显示)", "新名字"); await click(button("保存"));
    expect(saved[0].characters![1]).toMatchObject({ id: "role-2", name: "新名字" });
    expect(saved[0].agents[1]).toMatchObject({ id: "role-2", name: "新名字" });
    expect(saved[0].characters![0]).toEqual(fixture().characters![0]);
    expect(saved[0].characters).toHaveLength(16);
  });

  it("新建单卡保存后可再创建另一张，旧角色与新ID不丢失", async () => {
    await render(); await click(button("新建角色"));
    expect(container.querySelectorAll(".profile")).toHaveLength(1);
    await input("名字(台面上显示)", "新卡甲"); await click(button("保存"));
    const firstId = saved[0].characters![16].id;
    expect(firstId).toBeTruthy();
    await click(button("新建角色")); await input("名字(台面上显示)", "新卡乙");
    await click(button("保存"));
    expect(saved[1].characters).toHaveLength(18);
    expect(saved[1].characters!.slice(0, 16)).toEqual(fixture().characters);
    expect(saved[1].characters![16]).toMatchObject({ id: firstId, name: "新卡甲" });
    expect(saved[1].characters![17].id).not.toBe(firstId);
    expect(saved[1].characters![17].name).toBe("新卡乙");
  });

  it("连接成功展示模型、延迟与回复，修改模型使旧结果失效", async () => {
    await render(); await choose(1); await click(button("测试连接"));
    expect(container.textContent).toContain("连接成功 · chat-1 · 42ms · 连接回复");
    await input("模型 model", "changed-chat");
    expect(container.textContent).not.toContain("连接成功");
    expect(useApp.getState().connectionResults["role-1"]).toBeUndefined();
  });

  it("连接失败展示鉴权错误，修改Key使失败结果失效", async () => {
    connection = async () => { throw new Error("401: 无效凭证"); };
    await render(); await choose(1); await click(button("测试连接"));
    expect(container.textContent).toContain("连接失败: 401: 无效凭证");
    await input("API Key", "replacement-fake-key");
    expect(container.textContent).not.toContain("连接失败");
  });

  it("选择第16张卡探测使用该卡地址凭证并只为该卡提供模型", async () => {
    await render(); await choose(16); await click(button("探测"));
    expect(detected).toEqual([{ baseUrl: "https://role-16.example/v1", apiKey: "fake-key-16" }]);
    expect(container.querySelector("select")?.textContent).toContain("detected-chat");
    await choose(1);
    expect(container.querySelector("select")).toBeNull();
  });

  it("连接请求未完成时修改模型，旧请求完成后不能恢复过期成功结果", async () => {
    let resolve!: (result: ConnectionResult) => void;
    connection = () => new Promise((done) => { resolve = done; });
    await render(); await choose(1); await click(button("测试连接"));
    await input("模型 model", "changed-chat");
    await act(async () => { resolve({ model: "chat-1", ms: 42, text: "旧回复" }); });
    expect(container.textContent).not.toContain("连接成功");
    expect(useApp.getState().connectionResults["role-1"]).toBeUndefined();
  });
  it("选择第16张卡测试连接使用该卡且结果显示在该卡", async () => {
    await render(); await choose(16); await click(button("测试连接"));
    expect(requested).toEqual([fixture().characters![15]]);
    expect(container.textContent).toContain("连接成功 · chat-16 · 42ms · 连接回复");
  });

  it("切到其他页卸载后返回，保留未保存的名字草稿", async () => {
    await render(); await choose(1); await input("名字(台面上显示)", "未保存名字");
    await act(async () => {
      useApp.getState().setView("stage"); root.render(<div>台面</div>);
    });
    expect(useApp.getState().setupDraft!.characters![0].name).toBe("未保存名字");
    await act(async () => { useApp.getState().setView("setup"); });
    await render(); await click(button("未保存名字 · chat-1"));
    expect(field("名字(台面上显示)").value).toBe("未保存名字");
    expect(saved).toEqual([]);
  });
});
