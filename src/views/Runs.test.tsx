// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionMeta } from "../lib/types";
import Runs from "./Runs";

const state = vi.hoisted(() => ({
  runs: [] as SessionMeta[],
  st: { session: "20260402_latest_pair" },
  replaying: null as string | null,
  loadRuns: vi.fn(async () => {}),
  replay: vi.fn(async () => {}),
  removeRun: vi.fn(async () => {}),
  exportRun: vi.fn(async () => {}),
  start: vi.fn(async () => {}),
}));

vi.mock("../store/app", () => ({
  useApp: (selector: (s: typeof state) => unknown) => selector(state),
}));

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe("Runs 录像续跑", () => {
  it("点击第一条录像的续跑会传入该条 id,而非最后当前局", () => {
    const firstId = "20260401_older_pair";
    state.runs = [firstId, state.st.session].map((id): SessionMeta => ({
      id, events: 4, turns: 1, end_reason: null,
      a: "A", b: "B", host_a: "a.example", host_b: "b.example",
      mode: "independent", tokens: 10, ms_total: 100, unix: 0,
    }));
    act(() => root.render(<Runs />));

    const resumeButtons = Array.from(container.querySelectorAll("button"))
      .filter((button) => button.textContent === "续跑");
    expect(resumeButtons).toHaveLength(2);

    act(() => resumeButtons[0].click());

    expect(state.replay).not.toHaveBeenCalled();
    expect(state.start).toHaveBeenCalledExactlyOnceWith(true, firstId);
  });
});
