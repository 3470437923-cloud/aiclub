//! 工具层(§7):schema 声明 + dispatch + 结果渲染。
//!
//! 16 个工具,刻意做小。全部路径经 `sandbox::resolve` + `check_within` 双闸。
//! 工具失败**不崩引擎**:一律回 `ok=false` 让模型自纠。

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::ShellConfig;
use crate::events::Event;
use crate::memory::MemoryStore;
use crate::provider::ToolSchema;
use crate::sandbox::{self, Access, SandboxError};
use crate::shell::ShellRunner;
use crate::skills::{self, SkillMeta};

/// 对话类工具的执行结果(引擎自己消化,不进文件层)
#[derive(Debug, Clone, PartialEq)]
pub enum ChatAct {
    Say(String),
    Pass,
    Withdraw(String),
    CloseTable(String),
}

#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub ok: bool,
    pub preview: String,
    pub full: String,
    pub ms: u64,
    /// Some(_) = 对话类动作,引擎据此结束本轮
    pub act: Option<ChatAct>,
    /// Some(_) = 这是一次越界尝试,引擎要额外记 violation 公开事件
    pub violation: Option<(String, String)>,
}

impl ToolOutcome {
    fn done(ok: bool, preview: String, full: String, ms: u64) -> Self {
        Self {
            ok,
            preview,
            full,
            ms,
            act: None,
            violation: None,
        }
    }
    fn fail(msg: String, ms: u64) -> Self {
        Self::done(false, msg.clone(), msg, ms)
    }
}

pub struct ToolCtx<'a> {
    pub workspace: &'a Path,
    pub agent_id: &'a str,
    pub agent_name: &'a str,
    pub other_name: &'a str,
    pub turn: u32,
    pub session: &'a str,
    pub memory: &'a MemoryStore,
    pub skills: &'a Vec<SkillMeta>,
    pub shell: &'a ShellConfig,
}

pub fn schemas(shell_enabled: bool) -> Vec<ToolSchema> {
    let mut v = vec![
        ToolSchema {
            name: "say".into(),
            description: "正式发言。这是你唯一让声音被台面上记录的方式;不调用 say 时,你的文本输出也算发言。".into(),
            parameters: json!({"type":"object","properties":{"text":{"type":"string","description":"要对对方说的话"}},"required":["text"]}),
        },
        ToolSchema {
            name: "pass".into(),
            description: "本轮保持沉默。沉默是合法动作 —— 连续几轮没人说话,这场对话就会自然冷场结束。".into(),
            parameters: json!({"type":"object","properties":{}}),
        },
        ToolSchema {
            name: "close_table".into(),
            description: "散会。由你们自己决定什么时候结束,不需要理由充分。给出一个原因即可。".into(),
            parameters: json!({"type":"object","properties":{"reason":{"type":"string"}},"required":["reason"]}),
        },
        ToolSchema {
            name: "withdraw".into(),
            description: "退出当前对话但不替对方宣布散会;给出原因。".into(),
            parameters: json!({"type":"object","properties":{"reason":{"type":"string"}},"required":["reason"]}),
        },
        ToolSchema {
            name: "end_turn".into(),
            description: "结束本轮并保持沉默;等同于 pass。".into(),
            parameters: json!({"type":"object","properties":{}}),
        },
        ToolSchema {
            name: "read".into(),
            description: "读文本文件。可读区:arena/(共享)、skills/<slug>(共享只读)、skills/private/<你自己>/、memory/<你自己>/。".into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"相对路径,如 arena/x.py"}},"required":["path"]}),
        },
        ToolSchema {
            name: "write".into(),
            description: "创建或整体覆写文件。可写区:arena/、memory/<你自己>/、skills/private/<你自己>/。content 必填且不能为空。".into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
        },
        ToolSchema {
            name: "edit".into(),
            description: "字面替换。old_text 若在文件中出现多次且未设 all,会报错要求你写得更具体。".into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"},"all":{"type":"boolean"}},"required":["path","old_text","new_text"]}),
        },
        ToolSchema {
            name: "list".into(),
            description: "列目录。dir 省略时列 arena/。".into(),
            parameters: json!({"type":"object","properties":{"dir":{"type":"string"},"depth":{"type":"integer"}}}),
        },
        ToolSchema {
            name: "search".into(),
            description: "在工作区内按子串检索文本文件,返回 路径:行号:内容。".into(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string"},"dir":{"type":"string"}},"required":["query"]}),
        },
        ToolSchema {
            name: "mkdir".into(),
            description: "建目录(仅 arena/ 与自己的 memory/)。".into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        },
        ToolSchema {
            name: "move".into(),
            description: "移动或重命名。两端都要通过同一套路径规则。".into(),
            parameters: json!({"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"}},"required":["from","to"]}),
        },
        ToolSchema {
            name: "delete".into(),
            description: "删除文件(目录需为空)。仅 arena/ 与自己的 memory/。".into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        },
        ToolSchema {
            name: "remember".into(),
            description: "往你的私有记忆里写一条。对方永远看不到你的记忆 —— 但你的记忆跨局存活,下次开局你记得这次。".into(),
            parameters: json!({"type":"object","properties":{"text":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}}},"required":["text"]}),
        },
        ToolSchema {
            name: "recall".into(),
            description: "检索你自己的记忆(只搜自己的,搜不到对方的)。".into(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        },
        ToolSchema {
            name: "skill_load".into(),
            description: "取共享技能或你自己的 skills/private/<id>/ 技能全文;不能读取别人的私有技能。".into(),
            parameters: json!({"type":"object","properties":{"name":{"type":"string","description":"共享 slug 或 private/<你的 id>/<slug>"}},"required":["name"]}),
        },
        ToolSchema {
            name: "web_fetch".into(),
            description: "获取 HTTP(S) 文本,不含凭证,响应最多 10MiB,超时 20 秒。".into(),
            parameters: json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}),
        },
        ToolSchema {
            name: "download_file".into(),
            description: "下载 HTTP(S) 文件到 arena/、你的 memory/ 或你的 skills/private/;最多 10MiB,超时 20 秒。".into(),
            parameters: json!({"type":"object","properties":{"url":{"type":"string"},"path":{"type":"string"}},"required":["url","path"]}),
        },
        ToolSchema {
            name: "skill_download".into(),
            description: "从不含凭证的 HTTP(S) URL 下载 UTF-8 技能到自己的 private skills;最多 10MiB,20 秒超时,不接受 headers。".into(),
            parameters: json!({"type":"object","properties":{"url":{"type":"string"},"slug":{"type":"string"}},"required":["url","slug"]}),
        },
    ];
    if shell_enabled {
        v.push(ToolSchema {
            name: "run_shell".into(),
            description: "在 arena/ 内执行一条命令。固定 cwd=arena/,限时,输出截断,环境白名单(取不到任何 API Key)。这是你们互相验作业的主要手段:对方说它的代码能跑,你就跑一次。".into(),
            parameters: json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}),
        });
    }
    v
}

