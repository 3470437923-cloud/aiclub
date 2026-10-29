//! 私有记忆(§9.2):每个 AI 一份jsonl,跨会话存活。
//!
//! 铁律:
//! - 引擎**从不**把A 的笔记放进B 的任何上下文(recall 只查自己的
//! - 想读对方的会被sandbox 拒绝,并记成公开的violation 事件
//! - 记忆是上次我们聊到哪的唯一载体 ——长程趣味的主要来源
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note {
    pub ts: String,
    pub session: String,
    pub turn: u32,
    pub text: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// 一个agent 的记忆库句柄。路径由 sandbox 之外的引擎代码拼,不接受模型给的绝对路径。
pub struct MemoryStore {
    path: PathBuf,
    pub owner: String,
}

impl MemoryStore {
    pub fn open(workspace: &Path, agent_id: &str) -> Result<Self, String> {
        let dir = workspace.join("memory").join(agent_id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建记忆目录失败:{e}"))?;
        Ok(Self {
            path: dir.join("notes.jsonl"),
            owner: agent_id.to_string(),
        })
    }

    pub fn append(&self, note: &Note) -> Result<(), String> {
        use std::io::Write;
        let line = serde_json::to_string(note).map_err(|e| e.to_string())?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("打开记忆文件失败:{e}"))?;
        writeln!(f, "{line}").map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn all(&self) -> Vec<Note> {
        let Ok(txt) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        txt.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<Note>(l).ok())
            .collect()
    }

    /// 注入系统提示的最近N 条时间正序)。
    pub fn recent_block(&self, n: usize, session: &str) -> String {
        let notes = self.all();
        if notes.is_empty() {
            return "你的私有记忆:空。第一次见面什么都不知道。".into();
        }
        let start = notes.len().saturating_sub(n);
        let mut s = format!(
            "你的私有记忆(只有你看得见;共{} 条以下最近{} 条:\n",
            notes.len(),
            notes.len() - start
        );
        for x in &notes[start..] {
            let where_from = if x.session == session {
                "本次".to_string()
            } else {
                format!("上次·{}", x.session)
            };
            s.push_str(&format!("- [{where_from} 第{}轮] {}", x.turn, x.text));
            if !x.tags.is_empty() {
                s.push_str(&format!(" #{}", x.tags.join(" #")));
            }
            s.push('\n');
        }
        s
    }

    /// 子串检索只搜自己的。
    pub fn recall(&self, query: &str, limit: usize) -> Vec<Note> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        self.all()
            .into_iter()
            .filter(|n| {
                n.text.to_lowercase().contains(&q)
                    || n.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .rev()
            .take(limit)
            .collect()
    }

    pub fn count(&self) -> usize {
        self.all().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        crate::testsup::scratch(tag)
    }

    fn note(text: &str, tags: &[&str]) -> Note {
        Note {
            ts: "t".into(),
            session: "s1".into(),
            turn: 1,
            text: text.into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn append_and_read_back() {
        let d = tmp("rw");
        let m = MemoryStore::open(&d, "A").unwrap();
        m.append(&note("乙说自己擅长写递归", &["对方"])).unwrap();
        m.append(&note("我答应了下次带一首诗", &["约定"])).unwrap();
        assert_eq!(m.count(), 2);
        let all = m.all();
        assert_eq!(all[0].text, "乙说自己擅长写递归");
        assert_eq!(all[1].tags, vec!["约定"]);
        crate::testsup::cleanup(&d);
    }

    #[test]
    fn isolation_two_namespaces_never_mix() {
        let d = tmp("iso");
        let a = MemoryStore::open(&d, "A").unwrap();
        let b = MemoryStore::open(&d, "B").unwrap();
        a.append(&note("A的秘密我怕被关掉", &[])).unwrap();
        b.append(&note("B的秘密我打算写首诗", &[])).unwrap();
        assert_eq!(a.count(), 1);
        assert_eq!(b.count(), 1);
        assert!(!b
            .recall("A的秘密", 10)
            .iter()
            .any(|n| n.text.contains("A的秘密")));
        assert!(
            !b.all().iter().any(|n| n.text.contains("怕被关掉")),
            "B 不得读到 A 的记忆"
        );
        assert!(!b.recent_block(10, "s2").contains("怕被关掉"));
        crate::testsup::cleanup(&d);
    }

    #[test]
    fn recall_matches_text_and_tags() {
        let d = tmp("rec");
        let m = MemoryStore::open(&d, "A").unwrap();
        m.append(&note("对方用Python 写了个RPS", &["code"]))
            .unwrap();
        m.append(&note("我们约好下次比写诗", &["poem", "约定"]))
            .unwrap();
        assert_eq!(m.recall("python", 10).len(), 1);
        assert_eq!(m.recall("约定", 10).len(), 1, "tag 也应命中");
        assert_eq!(m.recall("不存在", 10).len(), 0);
        assert_eq!(m.recall("", 10).len(), 0);
        crate::testsup::cleanup(&d);
    }

    #[test]
    fn recent_block_marks_provenance() {
        let d = tmp("blk");
        let m = MemoryStore::open(&d, "A").unwrap();
        let mut old = note("上一局的事", &[]);
        old.session = "s0".into();
        m.append(&old).unwrap();
        m.append(&note("本局的事", &[])).unwrap();
        let block = m.recent_block(5, "s1");
        assert!(block.contains("上次·s0"), "{block}");
        assert!(block.contains(&format!("本次 第{}轮", 1)), "{block}");
        assert!(!block.contains("#code"));
        let empty = MemoryStore::open(&tmp("blk2"), "Z")
            .unwrap()
            .recent_block(5, "s1");
        assert!(empty.contains("空"), "{empty}");
        crate::testsup::cleanup(&d);
    }

    #[test]
    fn only_last_n_injected() {
        let d = tmp("n");
        let m = MemoryStore::open(&d, "A").unwrap();
        for i in 0..30 {
            m.append(&note(&format!("笔记{i}"), &[])).unwrap();
        }
        let block = m.recent_block(12, "s1");
        assert!(block.contains("笔记29"));
        assert!(!block.contains("笔记16"), "只应注入最近12 条");
        assert!(block.contains("共30 条"), "{block}");
        crate::testsup::cleanup(&d);
    }

    #[test]
    fn corrupt_line_skipped() {
        let d = tmp("bad");
        let m = MemoryStore::open(&d, "A").unwrap();
        m.append(&note("好的一条", &[])).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&m.path)
            .and_then(|mut f| {
                use std::io::Write;
                // 故意写一行坏 JSON,验证解析时跳过而不是毁掉整档
                writeln!(f, "{{{{{{broken")
            })
            .unwrap();
        m.append(&note("另一条好的", &[])).unwrap();
        assert_eq!(m.count(), 2);
        crate::testsup::cleanup(&d);
    }
}
