import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { __setHost } from "../lib/bridge";
import { buildLog } from "../lib/eventlog";
import type { AgentProfile, AppConfig, Event, RunStatus } from "../lib/types";
import { useApp } from "./app";

const initialState = useApp.getState();
const oldId = "20260401_old_pair";
const otherId = "20260402_other_pair";

function config(selected: number): AppConfig {
  const characters: AgentProfile[] = ["a", "b"].map((id) => ({
    id, name: id.toUpperCase(), company: "test", base_url: "https://example.test/v1",
    api_key: "test-key", model: "chat", temperament: "", temperature: 0.7,
    max_output_tokens: 2048, context_window: 32768,
  }));
  return {
    version: 1, characters, agents: characters.slice(0, selected),
    workspace: { root: "/test-workspace", mode: "shared_endpoint" },
    globals: { theme: "dark", pause_between_turns_ms: 0, max_turns: 20,
      cold_field_rounds: 3, budget_tokens: 0, turn_timeout_ms: 30000, turn_retries: 0 },
    shell: { enabled: false, timeout_ms: 1000, output_bytes: 1024, env_passthrough: [] },
    persona_template: "{name}",
  };
}

function status(session: string): RunStatus {
  return {
    session, running: true, paused: false, stop_requested: false, turn: 2, max_turns: 20,
    mode: "shared_endpoint", usage_a: { input: 0, output: 0 }, usage_b: { input: 0, output: 0 },
    violations: 0, tool_calls: 0, end_reason: null, end_text: null, ms_total: 0,
    agents: [], current: null, shell_enabled: false, workspace: "/test-workspace",
  };
}

function history(session: string, text: string): string[] {
  const events: Event[] = [
    { type: "session_open", session_id: session, agents: [], mode: "shared_endpoint",
      opening_note: text, ts: "2026-04-01T00:00:00Z" },
    { type: "system_note", turn: 1, text },
  ];
  return events.map((event) => JSON.stringify(event));
}

function fakeIPC(lines = history(oldId, "A history")) {
  const calls: { cmd: string; args?: Record<string, unknown> }[] = [];
  __setHost({
    async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
      calls.push({ cmd, args });
      if (cmd === "save_config") return "saved" as T;
      if (cmd === "start_session") return { session: oldId } as T;
      if (cmd === "status") return status(oldId) as T;
      if (cmd === "events_of" && args?.session === oldId) return lines as T;
      throw new Error(`Unexpected IPC: ${cmd}`);
    },
    async listen() { return () => {}; },
  });
  return calls;
}

beforeEach(() => useApp.setState({ ...initialState, toast: vi.fn() }, true));
afterEach(() => {
  __setHost(null);
  useApp.setState(initialState, true);
});

describe("start 历史续跑", () => {
  it.each([0, 1])("当前只选 %i 人但原角色卡仍在时允许续跑历史快照", async (selected) => {
    const cfg = config(selected);
    const calls = fakeIPC();
    useApp.setState({ config: cfg, setupDraft: cfg, st: status(otherId) });

    await useApp.getState().start(true, oldId);

    expect(calls.find((call) => call.cmd === "start_session")?.args)
      .toEqual({ cfg, resumeOf: oldId });
    expect(useApp.getState().st?.session).toBe(oldId);
  });

  it("从局 B 直接续跑局 A 后显示 A 历史且不混入 B", async () => {
    const cfg = config(2);
    const linesA = history(oldId, "A history");
    fakeIPC(linesA);
    useApp.setState({ config: cfg, setupDraft: cfg, st: status(otherId),
      log: buildLog(history(otherId, "B history")), live: true });

    await useApp.getState().start(true, oldId);

    expect(useApp.getState().log).toEqual(buildLog(linesA));
    expect(useApp.getState().st?.session).toBe(oldId);
    expect(useApp.getState().live).toBe(true);
    expect(useApp.getState().replaying).toBeNull();
  });

  it.each([0, 1])("新开局只选 %i 人时仍拦截 IPC 启动", async (selected) => {
    const cfg = config(selected);
    const calls = fakeIPC();
    useApp.setState({ config: cfg, setupDraft: cfg });

    await useApp.getState().start(false);

    expect(calls).toEqual([]);
    expect(useApp.getState().view).toBe("setup");
    expect(useApp.getState().toast).toHaveBeenCalledWith("至少选择两位参与者", "error");
  });
});
