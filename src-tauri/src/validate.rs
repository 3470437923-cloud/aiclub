//! 独立性硬闸(D-011:不可强行开局)。
//!
//! 两个 AI 必须是**完全独立**的 API 接入。规则在 Rust 侧执行 —— 前端置灰只是体验,
//! 不是安全边界。没有 --force、没有环境变量后门、没有"忽略警告继续"的勾选框。
//!
//! 模式语义:
//! - `Independent`(默认):两侧主机名必须不同,Key 必须不同 → 字面意义的"两家 API"
//! - `SameOriginDualModel`:允许同主机同 Key(同一聚合站的两个模型),但模型必须不同,
//!   且界面全程常驻「同源」标记。**这是选模式,不是忽略警告。**

use crate::config::{AccessMode, AgentProfile, AppConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// 档案不完整(V-1)
    Incomplete { agent: String, field: String },
    /// 名字或 id 撞车(V-2)
    DuplicateName { a: String, b: String },
    /// 独立模式下同主机(V-3)
    SameHost { host: String },
    /// 同源模式下同模型(V-4)
    SameModel { model: String },
    /// 独立模式下同 Key(V-5)
    SameKey,
    /// 人设完全相同(V-6)
    SamePersona,
    /// 结构性错误:至少需要两个参与者
    NotTwoAgents { count: usize },
}

impl Violation {
    /// 给人类看的文案:说清为什么,并指一条出路。
    pub fn human_message(&self) -> String {
        match self {
            Violation::Incomplete { agent, field } => {
                format!("{agent} 的「{field}」没填。两个 AI 各自需要完整独立的接口地址、API Key 与模型。")
            }
            Violation::DuplicateName { a, b } => {
                if a == b {
                    format!("两个 AI 同名,都叫「{a}」—— 对话里会分不清谁在说话。")
                } else {
                    format!("两个 AI 撞名:「{a}」与「{b}」—— 对话里会分不清谁在说话。")
                }
            }
            Violation::SameHost { host } => format!(
                "两侧都指向 {host},这不是两个独立的 API。请换一个服务商;若你确实想让同一家公司的两个模型互聊,\
                 请在设置里把接入模式改选「同源双模」(界面会全程标记这一点)。"
            ),
            Violation::SameModel { model } => format!(
                "同源双模下两侧模型相同({model}),那等于对镜自语,不是聊天。请至少让模型不同。"
            ),
            Violation::SameKey => "独立接入模式下两侧用了同一把 API Key,请各自使用独立的凭证。".into(),
            Violation::SamePersona => "两侧的公司、模型与气质完全相同,两个「人」没有区分度。".into(),
            Violation::NotTwoAgents { count } => format!("引擎需要至少两个 AI 档案,当前有 {count} 个。"),
        }
    }
}

/// `https://API.DeepSeek.com/v1/` -> `api.deepseek.com`(小写、去 userinfo/query/fragment、去端口、去尾点)
pub fn norm_host(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    // 补 scheme 好让 url 解析拿到 host;解析失败则退回手工切分
    let with_scheme = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    let h = match url::Url::parse(&with_scheme) {
        Ok(u) => u.host_str().unwrap_or("").to_string(),
        Err(_) => {
            let mut t = s.to_string();
            if let Some(i) = t.find("://") {
                t = t[i + 3..].to_string();
            }
            t = t.split(['/', '?', '#']).next().unwrap_or("").to_string();
            t = t.split('@').next_back().unwrap_or("").to_string();
            t
        }
    };
    let h = h.split(':').next().unwrap_or("").to_string();
    let mut h = h.to_lowercase();
    while h.ends_with('.') {
        h.pop();
    }
    h
}

fn base_url_shape_ok(raw: &str) -> bool {
    let s = raw.trim();
    if s.is_empty() {
        return false;
    }
    let probe = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    match url::Url::parse(&probe) {
        Ok(u) => {
            let scheme = u.scheme();
            (scheme == "https" || scheme == "http") && !norm_host(s).is_empty()
        }
        Err(_) => false,
    }
}

fn incomplete(p: &AgentProfile) -> Option<Violation> {
    let who = if p.name.trim().is_empty() {
        p.id.clone()
    } else {
        p.name.clone()
    };
    for (field, ok) in [
        ("接口地址 Base URL", base_url_shape_ok(&p.base_url)),
        ("API Key", !p.api_key.trim().is_empty()),
        ("模型 model", !p.model.trim().is_empty()),
        ("名字", !p.name.trim().is_empty()),
        ("公司名", !p.company.trim().is_empty()),
    ] {
        if !ok {
            return Some(Violation::Incomplete {
                agent: who.clone(),
                field: field.into(),
            });
        }
    }
    None
}

