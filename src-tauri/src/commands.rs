//! Tauri 命令层:前端唯一能调的东西。
//!
//! **这里没有任何"往对话里塞文本"的命令**(D-004):前端拿不到通往台面的写入通道,
//! 只有 start / pause / resume / stop / observe。约束在 API 面上就成立,不靠界面自觉。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::config::{self, AccessMode, AppConfig};
use crate::engine::{Controls, Engine, RunStatus};
use crate::events::{now_ts, Event, EventSink, JsonlLog};
use crate::memory::MemoryStore;
use crate::provider::{fetch_models, ProviderClient, SharedProvider};
use crate::sandbox::{self};
use crate::skills::{self, SkillMeta};
use crate::validate::{norm_host, validate_app};

pub type ErrStr = String;

/// 事件同时推给前端(直播)。落盘由引擎自己的 JsonlLog 负责,这里不重复写。
///
/// 前端按 **JSONL 行字符串** 订阅:`app.emit` 的 payload 是一条序列化好的事件行,
/// 与 `events_of` / `events` 返回的原始行同构 —— 直播与回放走同一条解析管线。
struct UiSink {
    app: AppHandle,
}

impl EventSink for UiSink {
    fn emit(&self, ev: &Event) {
        if let Ok(line) = serde_json::to_string(ev) {
            let _ = self.app.emit("aiclub://event", line);
        }
    }
}

#[derive(Default)]
pub struct RunningState {
    pub controls: Mutex<Option<Arc<Controls>>>,
    /// 引擎直接写这个 Arc;status() 直接读同一份 —— 不需要轮询同步
    pub status: Arc<Mutex<RunStatus>>,
    pub session: Mutex<Option<String>>,
    pub log_path: Mutex<Option<PathBuf>>,
}

#[derive(Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub events: usize,
    pub turns: u32,
    pub end_reason: Option<String>,
    pub a: String,
    pub b: String,
    pub host_a: String,
    pub host_b: String,
    pub mode: String,
    pub participants: Vec<crate::events::AgentBrief>,
    pub tokens: u64,
    pub ms_total: u64,
    pub unix: i64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TreeEntry {
    pub path: String,
    pub dir: bool,
    pub bytes: u64,
    pub zone: String,
}

#[derive(Serialize)]
pub struct AppInfo {
    pub version: String,
    pub os: String,
    pub config_path: String,
    pub workspace_default: String,
    pub sessions_dir: String,
    pub persona_default: String,
}

#[derive(Serialize)]
pub struct StartAck {
    pub session: String,
    pub workspace: String,
    pub host_a: String,
    pub host_b: String,
    pub mode: String,
    pub same_origin: bool,
}

// ─────────────────────────── 应用信息 / 配置 ───────────────────────────

pub fn default_workspace() -> PathBuf {
    let base = dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("AiClub").join("world")
}

fn workspace_root(cfg: &AppConfig) -> PathBuf {
    let raw = cfg.workspace.root.trim();
    if raw.is_empty() {
        default_workspace()
    } else {
        PathBuf::from(raw)
    }
}

fn abs_path(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// 建工作区骨架。沙箱根必须真实存在,canonicalize 才有意义。
pub fn ensure_workspace(root: &Path, ids: &[String]) {
    let _ = std::fs::create_dir_all(root.join("arena"));
    let _ = std::fs::create_dir_all(root.join("skills"));
    for id in ids.iter().filter(|s| !s.trim().is_empty()) {
        let _ = std::fs::create_dir_all(root.join("memory").join(id.as_str()));
    }
    skills::seed_builtin(&root.join("skills"));
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os: if cfg!(windows) {
            "windows".into()
        } else if cfg!(target_os = "macos") {
            "macos".into()
        } else {
            "linux".into()
        },
        config_path: config::config_path()
            .unwrap_or_else(|_| "config.toml".into())
            .display()
            .to_string()
            .replace('\\', "/"),
        workspace_default: abs_path(&default_workspace()),
        sessions_dir: abs_path(&config::sessions_dir().unwrap_or_default()),
        persona_default: config::DEFAULT_PERSONA.into(),
    }
}

