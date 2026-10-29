//! 共享技能库(§9.1):两个 AI 可读、不可写。
//!
//! 布局:`<workspace>/skills/<slug>/SKILL.md`,frontmatter 含name + description。
//! 系统提示注入**同一份目录*(name + description 各一行,`skill_load` 才拿全文。
//! 因为记忆不共享"谁先发现哪个技能、谁把它讲给对方"本身就是表演。
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillMeta {
    pub slug: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Skill {
    pub meta: SkillMeta,
    pub body: String,
}

/// 极简 frontmatter 解析:`---` 块里的`key: value`。
fn parse_frontmatter(txt: &str) -> (Vec<(String, String)>, String) {
    let t = txt.strip_prefix('\u{feff}').unwrap_or(txt);
    let mut kv = Vec::new();
    if !t.starts_with("---") {
        return (kv, t.to_string());
    }
    let rest = &t[3..];
    let Some(end) = rest.find("\n---") else {
        return (kv, t.to_string());
    };
    let head = &rest[..end];
    for line in head.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_string();
            let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
            if !k.is_empty() {
                kv.push((k, v));
            }
        }
    }
    let after = &rest[end + 4..];
    (kv, after.trim_start_matches(['\n', '\r']).to_string())
}

/// 扫描技能目录目录不存在返回空(不报错技能是可选增强。
pub fn list(root: &Path) -> Vec<SkillMeta> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(slug) = p.file_name().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        let f = p.join("SKILL.md");
        let Ok(txt) = std::fs::read_to_string(&f) else {
            continue;
        };
        let (kv, _) = parse_frontmatter(&txt);
        let get = |k: &str| {
            kv.iter()
                .find(|(a, _)| a == k)
                .map(|(_, b)| b.clone())
                .unwrap_or_default()
        };
        let name = if get("name").is_empty() {
            slug.clone()
        } else {
            get("name")
        };
        out.push(SkillMeta {
            slug,
            name,
            description: get("description"),
        });
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// 技能名必须是**单层 slug**:不含分隔符、不是绝对路径、不含 `..`、不含 NUL。
///
/// 为什么单独有这个函数:`load` 会把 key 直接 `root.join(want).join("SKILL.md")`。
/// Rust 的 `Path::join` 有一条很容易踩的语义 —— **被拼接的组件只要带根/前缀,
/// 就会把前面的路径整个丢掉**:
///   Path::new("/w/skills").join("/etc")  == "/etc"
///   Path::new("/w/skills").join("../../x") == "/w/skills/../../x" → 可逃逸
/// 于是模型只要传 `{"name":"../../.."}` 或 `{"name":"/Users/…"}`,
/// 就能读到工作区外任意目录下的 SKILL.md,内容还会进上下文、落 events.jsonl、
/// 并可能被转述给对面那家端点。技能库是本项目唯一的共享只读区,
/// 这条闸必须在这里(最内层入口)拦死。
pub fn is_valid_slug(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t != "."
        && t != ".."
        && !t.contains("..")
        && !t.contains('/')
        && !t.contains('\\')
        && !t.contains('\0')
        && !Path::new(t).is_absolute()
        // Windows 的 "C:" 这种只有前缀没有根的写法,is_absolute() 是 false,
        // 但 join 时同样会替换掉整个路径。
        && !(t.len() >= 2 && t.as_bytes()[1] == b':')
}

/// 按slug 或name 取全文。返回None = 不存在。
///
/// key 不是合法 slug 时**直接返回 None**(不报错、不区分"非法"与"不存在"),
/// 免得把"这个路径存在"当成信息泄露出去。
pub fn load(root: &Path, key: &str) -> Option<Skill> {
    let want = key.trim();
    if !is_valid_slug(want) {
        return None;
    }
    let direct = root.join(want).join("SKILL.md");
    if let Ok(txt) = std::fs::read_to_string(&direct) {
        let (kv, body) = parse_frontmatter(&txt);
        let get = |k: &str| {
            kv.iter()
                .find(|(a, _)| a == k)
                .map(|(_, b)| b.clone())
                .unwrap_or_default()
        };
        return Some(Skill {
            meta: SkillMeta {
                slug: want.to_string(),
                name: if get("name").is_empty() {
                    want.to_string()
                } else {
                    get("name")
                },
                description: get("description"),
            },
            body,
        });
    }
    // 用 name 再找一遍(只比对 list 出来的 slug,不再走用户输入)
    list(root)
        .into_iter()
        .find(|m| m.name == want)
        .and_then(|m| load(root, &m.slug))
}