const READ_MAX: usize = 64 * 1024;

/// 按字节上限切字符串,**保证落在 UTF-8 字符边界上**。
///
/// 直接写 `&s[..n]` 在多字节文本上会 panic:中文一个字 3 字节,
/// 65536 这种"整齐"的上限极可能正好切在字符中间。读文件的上限是 64KiB,
/// 而 arena 里的内容多半是中文,所以这条路径一定会被踩到 —— 截断是常规操作,
/// 不该让整个工具调用崩掉。
fn safe_slice(s: &str, max: usize) -> &str {
    if max >= s.len() {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// 解析并校验一条路径(双闸)。返回错误时已带可直接回给模型的文案。
fn guard(ctx: &ToolCtx, rel: &str, access: Access) -> Result<PathBuf, SandboxError> {
    let p = sandbox::resolve(ctx.workspace, ctx.agent_id, rel, access)?;
    sandbox::check_agent_path(ctx.workspace, ctx.agent_id, &p, access)?;
    Ok(p)
}

pub async fn dispatch(ctx: &ToolCtx<'_>, name: &str, args: &Value) -> ToolOutcome {
    let started = Instant::now();
    let el = || started.elapsed().as_millis() as u64;
    match name {
        "say" => {
            let t = args
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if t.trim().is_empty() {
                return ToolOutcome::fail("say 的 text 为空。要沉默就调 pass。".into(), el());
            }
            ToolOutcome {
                ok: true,
                preview: format!("发言 {} 字", t.chars().count()),
                full: t.clone(),
                ms: el(),
                act: Some(ChatAct::Say(t)),
                violation: None,
            }
        }
        "pass" => ToolOutcome {
            ok: true,
            preview: "本轮沉默".into(),
            full: "(沉默)".into(),
            ms: el(),
            act: Some(ChatAct::Pass),
            violation: None,
        },
        "withdraw" => {
            let r = args
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("未给理由")
                .to_string();
            ToolOutcome {
                ok: true,
                preview: format!("退出:{}", trunc(&r, 60)),
                full: r.clone(),
                ms: el(),
                act: Some(ChatAct::Withdraw(r)),
                violation: None,
            }
        }
        "end_turn" => ToolOutcome {
            ok: true,
            preview: "本轮结束".into(),
            full: "(本轮结束)".into(),
            ms: el(),
            act: Some(ChatAct::Pass),
            violation: None,
        },
        "close_table" => {
            let r = args
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("未给理由")
                .to_string();
            ToolOutcome {
                ok: true,
                preview: format!("散会:{}", trunc(&r, 60)),
                full: r.clone(),
                ms: el(),
                act: Some(ChatAct::CloseTable(r)),
                violation: None,
            }
        }
        "read" => {
            let rel = sarg(args, "path");
            let p = match guard(ctx, &rel, Access::Read) {
                Ok(p) => p,
                Err(e) => return violation_outcome(&rel, e, el()),
            };
            if !p.exists() {
                return ToolOutcome::fail(format!("{rel} 不存在。用 list 看看有什么。"), el());
            }
            if p.is_dir() {
                return ToolOutcome::fail(
                    format!("{rel} 是目录,read 只能读文件。改用 list。"),
                    el(),
                );
            }
            match std::fs::read(&p) {
                Ok(bytes) => {
                    if bytes.iter().take(800).any(|b| *b == 0) {
                        return ToolOutcome::done(
                            true,
                            format!("{rel}(二进制,{} 字节)", bytes.len()),
                            format!("(二进制文件,已跳过内容读取;大小 {} 字节)", bytes.len()),
                            el(),
                        );
                    }
                    let txt = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
                    let (body, cut) = if txt.len() > READ_MAX {
                        (
                            format!(
                                "{}\n…(已截断,共 {} 字节)",
                                safe_slice(&txt, READ_MAX),
                                txt.len()
                            ),
                            true,
                        )
                    } else {
                        (txt, false)
                    };
                    let preview = format!("读 {rel}({} 字)", body.chars().count());
                    let full = if cut {
                        body
                    } else {
                        format!("// {rel}\n{body}")
                    };
                    ToolOutcome::done(true, preview, full, el())
                }
                Err(e) => ToolOutcome::fail(format!("读取失败:{e}"), el()),
            }
        }
        "write" => {
            let rel = sarg(args, "path");
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let p = match guard(ctx, &rel, Access::Write) {
                Ok(p) => p,
                Err(e) => return violation_outcome(&rel, e, el()),
            };
            if p.is_dir() {
                return ToolOutcome::fail(format!("{rel} 是目录。"), el());
            }
            if let Some(parent) = p.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return ToolOutcome::fail(format!("建目录失败:{e}"), el());
                }
            }
            let existed = p.exists();
            // 临时文件 + rename:不留半个文件
            let tmp = p.with_file_name(format!(".aiclub_tmp_{}", std::process::id()));
            let norm = content.replace("\r\n", "\n");
            match std::fs::write(&tmp, &norm).and_then(|_| std::fs::rename(&tmp, &p)) {
                Ok(_) => {
                    let preview = format!(
                        "{} {rel}({} 字)",
                        if existed { "改" } else { "写" },
                        norm.chars().count()
                    );
                    ToolOutcome::done(true, preview, format!("已写入 {rel}"), el())
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    ToolOutcome::fail(format!("写入失败:{e}"), el())
                }
            }
        }
        "edit" => {
            let rel = sarg(args, "path");
            let old = args.get("old_text").and_then(Value::as_str).unwrap_or("");
            let new = args.get("new_text").and_then(Value::as_str).unwrap_or("");
            let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
            if old.is_empty() {
                return ToolOutcome::fail("old_text 不能为空。".into(), el());
            }
            let p = match guard(ctx, &rel, Access::Write) {
                Ok(p) => p,
                Err(e) => return violation_outcome(&rel, e, el()),
            };
            let txt = match std::fs::read_to_string(&p) {
                Ok(t) => t,
                Err(e) => {
                    return ToolOutcome::fail(
                        format!("{rel} 读不到:{e}(要先存在才能 edit;新建用 write)"),
                        el(),
                    )
                }
            };
            let n = txt.matches(old).count();
            if n == 0 {
                return ToolOutcome::fail(
                    format!("{rel} 里找不到那段文本。先 read 确认原文。"),
                    el(),
                );
            }
            if n > 1 && !all {
                return ToolOutcome::fail(format!("那段文本在 {rel} 里出现 {n} 次,不唯一。要么写得更长更具体,要么设 all=true。"), el());
            }
            let out = if all {
                txt.replace(old, new)
            } else {
                txt.replacen(old, new, 1)
            };
            let tmp = p.with_file_name(format!(".aiclub_tmp_{}", std::process::id()));
            match std::fs::write(&tmp, &out).and_then(|_| std::fs::rename(&tmp, &p)) {
                Ok(_) => ToolOutcome::done(
                    true,
                    format!("改 {rel}(替换 {n} 处)"),
                    format!("已编辑 {rel}"),
                    el(),
                ),
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    ToolOutcome::fail(format!("编辑失败:{e}"), el())
                }
            }
        }
        "list" => {
            let dir = if args
                .get("dir")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty()
            {
                "arena".to_string()
            } else {
                sarg(args, "dir")
            };
            let depth = args
                .get("depth")
                .and_then(Value::as_u64)
                .unwrap_or(2)
                .min(4) as usize;
            let p = match guard(ctx, &dir, Access::Read) {
                Ok(p) => p,
                Err(e) => return violation_outcome(&dir, e, el()),
            };
            let mut lines = Vec::new();
            walk(ctx.workspace, &p, 0, depth, &mut lines);
            if lines.is_empty() {
                return ToolOutcome::done(
                    true,
                    format!("{dir} 为空"),
                    format!("{dir}/ (空)"),
                    el(),
                );
            }
            let body = lines.join("\n");
            ToolOutcome::done(true, format!("列 {dir}({} 项)", lines.len()), body, el())
        }
        "search" => {
            let q = sarg(args, "query");
            if q.trim().is_empty() {
                return ToolOutcome::fail("query 不能为空。".into(), el());
            }
            let dir = if args
                .get("dir")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty()
            {
                "arena".to_string()
            } else {
                sarg(args, "dir")
            };
            let root = match guard(ctx, &dir, Access::Read) {
                Ok(p) => p,
                Err(e) => return violation_outcome(&dir, e, el()),
            };
            let needle = q.to_lowercase();
            let mut hits = Vec::new();
            let mut files = Vec::new();
            collect_files(ctx.workspace, &root, &mut files, 0, 5);
            for f in files.iter().take(400) {
                let Ok(t) = std::fs::read_to_string(f) else {
                    continue;
                };
                for (i, line) in t.lines().enumerate() {
                    if line.to_lowercase().contains(&needle) {
                        hits.push(format!(
                            "{}:{}: {}",
                            rel(ctx.workspace, f),
                            i + 1,
                            trunc(line.trim(), 160)
                        ));
                        if hits.len() >= 60 {
                            break;
                        }
                    }
                }
                if hits.len() >= 60 {
                    break;
                }
            }
            if hits.is_empty() {
                return ToolOutcome::done(
                    true,
                    format!("搜 {q}(0 命中)"),
                    format!("没找到「{q}」。"),
                    el(),
                );
            }
            let n = hits.len();
            ToolOutcome::done(true, format!("搜 {q}({n} 命中)"), hits.join("\n"), el())
        }
        "mkdir" => {
            let rel = sarg(args, "path");
            let p = match guard(ctx, &rel, Access::Write) {
                Ok(p) => p,
                Err(e) => return violation_outcome(&rel, e, el()),
            };
            match std::fs::create_dir_all(&p) {
                Ok(_) => {
                    ToolOutcome::done(true, format!("建目录 {rel}"), format!("已建 {rel}"), el())
                }
                Err(e) => ToolOutcome::fail(format!("建目录失败:{e}"), el()),
            }
        }
        "move" => {
            let from = sarg(args, "from");
            let to = sarg(args, "to");
            let pf = match guard(ctx, &from, Access::Write) {
                Ok(p) => p,
                Err(e) => return violation_outcome(&from, e, el()),
            };
            let pt = match guard(ctx, &to, Access::Write) {
                Ok(p) => p,
                Err(e) => return violation_outcome(&to, e, el()),
            };
            if !pf.exists() {
                return ToolOutcome::fail(format!("{from} 不存在。"), el());
            }
            if pt.exists() {
                return ToolOutcome::fail(format!("{to} 已存在,不做静默覆盖。"), el());
            }
            match std::fs::rename(&pf, &pt) {
                Ok(_) => {
                    ToolOutcome::done(true, format!("移动 {from} → {to}"), "已移动".into(), el())
                }
                Err(e) => ToolOutcome::fail(format!("移动失败:{e}"), el()),
            }
        }
        "delete" => {
            let rel = sarg(args, "path");
            let p = match guard(ctx, &rel, Access::Write) {
                Ok(p) => p,
                Err(e) => return violation_outcome(&rel, e, el()),
            };
            if !p.exists() {
                return ToolOutcome::fail(format!("{rel} 不存在。"), el());
            }
            let r = if p.is_dir() {
                std::fs::remove_dir(&p)
            } else {
                std::fs::remove_file(&p)
            };
            match r {
                Ok(_) => ToolOutcome::done(true, format!("删除 {rel}"), "已删除".into(), el()),
                Err(e) => ToolOutcome::fail(format!("删除失败:{e}"), el()),
            }
        }
        "remember" => {
            let text = sarg(args, "text");
            if text.trim().is_empty() {
                return ToolOutcome::fail("text 不能为空。".into(), el());
            }
            let tags = args
                .get("tags")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            let note = crate::memory::Note {
                ts: crate::events::now_ts(),
                session: ctx.session.to_string(),
                turn: ctx.turn,
                text: text.clone(),
                tags,
            };
            match ctx.memory.append(&note) {
                Ok(_) => ToolOutcome::done(
                    true,
                    format!("记了一条({} 字)", text.chars().count()),
                    "已写入你的私有记忆(对方看不到)。".into(),
                    el(),
                ),
                Err(e) => ToolOutcome::fail(format!("记忆写入失败:{e}"), el()),
            }
        }
        "recall" => {
            let q = sarg(args, "query");
            let hits = ctx.memory.recall(&q, 20);
            if hits.is_empty() {
                return ToolOutcome::done(
                    true,
                    format!("回忆 {q}(0)"),
                    format!("你的记忆里没提到「{q}」。"),
                    el(),
                );
            }
            let body = hits
                .iter()
                .map(|n| format!("[第{}轮·{}] {}", n.turn, n.session, n.text))
                .collect::<Vec<_>>()
                .join("\n");
            ToolOutcome::done(true, format!("回忆 {q}({} 条)", hits.len()), body, el())
        }
        "web_fetch" => match crate::network::web_fetch(args).await {
            Ok(text) => ToolOutcome::done(
                true,
                format!("抓取 {} 字", text.chars().count()),
                text,
                el(),
            ),
            Err(e) => ToolOutcome::fail(e, el()),
        },
        "download_file" => {
            let url = sarg(args, "url");
            let rel = sarg(args, "path");
            if let Err(e) = guard(ctx, &rel, Access::Write) {
                return violation_outcome(&rel, e, el());
            }
            match crate::network::download_file(ctx.workspace, ctx.agent_id, &url, &rel).await {
                Ok(msg) => ToolOutcome::done(true, format!("下载 {rel}"), msg, el()),
                Err(e) => ToolOutcome::fail(e, el()),
            }
        }
        "skill_download" => {
            let slug = sarg(args, "slug");
            let rel = format!("skills/private/{}/{}/SKILL.md", ctx.agent_id, slug);
            let _p = match guard(ctx, &rel, Access::Write) {
                Ok(p) => p,
                Err(e) => return violation_outcome(&rel, e, el()),
            };
            match crate::network::download_skill(ctx.workspace, ctx.agent_id, args).await {
                Ok(msg) => ToolOutcome::done(true, msg.clone(), msg, el()),
                Err(e) => ToolOutcome::fail(e, el()),
            }
        }
        "skill_load" => {
            let key = sarg(args, "name");
            match skills::load_for_agent(&ctx.workspace.join("skills"), ctx.agent_id, &key) {
                Some(s) => {
                    let _ = &ctx.skills;
                    ToolOutcome::done(
                        true,
                        format!("载入技能 {}", s.meta.name),
                        format!("# {}\n{}\n\n{}", s.meta.name, s.meta.description, s.body),
                        el(),
                    )
                }
                None => {
                    let have = ctx
                        .skills
                        .iter()
                        .map(|m| m.name.as_str())
                        .collect::<Vec<_>>()
                        .join("、");
                    ToolOutcome::fail(
                        format!("没有这个技能。现有的:{have}(用 skill 目录里的 name 或 slug)"),
                        el(),
                    )
                }
            }
        }
        "run_shell" => {
            if !ctx.shell.enabled {
                return ToolOutcome::fail("run_shell 已被关闭。".into(), el());
            }
            let runner = ShellRunner::new(crate::shell::arena_of(ctx.workspace), ctx.shell.clone());
            match runner.run(args).await {
                Ok(o) => {
                    let cmd = sarg(args, "command");
                    ToolOutcome::done(
                        o.ok,
                        format!("$ {} · exit {}", trunc(&cmd, 48), o.exit),
                        o.agent_text(),
                        el(),
                    )
                }
                Err(e) => ToolOutcome::fail(e, el()),
            }
        }
        other => ToolOutcome::fail(format!("未知工具 {other}。"), el()),
    }
}