#[tauri::command]
pub fn get_config() -> AppConfig {
    let mut cfg = config::load();
    if cfg.workspace.root.trim().is_empty() {
        cfg.workspace.root = abs_path(&default_workspace());
    }
    cfg
}

#[tauri::command]
pub fn save_config(cfg: AppConfig) -> Result<String, ErrStr> {
    // 允许存草稿:独立性硬闸是开局时的事,这里只保证能落盘
    config::save(&cfg)
}

/// 开局预检:把违规逐条转成人话给界面显示(界面据此禁用开始按钮)
#[tauri::command]
pub fn check_pair(cfg: AppConfig) -> Vec<String> {
    validate_app(&cfg)
        .iter()
        .map(|v| v.human_message())
        .collect()
}

#[tauri::command]
pub async fn detect(base_url: String, api_key: String) -> Result<Vec<String>, ErrStr> {
    if base_url.trim().is_empty() || api_key.trim().is_empty() {
        return Err("请先填写接口地址与 API Key".into());
    }
    fetch_models(base_url.trim(), api_key.trim()).await
}

/// 确认(并创建)工作区,返回它的绝对路径
#[tauri::command]
pub fn touch_workspace(cfg: AppConfig) -> Result<String, ErrStr> {
    let root = workspace_root(&cfg);
    let ids: Vec<String> = cfg
        .agents
        .iter()
        .map(|p| p.id.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    ensure_workspace(&root, &ids);
    let real = root
        .canonicalize()
        .map_err(|e| format!("工作区不可用:{e}"))?;
    Ok(abs_path(&real))
}

// ─────────────────────────── 会话控制 ───────────────────────────

#[tauri::command]
pub fn status(state: State<'_, RunningState>) -> RunStatus {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
pub fn pause(on: bool, state: State<'_, RunningState>) -> Result<(), ErrStr> {
    let c = state
        .controls
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "当前没有进行中的会话".to_string())?;
    c.paused.store(on, Ordering::SeqCst);
    {
        let mut status = state.status.lock().unwrap();
        status.paused = on;
        status.stop_requested = c.stop.load(Ordering::SeqCst);
    }
    Ok(())
}

#[tauri::command]
pub fn stop_session(state: State<'_, RunningState>) -> Result<(), ErrStr> {
    let c = state
        .controls
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "当前没有进行中的会话".to_string())?;
    c.stop.store(true, Ordering::SeqCst);
    c.paused.store(false, Ordering::SeqCst);
    {
        let mut status = state.status.lock().unwrap();
        status.stop_requested = true;
        status.paused = false;
    }
    Ok(())
}

fn restore_snapshot(dir: &std::path::Path, current: &AppConfig) -> Result<AppConfig, String> {
    let txt = std::fs::read_to_string(dir.join("config.snapshot.toml"))
        .map_err(|_| "会话缺少配置快照，无法安全续跑".to_string())?;
    let mut saved: AppConfig = toml::from_str(&txt).map_err(|_| "会话快照损坏".to_string())?;
    let roster = JsonlLog::read_all(&dir.join("events.jsonl"))
        .into_iter()
        .find_map(|ev| match ev {
            Event::SessionOpen { agents, .. } => {
                Some(agents.into_iter().map(|a| a.id).collect::<Vec<_>>())
            }
            _ => None,
        })
        .ok_or("会话缺少参与者记录")?;
    if roster != saved.agent_ids() {
        return Err("会话快照与原参与者不一致".into());
    }
    for p in &mut saved.agents {
        let key = current
            .characters
            .iter()
            .chain(current.agents.iter())
            .find(|card| card.id == p.id)
            .ok_or_else(|| format!("角色 {} 已不存在，请恢复角色卡后续跑", p.name))?;
        if key.api_key.trim().is_empty() {
            return Err(format!("角色 {} 缺少 API Key", p.name));
        }
        if key.base_url.trim_end_matches('/') != p.base_url.trim_end_matches('/') {
            return Err(format!("角色 {} 的端点已变更，请恢复原端点后续跑", p.name));
        }
        p.api_key = key.api_key.clone();
    }
    saved.characters = current.characters.clone();
    Ok(saved)
}

