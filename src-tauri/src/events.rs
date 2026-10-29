//! 事件模型 —— 唯一事实源(§3.3)。
//!
//! 直播、回放、崩溃续跑、上下文重建全部读这一份 events.jsonl,不各写一套。
//! 追加式写入,坏行跳过并记录,绝不整档放弃。

use serde::{Deserialize, Serialize};

pub type Usage = Option<u64>;

/// 一次调用的用量。
///
/// 标为 `Copy`:三个字段都是标量,而同一份用量在一次工具调用里会被读好几遍
/// (累计账本、落 SayEnd 事件、汇总到 status)。派生 Copy 比到处 `.clone()` 干净。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct UsagePair {
    pub input: u64,
    pub output: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<u64>,
}

impl UsagePair {
    pub fn total(&self) -> u64 {
        self.input + self.output
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    SessionOpen {
        session_id: String,
        agents: Vec<AgentBrief>,
        mode: String,
        opening_note: String,
        ts: String,
    },
    TurnBegin {
        turn: u32,
        agent: String,
        ts: String,
    },
    SayDelta {
        turn: u32,
        agent: String,
        /// 轮内发言序号(第几条发言,从 0 起)。
        ///
        /// 一轮之内可以有多条发言:模型先说一句、调个工具、再说一句,
        /// 或者同一轮里连调两次 say。这些事件的 (turn, agent) 完全相同,
        /// 前端光靠这两个字段无法区分,会把它们归并成一条 —— 前一段发言就消失了。
        /// `#[serde(default)]` 让旧日志(没有这个字段)仍然读得进来,默认都算第 0 条。
        #[serde(default)]
        seq: u32,
        text: String,
    },
    SayEnd {
        turn: u32,
        agent: String,
        /// 见 SayDelta.seq —— 同一个序号表示这是同一条发言的收尾。
        #[serde(default)]
        seq: u32,
        text: String,
        #[serde(flatten)]
        usage: UsagePair,
        ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish_reason: Option<String>,
    },
    ToolCall {
        turn: u32,
        agent: String,
        call_id: String,
        name: String,
        args: serde_json::Value,
        ok: bool,
        /// 一行摘要(喂给对面 AI 的就是这一行)
        preview: String,
        /// 完整结果(只喂给动作的主人自己)
        result_full: String,
        ms: u64,
    },
    Pass {
        turn: u32,
        agent: String,
    },
    /// 沙箱拒绝 / 越界读取 —— 公开可见,这本身是剧情
    Violation {
        turn: u32,
        agent: String,
        kind: String,
        path: String,
        detail: String,
    },
    /// 系统广播(如「文件被对方改过」)
    SystemNote {
        turn: u32,
        text: String,
    },
    TurnError {
        turn: u32,
        agent: String,
        attempt: u32,
        error: String,
        retryable: bool,
    },
    AgentUnreachable {
        agent: String,
        consecutive_failures: u32,
    },
    /// 上下文快照索引(内容落 context_<agent>.md)
    ContextSnapshot {
        turn: u32,
        agent: String,
        bytes: usize,
        compressed: bool,
    },
    Compress {
        turn: u32,
        agent: String,
        via_model: String,
        tokens_in: u64,
        tokens_out: u64,
    },
    SessionEnd {
        reason: String,
        turns: u32,
        by_agent: serde_json::Value,
        ms_total: u64,
        ts: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentBrief {
    pub id: String,
    pub name: String,
    pub company: String,
    pub model: String,
    pub host: String,
}

/// 落盘 sink(引擎只认这个 trait,便于测试里换成内存版)。
pub trait EventSink: Send + Sync {
    fn emit(&self, ev: &Event);
}

/// 追加式 JSONL 日志 = **唯一事实源**。
///
/// 直播、回放、崩溃续跑、上下文重建都读这一份。引擎每写一条事件走 `append`,
/// 同时把同一条转发给 UI sink(commands.rs 里的广播器),所以"看到什么"与"存了什么"同源。
///
/// `Clone` 克隆的是**同一个文件句柄**(Arc 包裹):上下文压缩、工具层都可能需要落事件,
/// 但不能各自开一个新 File 去追加同一份日志。
#[derive(Clone)]
pub struct JsonlLog {
    path: std::path::PathBuf,
    file: std::sync::Arc<std::sync::Mutex<std::fs::File>>,
}

impl JsonlLog {
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("打开事件日志失败:{e}"))?;
        Ok(Self {
            path: path.to_path_buf(),
            file: std::sync::Arc::new(std::sync::Mutex::new(file)),
        })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 落一条事件(引擎主路径)。写失败静默跳过 —— 观察台不能因为 IO 抖动弄死两个 AI 的对话。
    pub fn append(&self, ev: &Event) {
        let line = match serde_json::to_string(ev) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Ok(mut f) = self.file.lock() {
            use std::io::Write;
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }

    pub fn read_all(path: &std::path::Path) -> Vec<Event> {
        let txt = std::fs::read_to_string(path).unwrap_or_default();
        decode_lines(&txt)
    }

    /// 读原始 JSONL 行(不解析,保留坏行)。直播/回放交给前端按行解析,
    /// 坏行由前端自己的 `bad` 机制捕获 —— 与 UiSink 推送的 payload 同构。
    pub fn read_lines(path: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .map(|t| {
                t.lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 当前事件条数
    pub fn count(&self) -> usize {
        std::fs::read_to_string(&self.path)
            .map(|t| decode_lines(&t).len())
            .unwrap_or(0)
    }
}

fn decode_lines(txt: &str) -> Vec<Event> {
    let mut out = Vec::new();
    for line in txt.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        match serde_json::from_str::<Event>(l) {
            Ok(ev) => out.push(ev),
            // 坏行跳过:半条日志好过没有日志(§12)
            Err(_) => continue,
        }
    }
    out
}

impl EventSink for JsonlLog {
    fn emit(&self, ev: &Event) {
        self.append(ev);
    }
}

/// 内存 sink(引擎测试用)
#[derive(Default)]
pub struct MemSink(pub std::sync::Mutex<Vec<Event>>);

impl EventSink for MemSink {
    fn emit(&self, ev: &Event) {
        self.0.lock().unwrap().push(ev.clone());
    }
}

impl MemSink {
    pub fn events(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }
    pub fn has(&self, pred: impl Fn(&Event) -> bool) -> bool {
        self.0.lock().unwrap().iter().any(pred)
    }
}

pub fn now_ts() -> String {
    use time::format_description::well_known::Rfc3339;
    time::OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "0".into())
}

/// 会话目录名:`2026-10-12T17-50-03_甲_vs_乙`
pub fn session_id(a_name: &str, b_name: &str) -> String {
    // 宏版本在编译期定型,不依赖运行期 parse(旧 API 已废弃且会多一次分配)
    const FMT: &[time::format_description::BorrowedFormatItem<'_>] =
        time::macros::format_description!("[year]-[month]-[day]T[hour]-[minute]-[second]");
    let stamp = time::OffsetDateTime::now_utc()
        .format(FMT)
        .unwrap_or_else(|_| "session".into());
    // 先 trim 再替换:否则 "甲 / 一号" 里的 " / " 会变成三个下划线,
    // 目录名读起来像坏了(而且长度白白膨胀)。
    let slug = |s: &str| -> String {
        let t = s.trim().trim_matches('_');
        let flat: String = t
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        // 连续下划线压成一个:名字里带标点时目录名仍然可读
        let mut out = String::with_capacity(flat.len());
        let mut prev_us = false;
        for c in flat.chars() {
            if c == '_' {
                if prev_us {
                    continue;
                }
                prev_us = true;
            } else {
                prev_us = false;
            }
            out.push(c);
        }
        out.trim_matches('_').to_string()
    };
    format!("{stamp}_{}_vs_{}", slug(a_name), slug(b_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_roundtrip_skips_corrupt_lines() {
        let dir = crate::testsup::scratch("ev");
        let p = dir.join("events.jsonl");
        {
            let log = JsonlLog::open(&p).unwrap();
            log.emit(&Event::TurnBegin {
                turn: 1,
                agent: "A".into(),
                ts: "t".into(),
            });
            log.emit(&Event::SayEnd {
                turn: 1,
                agent: "A".into(),
                seq: 0,
                text: "你好".into(),
                usage: UsagePair {
                    input: 10,
                    output: 5,
                    cached: None,
                },
                ms: 120,
                finish_reason: Some("stop".into()),
            });
        }
        // 人为插入半行损坏数据
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        use std::io::Write;
        writeln!(f, "{{\"type\":\"turn_begin\",\"turn\":9").unwrap();
        drop(f);
        {
            let log = JsonlLog::open(&p).unwrap();
            log.emit(&Event::Pass {
                turn: 2,
                agent: "B".into(),
            });
        }
        let evs = JsonlLog::read_all(&p);
        assert_eq!(evs.len(), 3, "坏行应被跳过而非毁掉整档");
        assert!(matches!(evs[2], Event::Pass { turn: 2, .. }));
        crate::testsup::cleanup(&dir);
    }

    #[test]
    fn say_end_usage_flattens_to_top_level() {
        let e = Event::SayEnd {
            turn: 1,
            agent: "A".into(),
            seq: 0,
            text: "x".into(),
            usage: UsagePair {
                input: 3,
                output: 4,
                cached: None,
            },
            ms: 1,
            finish_reason: None,
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"input\":3"), "{s}");
        assert!(s.contains("\"type\":\"say_end\""), "{s}");
    }

    #[test]
    fn session_id_is_slugified_and_has_both_names() {
        let s = session_id("甲 / 一号", "B!2");
        // 标点被折掉,连续下划线压成一个 —— 目录名要能读
        assert!(s.ends_with("_甲_一号_vs_B_2"), "{s}");
        assert!(!s.contains("___"), "不该留下连续下划线:{s}");
        assert!(!s.contains('/') && !s.contains('\\'), "{s}");
        // 首尾留白不得变成下划线
        assert!(session_id(" 青 ", "白").ends_with("_青_vs_白"));
    }

    #[test]
    fn session_id_never_breaks_out_of_sessions_dir() {
        // 名字是用户输入,绝不能让会话目录跳到 sessions/ 之外
        for evil in ["../../x", "a/b", "a\\b", "..", "  ", "C:\\x"] {
            let s = session_id(evil, evil);
            assert!(!s.contains('/'), "{s}");
            assert!(!s.contains('\\'), "{s}");
            assert!(!s.contains(".."), "{s}");
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn reopen_appends_not_overwrites() {
        let dir = crate::testsup::scratch("ev2");
        let p = dir.join("events.jsonl");
        {
            let l = JsonlLog::open(&p).unwrap();
            l.emit(&Event::Pass {
                turn: 1,
                agent: "A".into(),
            });
        }
        {
            let l = JsonlLog::open(&p).unwrap();
            l.emit(&Event::Pass {
                turn: 2,
                agent: "B".into(),
            });
            assert_eq!(l.count(), 2);
        }
        crate::testsup::cleanup(&dir);
    }
}
