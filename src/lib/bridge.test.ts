import { afterEach, describe, expect, it, vi } from "vitest";
import {
  __setHost, getConfig, saveConfig, checkPair, touchWorkspace, detect,
  startSession, pause, eventsOf, deleteSession, exportSession, tree,
  readWorkspace, memories, skillsList, onEventLine, EVENT_CHANNEL, startDragging,
} from "./bridge";
import type { AppConfig } from "./types";

const { tauriStartDragging } = vi.hoisted(() => ({ tauriStartDragging: vi.fn(async () => {}) }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging: tauriStartDragging }),
}));

// bridge 是唯一碰 IPC 的地方 —— 用一个假宿主把它单独测掉,
// 界面层就不用再 mock 任何东西了。
//
// 这里的断言就是「前后端契约」本身:Tauri v2 会把 Rust 函数的参数名
// (snake_case) 与 JS 传的键(camelCase)一一匹配。键名错一个,调用就
// 变成「找不到参数」错误 —— 这组测试必须先红再绿。

afterEach(() => __setHost(null));

function fakeHost(over: Partial<{ invoke: unknown; listen: unknown }> = {}) {
  const calls: { cmd: string; args?: Record<string, unknown> }[] = [];
  const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args });
    if (cmd === "get_config") return { version: 1, workspace: { root: "/w", mode: "independent" }, globals: {}, agents: [], shell: {}, persona_template: "" };
    return null;
  });
  const unsub = vi.fn();
  type Listener = (ev: string, cb: (e: { payload: string }) => void) => Promise<() => void>;
  const listen = vi.fn<Listener>(async () => unsub);
  __setHost({ invoke: invoke as never, listen: listen as never, ...over } as never);
  return { calls, invoke, listen, unsub };
}

const dummyCfg = { version: 1 } as unknown as AppConfig;

describe("命令转发(参数名必须与 commands.rs 一致)", () => {
  it("get_config / save_config / check_pair", async () => {
    const h = fakeHost();
    await getConfig();
    expect(h.calls[0].cmd).toBe("get_config");

    await saveConfig(dummyCfg);
    expect(h.calls[1].cmd).toBe("save_config");
    expect(Object.keys(h.calls[1].args ?? {})).toEqual(["cfg"]);

    await checkPair(dummyCfg);
    expect(h.calls[2].cmd).toBe("check_pair");
    expect(Object.keys(h.calls[2].args ?? {})).toEqual(["cfg"]);
  });

  it("detect 用 baseUrl + apiKey(不带配置包)", async () => {
    const h = fakeHost();
    await detect("https://api.deepseek.com", "sk-x");
    expect(h.calls[0].cmd).toBe("detect");
    expect(Object.keys(h.calls[0].args ?? {}).sort()).toEqual(["apiKey", "baseUrl"]);
  });

  it("touch_workspace / tree / memories / skills_list 都传完整 cfg", async () => {
    const h = fakeHost();
    await touchWorkspace(dummyCfg);
    await tree(dummyCfg);
    await memories(dummyCfg);
    await skillsList(dummyCfg);
    expect(h.calls[0].cmd).toBe("touch_workspace");
    expect(Object.keys(h.calls[0].args ?? {})).toEqual(["cfg"]);
    expect(h.calls[1].cmd).toBe("tree");
    expect(Object.keys(h.calls[1].args ?? {})).toEqual(["cfg"]);
    expect(h.calls[2].cmd).toBe("memories");
    expect(Object.keys(h.calls[2].args ?? {})).toEqual(["cfg"]);
    expect(h.calls[3].cmd).toBe("skills_list");
    expect(Object.keys(h.calls[3].args ?? {})).toEqual(["cfg"]);
  });

  it("read_workspace 传 cfg + path", async () => {
    const h = fakeHost();
    await readWorkspace(dummyCfg, "arena/x.md");
    expect(h.calls[0].cmd).toBe("read_workspace");
    expect(Object.keys(h.calls[0].args ?? {}).sort()).toEqual(["cfg", "path"]);
  });

  it("start_session 传 cfg + resumeOf;pause 传 on", async () => {
    const h = fakeHost();
    await startSession(dummyCfg, null);
    expect(h.calls[0].cmd).toBe("start_session");
    expect(Object.keys(h.calls[0].args ?? {}).sort()).toEqual(["cfg", "resumeOf"]);
    expect(h.calls[0].args?.resumeOf).toBeNull();

    await pause(true);
    expect(h.calls[1].cmd).toBe("pause");
    expect(Object.keys(h.calls[1].args ?? {})).toEqual(["on"]);
  });

  it("events_of / delete_session / export_session 用 session 名", async () => {
    const h = fakeHost();
    await eventsOf("s1");
    await deleteSession("s1");
    await exportSession("s1");
    expect(h.calls[0].cmd).toBe("events_of");
    expect(h.calls[1].cmd).toBe("delete_session");
    expect(h.calls[2].cmd).toBe("export_session");
    for (const c of h.calls) {
      expect(Object.keys(c.args ?? {})).toEqual(["session"]);
    }
  });

  it("调用失败会把错误原样抛给上层(不吞)", async () => {
    const invoke = vi.fn(async () => { throw new Error("后端炸了"); });
    __setHost({ invoke: invoke as never, listen: vi.fn() as never } as never);
    await expect(getConfig()).rejects.toThrow("后端炸了");
  });
});

describe("事件订阅", () => {
  it("用固定的频道名,并把 payload(原始行)原样给回调", async () => {
    const h = fakeHost();
    const got: string[] = [];
    const un = await onEventLine((l) => got.push(l));
    expect(h.listen).toHaveBeenCalledTimes(1);
    expect(h.listen.mock.calls[0]?.[0]).toBe(EVENT_CHANNEL);

    // 模拟一次推送
    const cb = h.listen.mock.calls[0]?.[1];
    if (!cb) throw new Error("listen 回调没被注册");
    cb({ payload: '{"type":"pass","turn":1,"agent":"A"}' });
    expect(got).toEqual(['{"type":"pass","turn":1,"agent":"A"}']);

    un();
    expect(h.unsub).toHaveBeenCalled();
  });
});

describe("窗口控制", () => {
  it("通过 Tauri Window API 启动无边框窗口拖动", async () => {
    await startDragging();
    expect(tauriStartDragging).toHaveBeenCalledTimes(1);
  });
});

describe("不在桌面壳里时", () => {
  it("给出人话错误,而不是 undefined 报错", async () => {
    __setHost(null);
    // 测试环境没有 window.__TAURI_INTERNALS__,应当被拦在门口
    await expect(getConfig()).rejects.toThrow(/桌面壳/);
  });
});