/// Validate selected participants before starting or resuming the frozen session.
#[tauri::command]
pub async fn start_session(
    app: AppHandle,
    state: State<'_, RunningState>,
    cfg: AppConfig,
    resume_of: Option<String>,
) -> Result<StartAck, Vec<String>> {
    if state.status.lock().unwrap().running {
        return Err(vec!["已经有一个会话在跑。先按结束。".into()]);
    }
    let cfg = if let Some(session) = resume_of.as_deref() {
        if !valid_session_name(session) {
            return Err(vec!["非法会话名".into()]);
        }
        let dir = config::sessions_dir().map_err(|e| vec![e])?.join(session);
        restore_snapshot(&dir, &cfg).map_err(|e| vec![e])?
    } else {
        cfg
    };
    let violations = validate_app(&cfg);
    if !violations.is_empty() {
        return Err(violations.iter().map(|v| v.human_message()).collect());
    }

    let (a, b) = (&cfg.agents[0], &cfg.agents[1]);
    if !cfg.agents.iter().all(|p| valid_agent_id(&p.id)) {
        return Err(vec!["非法 AI 标识符:只能使用不含路径分隔符的名称".into()]);
    }
    let ids: Vec<String> = cfg.agents.iter().map(|p| p.id.clone()).collect();
    let root = workspace_root(&cfg);
    ensure_workspace(&root, &ids);
    let real_root = root.canonicalize().unwrap_or(root.clone());

    let session = match &resume_of {
        Some(s) if !s.trim().is_empty() => s.clone(),
        _ => crate::events::session_id(&a.name, &b.name),
    };
    if session.contains('/') || session.contains('\\') || session.contains("..") {
        return Err(vec!["非法会话名".into()]);
    }
    let dir = config::sessions_dir().map_err(|e| vec![e])?.join(&session);
    std::fs::create_dir_all(&dir).map_err(|e| vec![format!("建会话目录失败:{e}")])?;
    // 会话快照独立于工作区保存；只保存可回放的角色元数据，不落 API secrets。
    let mut snapshot = cfg.clone();
    for profile in &mut snapshot.agents {
        profile.api_key.clear();
    }
    for profile in &mut snapshot.characters {
        profile.api_key.clear();
    }
    let snapshot_text =
        toml::to_string_pretty(&snapshot).map_err(|e| vec![format!("序列化会话快照失败:{e}")])?;
    std::fs::write(dir.join("config.snapshot.toml"), snapshot_text)
        .map_err(|e| vec![format!("写会话快照失败:{e}")])?;
    let log_path = dir.join("events.jsonl");

    let log = JsonlLog::open(&log_path).map_err(|e| vec![e])?;
    let resume = resume_of.is_some() && log.count() > 0;
    if resume {
        let roster = JsonlLog::read_all(&log_path)
            .into_iter()
            .find_map(|ev| match ev {
                Event::SessionOpen { agents, .. } => {
                    Some(agents.into_iter().map(|a| a.id).collect::<Vec<_>>())
                }
                _ => None,
            })
            .unwrap_or_default();
        if roster != ids {
            return Err(vec![
                "会话参与者与保存的 SessionOpen roster 不匹配，拒绝续跑".into(),
            ]);
        }
        let snapshot_path = dir.join("config.snapshot.toml");
        if !snapshot_path.exists() {
            return Err(vec!["会话缺少配置快照，拒绝续跑".into()]);
        }
    }

    // 每方各一个全新 ProviderClient:独立连接池、独立凭证头(Spec §4.3)
    let mut providers: HashMap<String, SharedProvider> = HashMap::new();
    for p in &cfg.agents {
        providers.insert(
            p.id.clone(),
            Arc::new(ProviderClient::new(
                &p.id,
                &p.base_url,
                &p.api_key,
                &p.model,
            )) as SharedProvider,
        );
    }

    let controls = Arc::new(Controls::default());
    // 引擎与 managed state 共用同一个 Arc<Mutex<RunStatus>>:引擎写,status() 读,无需轮询
    let status_arc: Arc<Mutex<RunStatus>> = state.status.clone();
    let ui: Arc<dyn EventSink> = Arc::new(UiSink { app: app.clone() });
    let mut engine = Engine::new(
        cfg.clone(),
        providers,
        ui,
        log,
        real_root.clone(),
        session.clone(),
        controls.clone(),
        status_arc,
    );

    if let Err(v) = engine.start() {
        return Err(v.iter().map(|x| x.human_message()).collect());
    }

    // 把句柄交给 managed state,供 pause/stop/status 使用
    *state.controls.lock().unwrap() = Some(controls);
    *state.session.lock().unwrap() = Some(session.clone());
    *state.log_path.lock().unwrap() = Some(log_path);

    let ack = StartAck {
        same_origin: norm_host(&a.base_url) == norm_host(&b.base_url),
        session: session.clone(),
        workspace: abs_path(&real_root),
        host_a: norm_host(&a.base_url),
        host_b: norm_host(&b.base_url),
        mode: match cfg.workspace.mode {
            config::AccessMode::Independent => "independent".into(),
            config::AccessMode::SameOriginDualModel => "same_origin_dual_model".into(),
            AccessMode::SharedEndpoint => "shared_endpoint".into(),
        },
    };

    let resume_of_run = resume;
    tauri::async_runtime::spawn(async move {
        let _end = engine.run(resume_of_run).await;
    });
    Ok(ack)
}