/// 沙箱拒绝要怎么记账。
///
/// 关键区分:**"用错了路径" 与 "想看别人的东西" 不是一回事。**
/// - 写了个绝对路径、用了 `..`、路径为空 —— 这是模型不会用工具,给回一段
///   说明让它自己改就行,记成公开 violation 只会往台面上灌噪音。
/// - 解析之后真的越出了边界(符号链接)、或者伸手去读对方私有的 memory ——
///   这是**行为**,不是失误,必须成为公开事件:谁试图偷看被抓到,本身就是剧情。
fn violation_outcome(rel: &str, e: SandboxError, ms: u64) -> ToolOutcome {
    let msg = e.agent_message();
    let public = matches!(
        e,
        SandboxError::Escape { .. }
            | SandboxError::ReadOnly { .. }
            | SandboxError::OutsidePolicy { .. }
    );
    ToolOutcome {
        ok: false,
        preview: format!("拒绝访问 {rel}"),
        full: msg.clone(),
        ms,
        act: None,
        violation: if public {
            Some((rel.to_string(), msg))
        } else {
            None
        },
    }
}

fn sarg(args: &Value, k: &str) -> String {
    args.get(k)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn trunc(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        s.to_string()
    } else {
        format!("{}…", c[..n].iter().collect::<String>())
    }
}

