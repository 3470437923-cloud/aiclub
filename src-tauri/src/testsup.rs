//! 测试辅助 —— 只编译进 `cargo test`。
//!
//! 为什么需要它:测试用 `std::env::temp_dir()` 建工作区,但那个值在 Windows 上
//! 可能是 8.3 短名(如 `C:\Users\ADMINI~1\AppData\Local\Temp`),而
//! `canonicalize()` 返回的是长名。两者字符串前缀对不上,`sandbox::check_within`
//! 会把**每一个**用例都判成越界 —— 于是整套沙箱测试红成一片,而代码其实是对的。
//!
//! 规矩:**测试里任何要参与路径比较的根目录,都必须先 canonicalize**。
//! 这个函数就是那条规矩的唯一入口。

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 测试用的私有根目录。
///
/// 优先放在 `CARGO_TARGET_DIR/tmp`(即 `src-tauri/target/tmp`),理由有二:
/// 1. 它一定在仓库内 —— 受约束的执行环境只允许往工作区里写,
///    系统临时目录常常直接 `PermissionDenied`(错误码 5)。
/// 2. `cargo clean` 顺手就把它清了,不会在系统盘里攒垃圾。
///
/// 只有在连 target 都建不出来时,才退回系统临时目录。
fn root() -> PathBuf {
    let target = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("tmp");
    if std::fs::create_dir_all(&target).is_ok() {
        return target.canonicalize().unwrap_or(target);
    }
    let t = std::env::temp_dir();
    let _ = std::fs::create_dir_all(&t);
    t.canonicalize().unwrap_or(t)
}

/// 建一个**已 canonicalize** 的私有临时目录。同名目录先被清掉,保证用例互不污染。
///
/// canonicalize 是硬要求:Windows 的 8.3 短名(`ADMINI~1`)会让沙箱的前缀比较
/// 全线误判成越界,而那看起来像"沙箱坏了",实际上只是路径没归一。
pub fn scratch(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = root().join(format!("aiclub_t_{}_{}_{}", tag, std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    match std::fs::create_dir_all(&dir) {
        Ok(()) => {}
        Err(e) => panic!("创建测试临时目录失败:{} —— 目标 {}", e, dir.display()),
    }
    dir.canonicalize().unwrap_or(dir)
}

/// 在 scratch 目录下建好沙箱骨架(arena / skills / memory/A / memory/B),返回根目录。
pub fn world(tag: &str) -> PathBuf {
    let d = scratch(tag);
    for sub in ["arena", "skills", "memory/A", "memory/B"] {
        std::fs::create_dir_all(d.join(sub)).expect("建工作区骨架失败");
    }
    d
}

pub fn cleanup(p: &Path) {
    let _ = std::fs::remove_dir_all(p);
}

/// 一个够用的技能元数据构造(避免各处重复写 frontmatter)。
pub fn skill_meta(slug: &str, name: &str) -> crate::skills::SkillMeta {
    crate::skills::SkillMeta {
        slug: slug.into(),
        name: name.into(),
        description: format!("{name} 的说明"),
    }
}