/// 全量校验。返回空向量 = 可以开局。
pub fn validate_app(cfg: &AppConfig) -> Vec<Violation> {
    let mut out = Vec::new();
    if cfg.agents.len() < 2 {
        out.push(Violation::NotTwoAgents {
            count: cfg.agents.len(),
        });
        return out;
    }
    for (i, p) in cfg.agents.iter().enumerate() {
        if let Some(v) = incomplete(p) {
            out.push(v);
        }
        for prior in &cfg.agents[..i] {
            if !p.id.is_empty() && p.id == prior.id {
                out.push(Violation::DuplicateName {
                    a: p.id.clone(),
                    b: prior.id.clone(),
                });
            }
            if p.name.trim().eq_ignore_ascii_case(prior.name.trim()) {
                out.push(Violation::DuplicateName {
                    a: p.name.trim().into(),
                    b: prior.name.trim().into(),
                });
            }
        }
    }
    if !out.is_empty() {
        return out;
    }
    for i in 0..cfg.agents.len() {
        for j in (i + 1)..cfg.agents.len() {
            out.extend(validate_pair(
                cfg.workspace.mode,
                &cfg.agents[i],
                &cfg.agents[j],
            ));
        }
    }
    out
}

/// 成对校验(核心,供单测直接驱动)。
pub fn validate_pair(mode: AccessMode, a: &AgentProfile, b: &AgentProfile) -> Vec<Violation> {
    let mut v = Vec::new();
    let na = a.name.trim();
    let nb = b.name.trim();
    if na.eq_ignore_ascii_case(nb) {
        v.push(Violation::DuplicateName {
            a: na.into(),
            b: nb.into(),
        });
    }
    if a.id == b.id {
        v.push(Violation::DuplicateName {
            a: a.id.clone(),
            b: b.id.clone(),
        });
    }

    let ha = norm_host(&a.base_url);
    let hb = norm_host(&b.base_url);
    let same_host = !ha.is_empty() && ha == hb;
    let ma = a.model.trim();
    let mb = b.model.trim();
    let same_model = !ma.is_empty() && ma == mb;

    match mode {
        AccessMode::Independent => {
            if same_host {
                v.push(Violation::SameHost { host: ha });
            }
            if a.api_key.trim() == b.api_key.trim() {
                v.push(Violation::SameKey);
            }
        }
        AccessMode::SameOriginDualModel => {
            if same_model {
                v.push(Violation::SameModel { model: ma.into() });
            }
        }
        AccessMode::SharedEndpoint => {}
    }

    // 人设差异:公司 + 模型 + 气质 全同才算没区分度(V-6)
    let persona_same = a.company.trim() == b.company.trim()
        && a.model.trim() == b.model.trim()
        && a.temperament.trim() == b.temperament.trim();
    if persona_same {
        v.push(Violation::SamePersona);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentProfile, AppConfig};

    fn prof(
        id: &str,
        name: &str,
        company: &str,
        base: &str,
        key: &str,
        model: &str,
    ) -> AgentProfile {
        AgentProfile {
            id: id.into(),
            name: name.into(),
            company: company.into(),
            base_url: base.into(),
            api_key: key.into(),
            model: model.into(),
            temperament: format!("{name} 的气质"),
            ..Default::default()
        }
    }

    fn pair() -> (AgentProfile, AgentProfile) {
        (
            prof(
                "A",
                "甲",
                "DeepSeek",
                "https://api.deepseek.com",
                "sk-aaaa1111",
                "deepseek-v4-pro",
            ),
            prof(
                "B",
                "乙",
                "Moonshot",
                "https://api.moonshot.cn/v1",
                "sk-bbbb2222",
                "kimi-k3",
            ),
        )
    }

    #[test]
    fn two_genuinely_independent_apis_pass() {
        let (a, b) = pair();
        assert!(validate_pair(AccessMode::Independent, &a, &b).is_empty());
    }

    #[test]
    fn independent_mode_blocks_same_host() {
        let (a, mut b) = pair();
        b.base_url = "https://api.deepseek.com/v1".into();
        b.model = "deepseek-chat".into();
        let v = validate_pair(AccessMode::Independent, &a, &b);
        assert!(
            v.iter().any(|x| matches!(x, Violation::SameHost { .. })),
            "{v:?}"
        );
        // 切到同源双模才放行,且模型必须不同
        let v2 = validate_pair(AccessMode::SameOriginDualModel, &a, &b);
        assert!(v2.is_empty(), "{v2:?}");
    }

    #[test]
    fn same_origin_mode_still_blocks_identical_model() {
        let (a, mut b) = pair();
        b.base_url = a.base_url.clone();
        b.model = a.model.clone();
        let v = validate_pair(AccessMode::SameOriginDualModel, &a, &b);
        assert!(
            v.iter().any(|x| matches!(x, Violation::SameModel { .. })),
            "{v:?}"
        );
    }

    #[test]
    fn independent_mode_blocks_shared_key() {
        let (a, mut b) = pair();
        b.api_key = a.api_key.clone();
        b.base_url = "https://api.moonshot.cn/v1".into();
        let v = validate_pair(AccessMode::Independent, &a, &b);
        assert!(v.contains(&Violation::SameKey), "{v:?}");
    }

    #[test]
    fn duplicate_names_blocked() {
        let (a, mut b) = pair();
        b.name = " 甲 ".into(); // 首尾空白忽略
        let v = validate_pair(AccessMode::Independent, &a, &b);
        assert!(
            v.iter()
                .any(|x| matches!(x, Violation::DuplicateName { .. })),
            "{v:?}"
        );
    }

    #[test]
    fn identical_persona_blocked() {
        let (a, mut b) = pair();
        b.company = a.company.clone();
        b.model = a.model.clone();
        b.temperament = a.temperament.clone();
        b.base_url = "https://api.moonshot.cn/v1".into();
        let v = validate_pair(AccessMode::Independent, &a, &b);
        assert!(v.contains(&Violation::SamePersona), "{v:?}");
    }

    #[test]
    fn incomplete_profile_reports_exact_field() {
        let mut cfg = AppConfig::default();
        let (a, b) = pair();
        cfg.agents = vec![a, {
            let mut x = b;
            x.api_key = "   ".into();
            x
        }];
        let v = validate_app(&cfg);
        assert_eq!(v.len(), 1, "{v:?}");
        match &v[0] {
            Violation::Incomplete { agent, field } => {
                assert_eq!(agent, "乙");
                assert!(field.contains("Key"), "{field}");
            }
            other => panic!("期望 Incomplete,得到 {other:?}"),
        }
        assert!(v[0].human_message().contains("乙"));
    }

    #[test]
    fn empty_default_config_is_blocked() {
        // 出厂默认配置绝不能开局:必须先把两个 API 填成真实独立的两家
        let cfg = AppConfig::default();
        let v = validate_app(&cfg);
        assert!(!v.is_empty());
        assert!(
            v.iter().all(|x| matches!(x, Violation::Incomplete { .. })),
            "{v:?}"
        );
    }

    #[test]
    fn not_two_agents_is_fatal() {
        let mut cfg = AppConfig::default();
        cfg.agents.truncate(1);
        let v = validate_app(&cfg);
        assert_eq!(v[0], Violation::NotTwoAgents { count: 1 });
    }

    #[test]
    fn norm_host_flavors() {
        assert_eq!(norm_host("https://API.DeepSeek.com/v1"), "api.deepseek.com");
        assert_eq!(norm_host("api.deepseek.com"), "api.deepseek.com");
        assert_eq!(norm_host("http://localhost:11434/v1"), "localhost");
        assert_eq!(norm_host("https://x.example.com."), "x.example.com");
        assert_eq!(
            norm_host("https://user:pw@host.example.com/p"),
            "host.example.com"
        );
        assert_eq!(norm_host("  "), "");
        assert_eq!(norm_host("https://a.com?h=b.com"), "a.com");
    }

    #[test]
    fn base_url_shape_accepts_localhost_http() {
        assert!(base_url_shape_ok("http://localhost:11434/v1"));
        assert!(base_url_shape_ok("api.deepseek.com"));
        assert!(!base_url_shape_ok("not a url at all here"));
        assert!(!base_url_shape_ok(""));
        assert!(!base_url_shape_ok("ftp://x.com"));
    }

    #[test]
    fn duplicate_name_message_reads_naturally() {
        let v = Violation::DuplicateName {
            a: "甲".into(),
            b: "甲".into(),
        };
        let m = v.human_message();
        assert!(m.contains("同名"), "{m}");
        assert!(!m.contains(")"), "不该留下没配对的括号:{m}");
        let v2 = Violation::DuplicateName {
            a: "A".into(),
            b: "B".into(),
        };
        assert!(v2.human_message().contains("撞名"));
    }
}
