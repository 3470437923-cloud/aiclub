//! 上下文重建(§6):每轮喂给模型的不是内存 history,而是从 events.jsonl 重新渲染的文本。
//!
//! 隔离性铁律(D-011 / §4.3):
//! - 某方的上下文**只含**共享台面(双方发言 + 双方动作的一行摘要)+ 该方自己的私有记忆;
//! - **绝不**把对方的记忆、对方的工具结果全文放进这一方的上下文;
//! - 对方的工具动作只给一行摘要 —— 想看内容必须自己去 read(这是"互相测试"的引擎)。

use crate::events::{Event, UsagePair};
use std::collections::HashMap;

pub struct CtxOpts<'a> {
    pub self_id: &'a str,
    pub self_name: &'a str,
    pub other_name: &'a str,
    pub opening_note: &'a str,
    pub system_notes: &'a [String],
    pub budget_chars: usize,
}

/// 摘要行:对方做了什么,一行,不给内容。
fn summarize_other(ev: &Event, other_name: &str) -> Option<String> {
    if let Event::ToolCall {
        name,
        ok,
        preview,
        ms,
        ..
    } = ev
    {
        let mark = if *ok { "ok" } else { "失败" };
        Some(format!(
            "[动作·{other_name}] {name} → {preview}({mark},{ms}ms)"
        ))
    } else {
        None
    }
}

fn render_turn(ev: &Event, o: &CtxOpts, names: &HashMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    match ev {
        Event::SayEnd { agent, text, .. } => {
            if text.trim().is_empty() {
                return out;
            }
            let who = if agent == o.self_id {
                o.self_name.to_string()
            } else {
                names
                    .get(agent)
                    .cloned()
                    .unwrap_or_else(|| o.other_name.to_string())
            };
            out.push(format!("{who}: {text}"));
        }
        Event::Pass { agent, .. } => {
            let who = if agent == o.self_id {
                o.self_name.to_string()
            } else {
                names
                    .get(agent)
                    .cloned()
                    .unwrap_or_else(|| o.other_name.to_string())
            };
            out.push(format!("{who}:(本轮沉默)"));
        }
        Event::ToolCall { agent, name, .. } => {
            if agent == o.self_id {
                // 自己的动作:给全文结果(自己写的东西自己当然看得见)
                if let Event::ToolCall {
                    args,
                    ok,
                    result_full,
                    ms,
                    ..
                } = ev
                {
                    let mark = if *ok { "成功" } else { "失败" };
                    out.push(format!(
                        "[你的动作] {name}({}) → {mark},{ms}ms\n{}",
                        serde_json::to_string(args).unwrap_or_default(),
                        result_full
                    ));
                }
            } else if let Some(s) = summarize_other(
                ev,
                names.get(agent).map(String::as_str).unwrap_or(o.other_name),
            ) {
                // 对方的动作:只有一行摘要,想看内容必须自己 read
                out.push(s);
            }
        }
        Event::Violation {
            agent,
            path,
            detail,
            ..
        } => {
            let who = if agent == o.self_id {
                o.self_name.to_string()
            } else {
                names
                    .get(agent)
                    .cloned()
                    .unwrap_or_else(|| o.other_name.to_string())
            };
            // 越界尝试是**公开**的:谁试图偷看被抓到,本身就是剧情
            out.push(format!("[系统] {who} 尝试访问 {path} 被拒绝:{detail}"));
        }
        Event::SystemNote { text, .. } => out.push(format!("[系统] {text}")),
        Event::TurnError {
            agent,
            error,
            attempt,
            retryable,
            ..
        } => {
            let who = if agent == o.self_id {
                "你".to_string()
            } else {
                names
                    .get(agent)
                    .cloned()
                    .unwrap_or_else(|| "对方".to_string())
            };
            out.push(format!(
                "[系统] {who}第 {attempt} 次调用失败:{error}{}",
                if *retryable { "(已重试)" } else { "" }
            ));
        }
        _ => {}
    }
    out
}

