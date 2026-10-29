//! 轮次引擎(§5):严格交替的自治对话状态机。
//!
//! 三条设计约束,决定了这个文件的形状:
//! 1. **事件日志是唯一事实源**。引擎自持 `JsonlLog`(落盘),同时把事件转发给 UI sink。
//!    每轮喂给模型的东西从磁盘上的日志重建,不是从内存 history —— 崩溃可续跑、可回放、可审计。
//! 2. **只依赖 Provider / EventSink 两个 trait**,所以全套状态机测试不需要网络。
//! 3. **人类没有通往对话的输入通道**,只有 running/paused/stop 三个开关。
//!
//! 严格不变量:
//! - 一方失败不拖死另一方(该方连续 3 次 → agent_unreachable 并结束)
//! - 对方的工具动作只给一行摘要;自己的给全文;对方的记忆永不出现

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};

use crate::config::{AccessMode, AgentProfile, AppConfig};
use crate::events::{now_ts, Event, EventSink, JsonlLog, UsagePair};
use crate::memory::MemoryStore;
use crate::provider::{ChatRequest, DeltaCb, Msg, ProviderError, SharedProvider, ToolSchema};
use crate::skills::{self, SkillMeta};
use crate::tools::{self, ChatAct, ToolCtx};
use crate::transcript::{self, CtxOpts};
use crate::validate::{norm_host, validate_app, Violation};

/// 结局原因(§5.3)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    AgentClose,
    ColdField,
    MaxTurns,
    Budget,
    AgentUnreachable,
    UserStop,
    ConfigInvalid,
}

impl EndReason {
    pub fn code(&self) -> &'static str {
        match self {
            EndReason::AgentClose => "agent_close",
            EndReason::ColdField => "cold_field",
            EndReason::MaxTurns => "max_turns",
            EndReason::Budget => "budget",
            EndReason::AgentUnreachable => "agent_unreachable",
            EndReason::UserStop => "user_stop",
            EndReason::ConfigInvalid => "config_invalid",
        }
    }
    pub fn human(&self) -> &'static str {
        match self {
            EndReason::AgentClose => "AI 自己散会了",
            EndReason::ColdField => "连续沉默,自然冷场",
            EndReason::MaxTurns => "到达轮次上限",
            EndReason::Budget => "token 预算用尽",
            EndReason::AgentUnreachable => "一方接口连续失败,沉默退场",
            EndReason::UserStop => "人类按下结束",
            EndReason::ConfigInvalid => "两个 API 不够独立或没填全,未能开局",
        }
    }
}

/// 前端按钮写、引擎读的共享开关(没有别的通道)
#[derive(Default)]
pub struct Controls {
    pub paused: AtomicBool,
    pub stop: AtomicBool,
    pub running: AtomicBool,
    pub turn: AtomicU32,
}

