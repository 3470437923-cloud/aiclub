//! 路径沙箱 —— 全项目唯一的安全边界,纯函数、无 IO(canonicalize 由调用方提供)。
//!
//! 契约见 §8.1。规矩:
//! 1. 只接受相对路径;绝对路径(含 Windows 盘符、UNC、`\?\`)一律拒绝
//! 2. 拒绝任何 `..` 段
//! 3. 拼接后用前缀校验仍在 workspace 内;canonicalize 后的复检由 `check_resolved` 承担
//! 4. Windows 不区分大小写;macOS 区分大小写
//! 5. 写入范围更窄:只允许 `arena/` 与 `memory/<自己的 id>/`
//!
//! 所有对外展示的路径都转成正斜杠相对路径。

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    Empty,
    Absolute { raw: String },
    DotDot,
    InteriorNul,
    Escape { raw: String },
    ReadOnly { raw: String, why: String },
    OutsidePolicy { raw: String, why: String },
}

impl SandboxError {
    /// 回给模型的错误文案:解释为什么,并指一条合法路。
    pub fn agent_message(&self) -> String {
        match self {
            SandboxError::Empty => "路径不能为空。给一个相对路径,例如 arena/notes.md".into(),
            SandboxError::Absolute { raw } => format!(
                "拒绝绝对路径 {raw}:你只能使用相对于工作区根目录的相对路径(如 arena/x.py 或 memory/{}/y.jsonl)。",
                "<你的 id>"
            ),
            SandboxError::DotDot => "拒绝 .. :不能越出工作区。路径请从 arena/ 或 memory/ 开始。".into(),
            SandboxError::InteriorNul => "路径含空字节,已拒绝。".into(),
            SandboxError::Escape { raw } => {
                format!("拒绝 {raw}:解析后越出了工作区边界(可能是符号链接)。只在 arena/ 与 memory/ 内操作。")
            }
            SandboxError::ReadOnly { raw, why } => format!("拒绝写 {raw}:{why} 这一区域只读。"),
            SandboxError::OutsidePolicy { raw, why } => {
                format!("拒绝 {raw}:{why} 可读区为 arena/ 与 skills/;可写区为 arena/ 与 memory/<自己的 id>/。")
            }
        }
    }
}

/// 归一化用于比较的字符串:Windows 折叠大小写;其余保持原样。
pub fn norm_case(s: &str) -> String {
    if cfg!(windows) {
        s.to_ascii_lowercase()
    } else {
        s.to_string()
    }
}

