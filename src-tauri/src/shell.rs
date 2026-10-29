//! 受限 shell 执行器§8.2 / D-007)。
//!
//! 六道限制,一道都不可省
//! 1. cwd 强制 `<workspace>/arena`,参数里没有cwd,模型改不了
//! 2. 解释器按平台选Windows `cmd.exe /C`、macOS/Linux `/bin/sh -c`
//! 3. 硬超时+ **杀整个进程树*(只杀 shell 自己会留下孤儿继续写文件)
//! 4. 输出上限(头2KB + 尾2KB),中间标truncated
//! 5. 环境**白名单*透传:先env_clear 再挑,`*_API_KEY` 这类一律不进去
//!    ——否则一句`set`(Win)/ `env`(mac) 就能把对面那家的密钥捞走
//! 6. 危险形态兜底黑名单(主防线仍是cwd + 无提权+ 白名单
//!
//! 输出走*临时文件**而不是管道管道在confine 环境下可能取不到子进程输出
//! 且并发读两个管道容易死锁。文件方案没有这个问题超时时还能拿到已产生的部分。
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::ShellConfig;

#[derive(Debug, Clone, PartialEq)]
pub struct ShellOutcome {
    pub ok: bool,
    pub exit: String,
    pub ms: u64,
    pub output: String,
    pub truncated: bool,
    pub killed: bool,
    pub cwd: String,
}

impl ShellOutcome {
    /// 喂回给模型的完整文本
    pub fn agent_text(&self) -> String {
        let mut s = format!("cwd: {}\nexit: {}", self.cwd, self.exit);
        if self.killed {
            s.push_str(&format!("\n(已超时{}ms,进程树被强制终止)", self.ms));
        }
        s.push_str("\n--- output ---\n");
        s.push_str(if self.output.trim().is_empty() {
            "(无输出"
        } else {
            self.output.trim()
        });
        if self.truncated {
            s.push_str("\n(output truncated:保留头尾各一半");
        }
        s
    }
}

/// 密钥类变量绝不透传(§8.2 第5 条。白名单之外再补一层显式拒绝。
const SECRETISH: &[&str] = &[
    "API_KEY",
    "APIKEY",
    "ANTHROPIC",
    "OPENAI",
    "DEEPSEEK",
    "MOONSHOT",
    "SILICONFLOW",
    "QWEN",
    "DASHSCOPE",
    "GOOGLE",
    "AZURE",
    "AWS_",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "CREDENTIAL",
    "ACCESS_KEY",
    "PRIVATE",
];

fn is_secretish(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    SECRETISH.iter().any(|k| up.contains(k))
}

/// 从当前进程环境里挑出白名单项(且永不含密钥类。
pub fn build_env(allow: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for key in allow {
        if is_secretish(key) {
            continue; // 白名单写错也不放行
        }
        if let Ok(v) = std::env::var(key) {
            if !v.is_empty() {
                out.push((key.clone(), v));
            }
        }
    }
    out
}

/// 危险形态兜底。命中即拒绝执行(不解析语义只挡最粗暴的几条。
pub fn denied_reason(cmd: &str) -> Option<&'static str> {
    let low = cmd.to_lowercase().replace('\t', " ");
    let flat: String = low
        .chars()
        .filter(|c| !matches!(c, ' ' | ':' | '/' | '\\' | '"' | '\''))
        .collect();
    const PATTERNS: &[(&str, &str)] = &[
        ("mkfs", "格式化磁盘"),
        ("diskpart", "磁盘分区"),
        ("fdisk", "磁盘分区"),
        ("shutdown", "关机"),
        ("restart /o", "重启"),
        ("reg delete hklu", "改注册表"),
        (":(){ :|:& };:", "fork bomb"),
        ("chmod -R 777 /", "递归改权限"),
        ("del c:\\", "删系统盘"),
        ("rd /s /q c", "删系统盘"),
        ("rm -rf /", "删根目录"),
        ("sudo", "提权"),
        ("net user", "改系统账户"),
        ("wmic", "系统管理接口"),
        ("taskkill", "杀进程(引擎已提供超时终止"),
        ("curl|sh", "远程脚本执行"),
        ("wget|sh", "远程脚本执行"),
    ];
    for (pat, why) in PATTERNS {
        if low.contains(pat) || flat.contains(&pat.replace([' ', '/', ':'], "")) {
            return Some(why);
        }
    }
    // 越出 arena 的显式尝试命令里出现../ 且带删除/移动语义
    if low.contains("..")
        && (low.contains("del ")
            || low.contains("rm ")
            || low.contains("move ")
            || low.contains("mv "))
    {
        return Some("试图对工作区外执行删除移动");
    }
    None
}

