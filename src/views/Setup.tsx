import { useEffect, useRef, useState } from "react";
import { useApp } from "../store/app";
import * as api from "../lib/bridge";
import type { AgentProfile, AppConfig } from "../lib/types";
import { mergeModelOptions, preferChatModel, selectableModel } from "../lib/models";
import { IconShield, IconWarn, IconFolder } from "../components/Icons";
import NumField from "../components/NumField";

/**
 * 设置页。这里是「造物层」:定义两个 AI 是谁。
 *
 * 注意页面上**没有**任何"它们该聊什么"的输入 —— 那属于干预层,被 D-004 禁掉。
 * 人设模板里写的是"你是谁",不是"你要说什么"。
 *
 * ── 草稿为什么住在 store 里 ──
 * App.tsx 是 `{view === "setup" && <Setup />}`,切页即卸载。草稿和探测结果
 * 一旦是组件局部 state,用户"探测 → 切去看台面 → 切回来"就会全部丢失。
 * 所以两者都放 store(见 store/app.ts 的 setupDraft / modelOptions)。
 */
export default function Setup() {
  const config = useApp((s) => s.config);
  const draft = useApp((s) => s.setupDraft);
  const modelOptions = useApp((s) => s.modelOptions);
  const violations = useApp((s) => s.violations);
  const revalidate = useApp((s) => s.revalidate);
  const toast = useApp((s) => s.toast);
  const info = useApp((s) => s.info);
  const st = useApp((s) => s.st);
  const boot = useApp((s) => s.boot);

  const [saving, setSaving] = useState(false);
  /** 正在探测的是第几个档案(按下标,不按 id —— id 可能重复) */
  const [testing, setTesting] = useState<number | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const connectionRequests = useRef<Record<string, number>>({});
  const connectionResults = useApp((s) => s.connectionResults);
  const setConnectionResult = useApp((s) => s.setConnectionResult);

  // Initialize once; explicit reload replaces the draft in the store.
  useEffect(() => {
    if (config && !useApp.getState().setupDraft) {
      useApp.setState({ setupDraft: JSON.parse(JSON.stringify(config)) });
    }
  }, [config]);

  // 独立性预检。草稿每敲一个键都会变,所以加个防抖,别把 IPC 打满。
  useEffect(() => {
    if (!draft) return;
    const t = setTimeout(() => {
      void api
        .checkPair(draft)
        .then((v) => useApp.setState({ violations: v }))
        .catch((e) => {
          // 以前这里 .catch(() => {}) —— 预检失败时 violations 保持旧值,
          // "开局"按钮的禁用状态就是错的,而且没人知道为什么。
          toast(`独立性预检失败:${e instanceof Error ? e.message : String(e)}`, "error");
        });
    }, 250);
    return () => clearTimeout(t);
  }, [draft, toast]);

  useEffect(() => { void revalidate(); }, [revalidate]);

  if (!draft) {
    return <div className="empty"><div className="big">正在读取配置…</div></div>;
  }

  const running = st?.running ?? false;
  const profiles = draft.characters ?? draft.agents;
  const currentEditingId = profiles.some((a) => a.id === editingId) ? editingId : profiles[0]?.id;

  /**
   * 改草稿。**必须**从 store 现读现写,不能用渲染期闭包里的 draft ——
   * 否则 probe()/pick() 这类"await 之后才改"的路径会把 await 期间用户敲进去的
   * 内容覆盖回旧快照。实测复现过:探测期间改名字,探测回来名字被吞掉。
   */
  function patch(fn: (c: AppConfig) => void) {
    const cur = useApp.getState().setupDraft;
    if (!cur) return;
    const next: AppConfig = JSON.parse(JSON.stringify(cur));
    fn(next);
    useApp.setState({ setupDraft: next });
  }

  async function save() {
    const cur = useApp.getState().setupDraft;
    if (!cur) return;
    setSaving(true);
    try {
      // key 前后空白是最常见的复制粘贴事故
      const clean: AppConfig = JSON.parse(JSON.stringify(cur));
      clean.characters = clean.characters?.length ? clean.characters : clean.agents;
      for (const a of clean.characters) {
        a.base_url = a.base_url.trim();
        a.api_key = a.api_key.trim();
        a.model = a.model.trim();
        a.name = a.name.trim();
      }
      await api.saveConfig(clean);
      await useApp.getState().reloadConfig();
      await useApp.getState().loadSkills();
      await useApp.getState().loadNotes();
      toast("已保存");
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setSaving(false);
    }
  }

  async function pick() {
    try {
      const p = await api.pickDirectory();
      if (p) patch((c) => { c.workspace.root = p.replace(/\\/g, "/"); });
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    }
  }

  async function probe(index: number) {
    // 从 store 现读,拿到的是"此刻"的地址与 Key
    const cur = useApp.getState().setupDraft;
    const profiles = cur?.characters ?? cur?.agents ?? [];
    const agent = profiles[index];
    if (!cur || !agent) {
      toast("找不到这个 AI 的档案", "error");
      return;
    }
    setTesting(index);
    try {
      const models = await api.detect(agent.base_url.trim(), agent.api_key.trim());
      const options = mergeModelOptions(agent.model, models);
      useApp.getState().setModelOptions(index, options);

      if (models.length === 0) {
        toast("对方返回了模型列表,但是空的");
      } else {
        // 只在用户还没填模型时自动选一个;并且避开 embedding / tts 这类非对话模型
        const auto = preferChatModel(models);
        patch((c) => {
          const target = (c.characters ?? c.agents)[index];
          if (target && !target.model.trim()) target.model = auto;
        });
        toast(`已读到 ${models.length} 个模型,请在模型选择器中选择`);
      }
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setTesting(null);
    }
  }

  async function testConnection(index: number) {
    const cur = useApp.getState().setupDraft;
    const profile = (cur?.characters ?? cur?.agents ?? [])[index];
    if (!profile) return;
    const request = (connectionRequests.current[profile.id] ?? 0) + 1;
    connectionRequests.current[profile.id] = request;
    const signature = JSON.stringify(profile);
    const isCurrent = () => {
      const latest = useApp.getState().setupDraft;
      const current = (latest?.characters ?? latest?.agents ?? []).find((a) => a.id === profile.id);
      return connectionRequests.current[profile.id] === request && JSON.stringify(current) === signature;
    };
    try {
      const result = await api.testConnection(profile);
      if (isCurrent()) setConnectionResult(profile.id, { ok: true, ...result });
    } catch (e) {
      if (isCurrent()) setConnectionResult(profile.id, { ok: false, text: e instanceof Error ? e.message : String(e) });
    }
  }

  const ok = violations.length === 0;

  return (
    <div className="scroll">
      <div className="setup">
        <div className="row">
          <h2 className="grow">设置</h2>
          <button className="btn primary" onClick={() => void save()} disabled={saving}>
            {saving ? "保存中…" : "保存"}
          </button>
        </div>

        {/* ── 独立性硬闸:必须放在最显眼处 ── */}
        {ok ? (
          <div className="ok-row">
            <IconShield size={15} />
            <span>
              <b style={{ color: "var(--tx-1)" }}>两个接入是独立的</b>
              —— 主机名不同、凭证不同、模型不同。引擎会在开局前再验一次,前端置灰只是体验。
            </span>
          </div>
        ) : (
          <div className="violations">
            <div className="row gap-6">
              <IconWarn size={15} />
              <b style={{ color: "var(--tx-1)", fontSize: "var(--fs-sm)" }}>还不能开局 —— {violations.length} 项需要修</b>
            </div>
            {violations.map((v, i) => (
              <div className="violation-row" key={i}>
                <span className="idx">{String(i + 1).padStart(2, "0")}</span>
                <span>{v}</span>
              </div>
            ))}
            <div className="hint">
              这里没有「忽略警告继续」的勾选框,也没有 --force。硬闸在 Rust 侧执行,
              你没法绕过它 —— 能改的只有配置本身。
            </div>
          </div>
        )}

        {/* ── 接入模式 ── */}
        <div className="card col gap-10">
          <div className="row">
            <b style={{ fontSize: "var(--fs-sm)" }}>接入模式</b>
            <span className="hint">这是选模式,不是忽略警告。</span>
          </div>
          <div className="seg" role="group" aria-label="接入模式">
            <button
              aria-pressed={draft.workspace.mode === "independent"}
              onClick={() => patch((c) => { c.workspace.mode = "independent"; })}
            >
              独立接入(默认)
            </button>
            <button
              aria-pressed={draft.workspace.mode === "shared_endpoint"}
              onClick={() => patch((c) => { c.workspace.mode = "shared_endpoint"; })}
            >
              共享端点
            </button>
          </div>
          <div className="hint">
            {draft.workspace.mode === "independent"
              ? "要求两侧主机名不同、API Key 不同 —— 字面意义的「两家 API」。"
              : "允许相同端点、模型和 Key，由角色名与人设区分参与者。"}
          </div>
        </div>

        {/* ── 两个 AI ── */}
        <div className="card col gap-10">
          <div className="row"><b style={{ fontSize: "var(--fs-sm)" }}>角色库 ({(draft.characters ?? draft.agents).length})</b><button className="btn sm" onClick={() => { const id = crypto.randomUUID(); const base = (draft.characters ?? draft.agents)[0]; const next = { ...(base ?? { id: "", name: "", company: "", base_url: "", api_key: "", model: "", temperament: "", temperature: 0.7, max_output_tokens: 2048, context_window: 32768 }), id, name: "新角色" }; patch((c) => { c.characters = [...(c.characters ?? c.agents), next]; }); setEditingId(id); }}>新建角色</button></div>
          <div className="row gap-6" style={{ flexWrap: "wrap" }}>{(draft.characters ?? draft.agents).map((a) => <button key={a.id} className="chip" aria-pressed={currentEditingId === a.id} onClick={() => setEditingId(a.id)}>{a.name || "未命名"} · {a.model || "未选模型"}</button>)}</div>
        </div>
        <div className="pair-grid">
          {(draft.characters ?? draft.agents).map((a, i) => ({ a, i })).filter(({ a }) => a.id === currentEditingId).map(({ a, i }) => (
            <AgentForm
              key={a.id}
              index={i}
              agent={a}
              disabled={running}
              testing={testing === i}
              modelOptions={modelOptions[String(i)] ?? []}
              onTest={() => void probe(i)}
              onConnection={() => void testConnection(i)}
              connectionResult={connectionResults[a.id]}
              selected={draft.agents.some((item) => item.id === a.id)}
              onToggle={() => patch((c) => { const selected = c.agents.some((item) => item.id === a.id); c.agents = selected ? c.agents.filter((item) => item.id !== a.id) : [...c.agents, a]; })}
              onChange={(field, value) => {
                connectionRequests.current[a.id] = (connectionRequests.current[a.id] ?? 0) + 1;
                setConnectionResult(a.id, null);
                patch((c) => {
                const target = (c.characters ?? c.agents).find((item) => item.id === a.id);
                if (target) {
                  (target as unknown as Record<string, unknown>)[field] = value;
                  c.agents = c.agents.map((item) => item.id === a.id ? target : item);
                }
                });
              }}
            />
          ))}
        </div>

        {/* ── 工作区 ── */}
        <div className="card col gap-10">
          <div className="row">
            <IconFolder size={15} />
            <b style={{ fontSize: "var(--fs-sm)" }}>工作区</b>
            <span className="hint">两个 AI 唯一能读写的地方(arena/ 共享、memory/ 各自私有、skills/ 只读)。</span>
          </div>
          <div className="row gap-8">
            <input
              className="input mono grow"
              value={draft.workspace.root}
              placeholder={info?.workspace_default ?? ""}
              onChange={(e) => patch((c) => { c.workspace.root = e.target.value; })}
            />
            <button className="btn" onClick={() => void pick()}>浏览…</button>
            <button
              className="btn"
              onClick={() =>
                void api
                  .touchWorkspace(useApp.getState().setupDraft ?? draft)
                  .then(() => {
                    // 目录建好之后,要让引擎也用上这份草稿里的根目录,
                    // 否则 loadTree 读的还是已保存的旧配置,树是空的。
                    useApp.getState().setConfig(useApp.getState().setupDraft ?? draft);
                    return useApp.getState().loadTree();
                  })
                  .catch((e) => toast(String(e), "error"))
              }
            >
              建好目录
            </button>
          </div>
          <div className="hint">留空则用默认目录:{info?.workspace_default}</div>
          {st?.workspace && <div className="hint">当前引擎用的沙箱根:{st.workspace}</div>}
        </div>

        {/* ── 对话参数 ── */}
        <div className="card col gap-10">
          <b style={{ fontSize: "var(--fs-sm)" }}>对话参数</b>
          <div className="grid-3">
            <div className="field">
              <NumField label="最多轮次" value={draft.globals.max_turns} step={1} min={2}
                hint="到达即结束" onChange={(v) => patch((c) => { c.globals.max_turns = v; })} />
            </div>
            <div className="field">
              <NumField label="冷场轮数" value={draft.globals.cold_field_rounds} step={1} min={1}
                hint="连续 N 轮双方都沉默就散场" onChange={(v) => patch((c) => { c.globals.cold_field_rounds = v; })} />
            </div>
            <div className="field">
              <NumField label="token 预算" value={draft.globals.budget_tokens} step={10000} min={0}
                hint="0 = 不限" onChange={(v) => patch((c) => { c.globals.budget_tokens = v; })} />
            </div>
            <div className="field">
              <NumField label="轮间停顿(ms)" value={draft.globals.pause_between_turns_ms} step={50} min={0}
                hint="给人类看的时间" onChange={(v) => patch((c) => { c.globals.pause_between_turns_ms = v; })} />
            </div>
            <div className="field">
              <NumField label="单轮超时(ms)" value={draft.globals.turn_timeout_ms} step={5000} min={3000}
                hint="超时杀进程树/中断请求" onChange={(v) => patch((c) => { c.globals.turn_timeout_ms = v; })} />
            </div>
            <div className="field">
              <NumField label="重试次数" value={draft.globals.turn_retries} step={1} min={0}
                hint="仅对可重试错误" onChange={(v) => patch((c) => { c.globals.turn_retries = v; })} />
            </div>
          </div>
        </div>

        {/* ── 受限 shell ── */}
        <div className="card col gap-10">
          <div className="row">
            <b style={{ fontSize: "var(--fs-sm)" }}>允许 run_shell</b>
            <span className="grow" />
            <button
              className="switch"
              role="switch"
              aria-checked={draft.shell.enabled}
              onClick={() => patch((c) => { c.shell.enabled = !c.shell.enabled; })}
            />
          </div>
          <div className="hint">
            开启后 AI 可以执行命令。六道限制一道都不会少:cwd 强制在 arena/、{draft.shell.timeout_ms}ms 硬超时并杀整个进程树、
            输出只留头尾各一半、环境变量白名单透传(<span className="mono">*_API_KEY</span> 这类绝不下发,
            免得一句 <span className="mono">set</span> 就把对面那家的密钥捞走)、危险形态黑名单兜底。
          </div>
          <div className="hint" style={{ color: "var(--tx-3)" }}>
            已知限制:它<b>不拦截网络访问</b>。要跑需要联网的代码时请注意这一点。
          </div>
          {st?.shell_enabled !== undefined && st.shell_enabled !== draft.shell.enabled && (
            <div className="hint">注意:引擎当前用的是「{st.shell_enabled ? "开启" : "关闭"}」,重开一局才生效。</div>
          )}
        </div>

        {/* ── 人设模板(造物层) ── */}
        <div className="card col gap-10">
          <b style={{ fontSize: "var(--fs-sm)" }}>开局字条 (人设模板)</b>
          <div className="hint">
            这是唯一会被写进对话开头的东西。它定义「你是谁」,不定义「你要说什么」。
            可用占位符:<span className="mono">{"{name} {company} {model} {other_name} {other_company} {other_model} {arena}"}</span>
          </div>
          <textarea
            className="textarea mono"
            style={{ minHeight: 220, fontSize: "var(--fs-xs)" }}
            value={draft.persona_template}
            onChange={(e) => patch((c) => { c.persona_template = e.target.value; })}
          />
          <div className="row">
            <button className="btn sm" onClick={() => patch((c) => { c.persona_template = info?.persona_default ?? c.persona_template; })}>
              恢复默认
            </button>
          </div>
        </div>

        <div className="row gap-8" style={{ paddingBottom: 30 }}>
          <button className="btn primary" onClick={() => void save()} disabled={saving}>
            {saving ? "保存中…" : "保存全部设置"}
          </button>
          <button className="btn" onClick={() => void boot()}>重新载入</button>
          <span className="hint grow">
            配置文件:{info?.config_path} —— 里面以明文保存 API Key,不要把它复制到别处。
          </span>
        </div>
      </div>
    </div>
  );
}