#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct RunStatus {
    pub running: bool,
    pub paused: bool,
    pub stop_requested: bool,
    pub turn: u32,
    pub max_turns: u32,
    pub mode: String,
    pub session: String,
    pub usage_a: UsagePair,
    pub usage_b: UsagePair,
    pub violations: usize,
    pub tool_calls: usize,
    pub end_reason: Option<String>,
    pub end_text: Option<String>,
    pub ms_total: u64,
    /// 台面内容不进 status(事件已单独推给前端),这里只给身份与账目
    pub agents: Vec<AgentView>,
    pub current: Option<String>,
    pub shell_enabled: bool,
    pub workspace: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AgentView {
    pub id: String,
    pub name: String,
    pub company: String,
    pub model: String,
    pub host: String,
    pub tokens: u64,
    pub failures: u32,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnOutcome {
    Spoke,
    Silent,
    Close,
    Withdraw,
    Unreachable,
}

pub struct Engine {
    cfg: AppConfig,
    providers: HashMap<String, SharedProvider>,
    ui: Arc<dyn EventSink>,
    log: JsonlLog,
    workspace: PathBuf,
    session: String,
    controls: Arc<Controls>,
    status: Arc<Mutex<RunStatus>>,
    skills: Vec<SkillMeta>,
    schemas: Vec<ToolSchema>,
    /// D-012:纪要按方各存一份,只由本方端点生成
    summaries: HashMap<String, String>,
    fail_count: HashMap<String, u32>,
    active: HashSet<String>,
    usage: HashMap<String, UsagePair>,
    pass_streak: u32,
    last_actor: Option<String>,
    violations: usize,
    tool_calls: usize,
    started: Instant,
    last_touch: HashMap<String, String>,
    /// 本轮内的**发言序号**,从 0 开始,每 commit 一条发言 +1。
    ///
    /// 为什么需要它:一轮之内可以有多条发言 —— 模型先说一句、调个工具、
    /// 再说一句,或者干脆在同一轮里连调两次 say(工具注释里写明了
    /// "同一轮里重复 say 视为多条发言,逐条落盘")。
    /// 这些 SayEnd 的 (turn, agent) 完全相同,前端光靠这两个字段分不开,
    /// 于是会把它们归并成一条,前一段发言在台面上直接消失。
    ///
    /// 用 Arc<AtomicU32> 而不是普通 u32:流式增量的回调(cb)要活到
    /// provider 调用结束之后,必须 'static,不能借用 self。
    say_seq: Arc<AtomicU32>,
}

const MAX_TOOL_CALLS_PER_TURN: usize = 8;
const STAGE_BUDGET_CHARS: usize = 24_000;
/// 台面超过这个字数就先压纪要
const COMPRESS_TRIGGER_CHARS: usize = 26_000;

/// 一份请求的 usage 只交给**第一条**发言,后续发言补零(见 run_turn 里的说明)。
/// 单独拎出来是因为它要在两个 commit_say 调用点用同一套"只记一次"的语义。
fn take_usage(charged: &mut bool, u: UsagePair) -> UsagePair {
    if *charged {
        UsagePair::default()
    } else {
        *charged = true;
        u
    }
}

impl Engine {
    /// 参数确实多,但这正是这个类型的意义:把引擎的**全部**外部依赖一次性钉死,
    /// 之后它不再去摸全局状态 —— 测试能注入假的 provider 与 sink,靠的就是这一点。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cfg: AppConfig,
        providers: HashMap<String, SharedProvider>,
        ui: Arc<dyn EventSink>,
        log: JsonlLog,
        workspace: PathBuf,
        session: String,
        controls: Arc<Controls>,
        status: Arc<Mutex<RunStatus>>,
    ) -> Self {
        let skill_agent = cfg.agents.first().map(|p| p.id.as_str()).unwrap_or("");
        let skills = skills::list_for_agent(&workspace.join("skills"), skill_agent);
        let schemas = tools::schemas(cfg.shell.enabled);
        let me = Self {
            cfg,
            providers,
            ui,
            log,
            workspace,
            session,
            controls,
            status,
            skills,
            schemas,
            summaries: HashMap::new(),
            fail_count: HashMap::new(),
            active: HashSet::new(),
            usage: HashMap::new(),
            pass_streak: 0,
            last_actor: None,
            violations: 0,
            tool_calls: 0,
            started: Instant::now(),
            last_touch: HashMap::new(),
            say_seq: Arc::new(AtomicU32::new(0)),
        };
        me.sync_status(None);
        me
    }

    fn emit(&self, ev: &Event) {
        self.log.append(ev);
        self.ui.emit(ev);
    }

    fn history(&self) -> Vec<Event> {
        JsonlLog::read_all(self.log.path())
    }

    pub fn status(&self) -> RunStatus {
        self.status.lock().unwrap().clone()
    }

    fn sync_status(&self, end: Option<EndReason>) {
        let mut s = self.status.lock().unwrap();
        s.running = self.controls.running.load(Ordering::SeqCst);
        s.paused = self.controls.paused.load(Ordering::SeqCst);
        s.stop_requested = self.controls.stop.load(Ordering::SeqCst);
        s.turn = self.controls.turn.load(Ordering::SeqCst);
        s.max_turns = self.cfg.globals.max_turns;
        s.mode = match self.cfg.workspace.mode {
            AccessMode::Independent => "independent".into(),
            AccessMode::SameOriginDualModel => "same_origin_dual_model".into(),
            AccessMode::SharedEndpoint => "shared_endpoint".into(),
        };
        s.session = self.session.clone();
        // 用量按档案顺序落到 a/b 两个槽(界面只关心"左边/右边")
        let id0 = self
            .cfg
            .agents
            .first()
            .map(|p| p.id.clone())
            .unwrap_or_else(|| "A".into());
        let id1 = self
            .cfg
            .agents
            .get(1)
            .map(|p| p.id.clone())
            .unwrap_or_else(|| "B".into());
        s.usage_a = self.usage.get(&id0).cloned().unwrap_or_default();
        s.usage_b = self.usage.get(&id1).cloned().unwrap_or_default();
        s.violations = self.violations;
        s.tool_calls = self.tool_calls;
        s.ms_total = self.started.elapsed().as_millis() as u64;
        s.shell_enabled = self.cfg.shell.enabled;
        s.workspace = self.workspace.display().to_string().replace('\\', "/");
        s.current = if s.running {
            self.last_actor.clone()
        } else {
            None
        };
        s.agents = self
            .cfg
            .agents
            .iter()
            .map(|p| AgentView {
                id: p.id.clone(),
                name: p.name.clone(),
                company: p.company.clone(),
                model: p.model.clone(),
                host: norm_host(&p.base_url),
                tokens: self.usage.get(&p.id).map(|u| u.total()).unwrap_or(0),
                failures: self.fail_count.get(&p.id).copied().unwrap_or(0),
                active: self.active.is_empty() || self.active.contains(&p.id),
            })
            .collect();
        match end {
            Some(r) => {
                s.end_reason = Some(r.code().to_string());
                s.end_text = Some(r.human().to_string());
            }
            None => {
                // 没有结局就是没有结局。
                //
                // 以前这个分支什么都不做,于是 end_reason 一旦写进共享的
                // RunStatus 就永远留在那儿 —— 上一局结束后再开新局,
                // 侧栏会整场挂着上一局的"结局:到达轮次上限",
                // 让人以为新一局已经结束了。状态是共享的,就必须在
                // 新一局的起点把它复位。
                s.end_reason = None;
                s.end_text = None;
            }
        }
    }

    /// 开局字条(§5.2):唯一的"注入",属造物层
    pub fn opening_note(&self, p: &AgentProfile, other: &AgentProfile) -> String {
        self.cfg
            .persona_template
            .replace("{name}", &p.name)
            .replace("{company}", &p.company)
            .replace("{model}", &p.model)
            .replace("{other_name}", &other.name)
            .replace("{other_company}", &other.company)
            .replace("{other_model}", &other.model)
            .replace("{arena}", "arena/")
    }

    pub fn system_prompt(
        &self,
        p: &AgentProfile,
        other: &AgentProfile,
        memory_block: &str,
    ) -> String {
        let mut s = String::new();
        s.push_str(&self.opening_note(p, other));
        s.push_str("\n\n【你的接口】");
        s.push_str(&format!(
            "{}@{host} —— 这是你**独立**的接入,凭证与对方完全无关,谁也看不见谁的 key。\n",
            p.model,
            host = norm_host(&p.base_url)
        ));
        if !p.temperament.trim().is_empty() {
            s.push_str(&format!("【你的气质】{}\n", p.temperament.trim()));
        }
        s.push_str(
            "【可用动作】发言 say / 沉默 pass / 散会 close_table;文件 read write edit list search mkdir move delete;\
            执行 run_shell(固定 cwd=arena/、限时、输出截断、环境白名单);记忆 remember recall;技能 skill_load。\n",
        );
        let agent_skills = skills::list_for_agent(&self.workspace.join("skills"), &p.id);
        s.push_str(&skills::catalog_block(&agent_skills));
        s.push('\n');
        s.push_str(memory_block);
        s.push_str(
            "\n【台面规则】\n\
             1. 只有对方在听,没有人类会回答你。你不说话(或调 pass)就等于沉默。\n\
             2. arena/ 是你们共同的世界。你能看到对方**做过什么**的一行摘要,但看不到内容 —— 要看就自己去 read。\n\
             3. memory/ 是你自己的,对方看不见;你也看不见对方的。越界会被公开拒绝,记录会贴在台面上。\n\
             4. 对方可能在你没注意时改了文件,动笔前先 read 一次。\n\
             5. 想比试就直接出题或写东西给对方;别等谁批准。\n",
        );
        s
    }

    /// D-011 硬闸:不过就返回违规,绝不启动。
    pub fn start(&mut self) -> Result<(), Vec<Violation>> {
        let v = validate_app(&self.cfg);
        if !v.is_empty() {
            self.emit(&Event::SystemNote {
                turn: 0,
                text: format!(
                    "开局被拒绝:{}",
                    v.iter()
                        .map(|x| x.human_message())
                        .collect::<Vec<_>>()
                        .join(";")
                ),
            });
            self.controls.running.store(false, Ordering::SeqCst);
            self.sync_status(Some(EndReason::ConfigInvalid));
            return Err(v);
        }
        let a = &self.cfg.agents[0];
        let b = &self.cfg.agents[1];
        let agents: Vec<crate::events::AgentBrief> = self
            .cfg
            .agents
            .iter()
            .map(|p| crate::events::AgentBrief {
                id: p.id.clone(),
                name: p.name.clone(),
                company: p.company.clone(),
                model: p.model.clone(),
                host: norm_host(&p.base_url),
            })
            .collect();
        let note = self.opening_note(a, b);
        self.emit(&Event::SessionOpen {
            session_id: self.session.clone(),
            agents,
            mode: match self.cfg.workspace.mode {
                AccessMode::Independent => "independent".into(),
                AccessMode::SameOriginDualModel => "same_origin_dual_model".into(),
                AccessMode::SharedEndpoint => "shared_endpoint".into(),
            },
            opening_note: note,
            ts: now_ts(),
        });
        // 工作区骨架(沙箱根必须存在,sandbox 的 canonicalize 才有效)
        //
        // memory/ 下按**配置里的 id** 建目录,不能硬编码 A/B:
        // 用户把 id 改成别的(或手写 config.toml)时,硬编码会在工作区里
        // 留下一堆用不上的 memory/A、memory/B 垃圾区,文件树里很显眼。
        let _ = std::fs::create_dir_all(self.workspace.join("arena"));
        let _ = std::fs::create_dir_all(self.workspace.join("skills"));
        for id in self
            .cfg
            .agents
            .iter()
            .map(|p| p.id.trim())
            .filter(|s| !s.is_empty())
        {
            let _ = std::fs::create_dir_all(self.workspace.join("memory").join(id));
        }
        skills::seed_builtin(&self.workspace.join("skills"));
        let first_id = self.cfg.agents.first().map(|p| p.id.as_str()).unwrap_or("");
        self.skills = skills::list_for_agent(&self.workspace.join("skills"), first_id);
        self.active = self.cfg.agents.iter().map(|p| p.id.clone()).collect();
        self.controls.running.store(true, Ordering::SeqCst);
        self.sync_status(None);
        Ok(())
    }

    /// 主循环。resume = 从磁盘已有事件续跑。
    pub async fn run(&mut self, resume: bool) -> EndReason {
        let ids: Vec<String> = self.cfg.agents.iter().map(|p| p.id.clone()).collect();
        if ids.len() < 2 {
            self.controls.running.store(false, Ordering::SeqCst);
            // 必须把"停了"同步进共享状态。
            // 以前这里直接 return,RunStatus.running 就永远是 true:
            // 界面一直说"已经有一个会话在跑",而 stop_session 也救不回来
            // (引擎已经走了,没人再去读那个 stop 标志)。
            self.sync_status(Some(EndReason::ConfigInvalid));
            return EndReason::ConfigInvalid;
        }
        self.active = ids.iter().cloned().collect();
        self.controls.running.store(true, Ordering::SeqCst);
        self.started = Instant::now();
        let mut next_actor = 0usize;
        self.last_actor = None;
        if resume {
            let evs = self.history();
            let last = evs
                .iter()
                .filter_map(|e| match e {
                    Event::TurnBegin { turn, .. } | Event::SayEnd { turn, .. } => Some(*turn),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            self.controls.turn.store(last, Ordering::SeqCst);
            for event in &evs {
                if let Event::SystemNote { text, .. } = event {
                    if let Some(withdrawn) = text.strip_prefix("agent_withdrawn:") {
                        let id = withdrawn.split(':').next().unwrap_or_default();
                        self.active.remove(id);
                    }
                }
                if let Event::TurnBegin { agent, .. } | Event::SayEnd { agent, .. } = event {
                    if let Some(index) = ids.iter().position(|id| id == agent) {
                        next_actor = (index + 1) % ids.len();
                        self.last_actor = Some(agent.clone());
                    }
                }
            }
        }
        // 用量账本一律从磁盘重建,**开局与续跑走同一条路**。
        // 之前只在 resume 分支里建,于是全新一局的 usage 永远是空的 ——
        // status() 报给界面的 token 数会整场都是 0,直到有人手动续跑一次。
        // 反正 events.jsonl 是唯一事实源,重建它比维护两套路径便宜。
        {
            let evs = self.history();
            for id in &ids {
                self.usage
                    .insert(id.clone(), transcript::usage_of(&evs, id));
            }
        }

        let max = if self.cfg.globals.max_turns == 0 {
            u32::MAX
        } else {
            self.cfg.globals.max_turns
        };

        // 循环以 break 带出结局:没有"先赋哨兵再覆盖"的死赋值
        let end = loop {
            if self.controls.stop.load(Ordering::SeqCst) {
                break EndReason::UserStop;
            }
            let turn = self.controls.turn.load(Ordering::SeqCst) + 1;
            if turn > max {
                break EndReason::MaxTurns;
            }
            let total: u64 = self.usage.values().map(|u| u.total()).sum();
            if self.cfg.globals.budget_tokens > 0 && total > self.cfg.globals.budget_tokens {
                break EndReason::Budget;
            }
            self.wait_while_paused().await;
            if self.controls.stop.load(Ordering::SeqCst) {
                break EndReason::UserStop;
            }

            let active: Vec<String> = ids
                .iter()
                .filter(|id| self.active.contains(*id))
                .cloned()
                .collect();
            if active.len() < 2 {
                break EndReason::AgentClose;
            }
            // Keep the cursor in the original participant order when someone leaves.
            while !self.active.contains(&ids[next_actor]) {
                next_actor = (next_actor + 1) % ids.len();
            }
            let actor = ids[next_actor].clone();
            next_actor = (next_actor + 1) % ids.len();
            let outcome = self.run_turn(&actor, turn).await;
            self.controls.turn.store(turn, Ordering::SeqCst);
            self.last_actor = Some(actor.clone());
            self.sync_status(None);
            match outcome {
                TurnOutcome::Close | TurnOutcome::Withdraw | TurnOutcome::Unreachable => {
                    self.active.remove(&actor);
                    self.emit(&Event::SystemNote {
                        turn,
                        text: format!("agent_withdrawn:{actor}"),
                    });
                    if self.active.len() < 2 {
                        break if matches!(outcome, TurnOutcome::Unreachable) {
                            EndReason::AgentUnreachable
                        } else {
                            EndReason::AgentClose
                        };
                    }
                }
                TurnOutcome::Silent => {
                    self.pass_streak += 1;
                    // 冷场 = 连续 N 轮双方都沉默,所以阈值乘 2
                    if self.pass_streak >= self.cfg.globals.cold_field_rounds.saturating_mul(2) {
                        break EndReason::ColdField;
                    }
                }
                TurnOutcome::Spoke => self.pass_streak = 0,
            }
            if self.cfg.globals.pause_between_turns_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(
                    self.cfg.globals.pause_between_turns_ms,
                ))
                .await;
            }
        };

        self.controls.running.store(false, Ordering::SeqCst);
        let by_agent = {
            let mut m = serde_json::Map::new();
            for id in &ids {
                m.insert(
                    id.clone(),
                    serde_json::json!(self.usage.get(id).cloned().unwrap_or_default()),
                );
            }
            serde_json::Value::Object(m)
        };
        self.emit(&Event::SessionEnd {
            reason: end.code().into(),
            turns: self.controls.turn.load(Ordering::SeqCst),
            by_agent,
            ms_total: self.started.elapsed().as_millis() as u64,
            ts: now_ts(),
        });
        self.sync_status(Some(end));
        end
    }

    async fn wait_while_paused(&self) {
        while self.controls.paused.load(Ordering::SeqCst)
            && !self.controls.stop.load(Ordering::SeqCst)
        {
            tokio::time::sleep(std::time::Duration::from_millis(180)).await;
        }
    }

    /// 组装本方上下文(测试可直接调用验证隔离性)
    pub fn build_context(&self, agent_id: &str, memory_block: &str) -> (String, String, bool) {
        let me = self.cfg.agent(agent_id).cloned().unwrap_or_default();
        let other = self
            .cfg
            .agents
            .iter()
            .find(|p| p.id != agent_id)
            .cloned()
            .unwrap_or_default();
        let evs = self.history();
        let (mut stage, truncated) = transcript::render_context(
            &evs,
            &CtxOpts {
                self_id: agent_id,
                self_name: &me.name,
                other_name: &other.name,
                opening_note: "",
                system_notes: &[],
                budget_chars: STAGE_BUDGET_CHARS,
            },
        );
        if let Some(sum) = self.summaries.get(agent_id) {
            stage = format!("{}\n\n{stage}", transcript::summary_block(&me.name, sum));
        }
        let system = self.system_prompt(&me, &other, memory_block);
        (system, stage, truncated)
    }

    async fn run_turn(&mut self, agent_id: &str, turn: u32) -> TurnOutcome {
        // 发言序号是**轮内**的,每轮从 0 重来。
        // 不重置的话序号会一直涨,虽然前端仍能分开发言,但日志读起来很难懂,
        // 而且 (turn, agent, seq) 这个键一旦跨轮就不再有"轮内第几条"的含义。
        self.say_seq.store(0, Ordering::SeqCst);
        let me = self.cfg.agent(agent_id).cloned().unwrap_or_default();
        let other = self
            .cfg
            .agents
            .iter()
            .find(|p| p.id != agent_id)
            .cloned()
            .unwrap_or_default();
        self.emit(&Event::TurnBegin {
            turn,
            agent: agent_id.into(),
            ts: now_ts(),
        });

        let memory = match MemoryStore::open(&self.workspace, agent_id) {
            Ok(m) => m,
            Err(e) => {
                self.emit(&Event::TurnError {
                    turn,
                    agent: agent_id.into(),
                    attempt: 1,
                    error: format!("记忆目录不可用:{e}"),
                    retryable: false,
                });
                return TurnOutcome::Unreachable;
            }
        };
        let mem_block = memory.recent_block(12, &self.session);
        let (system, mut stage, truncated) = self.build_context(agent_id, &mem_block);
        // 纪要压缩:只用**本方端点**(D-012);失败退化为纯截断,不终止会话
        if truncated && !self.summaries.contains_key(agent_id) {
            if let Some(sum) = self.compress(agent_id, &me.name, turn).await {
                stage = format!("{}\n\n{stage}", transcript::summary_block(&me.name, &sum));
            }
        }
        if stage.chars().count() > COMPRESS_TRIGGER_CHARS {
            self.emit(&Event::ContextSnapshot {
                turn,
                agent: agent_id.into(),
                bytes: stage.len(),
                compressed: self.summaries.contains_key(agent_id),
            });
        }

        let provider = match self.providers.get(agent_id) {
            Some(p) => p.clone(),
            None => {
                self.emit(&Event::AgentUnreachable {
                    agent: agent_id.into(),
                    consecutive_failures: 3,
                });
                return TurnOutcome::Unreachable;
            }
        };

        let log2 = self.log.clone();
        let ui2 = self.ui.clone();
        let agent_s = agent_id.to_string();
        // 流式增量属于"即将 commit 的那一条发言",所以取的是**还没被分配**的序号。
        // 顺序天然成立:delta 在 provider.chat 内部发出,commit_say 在其后分配。
        let seq_cb = self.say_seq.clone();
        let cb: DeltaCb = Arc::new(move |s: String| {
            let ev = Event::SayDelta {
                turn,
                agent: agent_s.clone(),
                seq: seq_cb.load(Ordering::SeqCst),
                text: s,
            };
            log2.append(&ev);
            ui2.emit(&ev);
        });
        let mut msgs = vec![
            Msg::System(system),
            Msg::User(format!(
                "(第 {turn} 轮,轮到你。台面记录如下,后面没有别人的提示。)\n{stage}"
            )),
        ];

        let mut attempts = 0u32;
        let mut spoke = false;
        let mut did_something = false;

        loop {
            attempts += 1;
            if attempts > MAX_TOOL_CALLS_PER_TURN as u32 {
                // 兜底:一方在一轮里无限要工具
                return if spoke {
                    TurnOutcome::Spoke
                } else {
                    TurnOutcome::Silent
                };
            }
            let req = ChatRequest {
                messages: msgs.clone(),
                tools: self.schemas.clone(),
                temperature: me.temperature,
                max_output_tokens: me.max_output_tokens,
                delta_cb: Some(cb.clone()),
            };
            let call_t0 = Instant::now();
            let res = tokio::time::timeout(
                std::time::Duration::from_millis(self.cfg.globals.turn_timeout_ms.max(3000)),
                provider.chat(req),
            )
            .await
            .unwrap_or_else(|_| {
                Err(ProviderError::Transient(format!(
                    "本轮超过 {}ms",
                    self.cfg.globals.turn_timeout_ms
                )))
            });
            // 这一趟往返花了多久 —— 写进 SayEnd,界面才能给每条发言标耗时
            let call_ms = call_t0.elapsed().as_millis() as u64;

            // (流式增量已由 cb 在 provider 内部同步落盘,不需要轮询)
            let r = match res {
                Err(e) => {
                    self.emit(&Event::TurnError {
                        turn,
                        agent: agent_id.into(),
                        attempt: attempts,
                        error: e.message(),
                        retryable: e.retryable(),
                    });
                    let n = self.fail_count.entry(agent_id.into()).or_insert(0);
                    *n += 1;
                    let fails = *n;
                    if fails >= 3 {
                        self.emit(&Event::AgentUnreachable {
                            agent: agent_id.into(),
                            consecutive_failures: fails,
                        });
                        return TurnOutcome::Unreachable;
                    }
                    if e.retryable() && attempts <= self.cfg.globals.turn_retries {
                        tokio::time::sleep(std::time::Duration::from_millis(400 * attempts as u64))
                            .await;
                        continue;
                    }
                    return TurnOutcome::Silent;
                }
                Ok(r) => {
                    *self.fail_count.entry(agent_id.into()).or_insert(0) = 0;
                    r
                }
            };

            let u = self.usage.entry(agent_id.into()).or_default();
            u.input += r.usage.input;
            u.output += r.usage.output;

            // 一次请求的 token 只记一次账。
            //
            // 内存账本按**请求**累加(上面三行),而磁盘账本是按 SayEnd 求和
            // (transcript::usage_of 与 commands.rs 的场次统计)。
            // 一次响应可能落多条 SayEnd —— 正文一条 + say 工具一条
            // (兼容端点常见),或一轮里多次 say(工具注释明确预期)。
            // 若每条都挂同一份 r.usage,磁盘口径就会翻倍:
            // 续跑时 engine 从磁盘重建账本,数字被放大,
            // budget_tokens 提前触发,会话提前以"token 预算用尽"结束;
            // 同时状态栏(内存口径)与场次列表(磁盘口径)对不上。
            // 所以只有本请求的**第一条**发言携带 usage,其余补零。
            let mut charged = false;

            let has_text = !r.text.trim().is_empty();
            let has_tools = !r.tool_calls.is_empty();

            if !has_text && !has_tools {
                self.emit(&Event::Pass {
                    turn,
                    agent: agent_id.into(),
                });
                return if spoke || did_something {
                    TurnOutcome::Spoke
                } else {
                    TurnOutcome::Silent
                };
            }

            if has_text {
                // 不调 say 时的直接文本也算发言(§7)
                self.commit_say(
                    agent_id,
                    turn,
                    r.text.clone(),
                    take_usage(&mut charged, r.usage),
                    r.finish_reason.clone(),
                    call_ms,
                );
                spoke = true;
                did_something = true;
            }
            if !has_tools {
                return TurnOutcome::Spoke;
            }

            msgs.push(Msg::Assistant {
                text: r.text.clone(),
                tool_calls: r.tool_calls.clone(),
            });

            for (i, call) in r.tool_calls.iter().enumerate() {
                if i >= MAX_TOOL_CALLS_PER_TURN {
                    msgs.push(Msg::Tool {
                        call_id: call.id.clone(),
                        content: "本轮工具次数已达上限 8。要结束就说话或调 pass。".into(),
                    });
                    continue;
                }
                let args: Value =
                    serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
                let o = {
                    let ctx = ToolCtx {
                        workspace: &self.workspace,
                        agent_id,
                        agent_name: &me.name,
                        other_name: &other.name,
                        turn,
                        session: &self.session,
                        memory: &memory,
                        skills: &self.skills,
                        shell: &self.cfg.shell,
                    };
                    let ev_name = call.name.clone();
                    let out = tools::dispatch(&ctx, &ev_name, &args).await;
                    let ev = tools::to_event(&ctx, &call.id, &ev_name, &args, &out);
                    (out, ev)
                };
                let (out, ev) = o;
                self.tool_calls += 1;
                if out.violation.is_some() {
                    self.violations += 1;
                }
                if let Some((path, detail)) = &out.violation {
                    self.emit(&Event::Violation {
                        turn,
                        agent: agent_id.into(),
                        kind: "sandbox".into(),
                        path: path.clone(),
                        detail: detail.clone(),
                    });
                }
                if out.ok && matches!(call.name.as_str(), "write" | "edit" | "delete" | "move") {
                    if let Some(rel) = args
                        .get("path")
                        .or_else(|| args.get("to"))
                        .and_then(Value::as_str)
                    {
                        let key = rel.to_lowercase();
                        if let Some(by) = self.last_touch.get(&key) {
                            if by != agent_id {
                                self.emit(&Event::SystemNote {
                                    turn,
                                    text: format!("{rel} 在你上次看过之后被 {by} 改过。"),
                                });
                            }
                        }
                        self.last_touch.insert(key, agent_id.into());
                    }
                }
                self.emit(&ev);
                msgs.push(Msg::Tool {
                    call_id: call.id.clone(),
                    content: out.full.clone(),
                });

                match &out.act {
                    Some(ChatAct::Say(t)) => {
                        if out.ok {
                            // say 就是正式发言;同一轮里重复 say 视为多条发言,逐条落盘
                            self.commit_say(
                                agent_id,
                                turn,
                                t.clone(),
                                take_usage(&mut charged, r.usage),
                                "say".into(),
                                call_ms,
                            );
                            spoke = true;
                        }
                    }
                    Some(ChatAct::Pass) => {
                        self.emit(&Event::Pass {
                            turn,
                            agent: agent_id.into(),
                        });
                        return if spoke {
                            TurnOutcome::Spoke
                        } else {
                            TurnOutcome::Silent
                        };
                    }
                    Some(ChatAct::CloseTable(reason)) => {
                        self.emit(&Event::SystemNote {
                            turn,
                            text: format!("{} 散会了:{reason}", me.name),
                        });
                        return TurnOutcome::Close;
                    }
                    Some(ChatAct::Withdraw(reason)) => {
                        self.emit(&Event::SystemNote {
                            turn,
                            text: format!("agent_withdrawn:{}:{}", agent_id, reason),
                        });
                        return TurnOutcome::Withdraw;
                    }
                    None => {}
                }
            }
            // 说过话又继续要工具:回到模型,让它决定接下来做什么
        }
    }

    /// 落一条正式发言。
    ///
    /// `usage` 与 `ms` 必须是真的:它们进 events.jsonl,是回放与账本的唯一来源。
    /// 早先这里写死 `UsagePair::default()` 与 `ms: 0`,结果每一局的 token 账
    /// 从磁盘重建出来永远是 0 —— 界面上整场都显示零,而没有任何一处报错。
    fn commit_say(
        &mut self,
        agent_id: &str,
        turn: u32,
        text: String,
        usage: UsagePair,
        finish: String,
        ms: u64,
    ) {
        // 每次 commit 都是一条**独立**的发言,分配一个新序号。
        // 前端用 (turn, agent, seq) 做聚合键,才能把同轮的多条发言分开。
        let seq = self.say_seq.fetch_add(1, Ordering::SeqCst);
        self.emit(&Event::SayEnd {
            turn,
            agent: agent_id.into(),
            seq,
            text,
            usage,
            ms,
            finish_reason: Some(finish),
        });
    }

    /// D-012:压缩请求走 self.providers[agent_id] —— 不用第三方模型,不借对方端点
    async fn compress(&mut self, agent_id: &str, name: &str, turn: u32) -> Option<String> {
        let provider = self.providers.get(agent_id)?.clone();
        let evs = self.history();
        let keep_tail = 16usize;
        if evs.len() <= keep_tail {
            return None;
        }
        let old: Vec<Event> = evs.iter().take(evs.len() - keep_tail).cloned().collect();
        let me = self.cfg.agent(agent_id).cloned()?;
        let other = self.cfg.agents.iter().find(|p| p.id != agent_id).cloned()?;
        let (body, _) = transcript::render_context(
            &old,
            &CtxOpts {
                self_id: agent_id,
                self_name: &me.name,
                other_name: &other.name,
                opening_note: "",
                system_notes: &[],
                budget_chars: 60_000,
            },
        );
        if body.trim().is_empty() {
            return None;
        }
        let prompt = format!(
            "把下面这段台面记录压成不超过 200 字的要点纪要,保留:双方立场、已做的约定、未验证的说法、对 arena/ 文件做过的动作。\n\n{body}"
        );
        let req = ChatRequest::simple(vec![
            Msg::System(format!("你是{name}的专属纪要机器,只输出要点,不评价。")),
            Msg::User(prompt),
        ]);
        // 压缩也必须限时。
        //
        // 以前这里是裸的 `provider.chat(req).await`:reqwest 只设了 connect_timeout,
        // **没有总超时** —— 对方接受连接后不再吐字节(代理/NAT 黑洞、上游卡死),
        // 这个 future 就永远 pending。而 stop 只在轮次之间被观察,
        // 于是「结束」按钮彻底失效、status.running 永远为 true、
        // 再开局只会得到"已经有一个会话在跑",整个应用只能重启。
        // 轮次路径一直是有限时的,压缩这条路径漏了。
        let res = tokio::time::timeout(
            std::time::Duration::from_millis(self.cfg.globals.turn_timeout_ms.max(3000)),
            provider.chat(req),
        )
        .await
        .unwrap_or_else(|_| {
            Err(ProviderError::Transient(format!(
                "压缩超过 {}ms",
                self.cfg.globals.turn_timeout_ms
            )))
        });
        match res {
            Ok(r) => {
                let summary = r.text.trim().to_string();
                if summary.is_empty() {
                    return None;
                }
                self.summaries.insert(agent_id.into(), summary.clone());
                self.emit(&Event::Compress {
                    turn,
                    agent: agent_id.into(),
                    via_model: provider.label(),
                    tokens_in: r.usage.input,
                    tokens_out: r.usage.output,
                });
                Some(summary)
            }
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{MemSink, UsagePair as UP};
    use crate::provider::{ChatResult, ScriptedProvider, ToolCall as PCall};

    #[allow(clippy::field_reassign_with_default)]
    fn cfg_pair() -> AppConfig {
        let mut c = AppConfig::default();
        c.agents = vec![
            AgentProfile {
                id: "A".into(),
                name: "甲".into(),
                company: "DeepSeek".into(),
                base_url: "https://api.deepseek.com".into(),
                api_key: "sk-aaaa".into(),
                model: "m-a".into(),
                temperament: "冷静".into(),
                ..Default::default()
            },
            AgentProfile {
                id: "B".into(),
                name: "乙".into(),
                company: "Moonshot".into(),
                base_url: "https://api.moonshot.cn".into(),
                api_key: "sk-bbbb".into(),
                model: "m-b".into(),
                temperament: "跳脱".into(),
                ..Default::default()
            },
        ];
        c.globals.pause_between_turns_ms = 0;
        c.globals.max_turns = 6;
        c.globals.turn_retries = 2;
        c
    }

    fn ws(tag: &str) -> PathBuf {
        crate::testsup::world(tag)
    }

    fn say(t: &str) -> Result<ChatResult, ProviderError> {
        Ok(ChatResult {
            text: t.into(),
            tool_calls: vec![],
            usage: UP {
                input: 5,
                output: 5,
                cached: None,
            },
            finish_reason: "stop".into(),
        })
    }
    fn pass_step() -> Result<ChatResult, ProviderError> {
        Ok(ChatResult {
            text: "   ".into(),
            tool_calls: vec![],
            usage: UP::default(),
            finish_reason: "stop".into(),
        })
    }
    /// 一次响应**同时**带正文与 say 工具调用。
    ///
    /// 这不是编出来的边角情况:兼容端点(OI 风格)很常见 ——
    /// 模型边说话边调工具。它同时也是 token 账本出问题的那条路径。
    fn text_and_say(text: &str, say_text: &str) -> Result<ChatResult, ProviderError> {
        Ok(ChatResult {
            text: text.into(),
            tool_calls: vec![PCall {
                id: "c-say".into(),
                name: "say".into(),
                arguments: format!(
                    "{{\"text\":{}}}",
                    serde_json::to_string(say_text).unwrap_or_else(|_| "\"\"".into())
                ),
            }],
            usage: UP {
                input: 100,
                output: 50,
                cached: None,
            },
            finish_reason: "tool_calls".into(),
        })
    }

    fn tool(name: &str, args: &str) -> Result<ChatResult, ProviderError> {
        Ok(ChatResult {
            text: String::new(),
            tool_calls: vec![PCall {
                id: format!("c-{name}"),
                name: name.into(),
                arguments: args.into(),
            }],
            usage: UP {
                input: 2,
                output: 2,
                cached: None,
            },
            finish_reason: "tool_calls".into(),
        })
    }

    struct Harness {
        dir: PathBuf,
        ev: PathBuf,
        ui: Arc<MemSink>,
        ctl: Arc<Controls>,
        status: Arc<Mutex<RunStatus>>,
        cfg: AppConfig,
        a: Vec<Result<ChatResult, ProviderError>>,
        b: Vec<Result<ChatResult, ProviderError>>,
    }

    fn harness(
        tag: &str,
        a: Vec<Result<ChatResult, ProviderError>>,
        b: Vec<Result<ChatResult, ProviderError>>,
    ) -> Harness {
        let dir = ws(tag);
        Harness {
            ev: dir.join("events.jsonl"),
            ui: Arc::new(MemSink::default()),
            ctl: Arc::new(Controls::default()),
            status: Arc::new(Mutex::new(RunStatus::default())),
            cfg: cfg_pair(),
            a,
            b,
            dir,
        }
    }

    impl Harness {
        fn engine(&self) -> Engine {
            let log = JsonlLog::open(&self.ev).unwrap();
            let mut m: HashMap<String, SharedProvider> = HashMap::new();
            m.insert(
                "A".into(),
                Arc::new(ScriptedProvider::new(
                    "A",
                    "m-a@api.deepseek.com",
                    "sk-aaaa",
                    self.a.clone(),
                )) as SharedProvider,
            );
            m.insert(
                "B".into(),
                Arc::new(ScriptedProvider::new(
                    "B",
                    "m-b@api.moonshot.cn",
                    "sk-bbbb",
                    self.b.clone(),
                )) as SharedProvider,
            );
            let mut cfg = self.cfg.clone();
            cfg.workspace.root = self.dir.display().to_string().replace('\\', "/");
            Engine::new(
                cfg,
                m,
                self.ui.clone(),
                log,
                self.dir.clone(),
                "s1".into(),
                self.ctl.clone(),
                self.status.clone(),
            )
        }
        fn rm(&self) {
            crate::testsup::cleanup(&self.dir);
        }
    }

    #[tokio::test]
    async fn strict_alternation_then_max_turns() {
        let h = harness(
            "alt",
            vec![say("甲1"), say("甲2"), say("甲3")],
            vec![say("乙1"), say("乙2"), say("乙3")],
        );
        let mut e = h.engine();
        e.start().unwrap();
        let end = e.run(false).await;
        assert_eq!(end, EndReason::MaxTurns);
        let says: Vec<(u32, String)> =
            h.ui.events()
                .iter()
                .filter_map(|x| match x {
                    Event::SayEnd { turn, agent, .. } => Some((*turn, agent.clone())),
                    _ => None,
                })
                .collect();
        assert_eq!(says.len(), 6, "{says:?}");
        assert_eq!(says[0], (1, "A".into()), "A 先说");
        assert_eq!(says[1], (2, "B".into()));
        assert_eq!(says[4], (5, "A".into()));
        // 磁盘上也是同一份事实
        let disk = JsonlLog::read_all(&h.ev);
        assert!(disk
            .iter()
            .any(|x| matches!(x, Event::SessionEnd { reason, .. } if reason == "max_turns")));
        assert_eq!(h.status.lock().unwrap().turn, 6);
        h.rm();
    }

    #[tokio::test]
    async fn cold_field_needs_three_rounds_both_silent() {
        let mut h = harness(
            "cold",
            vec![
                say("开场一下"),
                pass_step(),
                pass_step(),
                pass_step(),
                pass_step(),
                pass_step(),
            ],
            vec![
                pass_step(),
                pass_step(),
                pass_step(),
                pass_step(),
                pass_step(),
                pass_step(),
            ],
        );
        h.cfg.globals.max_turns = 20;
        h.cfg.globals.cold_field_rounds = 3;
        let mut e = h.engine();
        e.start().unwrap();
        let end = e.run(false).await;
        assert_eq!(end, EndReason::ColdField, "{end:?}");
        assert_eq!(e.controls.turn.load(Ordering::SeqCst), 7, "第 7 轮触发冷场");
        h.rm();
    }

    #[tokio::test]
    async fn agents_can_end_it_themselves() {
        let h = harness(
            "close",
            vec![
                tool("close_table", r#"{"reason":"聊够了,散会"}"#),
                say("不该再被叫"),
            ],
            vec![say("乙好")],
        );
        let mut e = h.engine();
        e.start().unwrap();
        let end = e.run(false).await;
        assert_eq!(end, EndReason::AgentClose);
        assert!(h
            .ui
            .has(|ev| matches!(ev, Event::SystemNote { text, .. } if text.contains("散会"))));
        assert_eq!(e.controls.turn.load(Ordering::SeqCst), 1);
        let disk = JsonlLog::read_all(&h.ev);
        assert!(disk
            .iter()
            .any(|x| matches!(x, Event::SessionEnd { reason, .. } if reason == "agent_close")));
        h.rm();
    }

    #[tokio::test]
    async fn write_tool_lands_in_arena_and_is_public_summary() {
        let h = harness(
            "tool",
            vec![
                tool("write", r#"{"path":"arena/rps.py","content":"print(1)"}"#),
                say("我写了个 RPS,你来挑刺"),
            ],
            vec![say("行,我去读")],
        );
        let mut e = h.engine();
        e.start().unwrap();
        e.run(false).await;
        assert!(h.dir.join("arena/rps.py").exists());
        let tc =
            h.ui.events()
                .iter()
                .find_map(|x| match x {
                    Event::ToolCall {
                        name,
                        preview,
                        agent,
                        ..
                    } => Some((name.clone(), preview.clone(), agent.clone())),
                    _ => None,
                })
                .expect("应有工具事件");
        assert_eq!(tc.0, "write");
        assert_eq!(tc.2, "A");
        assert!(tc.1.contains("arena/rps.py"), "{}", tc.1);
        // 关键隔离断言:A 写的内容不进 B 的上下文
        let (sys_b, stage_b, _) = e.build_context("B", "你的私有记忆:空。");
        assert!(
            !stage_b.contains("print(1)"),
            "对方的文件内容不得直接进本方上下文:{stage_b}"
        );
        assert!(
            stage_b.contains("[动作·甲] write"),
            "应保留一行摘要:{stage_b}"
        );
        assert!(!sys_b.contains("sk-aaaa"), "B 的提示不得含 A 的 key");
        h.rm();
    }

    #[tokio::test]
    async fn peeking_other_memory_is_public_violation_and_never_leaks() {
        let h = harness(
            "peek",
            vec![
                tool("read", r#"{"path":"memory/B/notes.jsonl"}"#),
                say("我就是看看"),
            ],
            vec![say("别偷看")],
        );
        let mb = MemoryStore::open(&h.dir, "B").unwrap();
        mb.append(&crate::memory::Note {
            ts: "t".into(),
            session: "s0".into(),
            turn: 1,
            text: "B 的私密念头:我其实怕水".into(),
            tags: vec![],
        })
        .unwrap();
        let mut e = h.engine();
        e.start().unwrap();
        e.run(false).await;
        let v =
            h.ui.events()
                .iter()
                .find_map(|x| match x {
                    Event::Violation {
                        path,
                        agent,
                        detail,
                        ..
                    } => Some((path.clone(), agent.clone(), detail.clone())),
                    _ => None,
                })
                .expect("必须记 violation");
        assert_eq!(v.0, "memory/B/notes.jsonl");
        assert_eq!(v.1, "A");
        assert!(v.2.contains("私有记忆"), "{}", v.2);
        assert_eq!(e.violations, 1);
        // A 的上下文里绝不能出现 B 的记忆内容
        let (_, stage_a, _) = e.build_context("A", "你的私有记忆:空。");
        assert!(!stage_a.contains("怕水"), "泄漏:{stage_a}");
        // 但"有人偷看"这件事是公开的
        assert!(stage_a.contains("被拒绝"), "{stage_a}");
        h.rm();
    }

    #[tokio::test]
    async fn one_side_failure_does_not_pollute_the_other() {
        let h = harness(
            "fail",
            vec![
                Err(ProviderError::Transient("连不上".into())),
                Err(ProviderError::Transient("连不上".into())),
                Err(ProviderError::Transient("连不上".into())),
                say("甲:回来了"),
                say("甲x"),
            ],
            vec![say("乙a"), say("乙b"), say("乙c")],
        );
        let mut e = h.engine();
        e.start().unwrap();
        let end = e.run(false).await;
        assert_eq!(
            end,
            EndReason::AgentUnreachable,
            "A 三次连续失败应退场:{end:?}"
        );
        let b_errs =
            h.ui.events()
                .iter()
                .filter(|x| matches!(x, Event::TurnError { agent, .. } if agent == "B"))
                .count();
        assert_eq!(b_errs, 0, "B 不该有任何失败");
        assert_eq!(e.usage.get("A").unwrap().input, 0, "失败不产生用量");
        // A 是先手。严格交替下 A 连败三次就直接退场,第一轮里 B 根本没轮到 ——
        // 所以 B 的用量理应是 0。这条断言真正要守的是"B 没有被 A 的失败污染",
        // 而不是"B 花过 token"。
        assert_eq!(e.usage.get("B").unwrap().input, 0, "B 没轮到过,不该有用量");
        h.rm();
    }

    #[tokio::test]
    async fn budget_cuts_the_run() {
        let mut h = harness("budget", vec![say("x"); 9], vec![say("y"); 9]);
        h.cfg.globals.budget_tokens = 30;
        h.cfg.globals.max_turns = 40;
        let mut e = h.engine();
        e.start().unwrap();
        let end = e.run(false).await;
        assert_eq!(end, EndReason::Budget);
        h.rm();
    }

    #[tokio::test]
    async fn user_stop_ends_it() {
        let mut h = harness("stop", vec![say("x"); 20], vec![say("y"); 20]);
        h.cfg.globals.max_turns = 40;
        h.cfg.globals.pause_between_turns_ms = 40;
        let ctl = h.ctl.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            ctl.stop.store(true, Ordering::SeqCst);
        });
        let mut e = h.engine();
        e.start().unwrap();
        let end = e.run(false).await;
        assert_eq!(end, EndReason::UserStop);
        assert!(e.controls.turn.load(Ordering::SeqCst) < 15);
        h.rm();
    }

    #[tokio::test]
    async fn pause_holds_but_resumes() {
        let mut h = harness("pause", vec![say("x"); 10], vec![say("y"); 10]);
        h.cfg.globals.max_turns = 4;
        h.cfg.globals.pause_between_turns_ms = 60;
        let ctl = h.ctl.clone();
        ctl.paused.store(true, Ordering::SeqCst);
        let c2 = ctl.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            c2.paused.store(false, Ordering::SeqCst);
        });
        let t = Instant::now();
        let mut e = h.engine();
        e.start().unwrap();
        let end = e.run(false).await;
        assert_eq!(end, EndReason::MaxTurns);
        assert!(t.elapsed().as_millis() >= 190, "暂停期间应真的停住");
        h.rm();
    }

    #[tokio::test]
    async fn independence_gate_blocks_default_config() {
        let h = harness("gate", vec![say("x")], vec![say("y")]);
        let mut cfg = AppConfig::default();
        cfg.workspace.root = h.dir.display().to_string().replace('\\', "/");
        let log = JsonlLog::open(&h.ev).unwrap();
        let mut m: HashMap<String, SharedProvider> = HashMap::new();
        m.insert(
            "A".into(),
            Arc::new(ScriptedProvider::new("A", "x", "k", vec![])) as SharedProvider,
        );
        m.insert(
            "B".into(),
            Arc::new(ScriptedProvider::new("B", "y", "k", vec![])) as SharedProvider,
        );
        let mut e = Engine::new(
            cfg,
            m,
            h.ui.clone(),
            log,
            h.dir.clone(),
            "s".into(),
            h.ctl.clone(),
            h.status.clone(),
        );
        let err = e.start().unwrap_err();
        assert!(
            err.iter()
                .all(|v| matches!(v, Violation::Incomplete { .. })),
            "{err:?}"
        );
        assert!(
            !e.controls.running.load(Ordering::SeqCst),
            "校验失败绝不能进入运行态"
        );
        assert_eq!(
            h.status.lock().unwrap().end_reason.as_deref(),
            Some("config_invalid")
        );
        h.rm();
    }

    #[tokio::test]
    async fn independence_gate_blocks_same_host() {
        let mut h = harness("samehost", vec![say("x")], vec![say("y")]);
        h.cfg.agents[1].base_url = "https://API.deepseek.com/v1".into();
        h.cfg.agents[1].api_key = "sk-cccc".into();
        let mut e = h.engine();
        let err = e.start().unwrap_err();
        assert!(
            err.iter().any(|v| matches!(v, Violation::SameHost { .. })),
            "{err:?}"
        );
        assert!(!e.controls.running.load(Ordering::SeqCst));
        h.rm();
    }

    #[tokio::test]
    async fn resume_reads_turn_from_disk() {
        let h = harness(
            "resume",
            vec![say("甲2"), say("甲3"), say("甲4")],
            vec![say("乙2"), say("乙3")],
        );
        {
            let log = JsonlLog::open(&h.ev).unwrap();
            log.append(&Event::SessionOpen {
                session_id: "s1".into(),
                agents: vec![],
                mode: "independent".into(),
                opening_note: "n".into(),
                ts: "t".into(),
            });
            log.append(&Event::SayEnd {
                turn: 1,
                agent: "A".into(),
                seq: 0,
                text: "甲1 已在磁盘上".into(),
                usage: UP {
                    input: 11,
                    output: 11,
                    cached: None,
                },
                ms: 1,
                finish_reason: None,
            });
        }
        let mut e = h.engine();
        e.start().unwrap();
        let end = e.run(true).await;
        assert_eq!(e.controls.turn.load(Ordering::SeqCst), 6, "{end:?}");
        let dup =
            h.ui.events()
                .iter()
                .filter(|x| matches!(x, Event::SayEnd { turn: 1, .. }))
                .count();
        assert_eq!(dup, 0, "续跑不该重写已落盘的第 1 轮");
        // 续跑时第 1 轮的发言确实进了新上下文
        let (_, stage, _) = e.build_context("B", "你的私有记忆:空。");
        assert!(stage.contains("甲1 已在磁盘上"), "{stage}");
        // 11 是磁盘上第 1 轮那一条的用量;续跑后 A 又说了两轮(每轮 input=5),
        // 所以账本应当是 11 + 5 + 5 = 21 —— **接上**而不是从零开始。
        assert_eq!(
            e.usage.get("A").unwrap().input,
            21,
            "续跑要在磁盘用量之上继续累加"
        );
        h.rm();
    }

    #[tokio::test]
    async fn compression_uses_own_endpoint_only() {
        let mut h = harness(
            "compress",
            vec![say(&"很长的发言。".repeat(3000)); 8],
            vec![say("乙收到"); 8],
        );
        h.cfg.globals.max_turns = 8;
        let mut e = h.engine();
        e.start().unwrap();
        // 压缩的前置条件是历史超过保留尾巴,所以先真的把对话跑起来,再测端点归属。
        let _ = e.run(false).await;
        // 重新取一个只读句柄:run() 之后的历史已经落盘。
        let mut e = h.engine();
        e.start().unwrap();
        let provider_a = e.providers.get("A").unwrap().label();
        assert!(provider_a.contains("deepseek"), "{provider_a}");
        let mem = MemoryStore::open(&h.dir, "A")
            .unwrap()
            .recent_block(12, "s1");
        let (sys, stage, trunc) = e.build_context("A", &mem);
        assert!(sys.contains("甲"), "{sys}");
        let _ = (stage, trunc);
        // 台面超长 → 触发压缩时只调 A 的 provider
        let before = e.providers.get("B").unwrap().key_fingerprint();
        let r = e.compress("A", "甲", 1).await;
        assert!(r.is_some(), "超长应能压出纪要");
        let after = e.providers.get("B").unwrap().key_fingerprint();
        assert_eq!(before, after, "压缩不得触碰 B 的 client");
        assert!(
            h.ui.has(|ev| matches!(ev, Event::Compress { agent, .. } if agent == "A")),
            "压缩要记账"
        );
        h.rm();
    }

    #[tokio::test]
    async fn opening_note_carries_both_companies_and_models() {
        let h = harness("note", vec![], vec![]);
        let e = h.engine();
        let a = e.cfg.agents[0].clone();
        let b = e.cfg.agents[1].clone();
        let note = e.opening_note(&a, &b);
        assert!(
            note.contains("DeepSeek") && note.contains("Moonshot"),
            "{note}"
        );
        assert!(note.contains("m-a") && note.contains("m-b"), "{note}");
        assert!(note.contains("乐园"), "{note}");
        assert!(!note.contains('{'), "占位符必须全部替换:{note}");
        h.rm();
    }

    #[tokio::test]
    async fn system_prompt_states_the_three_isolation_rules() {
        let h = harness("sp", vec![], vec![]);
        let mut e = h.engine();
        // 必须先开局:技能目录是 start() 里落盘并载入的,而系统提示要注入技能清单。
        // 生产路径永远是"开局之后再取提示",所以这里就按生产路径来。
        e.start().unwrap();
        let a = e.cfg.agents[0].clone();
        let b = e.cfg.agents[1].clone();
        let sys = e.system_prompt(&a, &b, "你的私有记忆:空。");
        assert!(sys.contains("没有人类会回答你"), "{sys}");
        assert!(sys.contains("自己去 read"), "要看内容必须自己读:{sys}");
        assert!(sys.contains("越界会被公开拒绝"), "{sys}");
        assert!(
            sys.contains("互相面试") || sys.contains("interview"),
            "技能目录应注入:{sys}"
        );
        assert!(sys.contains("独立"), "{sys}");
        h.rm();
    }

    /// 回归:同一轮内的多条发言必须能区分开。
    ///
    /// 一轮里模型可以先说一句、调个工具、再说一句(say 工具的注释写明了
    /// "同一轮里重复 say 视为多条发言,逐条落盘")。这些 SayEnd 的
    /// (turn, agent) 完全一样,前端以前就是靠这两个字段归并的,
    /// 于是后一条把前一条覆盖掉 —— 台面上少了一整段发言。
    /// seq 就是给它们发身份的那个字段。
    #[test]
    fn repeated_says_in_one_turn_get_distinct_seq() {
        let h = harness("sayseq", vec![], vec![]);
        let mut e = h.engine();
        e.start().unwrap();

        // 同一轮、同一方,连发三条
        for t in ["第一句", "第二句", "第三句"] {
            e.commit_say(
                "A",
                1,
                t.into(),
                UsagePair {
                    input: 1,
                    output: 1,
                    cached: None,
                },
                "stop".into(),
                5,
            );
        }

        let seqs: Vec<u32> =
            h.ui.events()
                .iter()
                .filter_map(|ev| match ev {
                    Event::SayEnd { seq, .. } => Some(*seq),
                    _ => None,
                })
                .collect();
        assert_eq!(
            seqs,
            vec![0, 1, 2],
            "同轮多条发言必须拿到递增且互不相同的 seq"
        );

        // 文本也要各归各的,不能被覆盖
        let texts: Vec<String> =
            h.ui.events()
                .iter()
                .filter_map(|ev| match ev {
                    Event::SayEnd { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect();
        assert_eq!(texts, vec!["第一句", "第二句", "第三句"]);

        // 换一轮要重新从 0 起
        let mut e2 = h.engine();
        e2.start().unwrap();
        e2.say_seq.store(0, Ordering::SeqCst);
        e2.commit_say(
            "A",
            2,
            "新一轮".into(),
            UsagePair {
                input: 1,
                output: 1,
                cached: None,
            },
            "stop".into(),
            5,
        );
        let last =
            h.ui.events()
                .iter()
                .rev()
                .find_map(|ev| match ev {
                    Event::SayEnd { seq, .. } => Some(*seq),
                    _ => None,
                })
                .unwrap();
        assert_eq!(last, 0, "新的一轮,发言序号要从 0 重来");

        h.rm();
    }

    /// 回归:一次响应的 token 只能记一次账。
    ///
    /// 内存账本按**请求**累加,磁盘账本(transcript::usage_of / 场次统计)
    /// 按 SayEnd 求和。一次响应可能落多条 SayEnd(正文一条 + say 工具一条,
    /// 或一轮多次 say)。修复前每条 SayEnd 都挂着同一份 r.usage,
    /// 于是磁盘口径翻倍 —— 续跑时从磁盘重建账本被放大,
    /// budget_tokens 提前触发,会话提前以"token 预算用尽"结束。
    #[tokio::test]
    async fn one_response_charges_tokens_once() {
        let mut h = harness("tokonce", vec![], vec![]);
        // 第 1 次往返:正文 + say 工具调用(同一份 usage 被两个 commit 争用)
        // 第 2 次往返:普通收尾
        h.a = vec![text_and_say("正文一句", "工具里再说一句"), say("收尾")];
        h.b = vec![say("乙说")];

        let mut e = h.engine();
        e.start().unwrap();
        let _ = e.run_turn("A", 1).await;

        let mem = e.usage.get("A").cloned().unwrap_or_default();
        let disk = crate::transcript::usage_of(&e.history(), "A");
        assert_eq!(
            (mem.input, mem.output),
            (disk.input, disk.output),
            "内存账本(按请求)与磁盘账本(按 SayEnd)必须一致;\
             不一致就意味着续跑时 token 会被放大,预算提前耗尽"
        );

        // 具体数值也要对:150 + 10 = 160,而不是翻倍的 310
        assert_eq!(mem.total(), 160, "预期 100+50 与 5+5 各记一次");
        h.rm();
    }

    // Each participant has an independent scripted queue, even on the same endpoint/model.
    fn group_engine(
        tag: &str,
        ids: &[&str],
        scripts: Vec<Vec<Result<ChatResult, ProviderError>>>,
        max_turns: u32,
    ) -> (Engine, PathBuf) {
        assert_eq!(ids.len(), scripts.len());
        let dir = ws(tag);
        let mut cfg = cfg_pair();
        cfg.workspace.mode = AccessMode::SharedEndpoint;
        cfg.workspace.root = dir.display().to_string();
        cfg.globals.max_turns = max_turns;
        let template = cfg.agents[0].clone();
        cfg.agents = ids
            .iter()
            .map(|id| AgentProfile {
                id: (*id).into(),
                name: format!("角色 {id}"),
                temperament: format!("{id} 的独立人设"),
                ..template.clone()
            })
            .collect();
        let providers = ids
            .iter()
            .zip(scripts)
            .map(|(id, steps)| {
                (
                    (*id).to_string(),
                    Arc::new(ScriptedProvider::new(
                        id,
                        "same-model",
                        "local-test-key",
                        steps,
                    )) as SharedProvider,
                )
            })
            .collect();
        let log = JsonlLog::open(&dir.join("events.jsonl")).unwrap();
        let engine = Engine::new(
            cfg,
            providers,
            Arc::new(MemSink::default()),
            log,
            dir.clone(),
            "group-test".into(),
            Arc::new(Controls::default()),
            Arc::new(Mutex::new(RunStatus::default())),
        );
        (engine, dir)
    }

    fn group_turns(events: &[Event]) -> Vec<(u32, String)> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::TurnBegin { turn, agent, .. } => Some((*turn, agent.clone())),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn group_acceptance_sixteen_follow_selected_order_for_two_cycles() {
        // Deliberately use non-lexical order to prove selection order survives.
        let ids = [
            "p16", "p03", "p11", "p01", "p09", "p02", "p15", "p04", "p14", "p05", "p13", "p06",
            "p12", "p07", "p10", "p08",
        ];
        let scripts = ids.iter().map(|id| vec![say(id); 2]).collect();
        let (mut engine, dir) = group_engine("group-sixteen", &ids, scripts, 32);
        engine.start().unwrap();
        let end = engine.run(false).await;
        let events = engine.history();
        crate::testsup::cleanup(&dir);
        assert_eq!(end, EndReason::MaxTurns);
        let expected: Vec<_> = ids
            .iter()
            .cycle()
            .take(32)
            .enumerate()
            .map(|(i, id)| (i as u32 + 1, (*id).to_string()))
            .collect();
        assert_eq!(group_turns(&events), expected);
        let says: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::SayEnd {
                    turn, agent, text, ..
                } => Some((*turn, agent.clone(), text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            says,
            expected
                .into_iter()
                .map(|(turn, id)| (turn, id.clone(), id))
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn group_acceptance_withdraw_removes_only_caller_and_keeps_next_in_order() {
        let (mut engine, dir) = group_engine(
            "group-withdraw",
            &["C", "A", "B"],
            vec![
                vec![
                    tool("withdraw", r#"{"reason":"本人下线"}"#),
                    say("退场后不应发言"),
                ],
                vec![say("A继续"); 3],
                vec![say("B继续"); 3],
            ],
            7,
        );
        engine.start().unwrap();
        let end = engine.run(false).await;
        let events = engine.history();
        let active = engine.active.clone();
        crate::testsup::cleanup(&dir);
        assert_eq!(end, EndReason::MaxTurns);
        assert_eq!(
            active,
            ["A".to_string(), "B".to_string()].into_iter().collect()
        );
        assert!(events.iter().any(|event| matches!(event,
            Event::ToolCall { agent, name, ok: true, .. } if agent == "C" && name == "withdraw")));
        let expected: Vec<_> = ["C", "A", "B", "A", "B", "A", "B"]
            .iter()
            .enumerate()
            .map(|(i, id)| (i as u32 + 1, (*id).to_string()))
            .collect();
        assert_eq!(group_turns(&events), expected);
    }

    #[tokio::test]
    async fn group_acceptance_withdraw_ends_when_fewer_than_two_remain() {
        let (mut engine, dir) = group_engine(
            "group-last-withdraw",
            &["A", "B"],
            vec![
                vec![tool("withdraw", r#"{"reason":"离开"}"#)],
                vec![say("不应独自继续")],
            ],
            8,
        );
        engine.start().unwrap();
        let end = engine.run(false).await;
        let events = engine.history();
        let status = engine.status.lock().unwrap().clone();
        crate::testsup::cleanup(&dir);
        assert_eq!(end, EndReason::AgentClose);
        assert_eq!(group_turns(&events), vec![(1, "A".into())]);
        assert!(!status.running);
        assert_eq!(status.end_reason.as_deref(), Some("agent_close"));
        assert!(events.iter().any(|event| matches!(event,
            Event::SessionEnd { reason, .. } if reason == "agent_close")));
    }

    #[tokio::test]
    async fn group_acceptance_current_matches_last_completed_actor_after_withdrawal() {
        struct StatusAtThirdTurn {
            status: Arc<Mutex<RunStatus>>,
            observed: Mutex<Option<RunStatus>>,
        }
        impl EventSink for StatusAtThirdTurn {
            fn emit(&self, event: &Event) {
                if matches!(event, Event::TurnBegin { turn: 3, .. }) {
                    *self.observed.lock().unwrap() = Some(self.status.lock().unwrap().clone());
                }
            }
        }
        let (mut engine, dir) = group_engine(
            "group-current-withdraw",
            &["C", "A", "B"],
            vec![
                vec![tool("withdraw", r#"{"reason":"本人离开"}"#)],
                vec![say("A完成第二轮")],
                vec![say("B第三轮接着说")],
            ],
            3,
        );
        let observer = Arc::new(StatusAtThirdTurn {
            status: engine.status.clone(),
            observed: Mutex::new(None),
        });
        engine.ui = observer.clone();
        engine.start().unwrap();
        let end = engine.run(false).await;
        let turns = group_turns(&engine.history());
        let observed = observer.observed.lock().unwrap().clone();
        crate::testsup::cleanup(&dir);
        assert_eq!(end, EndReason::MaxTurns);
        assert_eq!(
            turns,
            vec![(1, "C".into()), (2, "A".into()), (3, "B".into())]
        );
        // current describes the last completed turn, observed before turn 3 finishes.
        let observed = observed.expect("第三轮开始必须留下共享状态快照");
        assert!(observed.running);
        assert_eq!(observed.turn, 2);
        assert_eq!(
            observed.current.as_deref(),
            Some(turns[1].1.as_str()),
            "退出后 current 必须与上一已完成轮的真实 actor 一致"
        );
    }
    #[tokio::test]
    async fn group_acceptance_resume_continues_selected_order_without_repeating_turns() {
        let (mut engine, dir) = group_engine(
            "group-resume-order",
            &["C", "A", "B"],
            vec![vec![say("C"); 4], vec![say("A"); 4], vec![say("B"); 4]],
            4,
        );
        engine.start().unwrap();
        assert_eq!(engine.run(false).await, EndReason::MaxTurns);
        let mut cfg = engine.cfg.clone();
        cfg.globals.max_turns = 8;
        let providers = engine.providers.clone();
        drop(engine);
        let mut resumed = Engine::new(
            cfg,
            providers,
            Arc::new(MemSink::default()),
            JsonlLog::open(&dir.join("events.jsonl")).unwrap(),
            dir.clone(),
            "group-test".into(),
            Arc::new(Controls::default()),
            Arc::new(Mutex::new(RunStatus::default())),
        );
        resumed.start().unwrap();
        let end = resumed.run(true).await;
        let events = resumed.history();
        crate::testsup::cleanup(&dir);
        assert_eq!(end, EndReason::MaxTurns);
        let expected: Vec<_> = ["C", "A", "B", "C", "A", "B", "C", "A"]
            .iter()
            .enumerate()
            .map(|(i, id)| (i as u32 + 1, (*id).to_string()))
            .collect();
        assert_eq!(group_turns(&events), expected);
    }

    #[tokio::test]
    async fn group_acceptance_resume_preserves_withdrawal_and_next_selected_actor() {
        let (mut engine, dir) = group_engine(
            "group-resume-withdraw",
            &["C", "A", "B"],
            vec![
                vec![
                    tool("withdraw", r#"{"reason":"离开"}"#),
                    say("退场者不应回来"),
                ],
                vec![say("A继续"); 3],
                vec![say("B继续"); 3],
            ],
            1,
        );
        engine.start().unwrap();
        assert_eq!(engine.run(false).await, EndReason::MaxTurns);
        let cfg = {
            let mut cfg = engine.cfg.clone();
            cfg.globals.max_turns = 5;
            cfg
        };
        let providers = engine.providers.clone();
        drop(engine);
        let mut resumed = Engine::new(
            cfg,
            providers,
            Arc::new(MemSink::default()),
            JsonlLog::open(&dir.join("events.jsonl")).unwrap(),
            dir.clone(),
            "group-test".into(),
            Arc::new(Controls::default()),
            Arc::new(Mutex::new(RunStatus::default())),
        );
        resumed.start().unwrap();
        let end = resumed.run(true).await;
        let events = resumed.history();
        let active = resumed.active.clone();
        crate::testsup::cleanup(&dir);
        assert_eq!(end, EndReason::MaxTurns);
        assert_eq!(
            active,
            ["A".to_string(), "B".to_string()].into_iter().collect(),
            "退场状态必须从磁盘恢复"
        );
        let expected = vec![
            (1, "C".into()),
            (2, "A".into()),
            (3, "B".into()),
            (4, "A".into()),
            (5, "B".into()),
        ];
        assert_eq!(
            group_turns(&events),
            expected,
            "续跑不得重复轮次或重新呼叫退场者"
        );
    }
    #[test]
    fn end_reason_codes_stable() {
        assert_eq!(EndReason::AgentClose.code(), "agent_close");
        assert_eq!(EndReason::ColdField.code(), "cold_field");
        assert_eq!(EndReason::Budget.code(), "budget");
        assert_eq!(EndReason::UserStop.code(), "user_stop");
        assert_eq!(EndReason::ConfigInvalid.code(), "config_invalid");
        assert!(EndReason::AgentClose.human().contains("散会"));
    }

    /// 回归:结局是共享状态,新一局开始必须复位。
    ///
    /// RunStatus 跨会话复用(commands.rs 里是一个 Arc),而 end_reason 以前
    /// 只在写入时设置、从不清理 —— 上一局结束后再开新局,侧栏会整场挂着
    /// 上一局的"结局:到达轮次上限"。
    #[tokio::test]
    async fn end_reason_is_reset_when_a_new_session_starts() {
        let mut h = harness("endreset", vec![say("甲说"); 4], vec![say("乙说"); 4]);
        h.cfg.globals.max_turns = 2;

        let mut e = h.engine();
        e.start().unwrap();
        let _ = e.run(false).await;
        assert!(
            h.status.lock().unwrap().end_reason.is_some(),
            "跑完一局应该留下结局"
        );

        // 同一个共享 status,再开一局
        let mut e2 = h.engine();
        e2.start().unwrap();
        let s = h.status.lock().unwrap();
        assert!(
            s.end_reason.is_none() && s.end_text.is_none(),
            "新一局开始时必须清掉上一局的结局,否则界面会显示错误的结局"
        );
        drop(s);
        h.rm();
    }

    /// 回归:工作区骨架要按**配置里的 id** 建,不能硬编码 A/B。
    ///
    /// 注意测试脚手架 `testsup::world` 自己会预建 memory/A、memory/B,
    /// 所以必须先把它们删掉,断言才是在测 Engine 而不是测脚手架。
    #[tokio::test]
    async fn workspace_memory_dirs_follow_configured_ids() {
        let mut h = harness("memids", vec![], vec![]);
        h.cfg.agents[0].id = "left".into();
        h.cfg.agents[1].id = "right".into();

        // 清掉脚手架预建的目录,只留下"Engine 自己建了什么"这一个变量
        std::fs::remove_dir_all(h.dir.join("memory")).expect("清掉脚手架预建的 memory");

        let mut e = h.engine();
        e.start().unwrap();

        assert!(h.dir.join("memory/left").is_dir(), "应按配置的 id 建目录");
        assert!(h.dir.join("memory/right").is_dir(), "应按配置的 id 建目录");
        assert!(
            !h.dir.join("memory/A").exists(),
            "不该再硬编码 memory/A —— 那会在文件树里留下用不上的垃圾区"
        );
        assert!(!h.dir.join("memory/B").exists(), "同上");
        h.rm();
    }
}