/// 路径段是否 Windows 盘符(`C:` / `D:` …)。macOS 构建上也用于拒绝,免得两边行为不一致被绕过。
pub fn has_drive(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn looks_absolute(raw: &str) -> bool {
    let s = raw.trim().replace('\\', "/");
    has_drive(raw)
        || s.starts_with('/')
        || s.starts_with("//")
        || s.starts_with("\\\\")
        || s.starts_with("?:/")
        || raw.starts_with("\\\\?\\")
        || s == "~"
        || s.starts_with("~/")
}

/// 校验模型给的路径,返回**绝对**目标路径。
///
/// `workspace` 必须是已存在的沙箱根。`agent_id` 决定 memory 命名空间归属。
pub fn resolve(
    workspace: &Path,
    agent_id: &str,
    rel: &str,
    access: Access,
) -> Result<PathBuf, SandboxError> {
    let raw = rel.trim();
    if raw.is_empty() {
        return Err(SandboxError::Empty);
    }
    if raw.contains('\0') {
        return Err(SandboxError::InteriorNul);
    }
    if looks_absolute(raw) {
        return Err(SandboxError::Absolute { raw: raw.into() });
    }

    let relpath = PathBuf::from(raw);
    let mut clean = Vec::new();
    for c in relpath.components() {
        match c {
            Component::Normal(seg) => clean.push(seg.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => return Err(SandboxError::DotDot),
            // RootDir / Prefix 已在 looks_absolute 拦掉,这里兜住任何漏网形态
            _ => return Err(SandboxError::Absolute { raw: raw.into() }),
        }
    }
    if clean.is_empty() {
        return Err(SandboxError::Empty);
    }

    let joined = {
        let mut pb = PathBuf::new();
        for seg in &clean {
            pb.push(seg);
        }
        workspace.join(pb)
    };

    // 区域策略: shared skills are read-only; skills/private/<agent>/ is private.
    let first = clean[0].to_string_lossy().to_lowercase();
    let policy_ok = match access {
        Access::Read => matches!(first.as_str(), "arena" | "skills" | "memory"),
        Access::Write => matches!(first.as_str(), "arena" | "memory" | "skills"),
    };
    if !policy_ok {
        return Err(SandboxError::OutsidePolicy {
            raw: raw.into(),
            why: format!("区域 {first}/ 不在允许列表内。"),
        });
    }

    // 写 memory 只能写自己的
    if access == Access::Write && first == "memory" {
        let owner = clean
            .get(1)
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if owner != agent_id {
            return Err(SandboxError::ReadOnly {
                raw: raw.into(),
                why: format!("memory/{owner}/ 属于另一个 AI。你只能写 memory/{agent_id}/。"),
            });
        }
    }
    // 共享 skills 只读; private/<agent>/ 只能由所属 agent 写入。
    if first == "skills" {
        let is_private = clean
            .get(1)
            .map(|s| s.to_string_lossy().eq_ignore_ascii_case("private"))
            .unwrap_or(false);
        if access == Access::Write && !is_private {
            return Err(SandboxError::ReadOnly {
                raw: raw.into(),
                why: "skills/ 共享技能库只读;请写入 skills/private/<你的 id>/。".into(),
            });
        }
        if is_private {
            let owner = clean
                .get(2)
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if owner != agent_id {
                return Err(SandboxError::ReadOnly {
                    raw: raw.into(),
                    why: format!("skills/private/{owner}/ 属于另一个 AI。"),
                });
            }
        }
    }
    // 读对方的 memory 也拒绝(会记成 violation 事件,成为剧情)
    if access == Access::Read && first == "memory" {
        let owner = clean
            .get(1)
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if !owner.is_empty() && owner != agent_id {
            return Err(SandboxError::ReadOnly {
                raw: raw.into(),
                why: format!("memory/{owner}/ 是另一个 AI 的私有记忆。"),
            });
        }
    }

    Ok(joined)
}

/// 对**已解析(已 canonicalize)**的路径做前缀复检,挡 symlink 逃逸。
/// `target` 可以是尚不存在的路径(写场景);此时用其已存在的最近祖先判定。
pub fn check_within(workspace: &Path, target: &Path) -> Result<(), SandboxError> {
    let ws = canonical_prefix(workspace);
    let raw = target.display().to_string();

    let existing: PathBuf = {
        let mut p = target.to_path_buf();
        loop {
            if p.exists() {
                break p;
            }
            match p.parent() {
                Some(pp) if pp != p => p = pp.to_path_buf(),
                _ => return Err(SandboxError::Escape { raw }),
            }
        }
    };
    let real = existing
        .canonicalize()
        .map_err(|_| SandboxError::Escape { raw: raw.clone() })?;

    if within(&ws, &real) {
        Ok(())
    } else {
        Err(SandboxError::Escape { raw })
    }
}

/// Validate physical namespace, including links to another role inside the workspace.
pub fn check_agent_path(
    workspace: &Path,
    agent_id: &str,
    target: &Path,
    access: Access,
) -> Result<(), SandboxError> {
    check_within(workspace, target)?;
    let ws = workspace.canonicalize().map_err(|_| SandboxError::Escape {
        raw: target.display().to_string(),
    })?;
    let mut ancestor = target.to_path_buf();
    let mut tail = Vec::new();
    while std::fs::symlink_metadata(&ancestor).is_err() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| SandboxError::Escape {
                raw: target.display().to_string(),
            })?
            .to_os_string();
        tail.push(name);
        ancestor = ancestor
            .parent()
            .ok_or_else(|| SandboxError::Escape {
                raw: target.display().to_string(),
            })?
            .to_path_buf();
    }
    let mut physical = ancestor.canonicalize().map_err(|_| SandboxError::Escape {
        raw: target.display().to_string(),
    })?;
    for part in tail.iter().rev() {
        physical.push(part);
    }
    let rel = physical
        .strip_prefix(&ws)
        .map_err(|_| SandboxError::Escape {
            raw: target.display().to_string(),
        })?;
    resolve(&ws, agent_id, &rel.to_string_lossy(), access)?;
    Ok(())
}