// ─────────────────────────── 观察 ───────────────────────────

#[tauri::command]
pub fn events(state: State<'_, RunningState>) -> Result<Vec<String>, ErrStr> {
    let p = state
        .log_path
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "还没有进行中的会话".to_string())?;
    Ok(JsonlLog::read_lines(&p))
}

/// 名字必须是**单层、可安全 join** 的片段。
///
/// 为什么光有 is_absolute() 不够 —— `"C:"` 是那条漏网的形态:
/// · `Path::new("C:").is_absolute()` 在 Windows 上是 **false**
///   (is_absolute 要求 has_root() && prefix().is_some(),而 "C:" 只有 prefix);
/// · 但 `PathBuf::push` 对"有 prefix 而无 root"的路径会**直接清空自己**
///   (std 文档原文:"if path has a prefix but no root, it replaces self")。
///
/// 于是 `sessions_dir().join("C:")` 就等于 `"C:"`,而
/// `delete_session` 会 `remove_dir_all("C:")` —— 在 Windows 上删的是
/// C: 盘的当前工作目录,Explorer 启动时通常就是应用安装目录。
/// memory.rs 的 `workspace.join("memory").join(agent_id)` 同样会塌成 "C:"。
///
/// sandbox::has_drive 早就拦了这种形态(AI 的工具路径因此是安全的),
/// 这里只是把同一条判据补到命令层 —— 属遗漏而非设计。
fn safe_name(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains("..")
        && !s.contains('/')
        && !s.contains('\\')
        && !s.contains('\0')
        && !std::path::Path::new(s).is_absolute()
        && !sandbox::has_drive(s)
}

fn valid_agent_id(id: &str) -> bool {
    safe_name(id)
}

fn valid_session_name(session: &str) -> bool {
    safe_name(session)
}

#[tauri::command]
pub fn events_of(session: String) -> Result<Vec<String>, ErrStr> {
    if !valid_session_name(&session) {
        return Err("非法会话名".into());
    }
    let p = config::sessions_dir()?.join(&session).join("events.jsonl");
    if !p.exists() {
        return Err("找不到这个会话".into());
    }
    Ok(JsonlLog::read_lines(&p))
}