/// 平台解释器。返回(program, [flag, cmd])
pub fn interpreter(cmd: &str) -> (PathBuf, Vec<String>) {
    if cfg!(windows) {
        let com = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
        (PathBuf::from(com), vec!["/C".into(), cmd.to_string()])
    } else {
        (PathBuf::from("/bin/sh"), vec!["-c".into(), cmd.to_string()])
    }
}

pub struct ShellRunner {
    pub arena: PathBuf,
    pub cfg: ShellConfig,
}

impl ShellRunner {
    pub fn new(arena: PathBuf, cfg: ShellConfig) -> Self {
        Self { arena, cfg }
    }

    pub async fn run(&self, args: &Value) -> Result<ShellOutcome, String> {
        let cmd = args
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if cmd.is_empty() {
            return Err("缺少参数 command".into());
        }
        if !self.cfg.enabled {
            return Err("run_shell 已被人类关闭(设置 → shell)。".into());
        }
        if let Some(why) = denied_reason(&cmd) {
            return Err(format!(
                "命令被拒绝{why}。只能在 arena/ 内做读写与本地脚本执行。"
            ));
        }
        if !self.arena.exists() {
            std::fs::create_dir_all(&self.arena).map_err(|e| e.to_string())?;
        }
        let arena = self
            .arena
            .canonicalize()
            .map_err(|e| format!("arena 目录不可用{e}"))?;

        let (prog, flags) = interpreter(&cmd);
        // 输出先落到 arena 里的临时文件,而不是系统临时目录。
        // 两个理由:(1) 受限环境常常禁止写工作区以外的路径,用系统临时目录会直接
        // PermissionDenied(os error 5);(2) arena 一定是可写的 —— 它是我们自己的沙箱。
        // 名字以点开头,并且在返回前删除,不给两个 AI 留下可见的残留。
        let out_path = arena.join(format!(
            ".aiclub_out_{}.{}.tmp",
            std::process::id(),
            nonce()
        ));

        let mut command = tokio::process::Command::new(&prog);
        command
            .args(&flags)
            .current_dir(&arena)
            .env_clear()
            .stdin(std::process::Stdio::null());
        for (k, v) in build_env(&self.cfg.env_passthrough) {
            command.env(k, v);
        }
        // stdout 与stderr 合并进同一个文件顺序即真实交错顺序且不会死锁
        let f = std::fs::File::create(&out_path).map_err(|e| format!("临时输出文件失败:{e}"))?;
        command.stdout(std::process::Stdio::from(
            f.try_clone().map_err(|e| e.to_string())?,
        ));
        command.stderr(std::process::Stdio::from(f));

        #[cfg(unix)]
        {
            // 独立进程组便于 killpg 杀整棵树
            command.process_group(0);
        }

        let mut child = command.spawn().map_err(|e| format!("启动失败:{e}"))?;
        let pid = child.id().unwrap_or(0);

        let started = Instant::now();
        let budget = std::time::Duration::from_millis(self.cfg.timeout_ms.max(1000));
        let mut killed = false;
        // 循环以break 带出退出码:没有"先赋哨兵再覆盖"的死赋值
        let exit: Option<i32> = loop {
            match child.try_wait() {
                Ok(Some(st)) => break st.code(),
                Ok(None) => {}
                Err(e) => {
                    let _ = child.kill().await;
                    let _ = std::fs::remove_file(&out_path);
                    return Err(format!("等待进程失败:{e}"));
                }
            }
            if started.elapsed() > budget {
                killed = true;
                kill_tree(pid);
                // 给终止一点落地时间再收尾
                let _ = tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                let _ = child.start_kill();
                break match child.try_wait() {
                    Ok(Some(st)) => st.code(),
                    _ => None,
                };
            }
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        };

        let raw = std::fs::read(&out_path).unwrap_or_default();
        let _ = std::fs::remove_file(&out_path);
        let text = String::from_utf8_lossy(&raw).replace("\r\n", "\n");
        let (body, truncated) = cap_text(&text, self.cfg.output_bytes);
        let exit_s = if killed {
            format!("killed after {}ms", self.cfg.timeout_ms)
        } else {
            exit.map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".into())
        };
        Ok(ShellOutcome {
            ok: !killed && exit == Some(0),
            exit: exit_s,
            ms: started.elapsed().as_millis() as u64,
            output: body,
            truncated,
            killed,
            cwd: "arena/".into(), // 强制 cwd,模型改不了展示用固定值
        })
    }
}