/// 渲染完整上下文。返回 (正文, 是否触发了截断)。
pub fn render_context(events: &[Event], o: &CtxOpts) -> (String, bool) {
    let mut names = HashMap::new();
    for ev in events {
        if let Event::SessionOpen { agents, .. } = ev {
            for agent in agents {
                names.insert(agent.id.clone(), agent.name.clone());
            }
        }
    }
    let mut body = Vec::new();
    for ev in events {
        if matches!(
            ev,
            Event::SessionOpen { .. } | Event::SessionEnd { .. } | Event::TurnBegin { .. }
        ) {
            continue;
        }
        // 对方的工具结果全文不进本方上下文:分流在 render_turn 里按 agent 做
        body.extend(render_turn(ev, o, &names));
    }

    // 预算裁剪:从**最旧**开始丢,保留最近;开头插一条说明,免得模型以为没发生过
    let head: Vec<String> = {
        let mut h = Vec::new();
        h.push(format!("# 台面直播({} 视角)", o.self_name));
        for n in o.system_notes {
            h.push(format!("[规则] {n}"));
        }
        h
    };
    let head_len: usize = head.iter().map(|l| l.len() + 1).sum::<usize>() + 40;
    let mut keep: Vec<String> = Vec::new();
    let mut used = head_len;
    let mut dropped = false;
    for l in body.iter().rev() {
        let cost = l.chars().count() + 1;
        if used + cost > o.budget_chars {
            dropped = true;
            break;
        }
        used += cost;
        keep.push(l.clone());
    }
    keep.reverse();

    let mut lines: Vec<String> = Vec::new();
    lines.extend(head);
    if body.is_empty() {
        lines.push(format!(
            "你是 {},对方是 {}。还没有任何对话 —— 这是开局,说什么都行。",
            o.self_name, o.other_name
        ));
        lines.push(String::new());
        lines.push("## 对话与动作".into());
        lines.push("(空)".into());
    } else if dropped {
        lines.push(format!(
            "你是 {},对方是 {}。更早的若干条已被丢弃(超出上下文预算),以下从可见处开始。",
            o.self_name, o.other_name
        ));
        lines.push(String::new());
        lines.push("## 对话与动作(截断)".into());
        lines.extend(keep);
    } else {
        lines.push(format!(
            "你是 {},对方是 {}。以下按时间顺序记录到目前为止发生的事。",
            o.self_name, o.other_name
        ));
        lines.push(String::new());
        lines.push("## 对话与动作".into());
        lines.extend(body);
    }
    (lines.join("\n"), dropped)
}

/// 隔离性断言用:这一方上下文里**不该**出现的字符串集合。
pub fn forbidden_markers(events: &[Event], self_id: &str, other_id: &str) -> Vec<String> {
    let mut v = Vec::new();
    for ev in events {
        if let Event::ToolCall {
            agent,
            result_full,
            name,
            ..
        } = ev
        {
            // 对方工具结果全文绝不进本方上下文
            if agent == other_id && result_full.chars().count() > 40 {
                let head: String = result_full.chars().take(40).collect();
                v.push(format!("{name}:{head}"));
            }
        }
    }
    let _ = self_id;
    v
}

/// 压缩纪要注入格式。
pub fn summary_block(agent_name: &str, text: &str) -> String {
    format!("[系统纪要·关于{agent_name}视角的早期对话]\n{text}")
}