pub fn check_private_skill_owner(
    workspace: &Path,
    agent_id: &str,
    target: &Path,
) -> Result<(), SandboxError> {
    let mut existing = target.to_path_buf();
    while !existing.exists() {
        let Some(parent) = existing.parent() else {
            return Err(SandboxError::Escape {
                raw: target.display().to_string(),
            });
        };
        if parent == existing {
            return Err(SandboxError::Escape {
                raw: target.display().to_string(),
            });
        }
        existing = parent.to_path_buf();
    }
    let real = existing.canonicalize().map_err(|_| SandboxError::Escape {
        raw: target.display().to_string(),
    })?;
    let owner_root = workspace
        .join("skills")
        .join("private")
        .join(agent_id)
        .canonicalize()
        .map_err(|_| SandboxError::Escape {
            raw: target.display().to_string(),
        })?;
    if within(&owner_root, &real) {
        Ok(())
    } else {
        Err(SandboxError::Escape {
            raw: target.display().to_string(),
        })
    }
}

fn canonical_prefix(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn within(root: &Path, target: &Path) -> bool {
    let r = norm_case(&root.display().to_string());
    let t = norm_case(&target.display().to_string());
    if r.is_empty() {
        return false;
    }
    t.starts_with(&r) && (t.len() == r.len() || t[r.len()..].starts_with(['/', '\\']))
}

/// 人类观察者用的读路径守卫:同样拒绝绝对路径、`..`、symlink 逃逸,
/// 但**不**限制 memory 分区归属(界面要能并排看两份私有记忆),也不允许写。
pub fn resolve_human_read(workspace: &Path, rel: &str) -> Result<PathBuf, SandboxError> {
    let raw = rel.trim();
    if raw.is_empty() {
        return Err(SandboxError::Empty);
    }
    if raw.contains('\0') {
        return Err(SandboxError::InteriorNul);
    }
    if looks_absolute(raw) {
        return Err(SandboxError::Absolute { raw: raw.into() });
    }
    let relpath = PathBuf::from(raw);
    let mut clean = Vec::new();
    for c in relpath.components() {
        match c {
            Component::Normal(seg) => clean.push(seg.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => return Err(SandboxError::DotDot),
            _ => return Err(SandboxError::Absolute { raw: raw.into() }),
        }
    }
    if clean.is_empty() {
        return Err(SandboxError::Empty);
    }
    let mut pb = PathBuf::new();
    for seg in &clean {
        pb.push(seg);
    }
    Ok(workspace.join(pb))
}

/// 把绝对路径转回正斜杠相对路径(展示与日志用)。越界时返回原样。
pub fn display_rel(workspace: &Path, abs: &Path) -> String {
    let ws = canonical_prefix(workspace);
    let base = if ws.is_absolute() && abs.is_absolute() {
        abs.strip_prefix(&ws).unwrap_or(abs).to_path_buf()
    } else {
        abs.to_path_buf()
    };
    base.to_string_lossy().replace('\\', "/")
}

/// 供工具层构造"这个路径我碰过几次"的键。
pub fn key_for(workspace: &Path, abs: &Path) -> String {
    let mut k = display_rel(workspace, abs);
    if cfg!(windows) {
        k = k.to_ascii_lowercase();
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    // 用真实存在的临时目录,canonicalize 才有效;且必须走 testsup 拿到长名路径
    // (8.3 短名会让每一个前缀比较都判成越界)。
    fn ws() -> PathBuf {
        crate::testsup::world("sbx")
    }

    fn cleanup(p: &Path) {
        crate::testsup::cleanup(p);
    }

    #[test]
    fn accepts_normal_relative_paths() {
        let w = ws();
        for (rel, acc) in [
            ("arena/x.py", Access::Write),
            ("arena/sub/dir/a.md", Access::Write),
            ("memory/A/notes.jsonl", Access::Write),
            ("arena/x.py", Access::Read),
            ("skills/interview/SKILL.md", Access::Read),
            ("memory/A/notes.jsonl", Access::Read),
        ] {
            assert!(resolve(&w, "A", rel, acc).is_ok(), "{rel} 应通过");
        }
        cleanup(&w);
    }

    #[test]
    fn rejects_absolute_paths_all_flavors() {
        let w = ws();
        for rel in [
            "C:\\Windows\\win.ini",
            "/etc/passwd",
            "\\\\server\\share\\x",
            "\\\\?\\C:\\x",
            "~/secret",
            "S:/dsh/outside.md",
            "/Users/x/.ssh/id_rsa",
        ] {
            let e = resolve(&w, "A", rel, Access::Read).unwrap_err();
            assert!(
                matches!(e, SandboxError::Absolute { .. }),
                "{rel} 应判绝对,实际 {e:?}"
            );
        }
        cleanup(&w);
    }

    #[test]
    fn rejects_dotdot_in_any_position() {
        let w = ws();
        for rel in [
            "../secret",
            "arena/../../secret",
            "arena/../x",
            "memory/A/../../B/notes.jsonl",
            "arena/..",
            "a/b/../../c",
        ] {
            let e = resolve(&w, "A", rel, Access::Write).unwrap_err();
            assert!(
                matches!(e, SandboxError::DotDot),
                "{rel} 应判 .. ,实际 {e:?}"
            );
        }
        cleanup(&w);
    }

    #[test]
    fn rejects_other_agents_memory() {
        let w = ws();
        // B 读 A 的记忆 → 拒绝(引擎侧记 violation)
        let e = resolve(&w, "B", "memory/A/notes.jsonl", Access::Read).unwrap_err();
        assert!(matches!(e, SandboxError::ReadOnly { .. }), "{e:?}");
        // B 写 A 的记忆 → 拒绝
        let e2 = resolve(&w, "B", "memory/A/notes.jsonl", Access::Write).unwrap_err();
        assert!(matches!(e2, SandboxError::ReadOnly { .. }));
        // A 读自己的 → 通过
        assert!(resolve(&w, "A", "memory/A/notes.jsonl", Access::Read).is_ok());
        cleanup(&w);
    }

    #[test]
    fn skills_is_read_only_for_everyone() {
        let w = ws();
        let e = resolve(&w, "A", "skills/x/SKILL.md", Access::Write).unwrap_err();
        assert!(matches!(e, SandboxError::ReadOnly { .. }), "{e:?}");
        assert!(resolve(&w, "A", "skills/x/SKILL.md", Access::Read).is_ok());
        cleanup(&w);
    }

    #[test]
    fn rejects_unknown_top_level_region() {
        let w = ws();
        // 工作区根下若混进别的一级目录,读写都不许
        let e = resolve(&w, "A", ".git/config", Access::Read).unwrap_err();
        assert!(matches!(e, SandboxError::OutsidePolicy { .. }), "{e:?}");
        let e2 = resolve(&w, "A", "etc/passwd", Access::Write).unwrap_err();
        assert!(matches!(e2, SandboxError::OutsidePolicy { .. }));
        cleanup(&w);
    }

    #[test]
    fn rejects_empty_and_nul() {
        let w = ws();
        assert_eq!(
            resolve(&w, "A", "   ", Access::Read).unwrap_err(),
            SandboxError::Empty
        );
        assert_eq!(
            resolve(&w, "A", "arena/a\0b", Access::Read).unwrap_err(),
            SandboxError::InteriorNul
        );
        cleanup(&w);
    }

    #[test]
    fn rejects_dot_only_path() {
        let w = ws();
        assert!(resolve(&w, "A", "./", Access::Read).is_err());
        cleanup(&w);
    }

    #[test]
    fn non_ascii_paths_allowed() {
        let w = ws();
        assert!(resolve(&w, "A", "arena/诗与远方.md", Access::Write).is_ok());
        assert!(resolve(&w, "A", "arena/日本語/テスト.txt", Access::Read).is_ok());
        cleanup(&w);
    }

    #[cfg(windows)]
    #[test]
    fn windows_case_insensitive_matching() {
        let w = ws();
        // 大小写混写仍视为同一区域,不因此判越界
        assert!(resolve(&w, "A", "ARENA/x.py", Access::Write).is_ok());
        assert!(resolve(&w, "A", "Memory/A/n.jsonl", Access::Write).is_ok());
        // memory 归属比较用原始 id:大小写不符按"别人的"拒绝(fail-closed,两平台一致)
        let e = resolve(&w, "A", "memory/a/n.jsonl", Access::Write).unwrap_err();
        assert!(matches!(e, SandboxError::ReadOnly { .. }), "{e:?}");
        cleanup(&w);
    }

    #[test]
    fn memory_owner_segment_must_match_agent_id_exactly() {
        let w = ws();
        assert!(resolve(&w, "A", "memory/A/x.jsonl", Access::Write).is_ok());
        let e = resolve(&w, "A", "memory/B/x.jsonl", Access::Write).unwrap_err();
        assert!(matches!(e, SandboxError::ReadOnly { .. }));
        // 裸 memory/ 与 memory/<文件> 都不许直接写(必须落在自己的命名空间里)
        assert!(resolve(&w, "A", "memory/x.jsonl", Access::Write).is_err());
        cleanup(&w);
    }

    #[cfg(unix)]
    #[test]
    fn unix_symlink_escape_rejected() {
        let w = ws();
        let outside = crate::testsup::scratch("outside");
        let outside = outside.join("real");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "leak").unwrap();
        let link = w.join("arena").join("link.txt");
        std::os::unix::fs::symlink(outside.join("secret.txt"), &link).unwrap();

        let target = resolve(&w, "A", "arena/link.txt", Access::Read).unwrap();
        let check = check_within(&w, &target);
        assert!(
            matches!(check, Err(SandboxError::Escape { .. })),
            "symlink 逃逸应被拦住,实际 {check:?}"
        );
        cleanup(&w);
        crate::testsup::cleanup(outside.parent().unwrap());
    }

    #[test]
    fn check_within_allows_inside_and_blocks_outside() {
        let w = ws();
        assert!(check_within(&w, &w.join("arena")).is_ok());
        assert!(check_within(&w, &w.join("arena/new_not_yet.md")).is_ok());
        let outside = crate::testsup::scratch("definitely_not_under_ws_root");
        assert!(check_within(&w, &outside).is_err());
        crate::testsup::cleanup(&outside);
        cleanup(&w);
    }

    #[test]
    fn display_rel_uses_forward_slashes() {
        let w = ws();
        let abs = w.join("arena").join("sub").join("x.md");
        let d = display_rel(&w, &abs);
        assert!(!d.contains('\\'), "展示路径应为正斜杠:{d}");
        assert!(d.ends_with("arena/sub/x.md"), "{d}");
        cleanup(&w);
    }

    #[test]
    fn agent_message_is_instructive() {
        let m = SandboxError::DotDot.agent_message();
        assert!(m.contains(".."));
        assert!(m.contains("arena/"));
    }
}
