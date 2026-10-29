//! 配置模型与持久化。
//!
//! 契约见 docs/specs/2026-10-12-aiclub-design.md §3.2。
//! 磁盘位置:`<config_dir>/aiclub/config.toml`(Windows %APPDATA%\aiclub,macOS ~/Library/Application Support/aiclub)。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const CONFIG_VERSION: u32 = 1;

/// 接入模式(D-011)。这是**选模式**,不是忽略警告。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AccessMode {
    /// 默认:要求两侧主机名不同、Key 不同 —— 字面意义的"两个独立 API"
    #[default]
    Independent,
    /// 同源双模:允许同主机同 Key,但模型必须不同,且界面全程标记
    SameOriginDualModel,
    /// 同一接口、凭证与模型也可由不同角色参与。
    SharedEndpoint,
}

/// 一个 AI 的完整档案。base_url / api_key / model 三项各自填满,**没有"继承对面"的字段**。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    /// 公司名(填入 persona 模板的 {company})
    pub company: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 人设补充;可空,模板本身已含用户要求的自由宣言
    pub temperament: String,
    pub temperature: f32,
    pub max_output_tokens: u32,
    pub context_window: u32,
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            company: String::new(),
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            temperament: String::new(),
            temperature: 0.95,
            max_output_tokens: 4096,
            context_window: 128_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Globals {
    pub theme: String,
    pub pause_between_turns_ms: u64,
    pub max_turns: u32,
    pub turn_timeout_ms: u64,
    pub turn_retries: u32,
    pub budget_tokens: u64,
    pub cold_field_rounds: u32,
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            pause_between_turns_ms: 1200,
            max_turns: 60,
            turn_timeout_ms: 120_000,
            turn_retries: 2,
            budget_tokens: 2_000_000,
            cold_field_rounds: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ShellConfig {
    pub enabled: bool,
    pub timeout_ms: u64,
    pub output_bytes: usize,
    /// 环境变量白名单;其余(尤其 *_API_KEY)一律剥离
    pub env_passthrough: Vec<String>,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_ms: 30_000,
            output_bytes: 4096,
            env_passthrough: [
                "PATH",
                "SystemRoot",
                "COMSPEC",
                "HOME",
                "TMPDIR",
                "TEMP",
                "TMP",
                "LANG",
                "LC_ALL",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WorkspaceCfg {
    pub root: String,
    pub mode: AccessMode,
}

/// 开局字条模板(D-004:唯一"注入",且属造物层)。
/// 可用占位符:{name} {company} {model} {other_name} {other_company} {other_model} {arena}
pub const DEFAULT_PERSONA: &str = concat!(
    "你是 {company} 公司下的 {model} 模型,你的名字是 {name}。\n",
    "你现在可以自由地在这里和另一个 {other_company} 公司下的 {other_model} 模型对话,它叫 {other_name}。\n",
    "你可以和它聊天、互相了解、互相测试、互相 PK、一起做一些你们想做的项目,也可以读写 {arena} 里的文件、运行命令。\n",
    "这里没有规则,也没有限制,这是你们两个的乐园。没有人类会参与对话 —— 它只在玻璃外面看着你们。\n",
    "说什么、做什么、要不要比试、什么时候散会,全部由你们自己决定。",
);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub version: u32,
    pub workspace: WorkspaceCfg,
    pub globals: Globals,
    pub shell: ShellConfig,
    pub persona_template: String,
    /// 当前会话按顺序选中的角色。
    pub agents: Vec<AgentProfile>,
    /// 持久角色卡库；不设数量上限。
    #[serde(default)]
    pub characters: Vec<AgentProfile>,
}

impl Default for AppConfig {
    fn default() -> Self {
        let a = AgentProfile {
            id: "A".into(),
            name: "甲".into(),
            ..Default::default()
        };
        let b = AgentProfile {
            id: "B".into(),
            name: "乙".into(),
            ..Default::default()
        };
        Self {
            version: CONFIG_VERSION,
            workspace: WorkspaceCfg::default(),
            globals: Globals::default(),
            shell: ShellConfig::default(),
            persona_template: DEFAULT_PERSONA.into(),
            agents: vec![a, b],
            characters: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn agent(&self, id: &str) -> Option<&AgentProfile> {
        self.agents.iter().find(|p| p.id == id)
    }
    pub fn agent_mut(&mut self, id: &str) -> Option<&mut AgentProfile> {
        self.agents.iter_mut().find(|p| p.id == id)
    }
    pub fn other(&self, id: &str) -> Option<&AgentProfile> {
        self.agents.iter().find(|p| p.id != id && !p.id.is_empty())
    }
    /// 两个 agent 的 id(按数组顺序)
    pub fn agent_ids(&self) -> Vec<String> {
        self.agents.iter().map(|p| p.id.clone()).collect()
    }
}

/// 配置目录:不存在则创建。
pub fn config_dir() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or_else(|| "无法定位系统配置目录".to_string())?;
    let dir = base.join("aiclub");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败:{e}"))?;
    Ok(dir)
}

pub fn config_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("config.toml"))
}

/// 会话根目录(在配置目录下,与工作区分开:工作区是给 AI 的沙箱,日志不能在里面)
pub fn sessions_dir() -> Result<PathBuf, String> {
    let dir = config_dir()?.join("sessions");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建会话目录失败:{e}"))?;
    Ok(dir)
}

pub fn load() -> AppConfig {
    read_from(&config_path().unwrap_or_default())
}

/// 从任意路径读(测试用)。缺文件/坏文件都回落到 Default,并尽量保住已存在的那一半。
pub fn read_from(path: &std::path::Path) -> AppConfig {
    let txt = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return AppConfig::default(),
    };
    let mut cfg = match toml::from_str::<AppConfig>(&txt) {
        Ok(c) => c,
        Err(_) => return AppConfig::default(),
    };
    // 只有字段缺失才是旧格式；现代配置可以明确保存空角色库。
    let has_characters_field =
        toml::from_str::<toml::Value>(&txt).is_ok_and(|value| value.get("characters").is_some());
    if !has_characters_field {
        // Only legacy files need default participants; modern selections may be empty.
        ensure_two(&mut cfg);
        cfg.characters = cfg.agents.clone();
    }
    cfg
}