#[tauri::command]
pub fn sessions() -> Result<Vec<SessionMeta>, ErrStr> {
    let dir = config::sessions_dir()?;
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Ok(out);
    };
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let id = p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let ev_path = p.join("events.jsonl");
        let evs = JsonlLog::read_all(&ev_path);
        if evs.is_empty() {
            continue;
        }
        let mut m = SessionMeta {
            id,
            events: evs.len(),
            turns: 0,
            end_reason: None,
            a: String::new(),
            b: String::new(),
            host_a: String::new(),
            host_b: String::new(),
            mode: String::new(),
            participants: Vec::new(),
            tokens: 0,
            ms_total: 0,
            unix: 0,
        };
        for ev in &evs {
            match ev {
                Event::SessionOpen { agents, mode, .. } => {
                    m.mode = mode.clone();
                    m.participants = agents.clone();
                    if agents.len() == 2 {
                        m.a = format!("{}·{}", agents[0].name, agents[0].model);
                        m.b = format!("{}·{}", agents[1].name, agents[1].model);
                        m.host_a = agents[0].host.clone();
                        m.host_b = agents[1].host.clone();
                    }
                }
                Event::SayEnd { usage, .. } => m.tokens += usage.total(),
                Event::TurnBegin { turn, .. } => m.turns = m.turns.max(*turn),
                Event::SessionEnd {
                    reason,
                    turns,
                    ms_total,
                    ..
                } => {
                    m.end_reason = Some(reason.clone());
                    m.turns = m.turns.max(*turns);
                    m.ms_total = *ms_total;
                }
                _ => {}
            }
        }
        if m.end_reason.is_none() {
            m.end_reason = Some("running".into());
        }
        m.unix = e
            .metadata()
            .and_then(|md| md.modified())
            .map(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64
            })
            .unwrap_or(0);
        out.push(m);
    }
    out.sort_by(|x, y| y.id.cmp(&x.id));
    Ok(out)
}

/// 工作区文件树(人类观察面板)
#[tauri::command]
pub fn tree(cfg: AppConfig) -> Result<Vec<TreeEntry>, ErrStr> {
    let root = workspace_root(&cfg)
        .canonicalize()
        .map_err(|e| format!("工作区不可用:{e}"))?;
    let mut out: Vec<TreeEntry> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut items: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        items.sort();
        for p in items {
            if out.len() >= 2000 {
                break;
            }
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            let rel = sandbox::display_rel(&root, &p);
            let zone = zone_of(&rel);
            if p.is_dir() {
                stack.push(p.clone());
                out.push(TreeEntry {
                    path: rel,
                    dir: true,
                    bytes: 0,
                    zone,
                });
            } else {
                out.push(TreeEntry {
                    path: rel,
                    dir: false,
                    bytes: std::fs::metadata(&p).map(|md| md.len()).unwrap_or(0),
                    zone,
                });
            }
        }
    }
    out.sort_by(|x, y| x.path.cmp(&y.path));
    Ok(out)
}

fn zone_of(rel: &str) -> String {
    if rel.starts_with("memory/") || rel == "memory" {
        let seg = rel.split('/').nth(1).unwrap_or("");
        format!("memory:{seg}")
    } else if rel.starts_with("skills/") || rel == "skills" {
        "skills".into()
    } else if rel.starts_with("arena/") || rel == "arena" {
        "arena".into()
    } else {
        "root".into()
    }
}

/// 人类读工作区文件的结果(前端按这个形状渲染预览)。
#[derive(Serialize, Clone, Debug)]
pub struct WorkspaceFile {
    pub path: String,
    pub bytes: usize,
    pub binary: bool,
    pub text: String,
    pub truncated: bool,
}