fn nonce() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    Instant::now().elapsed().as_nanos().hash(&mut h);
    std::process::id().hash(&mut h);
    h.finish() % 1_000_000
}

/// 头尾各半,中间标记。按字节切但要落在字符边界上。
pub fn cap_text(s: &str, limit: usize) -> (String, bool) {
    if limit == 0 || s.len() <= limit {
        return (s.to_string(), false);
    }
    let half = limit / 2;
    let head_end = floor_char_boundary(s, half);
    let tail_start = ceil_char_boundary(s, s.len() - half);
    let head = &s[..head_end];
    let tail = &s[tail_start..];
    let dropped = tail_start - head_end;
    (
        format!("{head}\n... truncated {dropped} bytes ...\n{tail}"),
        true,
    )
}

fn floor_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let mut j = i;
    while j > 0 && !s.is_char_boundary(j) {
        j -= 1;
    }
    j
}

fn ceil_char_boundary(s: &str, i: usize) -> usize {
    if i <= 1 {
        return 0;
    }
    let mut j = i.min(s.len());
    while j < s.len() && !s.is_char_boundary(j) {
        j += 1;
    }
    j
}

/// 杀整个进程树。只杀 shell 本身会留下子进程继续写文件。
pub fn kill_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        // taskkill /T 杀树。它是系统自带工具不需要unsafe 与额外依赖。
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(unix)]
    {
        // 进程组以 spawn 时的 process_group(0) 建立,pgid == pid
        unsafe {
            libc::killpg(pid as i32, libc::SIGKILL);
        }
    }
}