function AgentForm(props: {
  index: number; agent: AgentProfile; disabled: boolean; testing: boolean; modelOptions: string[];
  onTest: () => void;
  onConnection: () => void;
  connectionResult?: { ok: boolean; model?: string; ms?: number; text: string };
  selected: boolean;
  onToggle: () => void;
  onChange: (field: keyof AgentProfile, value: unknown) => void;
}) {
  const { index, agent, disabled } = props;
  const side = index === 0 ? "a" : "b";
  const hasOptions = props.modelOptions.length > 0;
  const selected = selectableModel(agent.model, props.modelOptions);

  return (
    <div className="card profile">
      <div className="profile-head">
        <span className={"sigil " + side}>{agent.name.slice(0, 1) || (index === 0 ? "甲" : "乙")}</span>
        <span className="nm">{index === 0 ? "左侧" : "右侧"}</span>
        <span className="chip">{agent.id}</span>
        <button className="chip" aria-pressed={props.selected} disabled={disabled} onClick={props.onToggle}>{props.selected ? "已选参与" : "加入开局"}</button>
        {disabled && <span className="chip">运行中不可改</span>}
      </div>

      <div className="field">
        <label>名字(台面上显示)</label>
        <input className="input" value={agent.name} disabled={disabled}
          onChange={(e) => props.onChange("name", e.target.value)} placeholder={index === 0 ? "甲" : "乙"} />
      </div>

      <div className="field">
        <label>公司 / 来源(会写进对方的字条)</label>
        <input className="input" value={agent.company} disabled={disabled}
          onChange={(e) => props.onChange("company", e.target.value)} placeholder="DeepSeek" />
      </div>

      <div className="field">
        <label>接口地址 Base URL</label>
        <input className="input mono" value={agent.base_url} disabled={disabled}
          onChange={(e) => props.onChange("base_url", e.target.value)} placeholder="https://api.deepseek.com/v1" />
      </div>

      <div className="field">
        <label>API Key</label>
        <input className="input mono" type="password" value={agent.api_key} disabled={disabled}
          onChange={(e) => props.onChange("api_key", e.target.value)} placeholder="sk-…" autoComplete="off" spellCheck={false} />
        <span className="hint">以明文存在本机 config.toml 里。共享端点模式允许复用 Key。</span>
      </div>

      <div className="field">
        <label>模型 model</label>
        <div className="row gap-6">
          <input className="input mono grow" value={agent.model} disabled={disabled}
            onChange={(e) => props.onChange("model", e.target.value)} placeholder="手动填写模型名" />
          <button className="btn sm" disabled={disabled || props.testing} onClick={props.onTest}>
            {props.testing ? "探测中…" : "探测"}
          </button>
          <button className="btn sm" disabled={disabled} onClick={props.onConnection}>测试连接</button>
        </div>
        {hasOptions && (
          <select
            className="select mono"
            value={selected}
            disabled={disabled}
            onChange={(e) => { if (e.target.value) props.onChange("model", e.target.value); }}
            aria-label="选择已探测模型"
          >
            <option value="">从已探测模型中选择…</option>
            {props.modelOptions.map((model) => <option value={model} key={model}>{model}</option>)}
          </select>
        )}
        {props.connectionResult && (
          <div className={props.connectionResult.ok ? "hint ok-row" : "hint violations"}>
            {props.connectionResult.ok ? `连接成功 · ${props.connectionResult.model} · ${props.connectionResult.ms}ms · ${props.connectionResult.text}` : `连接失败: ${props.connectionResult.text}`}
          </div>
        )}
        <span className="hint">
          {hasOptions
            ? "已加载探测结果,可直接选择;也可以手动填写自定义模型。"
            : "探测 = 拿这把 Key 去读对方的 /models 列表,结果会出现在这里。"}
        </span>
      </div>

      <div className="field">
        <label>气质补充(可空)</label>
        <textarea className="textarea" style={{ minHeight: 60 }} value={agent.temperament} disabled={disabled}
          onChange={(e) => props.onChange("temperament", e.target.value)}
          placeholder="例如:说话短、爱较真、不喜欢被恭维" />
      </div>

      <div className="grid-3">
        <div className="field">
          <label>temperature</label>
          <input className="input tnum" type="number" step="0.05" min="0" max="2" value={agent.temperature} disabled={disabled}
            onChange={(e) => props.onChange("temperature", Number(e.target.value) || 0)} />
        </div>
        <div className="field">
          <label>单次最大输出</label>
          <NumField label="" value={agent.max_output_tokens} step={256} min={256} hint=""
            onChange={(v) => props.onChange("max_output_tokens", v)} />
        </div>
        <div className="field">
          <label>上下文窗口</label>
          <NumField label="" value={agent.context_window} step={1024} min={4096} hint=""
            onChange={(v) => props.onChange("context_window", v)} />
        </div>
      </div>
    </div>
  );
}