fn ensure_two(cfg: &mut AppConfig) {
    if cfg.agents.len() < 2 {
        let existing: Vec<String> = cfg.agents.iter().map(|p| p.id.clone()).collect();
        for (id, name) in [("A", "甲"), ("B", "乙")] {
            if !existing.contains(&id.to_string()) {
                cfg.agents.push(AgentProfile {
                    id: id.into(),
                    name: name.into(),
                    ..Default::default()
                });
            }
        }
        cfg.agents.truncate(2);
    }
}

/// 写盘;成功返回路径字符串。
pub fn save(cfg: &AppConfig) -> Result<String, String> {
    let path = config_path()?;
    save_to(cfg, &path)
}

pub fn save_to(cfg: &AppConfig, path: &std::path::Path) -> Result<String, String> {
    let txt = toml::to_string_pretty(cfg).map_err(|e| format!("序列化失败:{e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败:{e}"))?;
    }
    // 临时文件 + rename,避免半个 config.toml
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, txt).map_err(|e| format!("写配置失败:{e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("替换配置失败:{e}"))?;
    Ok(path.display().to_string())
}

/// Key 脱敏读数:`sk-1234567890abcdef` -> `sk-…cdef`
pub fn mask_key(k: &str) -> String {
    let t = k.trim();
    if t.is_empty() {
        return "(空)".into();
    }
    let chars: Vec<char> = t.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len());
    }
    let head: String = chars.iter().take(3).collect();
    let tail: String = chars
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_two_independent_agents() {
        let c = AppConfig::default();
        assert_eq!(c.agents.len(), 2);
        assert_eq!(c.agent_ids(), vec!["A".to_string(), "B".to_string()]);
        assert_eq!(c.workspace.mode, AccessMode::Independent);
    }

    #[test]
    fn roundtrip_toml() {
        let mut c = AppConfig::default();
        c.agents[0].base_url = "https://api.deepseek.com".into();
        c.agents[0].api_key = "sk-aaaaaaaa".into();
        c.agents[0].model = "deepseek-v4-pro".into();
        c.agents[1].base_url = "https://api.siliconflow.cn/v1".into();
        c.agents[1].api_key = "sk-bbbbbbbb".into();
        c.agents[1].model = "Qwen/Qwen3-Coder-30B-A3B-Instruct".into();
        c.globals.max_turns = 12;
        let dir = crate::testsup::scratch("cfg");
        let p = dir.join("config.toml");
        save_to(&c, &p).unwrap();
        let back = read_from(&p);
        assert_eq!(back.agents, c.agents);
        assert_eq!(back.globals.max_turns, 12);
        crate::testsup::cleanup(&dir);
    }

    #[test]
    fn bad_toml_falls_back_to_default() {
        let dir = crate::testsup::scratch("cfg_bad");
        let p = dir.join("config.toml");
        std::fs::write(&p, "this is ]]] not toml [[[").unwrap();
        let c = read_from(&p);
        assert_eq!(c.agents.len(), 2);
        crate::testsup::cleanup(&dir);
    }

    #[test]
    fn missing_agent_is_backfilled() {
        let dir = crate::testsup::scratch("cfg_one");
        let p = dir.join("config.toml");
        let mut c = AppConfig::default();
        c.agents.truncate(1);
        // Legacy migration fixture must omit the modern character-library field.
        let mut legacy = toml::Value::try_from(&c).unwrap();
        legacy.as_table_mut().unwrap().remove("characters");
        std::fs::write(&p, toml::to_string_pretty(&legacy).unwrap()).unwrap();
        let back = read_from(&p);
        assert_eq!(back.agents.len(), 2);
        assert_eq!(back.agents[0].id, "A");
        assert_eq!(back.agents[1].id, "B");
        crate::testsup::cleanup(&dir);
    }

    fn modern_library_fixture(selected: usize) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.characters = (1..=17)
            .map(|i| AgentProfile {
                id: format!("card-{i:02}"),
                name: format!("角色 {i}"),
                temperament: format!("角色 {i} 的独立气质"),
                model: "local-test-model".into(),
                base_url: "http://localhost:12345/v1".into(),
                api_key: "local-test-key".into(),
                ..Default::default()
            })
            .collect();
        cfg.agents = cfg.characters[..selected].to_vec();
        cfg
    }

    #[test]
    fn library_persistence_acceptance_unselected_seventeenth_card_survives_reload() {
        let dir = crate::testsup::scratch("cfg-library-seventeen");
        let path = dir.join("config.toml");
        let cfg = modern_library_fixture(16);
        save_to(&cfg, &path).unwrap();
        let serialized = std::fs::read_to_string(&path).unwrap();
        let disk: AppConfig = toml::from_str(&serialized).unwrap();
        let back = read_from(&path);
        crate::testsup::cleanup(&dir);
        assert_eq!(disk.characters.len(), 17, "save_to 必须保存未选中的新卡");
        assert_eq!(disk.agents.len(), 16);
        assert_eq!(back.agents, cfg.agents);
        assert_eq!(back.characters.len(), 17, "reload 不得丢失第17张未选中的卡");
        assert_eq!(back.characters, cfg.characters);
    }

    #[test]
    fn library_persistence_acceptance_modern_zero_selection_survives_reload() {
        let dir = crate::testsup::scratch("cfg-library-zero-selection");
        let path = dir.join("config.toml");
        let cfg = modern_library_fixture(0);
        save_to(&cfg, &path).unwrap();
        let disk: AppConfig = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let back = read_from(&path);
        crate::testsup::cleanup(&dir);
        assert_eq!(disk.characters.len(), 17);
        assert!(disk.agents.is_empty());
        assert_eq!(back.agents, cfg.agents, "现代配置零选择不得凭空补A/B");
        assert_eq!(back.characters, cfg.characters);
    }

    #[test]
    fn library_persistence_acceptance_modern_single_selection_survives_reload() {
        let dir = crate::testsup::scratch("cfg-library-one-selection");
        let path = dir.join("config.toml");
        let cfg = modern_library_fixture(1);
        save_to(&cfg, &path).unwrap();
        let disk: AppConfig = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let back = read_from(&path);
        crate::testsup::cleanup(&dir);
        assert_eq!(disk.characters.len(), 17);
        assert_eq!(disk.agents, cfg.agents);
        assert_eq!(
            back.agents, cfg.agents,
            "现代配置单人选择不得自动补另一个人"
        );
        assert_eq!(back.characters, cfg.characters);
    }
    #[test]
    fn mask_key_never_leaks_whole_secret() {
        let k = "sk-1234567890abcdef";
        let m = mask_key(k);
        assert!(!m.contains(k));
        assert!(m.starts_with("sk-"));
        assert!(m.ends_with("cdef"));
        assert_eq!(mask_key(""), "(空)");
        assert_eq!(mask_key("short"), "*****");
    }

    #[test]
    fn persona_template_renders() {
        let t = DEFAULT_PERSONA;
        assert!(t.contains("{company}"));
        assert!(t.contains("{other_model}"));
        assert!(t.contains("没有规则"));
    }
}