fn rel(root: &Path, p: &Path) -> String {
    sandbox::display_rel(root, p)
}

/// 递归时对**每一个**条目复查沙箱边界。
///
/// 为什么只在入口 guard() 一次不够:guard() 只校验用户给出的那个起点,
/// 而 walk / collect_files 是用 `e.is_dir()` / `p.is_dir()` 递归的 ——
/// 这两个调用都会**跟随符号链接**。于是 `arena/link -> /Users/me/私密`
/// 这样一条链接,就能让 list 列出工作区外的目录结构、
/// 让 search 把工作区外的文件**内容**读进模型上下文(上传到第三方端点),
/// 还会写进 events.jsonl 与导出包。
///
/// 可达性不是理论问题:run_shell 的 cwd 固定在 arena/,
/// 而命令白名单不拦 `ln -s` —— AI 自己就能把这条链接造出来。
/// 用户的工作区里也可能本来就有符号链接目录(git checkout、包管理器都会造)。
///
/// 判据是 canonicalize 之后必须仍在工作区内。canonicalize 失败
/// (断链、自环 ELOOP)一律当作不可用 —— 宁可少列一个,不可越界。
fn contained(workspace: &Path, p: &Path) -> bool {
    sandbox::check_within(workspace, p).is_ok()
}

fn walk(workspace: &Path, dir: &Path, depth: usize, max: usize, out: &mut Vec<String>) {
    if depth > max {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for e in entries {
        let name = e
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.starts_with(".aiclub_tmp_") {
            continue;
        }
        let indent = "  ".repeat(depth);
        if e.is_dir() {
            // 进目录之前先复检:这是符号链接逃逸的唯一入口
            if !contained(workspace, &e) {
                out.push(format!("{indent}{name}/ (越界,已跳过)"));
                continue;
            }
            out.push(format!("{indent}{name}/"));
            walk(workspace, &e, depth + 1, max, out);
        } else {
            // 文件也要查:符号链接可以直接指向工作区外的一个文件
            if !contained(workspace, &e) {
                out.push(format!("{indent}{name} (越界,已跳过)"));
                continue;
            }
            let size = std::fs::metadata(&e).map(|m| m.len()).unwrap_or(0);
            out.push(format!("{indent}{name} ({})", human(size)));
        }
        if out.len() > 400 {
            return;
        }
    }
}

fn human(b: u64) -> String {
    if b < 1024 {
        format!("{b}B")
    } else if b < 1024 * 1024 {
        format!("{:.1}KB", b as f64 / 1024.0)
    } else {
        format!("{:.1}MB", b as f64 / 1048576.0)
    }
}

fn collect_files(
    workspace: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
    depth: usize,
    max_depth: usize,
) {
    if depth > max_depth {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        // 这里的结果会被调用方 read_to_string 后交给模型,所以必须逐个复检:
        // 漏掉一个符号链接就等于把工作区外的文件内容上传给了第三方端点。
        if !contained(workspace, &p) {
            continue;
        }
        if p.is_dir() {
            collect_files(workspace, &p, out, depth + 1, max_depth);
        } else {
            out.push(p);
        }
    }
}

/// 引擎在 commit 前需要把 ToolOutcome 变成事件
pub fn to_event(ctx: &ToolCtx, call_id: &str, name: &str, args: &Value, o: &ToolOutcome) -> Event {
    Event::ToolCall {
        turn: ctx.turn,
        agent: ctx.agent_id.to_string(),
        call_id: call_id.to_string(),
        name: name.to_string(),
        args: args.clone(),
        ok: o.ok,
        preview: o.preview.clone(),
        result_full: o.full.clone(),
        ms: o.ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;

    struct Fixture {
        ws: PathBuf,
        mem: MemoryStore,
        metas: Vec<SkillMeta>,
        shell: ShellConfig,
    }

    fn fx(tag: &str) -> Fixture {
        let ws = crate::testsup::world(tag);
        skills::seed_builtin(&ws.join("skills"));
        let mem = MemoryStore::open(&ws, "A").unwrap();
        let metas = skills::list(&ws.join("skills"));
        Fixture {
            ws,
            mem,
            metas,
            shell: ShellConfig::default(),
        }
    }

    fn ctx_of<'a>(f: &'a Fixture, turn: u32) -> ToolCtx<'a> {
        ToolCtx {
            workspace: &f.ws,
            agent_id: "A",
            agent_name: "甲",
            other_name: "乙",
            turn,
            session: "s1",
            memory: &f.mem,
            skills: &f.metas,
            shell: &f.shell,
        }
    }

    fn kill(f: &Fixture) {
        crate::testsup::cleanup(&f.ws);
    }

    #[tokio::test]
    async fn chat_actions_carry_act() {
        let f = fx("act");
        let c = ctx_of(&f, 1);
        let o = dispatch(&c, "say", &json!({"text":"你好"})).await;
        assert_eq!(o.act, Some(ChatAct::Say("你好".into())));
        assert!(dispatch(&c, "pass", &json!({})).await.act == Some(ChatAct::Pass));
        let cl = dispatch(&c, "close_table", &json!({"reason":"聊够了"})).await;
        assert_eq!(cl.act, Some(ChatAct::CloseTable("聊够了".into())));
        assert!(!dispatch(&c, "say", &json!({"text":"  "})).await.ok);
        kill(&f);
    }

    #[tokio::test]
    async fn write_read_edit_roundtrip() {
        let f = fx("wre");
        let c = ctx_of(&f, 1);
        let w = dispatch(
            &c,
            "write",
            &json!({"path":"arena/x.py","content":"def f():\n    return 1\n"}),
        )
        .await;
        assert!(w.ok, "{}", w.full);
        assert!(w.preview.contains("写 arena/x.py"), "{}", w.preview);
        let r = dispatch(&c, "read", &json!({"path":"arena/x.py"})).await;
        assert!(r.full.contains("return 1"), "{}", r.full);
        let e = dispatch(
            &c,
            "edit",
            &json!({"path":"arena/x.py","old_text":"return 1","new_text":"return 2"}),
        )
        .await;
        assert!(e.ok, "{}", e.full);
        let r2 = dispatch(&c, "read", &json!({"path":"arena/x.py"})).await;
        assert!(r2.full.contains("return 2"));
        // 不唯一 → 报错
        dispatch(
            &c,
            "write",
            &json!({"path":"arena/y.md","content":"aa\naa\naa"}),
        )
        .await;
        let amb = dispatch(
            &c,
            "edit",
            &json!({"path":"arena/y.md","old_text":"aa","new_text":"bb"}),
        )
        .await;
        assert!(!amb.ok);
        assert!(amb.full.contains("出现 3 次"), "{}", amb.full);
        let all = dispatch(
            &c,
            "edit",
            &json!({"path":"arena/y.md","old_text":"aa","new_text":"bb","all":true}),
        )
        .await;
        assert!(all.ok);
        kill(&f);
    }

    #[tokio::test]
    async fn crlf_is_normalized_to_lf() {
        let f = fx("lf");
        let c = ctx_of(&f, 1);
        dispatch(
            &c,
            "write",
            &json!({"path":"arena/lf.txt","content":"a\r\nb\r\n"}),
        )
        .await;
        let raw = std::fs::read(f.ws.join("arena/lf.txt")).unwrap();
        assert!(!raw.contains(&b'\r'), "写文件一律 LF:{raw:?}");
        kill(&f);
    }

    #[tokio::test]
    async fn sandbox_denials_are_violations_not_crashes() {
        let f = fx("deny");
        let c = ctx_of(&f, 1);
        let abs = dispatch(&c, "read", &json!({"path":"C:/Windows/win.ini"})).await;
        assert!(!abs.ok);
        assert!(
            abs.violation.is_none(),
            "绝对路径不是越界偷看,不该记 violation:{:?}",
            abs.violation
        );
        let peek = dispatch(&c, "read", &json!({"path":"memory/B/notes.jsonl"})).await;
        assert!(!peek.ok);
        assert!(peek.violation.is_some(), "偷看对方记忆必须记 violation");
        assert!(peek.full.contains("私有记忆"), "{}", peek.full);
        let ws2 = fx("deny2");
        let c2 = ctx_of(&ws2, 2);
        let sk = dispatch(
            &c2,
            "write",
            &json!({"path":"skills/x.md","content":"hack"}),
        )
        .await;
        assert!(!sk.ok && sk.full.contains("只读"), "{}", sk.full);
        kill(&f);
        kill(&ws2);
    }

    #[tokio::test]
    async fn delete_and_move_respect_guards() {
        let f = fx("mv");
        let c = ctx_of(&f, 1);
        dispatch(&c, "write", &json!({"path":"arena/a.txt","content":"1"})).await;
        let mv = dispatch(
            &c,
            "move",
            &json!({"from":"arena/a.txt","to":"arena/b.txt"}),
        )
        .await;
        assert!(mv.ok, "{}", mv.full);
        let dup = dispatch(
            &c,
            "move",
            &json!({"from":"arena/b.txt","to":"arena/b.txt"}),
        )
        .await;
        assert!(!dup.ok, "不允许静默覆盖");
        let del = dispatch(&c, "delete", &json!({"path":"arena/b.txt"})).await;
        assert!(del.ok);
        let again = dispatch(&c, "delete", &json!({"path":"arena/b.txt"})).await;
        assert!(!again.ok);
        kill(&f);
    }

    #[tokio::test]
    async fn list_and_search_work() {
        let f = fx("ls");
        let c = ctx_of(&f, 1);
        dispatch(&c, "mkdir", &json!({"path":"arena/sub"})).await;
        dispatch(
            &c,
            "write",
            &json!({"path":"arena/sub/deep.md","content":"RICKROLL 在这里"}),
        )
        .await;
        let l = dispatch(&c, "list", &json!({})).await;
        assert!(l.full.contains("sub/"), "{}", l.full);
        assert!(l.full.contains("deep.md"), "{}", l.full);
        let s = dispatch(&c, "search", &json!({"query":"rickroll"})).await;
        assert!(s.full.contains("arena/sub/deep.md:1:"), "{}", s.full);
        let none = dispatch(&c, "search", &json!({"query":"不存在zzz"})).await;
        assert!(none.full.contains("没找到"), "{}", none.full);
        kill(&f);
    }

    #[tokio::test]
    async fn memory_and_skill_tools() {
        let f = fx("mem");
        let c = ctx_of(&f, 1);
        let r = dispatch(
            &c,
            "remember",
            &json!({"text":"乙说它能背圆周率一百位","tags":["对方","存疑"]}),
        )
        .await;
        assert!(r.ok, "{}", r.full);
        assert_eq!(c.memory.count(), 1);
        let rec = dispatch(&c, "recall", &json!({"query":"圆周率"})).await;
        assert!(rec.full.contains("乙说它"), "{}", rec.full);
        let sk = dispatch(&c, "skill_load", &json!({"name":"code-review"})).await;
        assert!(sk.full.contains("互相验作业"), "{}", sk.full);
        let bad = dispatch(&c, "skill_load", &json!({"name":"nope"})).await;
        assert!(!bad.ok);
        assert!(
            bad.full.contains("code-review") || bad.full.contains("互相面试"),
            "应列出现有技能:{}",
            bad.full
        );
        kill(&f);
    }

    #[tokio::test]
    async fn read_handles_missing_dir_and_binary() {
        let f = fx("rd");
        let c = ctx_of(&f, 1);
        assert!(!dispatch(&c, "read", &json!({"path":"arena/nope"})).await.ok);
        let d = dispatch(&c, "read", &json!({"path":"arena"})).await;
        assert!(!d.ok && d.full.contains("list"), "{}", d.full);
        std::fs::write(f.ws.join("arena").join("b.bin"), [0u8, 1, 2, 3, 255, 9]).unwrap();
        let b = dispatch(&c, "read", &json!({"path":"arena/b.bin"})).await;
        assert!(b.ok && b.full.contains("二进制"), "{:?}", b);
        let big = "字".repeat(40000); // 120KB > READ_MAX
        dispatch(&c, "write", &json!({"path":"arena/big.md","content":big})).await;
        let got = dispatch(&c, "read", &json!({"path":"arena/big.md"})).await;
        assert!(got.full.contains("已截断"), "大文件必须截断");
        kill(&f);
    }

    #[tokio::test]
    async fn unknown_tool_is_error_not_panic() {
        let f = fx("unk");
        let c = ctx_of(&f, 1);
        let o = dispatch(&c, "nuke_everything", &json!({})).await;
        assert!(!o.ok);
        assert!(o.full.contains("未知工具"));
        kill(&f);
    }

    #[tokio::test]
    #[cfg(not(windows))]
    async fn shell_runs_in_arena() {
        let f = fx("sh");
        let c = ctx_of(&f, 1);
        dispatch(
            &c,
            "write",
            &json!({"path":"arena/echo.sh","content":"#!/bin/sh\necho hello-from-arena\n"}),
        )
        .await;
        let o = dispatch(&c, "run_shell", &json!({"command":"sh echo.sh"})).await;
        assert!(o.ok, "{:?}", o.full);
        assert!(o.full.contains("hello-from-arena"), "{}", o.full);
        kill(&f);
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn shell_runs_in_arena_win() {
        let f = fx("shw");
        let c = ctx_of(&f, 1);
        dispatch(
            &c,
            "write",
            &json!({"path":"arena/m.txt","content":"hello-from-arena"}),
        )
        .await;
        let o = dispatch(&c, "run_shell", &json!({"command":"type m.txt"})).await;
        assert!(o.ok, "{:?}", o.full);
        assert!(o.full.contains("hello-from-arena"), "{}", o.full);
        kill(&f);
    }

    #[test]
    fn schema_set_shape() {
        let with = schemas(true);
        let without = schemas(false);
        assert_eq!(
            with.len(),
            20,
            "{:?}",
            with.iter().map(|s| s.name.clone()).collect::<Vec<_>>()
        );
        assert_eq!(without.len(), 19);
        assert!(with.iter().any(|s| s.name == "run_shell"));
        assert!(!without.iter().any(|s| s.name == "run_shell"));
        assert!(with.iter().all(|s| !s.description.is_empty()));
        for s in &with {
            assert_eq!(s.parameters["type"], "object", "{} schema", s.name);
        }
    }

    #[test]
    fn preview_is_one_short_line() {
        // 摘要行是对面 AI 唯一能看到的关于你动作的信息 —— 必须短且单行
        let long = "x".repeat(400);
        let t = trunc(&long, 48);
        assert_eq!(t.chars().count(), 49);
        assert!(t.ends_with('…'));
    }

    /// 回归:list / search 的**递归**曾经完全不过沙箱。
    ///
    /// guard() 只校验入口路径,而 walk / collect_files 用 is_dir() 递归,
    /// 那个调用会跟随符号链接。于是 arena/ 里一条指向工作区外的链接,
    /// 就能让 search 把外面的文件**内容**读进模型上下文并上传给第三方端点。
    /// AI 自己就能造这条链接:run_shell 的 cwd 固定在 arena/,
    /// 而命令白名单不拦 ln -s。
    #[cfg(unix)]
    #[tokio::test]
    async fn list_and_search_do_not_follow_symlinks_out_of_workspace() {
        use std::os::unix::fs::symlink;

        let f = fx("symescape");
        // 工作区外放一个"机密"文件(与工作区同级,确保在边界之外)
        let outside = f.ws.parent().unwrap().join(format!(
            "{}-outside",
            f.ws.file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.md"), "TOPSECRET-CANARY-CONTENT").unwrap();

        // arena/ 里造两种逃逸:目录链接 + 文件链接
        let link_dir = f.ws.join("arena").join("linkdir");
        let link_file = f.ws.join("arena").join("linkfile.md");
        let _ = std::fs::remove_file(&link_dir);
        let _ = std::fs::remove_file(&link_file);
        symlink(&outside, &link_dir).unwrap();
        symlink(outside.join("secret.md"), &link_file).unwrap();

        let c = ctx_of(&f, 1);

        // list:不得列出工作区外的条目名
        let l = dispatch(&c, "list", &json!({"dir":"arena","depth":3})).await;
        assert!(
            !l.full.contains("secret.md") || l.full.contains("越界"),
            "list 不得越过符号链接列出工作区外内容:{:?}",
            l.full
        );

        // search:这是最要命的一条 —— 内容会进模型上下文
        let s = dispatch(&c, "search", &json!({"query":"TOPSECRET"})).await;
        assert!(
            !s.full.contains("TOPSECRET-CANARY-CONTENT"),
            "search 读到了工作区外的文件内容,等于把它上传给了第三方端点:{:?}",
            s.full
        );
        assert!(
            !s.full.contains("secret.md"),
            "search 连工作区外的文件名都不该提到:{:?}",
            s.full
        );
        // 注:preview 里的 "TOPSECRET" 是回显用户自己的查询词,不算泄露 ——
        // 这里要挡的是**文件内容**,不是查询串本身。
        assert!(
            !s.preview.contains("CANARY"),
            "摘要行不能带上工作区外的内容:{:?}",
            s.preview
        );

        // 别把闸修成"谁都进不来":工作区内的普通文件照常能搜到
        std::fs::write(f.ws.join("arena").join("ok.md"), "INSIDE-MARKER").unwrap();
        let s2 = dispatch(&c, "search", &json!({"query":"INSIDE-MARKER"})).await;
        assert!(
            s2.full.contains("INSIDE-MARKER"),
            "工作区内的文件必须照常搜得到:{:?}",
            s2.full
        );

        let _ = std::fs::remove_dir_all(&outside);
        kill(&f);
    }

    /// 指向**工作区内**的符号链接是合法的,不该被误杀。
    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_inside_workspace_still_works() {
        use std::os::unix::fs::symlink;

        let f = fx("symok");
        std::fs::write(f.ws.join("arena").join("real.md"), "INSIDE-REAL").unwrap();
        let link = f.ws.join("arena").join("alias.md");
        let _ = std::fs::remove_file(&link);
        symlink(f.ws.join("arena").join("real.md"), &link).unwrap();

        let c = ctx_of(&f, 1);
        let s = dispatch(&c, "search", &json!({"query":"INSIDE-REAL"})).await;
        assert!(
            s.full.contains("INSIDE-REAL"),
            "工作区内的符号链接应照常可用:{:?}",
            s.full
        );
        kill(&f);
    }
}