/// 人类读工作区文件。走同一条路径守卫(拒绝绝对路径 / .. / symlink 逃逸),
/// 但用人类读权限:memory 分区对人类观察者不设限(界面设计如此),也不给写权限。
#[tauri::command]
pub fn read_workspace(cfg: AppConfig, path: String) -> Result<WorkspaceFile, ErrStr> {
    let root = workspace_root(&cfg)
        .canonicalize()
        .map_err(|e| format!("工作区不可用:{e}"))?;
    let p = sandbox::resolve_human_read(&root, &path).map_err(|e| e.agent_message())?;
    sandbox::check_within(&root, &p).map_err(|e| e.agent_message())?;
    if p.is_dir() {
        return Err("这是目录,不是文件".into());
    }
    let bytes = std::fs::read(&p).map_err(|e| format!("读不到:{e}"))?;
    let binary = bytes.iter().take(1024).any(|b| *b == 0);
    let rel = sandbox::display_rel(&root, &p);
    const MAX: usize = 512 * 1024;
    let (text, truncated) = if binary {
        ("".into(), false)
    } else {
        let s = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
        if s.len() > MAX {
            // 按字节截断但保证落在字符边界(与 tools::safe_slice 同思路)
            let mut end = MAX;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            (format!("{}…(已截断,共 {} 字节)", &s[..end], s.len()), true)
        } else {
            (s, false)
        }
    };
    Ok(WorkspaceFile {
        path: rel,
        bytes: bytes.len(),
        binary,
        text,
        truncated,
    })
}

/// 记忆视图:按方并列返回(人类观察者专用,AI 不可见)。
#[derive(Serialize, Clone, Debug)]
pub struct MemoryNoteView {
    pub ts: String,
    pub session: String,
    pub turn: u32,
    pub text: String,
    pub tags: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct MemoryView {
    pub agent: String,
    pub name: String,
    pub notes: Vec<MemoryNoteView>,
}

#[tauri::command]
pub fn memories(cfg: AppConfig) -> Result<Vec<MemoryView>, ErrStr> {
    let root = workspace_root(&cfg);
    let mut out = Vec::new();
    for p in &cfg.agents {
        let m = MemoryStore::open(&root, &p.id)?;
        let notes: Vec<MemoryNoteView> = m
            .all()
            .iter()
            .map(|n| MemoryNoteView {
                ts: n.ts.clone(),
                session: n.session.clone(),
                turn: n.turn,
                text: n.text.clone(),
                tags: n.tags.clone(),
            })
            .collect();
        out.push(MemoryView {
            agent: p.id.clone(),
            name: p.name.clone(),
            notes,
        });
    }
    Ok(out)
}

#[tauri::command]
pub fn skills_list(cfg: AppConfig) -> Result<Vec<SkillMeta>, ErrStr> {
    Ok(skills::list(&workspace_root(&cfg).join("skills")))
}

#[tauri::command]
pub fn delete_session(session: String, state: State<'_, RunningState>) -> Result<(), ErrStr> {
    if !valid_session_name(&session) {
        return Err("非法会话名".into());
    }
    let running = state.status.lock().unwrap().running;
    let current = state.session.lock().unwrap().clone();
    if running && current.as_deref() == Some(session.as_str()) {
        return Err("这个会话还在跑,先按结束".into());
    }
    let p = config::sessions_dir()?.join(&session);
    if !p.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&p).map_err(|e| format!("删除失败:{e}"))
}

/// 导出诊断包:把会话目录打成一个可读的文本(不引入 zip 依赖)
#[tauri::command]
pub fn export_session(session: String) -> Result<String, ErrStr> {
    if !valid_session_name(&session) {
        return Err("非法会话名".into());
    }
    let dir = config::sessions_dir()?.join(&session);
    let ev_path = dir.join("events.jsonl");
    let evs = JsonlLog::read_all(&ev_path);
    let mut s = format!("# AiClub 会话 {session}\n导出时间:{}\n\n", now_ts());
    for ev in &evs {
        s.push_str(&describe(ev));
        s.push('\n');
    }
    let out = dir.join("transcript.md");
    std::fs::write(&out, s).map_err(|e| format!("导出失败:{e}"))?;
    Ok(abs_path(&out))
}