/// 从事件流里取某方累计用量。
pub fn usage_of(events: &[Event], id: &str) -> UsagePair {
    let mut acc = UsagePair::default();
    for ev in events {
        if let Event::SayEnd { agent, usage, .. } = ev {
            if agent == id {
                acc.input += usage.input;
                acc.output += usage.output;
            }
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Event as E;

    fn say(turn: u32, who: &str, t: &str) -> E {
        E::SayEnd {
            turn,
            agent: who.into(),
            seq: 0,
            text: t.into(),
            usage: UsagePair {
                input: 1,
                output: 1,
                cached: None,
            },
            ms: 10,
            finish_reason: Some("stop".into()),
        }
    }
    fn tool(turn: u32, who: &str, name: &str, prev: &str, full: &str) -> E {
        E::ToolCall {
            turn,
            agent: who.into(),
            call_id: format!("{who}-{turn}"),
            name: name.into(),
            args: serde_json::json!({"path": "arena/x.py"}),
            ok: true,
            preview: prev.into(),
            result_full: full.into(),
            ms: 20,
        }
    }

    #[test]
    fn own_turns_show_full_results() {
        let evs = vec![tool(
            1,
            "A",
            "read",
            "arena/x.py",
            "def f():\n    return 42",
        )];
        let (ctx, _) = render_context(
            &evs,
            &CtxOpts {
                self_id: "A",
                self_name: "甲",
                other_name: "乙",
                opening_note: "",
                system_notes: &[],
                budget_chars: 8000,
            },
        );
        assert!(ctx.contains("def f()"), "自己的工具结果应给全文:\n{ctx}");
    }

    #[test]
    fn other_side_actions_are_summary_only() {
        let secret = "SECRET_BODY_should_never_leak=".to_string() + &"z".repeat(200);
        let evs = vec![tool(1, "B", "write", "arena/b.py", &secret)];
        let (ctx, _) = render_context(
            &evs,
            &CtxOpts {
                self_id: "A",
                self_name: "甲",
                other_name: "乙",
                opening_note: "",
                system_notes: &[],
                budget_chars: 8000,
            },
        );
        assert!(
            !ctx.contains("SECRET_BODY_should_never_leak"),
            "对方的动作全文不得进入本方上下文:\n{ctx}"
        );
        assert!(
            ctx.contains("[动作·乙] write → arena/b.py"),
            "应保留一行摘要:\n{ctx}"
        );
    }

    #[test]
    fn violation_is_public() {
        let evs = vec![E::Violation {
            turn: 2,
            agent: "B".into(),
            kind: "sandbox".into(),
            path: "memory/A/notes.jsonl".into(),
            detail: "那是另一个 AI 的私有记忆".into(),
        }];
        let (ctx, _) = render_context(
            &evs,
            &CtxOpts {
                self_id: "A",
                self_name: "甲",
                other_name: "乙",
                opening_note: "",
                system_notes: &[],
                budget_chars: 8000,
            },
        );
        assert!(
            ctx.contains("乙 尝试访问 memory/A/notes.jsonl 被拒绝"),
            "{ctx}"
        );
    }

    #[test]
    fn empty_stage_says_so() {
        let (ctx, _) = render_context(
            &[],
            &CtxOpts {
                self_id: "A",
                self_name: "甲",
                other_name: "乙",
                opening_note: "",
                system_notes: &[],
                budget_chars: 8000,
            },
        );
        assert!(ctx.contains("还没有任何对话"));
    }

    #[test]
    fn budget_truncation_keeps_recent_and_announces() {
        let mut evs = Vec::new();
        for i in 0..80u32 {
            evs.push(say(
                i,
                if i % 2 == 0 { "A" } else { "B" },
                &format!("第{i}句,这是一段足够长的中文内容用来制造压力。"),
            ));
        }
        let (ctx, dropped) = render_context(
            &evs,
            &CtxOpts {
                self_id: "A",
                self_name: "甲",
                other_name: "乙",
                opening_note: "",
                system_notes: &[],
                budget_chars: 1200,
            },
        );
        assert!(dropped);
        assert!(ctx.contains("更早的若干条已被丢弃"));
        assert!(ctx.contains("第79句"), "最近的必须留着");
        assert!(!ctx.contains("第0句"), "最旧的应被丢掉");
        assert!(
            ctx.chars().count() <= 1600,
            "不该远超预算(按字符计):{}",
            ctx.chars().count()
        );
    }

    #[test]
    fn pass_renders_as_silence() {
        let evs = vec![E::Pass {
            turn: 3,
            agent: "B".into(),
        }];
        let (ctx, _) = render_context(
            &evs,
            &CtxOpts {
                self_id: "A",
                self_name: "甲",
                other_name: "乙",
                opening_note: "",
                system_notes: &[],
                budget_chars: 8000,
            },
        );
        assert!(ctx.contains("乙:(本轮沉默)"), "{ctx}");
    }

    #[test]
    fn forbidden_markers_lists_other_full_results() {
        let evs = vec![tool(1, "B", "read", "a", &"y".repeat(80))];
        let f = forbidden_markers(&evs, "A", "B");
        assert_eq!(f.len(), 1);
        let (ctx, _) = render_context(
            &evs,
            &CtxOpts {
                self_id: "A",
                self_name: "甲",
                other_name: "乙",
                opening_note: "",
                system_notes: &[],
                budget_chars: 8000,
            },
        );
        for marker in &f {
            assert!(!ctx.contains(marker.as_str()), "泄漏:{marker}");
        }
    }

    #[test]
    fn usage_accumulates_per_agent() {
        let evs = vec![
            E::SayEnd {
                turn: 1,
                agent: "A".into(),
                seq: 0,
                text: "x".into(),
                usage: UsagePair {
                    input: 10,
                    output: 5,
                    cached: None,
                },
                ms: 1,
                finish_reason: None,
            },
            E::SayEnd {
                turn: 2,
                agent: "B".into(),
                seq: 0,
                text: "y".into(),
                usage: UsagePair {
                    input: 99,
                    output: 99,
                    cached: None,
                },
                ms: 1,
                finish_reason: None,
            },
            E::SayEnd {
                turn: 3,
                agent: "A".into(),
                seq: 0,
                text: "z".into(),
                usage: UsagePair {
                    input: 1,
                    output: 1,
                    cached: None,
                },
                ms: 1,
                finish_reason: None,
            },
        ];
        let a = usage_of(&evs, "A");
        assert_eq!((a.input, a.output), (11, 6), "分账不得互相污染");
        assert_eq!(usage_of(&evs, "B").input, 99);
    }
}