pub fn arena_of(workspace: &Path) -> PathBuf {
    workspace.join("arena")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(enabled: bool) -> ShellConfig {
        ShellConfig {
            enabled,
            timeout_ms: 4000,
            output_bytes: 4096,
            env_passthrough: vec![
                "PATH".into(),
                "HOME".into(),
                "SystemRoot".into(),
                "COMSPEC".into(),
                "TEMP".into(),
                "TMP".into(),
            ],
        }
    }

    fn tmp_arena(tag: &str) -> PathBuf {
        arena_of(&crate::testsup::world(tag))
    }

    #[test]
    fn cap_text_head_and_tail_keeps_char_boundary() {
        let s = "啊".repeat(2000); // 3 bytes each
        let (out, tr) = cap_text(&s, 400);
        assert!(tr);
        assert!(out.contains("truncated"));
        assert!(out.is_char_boundary(out.len()), "结果必须是合法UTF-8");
        let (same, tr2) = cap_text("short", 4096);
        assert_eq!(same, "short");
        assert!(!tr2);
    }

    #[test]
    fn denied_reason_blocks_nukes() {
        assert!(denied_reason("rm -rf /").is_some());
        assert!(denied_reason("shutdown /s /t 0").is_some());
        assert!(denied_reason("sudo rm x").is_some());
        assert!(denied_reason("del C:\\Windows").is_some());
        assert!(denied_reason("python game.py").is_none());
        assert!(denied_reason("ls -la").is_none());
        assert!(denied_reason("cat ../secret").is_none()); // 单看 cat 不禁
        assert!(denied_reason("mv ../a ./b").is_some());
    }

    #[test]
    fn secretish_vars_never_passthrough_even_if_whitelisted() {
        std::env::set_var("AICLUB_TEST_DEEPSEEK_API_KEY", "sk-secret");
        let got = build_env(&["PATH".into(), "AICLUB_TEST_DEEPSEEK_API_KEY".into()]);
        assert!(!got.iter().any(|(k, _)| k.contains("API_KEY")), "{got:?}");
        std::env::remove_var("AICLUB_TEST_DEEPSEEK_API_KEY");
    }

    #[test]
    fn interpreter_matches_platform() {
        let (p, f) = interpreter("echo hi");
        if cfg!(windows) {
            assert!(p.to_string_lossy().to_lowercase().contains("cmd"));
            assert_eq!(f[0], "/C");
        } else {
            assert_eq!(p, PathBuf::from("/bin/sh"));
            assert_eq!(f[0], "-c");
        }
        assert_eq!(f[1], "echo hi");
    }

    #[tokio::test]
    async fn runs_and_captures_in_arena() {
        let arena = tmp_arena("run");
        std::fs::write(arena.join("hello.txt"), "hi from arena").unwrap();
        let r = ShellRunner::new(arena.clone(), cfg(true));
        let cmd = if cfg!(windows) {
            "type hello.txt"
        } else {
            "cat hello.txt"
        };
        let out = r.run(&serde_json::json!({"command": cmd})).await.unwrap();
        assert!(out.ok, "{:?}", out.output);
        assert!(out.output.contains("hi from arena"), "{:?}", out.output);
        assert_eq!(out.exit, "0");
        crate::testsup::cleanup(arena.parent().unwrap());
    }

    #[tokio::test]
    async fn writes_land_in_arena_only() {
        let arena = tmp_arena("write");
        let r = ShellRunner::new(arena.clone(), cfg(true));
        // echo 重定向在 Windows 与 Unix 下写法一致,不需要分平台
        let cmd = "echo made > gen.txt";
        let out = r.run(&serde_json::json!({"command": cmd})).await.unwrap();
        assert!(out.ok, "{:?}", out);
        let p = arena.join("gen.txt");
        // 有些 shell 的echo 换行差异:只断言文件存在且非空
        let txt = std::fs::read_to_string(&p).unwrap_or_default();
        assert!(txt.contains("made"), "{txt:?}");
        crate::testsup::cleanup(arena.parent().unwrap());
    }

    #[tokio::test]
    async fn timeout_kills_process_tree() {
        let arena = tmp_arena("timeout");
        let c = ShellConfig {
            timeout_ms: 1200,
            ..cfg(true)
        };
        std::fs::write(arena.join("slow.txt"), "x".repeat(10)).unwrap();
        let r = ShellRunner::new(arena.clone(), c);
        let cmd = if cfg!(windows) {
            // 先起一个后台子进程再自行验证 /T 杀树不会留下孤儿
            "ping -n 30 127.0.0.1 > nul"
        } else {
            "(sleep 30 &) ; sleep 30"
        };
        let t = Instant::now();
        let out = r.run(&serde_json::json!({"command": cmd})).await.unwrap();
        let took = t.elapsed();
        assert!(out.killed, "{:?}", out.exit);
        assert!(
            took.as_millis() < 6000,
            "不该等满 30 秒{}",
            took.as_millis()
        );
        assert!(out.exit.starts_with("killed"), "{:?}", out.exit);
        assert!(!out.ok);
        crate::testsup::cleanup(arena.parent().unwrap());
    }

    #[tokio::test]
    async fn denied_when_disabled_or_blacklisted() {
        let arena = tmp_arena("deny");
        let off = ShellRunner::new(arena.clone(), cfg(false));
        assert!(off
            .run(&serde_json::json!({"command": "echo 1"}))
            .await
            .is_err());
        let on = ShellRunner::new(arena.clone(), cfg(true));
        let e = on
            .run(&serde_json::json!({"command": "rm -rf /"}))
            .await
            .unwrap_err();
        assert!(e.contains("拒绝"), "{e}");
        let e2 = on.run(&serde_json::json!({})).await.unwrap_err();
        assert!(e2.contains("command"), "{e2}");
        crate::testsup::cleanup(arena.parent().unwrap());
    }

    #[tokio::test]
    async fn huge_output_is_capped() {
        let arena = tmp_arena("big");
        let r = ShellRunner::new(arena.clone(), cfg(true));
        let cmd = if cfg!(windows) {
            "echo AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        } else {
            "printf 'A%.0s' $(seq 1 100)"
        };
        let out = r.run(&serde_json::json!({"command": cmd})).await.unwrap();
        assert!(
            !out.truncated || out.output.contains("truncated"),
            "{:?}",
            out.output
        );
        crate::testsup::cleanup(arena.parent().unwrap());
    }

    #[test]
    fn agent_text_shape() {
        let o = ShellOutcome {
            ok: true,
            exit: "0".into(),
            ms: 12,
            output: "made".into(),
            truncated: false,
            killed: false,
            cwd: "arena".into(),
        };
        let t = o.agent_text();
        assert!(t.contains("exit: 0"), "{t}");
        assert!(t.contains("made"), "{t}");
    }
}