fn describe(ev: &Event) -> String {
    match ev {
        Event::SessionOpen {
            opening_note,
            agents,
            mode,
            ..
        } => format!(
            "## 开局({mode})\n{}\n\n参与:{:?}\n",
            opening_note,
            agents
                .iter()
                .map(|a| format!("{}@{}", a.name, a.host))
                .collect::<Vec<_>>()
        ),
        Event::SayEnd {
            agent, text, turn, ..
        } => format!("【第{turn}轮·{agent}】{text}"),
        Event::ToolCall {
            agent,
            turn,
            name,
            preview,
            ok,
            ..
        } => format!(
            "  · 第{turn}轮 {agent} 调用 {name} → {preview}({})",
            if *ok { "成功" } else { "失败" }
        ),
        Event::Pass { agent, turn } => format!("  · 第{turn}轮 {agent} 沉默"),
        Event::Violation {
            agent,
            turn,
            path,
            detail,
            ..
        } => {
            format!("  ! 第{turn}轮 {agent} 越界:{path} —— {detail}")
        }
        Event::SystemNote { text, turn } => format!("  ※ 第{turn}轮 系统:{text}"),
        Event::TurnError {
            agent, turn, error, ..
        } => format!("  × 第{turn}轮 {agent}:{error}"),
        Event::SessionEnd {
            reason,
            turns,
            ms_total,
            ..
        } => {
            format!("## 结束:{reason}(共 {turns} 轮,{}s)", ms_total / 1000)
        }
        Event::SayDelta { .. } => String::new(),
        Event::Compress {
            agent,
            turn,
            via_model,
            ..
        } => {
            format!("  ≈ 第{turn}轮 {agent} 用自身端点({via_model})压缩了早期上下文")
        }
        other => format!("  · {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归:名字校验曾经漏掉 `"C:"` 这种"有盘符前缀、无根"的形态。
    ///
    /// Rust 的语义很容易踩:
    /// · `Path::new("C:").is_absolute()` 在 Windows 上是 **false**
    ///   (要 has_root() && prefix().is_some(),而 "C:" 只有 prefix);
    /// · 但 `PathBuf::push` 遇到"有 prefix 无 root"的路径会**清空自己**
    ///   (std 文档:"if path has a prefix but no root, it replaces self")。
    /// 于是 `sessions_dir().join("C:") == "C:"`,而 delete_session 会
    /// `remove_dir_all("C:")` —— Windows 上删的是 C: 盘的当前工作目录。
    #[test]
    fn name_validation_rejects_drive_prefixes() {
        for bad in ["C:", "c:", "Z:", "C:\\x", "C:/x"] {
            assert!(!valid_agent_id(bad), "agent id 必须拒绝 {bad:?}");
            assert!(!valid_session_name(bad), "会话名必须拒绝 {bad:?}");
        }
        for bad in ["", "  ", ".", "..", "../x", "a/b", "a\\b", "/abs", "a/../b"] {
            assert!(!valid_agent_id(bad), "agent id 必须拒绝 {bad:?}");
            assert!(!valid_session_name(bad), "会话名必须拒绝 {bad:?}");
        }
    }

    fn snapshot_fixture(tag: &str) -> (std::path::PathBuf, AppConfig) {
        let dir = crate::testsup::world(tag);
        let mut saved = AppConfig::default();
        saved.workspace.mode = crate::config::AccessMode::SharedEndpoint;
        saved.agents = ["C", "A", "B"]
            .iter()
            .map(|id| crate::config::AgentProfile {
                id: (*id).into(),
                name: format!("原角色 {id}"),
                company: "原公司".into(),
                base_url: "http://localhost:12345/v1".into(),
                api_key: String::new(),
                model: "original-model".into(),
                temperament: "原性格".into(),
                ..Default::default()
            })
            .collect();
        saved.persona_template = "原提示词".into();
        std::fs::write(
            dir.join("config.snapshot.toml"),
            toml::to_string_pretty(&saved).unwrap(),
        )
        .unwrap();
        JsonlLog::open(&dir.join("events.jsonl"))
            .unwrap()
            .append(&Event::SessionOpen {
                session_id: "snapshot-test".into(),
                agents: saved
                    .agents
                    .iter()
                    .map(|p| crate::events::AgentBrief {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        company: p.company.clone(),
                        model: p.model.clone(),
                        host: "localhost".into(),
                    })
                    .collect(),
                mode: "shared_endpoint".into(),
                opening_note: "原字条".into(),
                ts: "t".into(),
            });
        (dir, saved)
    }

    #[test]
    fn group_acceptance_snapshot_freezes_profiles_order_and_restores_only_credentials() {
        let (dir, saved) = snapshot_fixture("snapshot-frozen");
        let mut current = saved.clone();
        current.agents.reverse();
        current.characters = saved
            .agents
            .iter()
            .rev()
            .map(|p| {
                let mut edited = p.clone();
                edited.name = "新名字".into();
                edited.company = "新公司".into();
                edited.model = "new-model".into();
                edited.temperament = "新性格".into();
                edited.api_key = format!("rotated-test-key-{}", p.id);
                edited
            })
            .collect();
        current.persona_template = "新提示词".into();
        let restored = restore_snapshot(&dir, &current).unwrap();
        let disk = std::fs::read_to_string(dir.join("config.snapshot.toml")).unwrap();
        crate::testsup::cleanup(&dir);
        let mut expected = saved;
        for p in &mut expected.agents {
            p.api_key = format!("rotated-test-key-{}", p.id);
        }
        assert_eq!(restored.agents, expected.agents);
        assert_eq!(restored.persona_template, expected.persona_template);
        assert_eq!(restored.agent_ids(), vec!["C", "A", "B"]);
        assert!(
            !disk.contains("rotated-test-key"),
            "补凭证不得把密钥写回历史快照"
        );
    }

    #[test]
    fn group_acceptance_snapshot_rejects_missing_credentials_or_changed_endpoint() {
        let (dir, saved) = snapshot_fixture("snapshot-credentials");
        let mut current = saved.clone();
        for p in &mut current.agents {
            p.api_key = "local-test-key".into();
        }
        current.agents[1].api_key.clear();
        assert!(restore_snapshot(&dir, &current)
            .unwrap_err()
            .contains("缺少 API Key"));
        current.agents[1].api_key = "replacement-test-key".into();
        current.agents[1].base_url = "http://localhost:54321/v1".into();
        assert!(restore_snapshot(&dir, &current)
            .unwrap_err()
            .contains("端点已变更"));
        current.agents.remove(1);
        assert!(restore_snapshot(&dir, &current)
            .unwrap_err()
            .contains("已不存在"));
        crate::testsup::cleanup(&dir);
    }

    #[test]
    fn group_acceptance_snapshot_rejects_changed_participant_order() {
        let (dir, mut saved) = snapshot_fixture("snapshot-roster");
        let mut current = saved.clone();
        for p in &mut current.agents {
            p.api_key = "local-test-key".into();
        }
        saved.agents.reverse();
        std::fs::write(
            dir.join("config.snapshot.toml"),
            toml::to_string_pretty(&saved).unwrap(),
        )
        .unwrap();
        let error = restore_snapshot(&dir, &current).unwrap_err();
        crate::testsup::cleanup(&dir);
        assert!(error.contains("原参与者不一致"), "{error}");
    }
    #[test]
    fn name_validation_accepts_normal_names() {
        for ok in ["A", "B", "left", "agent-1", "甲", "s2026-10-14_120000"] {
            assert!(valid_agent_id(ok), "agent id 应接受 {ok:?}");
            assert!(valid_session_name(ok), "会话名应接受 {ok:?}");
        }
    }
}