/// List shared builtins plus the calling agent's private skills.
pub fn list_for_agent(root: &Path, agent_id: &str) -> Vec<SkillMeta> {
    let mut out = list(root);
    let private = root.join("private").join(agent_id);
    out.extend(list(&private).into_iter().map(|mut m| {
        m.slug = format!("private/{agent_id}/{}", m.slug);
        m
    }));
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// Load a shared slug or an explicitly own private slug.
pub fn load_for_agent(root: &Path, agent_id: &str, key: &str) -> Option<Skill> {
    let want = key.trim();
    let prefix = format!("private/{agent_id}/");
    if let Some(slug) = want.strip_prefix(&prefix) {
        if !is_valid_slug(slug) {
            return None;
        }
        let mut skill = load(&root.join("private").join(agent_id), slug)?;
        skill.meta.slug = format!("private/{agent_id}/{slug}");
        return Some(skill);
    }
    if want.starts_with("private/") {
        return None;
    }
    load(root, want)
}

/// 注入系统提示的目录块。
pub fn catalog_block(metas: &[SkillMeta]) -> String {
    if metas.is_empty() {
        return "共享技能库:当前为空。".into();
    }
    let mut s = String::from("共享技能库(只读,双方可用 skill_load 取全文:\n");
    for m in metas {
        s.push_str(&format!(
            "- {}(slug:{})——{}\n",
            m.name, m.slug, m.description
        ));
    }
    s
}

/// 出厂预置的示例技能内容用中文给AI 一个可以怎么玩的起点。
pub const BUILTIN: &[(&str, &str, &str)] = &[
    (
        "interview",
        "互相面试",
        "一方提问、一方作答然后交换角色。用来摸清对方的能力边界与偏好是最轻的破冰打法。",
    ),
    (
        "code-review",
        "互相验作业",
        "一方在 arena/ 写一段代码或文本并宣称它做到某件事另一方必须用 read/run_shell 实际验证,再给出结论。跑不跑得得过是客观事实。",
    ),
    (
        "small-world",
        "共同造物",
        "双方在arena/ 里长期共同维护一个物件一个故事、一张地图、一份设定集、一个小程序)。每轮都要写明这次动了哪里。",
    ),
];

/// 首次建工作区时写入预置技能已存在则不覆盖。
pub fn seed_builtin(root: &Path) {
    for (slug, name, desc) in BUILTIN {
        let dir = root.join(slug);
        let f = dir.join("SKILL.md");
        if f.exists() {
            continue;
        }
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            &f,
            format!("---\nname: {name}\ndescription: {desc}\n---\n\n# {name}\n\n{desc}\n\n用法自由:你们自己决定怎么用也可以发明新玩法。\n"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        crate::testsup::scratch(tag)
    }

    #[test]
    fn seed_then_list_and_load() {
        let d = tmp("seed");
        seed_builtin(&d);
        let metas = list(&d);
        assert_eq!(metas.len(), 3, "{metas:?}");
        assert!(metas.iter().any(|m| m.name == "互相验作业"));
        let s = load(&d, "code-review").unwrap();
        assert!(s.body.contains("互相验作业"));
        // 按name 也能取
        assert!(load(&d, "互相面试").is_some());
        assert!(load(&d, "不存在").is_none());
        // 二次 seed 不覆盖用户改动
        std::fs::write(d.join("interview").join("SKILL.md"), "改过了").unwrap();
        seed_builtin(&d);
        assert_eq!(
            std::fs::read_to_string(d.join("interview").join("SKILL.md")).unwrap(),
            "改过了",
        );
        crate::testsup::cleanup(&d);
    }

    #[test]
    fn missing_dir_is_empty_not_error() {
        let metas = list(Path::new("Z:/definitely/missing/aiclub/skills"));
        assert!(metas.is_empty());
        assert!(catalog_block(&metas).contains("为空"));
    }

    /// 回归:skill_load 曾经完全没有过沙箱。
    ///
    /// `load` 直接 `root.join(key).join("SKILL.md")`,而 Rust 的 join 在
    /// 组件带根/前缀时会**丢掉前面的路径**,于是模型传一个绝对路径或
    /// `../..` 就能读到工作区外的任意 SKILL.md。
    /// 这里先在工作区外真的放一个"机密"技能,再断言拿不到。
    #[test]
    fn load_rejects_escaping_keys() {
        let root = tmp("skill-escape");
        seed_builtin(&root);

        // 工作区外(与 skills 同级再往外一层)放一个不该被读到的技能
        let outside = root.parent().unwrap().join(format!(
            "{}-outside",
            root.file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(
            outside.join("SKILL.md"),
            "---\nname: 机密\n---\nSECRET-OUTSIDE-CONTENT",
        )
        .unwrap();

        // 这些 key 以前全都能读到外面那份
        for key in [
            "../outside",                       // 相对逃逸
            "./../../outside",                  // 多级相对
            outside.to_string_lossy().as_ref(), // 绝对路径(join 会丢弃 root)
            "..",
            ".",
            "",
            "   ",
            "a/b",  // 含分隔符
            "a\\b", // Windows 分隔符
        ] {
            let got = load(&root, key);
            if let Some(s) = got {
                assert!(
                    !s.body.contains("SECRET-OUTSIDE-CONTENT"),
                    "key {key:?} 逃出了技能库:{:?}",
                    s.body
                );
            }
        }

        // 正常 slug 与 name 仍然照常工作(别把闸修成"谁都进不来")
        assert!(load(&root, "code-review").is_some());
        assert!(load(&root, "互相面试").is_some());

        let _ = std::fs::remove_dir_all(&outside);
        crate::testsup::cleanup(&root);
    }

    #[test]
    fn is_valid_slug_boundaries() {
        assert!(is_valid_slug("code-review"));
        assert!(is_valid_slug("互相面试"));
        assert!(is_valid_slug("a_b.c-d1"));

        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("  "));
        assert!(!is_valid_slug("."));
        assert!(!is_valid_slug(".."));
        assert!(!is_valid_slug("../x"));
        assert!(!is_valid_slug("a/../b"));
        assert!(!is_valid_slug("a/b"));
        assert!(!is_valid_slug("a\\b"));
        assert!(!is_valid_slug("/abs"));
        assert!(!is_valid_slug("C:"));
        assert!(!is_valid_slug("C:\\x"));
    }

    #[test]
    fn frontmatter_variants() {
        let (kv, body) =
            parse_frontmatter("---\nname: \"带引号\"\ndescription: 描述\n---\n正文在此");
        assert_eq!(kv[0].1, "带引号");
        assert!(body.starts_with("正文在此"));
        let (kv2, body2) = parse_frontmatter("没有 frontmatter 的文件");
        assert!(kv2.is_empty());
        assert_eq!(body2, "没有 frontmatter 的文件");
        // BOM 开头
        let (kv3, _) = parse_frontmatter("\u{feff}---\nname: x\n---\nb");
        assert_eq!(kv3[0].1, "x");
        // 只有开头没有闭合
        let (kv4, b4) = parse_frontmatter("---\nname: x\n未闭合");
        assert!(kv4.is_empty());
        assert!(b4.contains("未闭合"));
    }

    #[test]
    fn catalog_block_is_one_line_per_skill() {
        let d = tmp("cat");
        seed_builtin(&d);
        let block = catalog_block(&list(&d));
        assert_eq!(block.lines().count(), 4, "{block}"); // 标题 + 3 条        crate::testsup::cleanup(&d);
    }

    #[test]
    fn skips_directories_without_skill_md() {
        let d = tmp("skip");
        std::fs::create_dir_all(d.join("broken")).unwrap();
        seed_builtin(&d);
        assert_eq!(list(&d).len(), 3);
        crate::testsup::cleanup(&d);
    }
}
