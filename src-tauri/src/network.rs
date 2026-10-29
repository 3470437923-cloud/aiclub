//! Bounded networking without provider credentials; writes use the role sandbox.
use crate::sandbox::{self, Access};
use futures_util::StreamExt;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
pub const MAX_DOWNLOAD_BYTES: usize = 10 * 1024 * 1024;
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20);
fn checked_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "URL 无效".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || credential_query(&url)
    {
        return Err("只允许不含凭证的 HTTP(S) URL".into());
    }
    Ok(url)
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3 {
                return attempt.stop();
            }
            let u = attempt.url();
            if !matches!(u.scheme(), "http" | "https")
                || !u.username().is_empty()
                || u.password().is_some()
                || credential_query(u)
            {
                return attempt.error("redirect URL rejected");
            }
            attempt.follow()
        }))
        .build()
        .map_err(|e| format!("HTTP 客户端失败:{e}"))
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let response = client()?
        .get(checked_url(url)?)
        .send()
        .await
        .map_err(|e| format!("请求失败:{e}"))?;
    if !response.status().is_success() {
        return Err(format!("请求失败:HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_DOWNLOAD_BYTES as u64)
    {
        return Err("下载超过10MiB上限".into());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("读取响应失败:{e}"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_DOWNLOAD_BYTES {
            return Err("下载超过10MiB上限".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub async fn web_fetch(args: &Value) -> Result<String, String> {
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or("url是必填项")?;
    let text = String::from_utf8(fetch_bytes(url).await?).map_err(|_| "响应不是UTF-8文本")?;
    let mut chars = text.chars();
    let result: String = chars.by_ref().take(32_768).collect();
    Ok(if chars.next().is_some() {
        format!("{result}\n…(响应已截断到32768字符)…")
    } else {
        result
    })
}
fn target_for(workspace: &Path, agent: &str, rel: &str) -> Result<PathBuf, String> {
    let target =
        sandbox::resolve(workspace, agent, rel, Access::Write).map_err(|e| e.agent_message())?;
    sandbox::check_agent_path(workspace, agent, &target, Access::Write)
        .map_err(|e| e.agent_message())?;
    Ok(target)
}
fn save_bytes(workspace: &Path, agent: &str, rel: &str, bytes: &[u8]) -> Result<String, String> {
    let target = target_for(workspace, agent, rel)?;
    let parent = target.parent().ok_or("目标没有父目录")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败:{e}"))?;
    target_for(workspace, agent, rel)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp = parent.join(format!(
        ".aiclub-download-{}-{nonce}.tmp",
        std::process::id()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| format!("创建临时文件失败:{e}"))?;
        file.write_all(bytes).map_err(|e| format!("写入失败:{e}"))?;
        file.sync_all().map_err(|e| format!("同步失败:{e}"))?;
        drop(file);
        target_for(workspace, agent, rel)?;
        sandbox::check_agent_path(workspace, agent, &tmp, Access::Write)
            .map_err(|e| e.agent_message())?;
        std::fs::rename(&tmp, &target).map_err(|e| format!("提交失败:{e}"))?;
        Ok(format!("已下载 {} 字节到 {rel}", bytes.len()))
    })();
    if result.is_err() && sandbox::check_agent_path(workspace, agent, &tmp, Access::Write).is_ok() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
pub async fn download_file(
    workspace: &Path,
    agent: &str,
    url: &str,
    rel: &str,
) -> Result<String, String> {
    target_for(workspace, agent, rel)?;
    let bytes = fetch_bytes(url).await?;
    // Revalidate after all network awaits; the workspace may have changed meanwhile.
    save_bytes(workspace, agent, rel, &bytes)
}
fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 96
        && slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn credential_query(url: &reqwest::Url) -> bool {
    url.query_pairs().any(|(k, _)| {
        let k = k.to_ascii_lowercase();
        [
            "key",
            "api_key",
            "apikey",
            "token",
            "secret",
            "password",
            "credential",
        ]
        .iter()
        .any(|bad| k.contains(bad))
    })
}

pub async fn download_skill(workspace: &Path, agent: &str, args: &Value) -> Result<String, String> {
    let slug = args
        .get("slug")
        .and_then(Value::as_str)
        .ok_or("slug是必填项")?;
    if !valid_slug(slug) {
        return Err("slug只能含字母、数字、-、_，最多96字符".into());
    }
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or("url是必填项")?;
    let rel = format!("skills/private/{agent}/{slug}/SKILL.md");
    target_for(workspace, agent, &rel)?;
    let bytes = fetch_bytes(url).await?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "技能必须为UTF-8文本")?;
    if text.trim().is_empty() {
        return Err("技能不能为空".into());
    }
    save_bytes(workspace, agent, &rel, &bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    fn serve(
        response: Vec<u8>,
        before_reply: impl FnOnce() + Send + 'static,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/test", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 8192];
            let n = socket.read(&mut request).unwrap();
            assert!(!String::from_utf8_lossy(&request[..n])
                .to_lowercase()
                .contains("authorization:"));
            before_reply();
            let _ = socket.write_all(&response);
        });
        (url, handle)
    }
    fn ok(body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend(body);
        response
    }
    #[tokio::test]
    async fn fetch_and_download_succeed_without_credentials() {
        let ws = crate::testsup::world("download-ok");
        let (url, server) = serve(ok(b"hello"), || {});
        assert_eq!(
            web_fetch(&serde_json::json!({"url":url})).await.unwrap(),
            "hello"
        );
        server.join().unwrap();
        for rel in [
            "arena/test.bin",
            "memory/A/test.bin",
            "skills/private/A/code/SKILL.md",
        ] {
            let (url, server) = serve(ok(b"bytes"), || {});
            download_file(&ws, "A", &url, rel).await.unwrap();
            server.join().unwrap();
            assert_eq!(std::fs::read(ws.join(rel)).unwrap(), b"bytes");
        }
        let (url, server) = serve(ok(b"# Sample skill"), || {});
        download_skill(&ws, "A", &serde_json::json!({"url":url,"slug":"sample"}))
            .await
            .unwrap();
        server.join().unwrap();
        assert!(ws.join("skills/private/A/sample/SKILL.md").exists());
        crate::testsup::cleanup(&ws);
    }
    #[tokio::test]
    async fn oversized_and_failed_downloads_preserve_existing_file_and_leave_no_temp() {
        let ws = crate::testsup::world("download-bounds");
        std::fs::write(ws.join("arena/old"), b"original").unwrap();
        for response in [
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_DOWNLOAD_BYTES + 1
            )
            .into_bytes(),
            b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        ] {
            let (url, server) = serve(response, || {});
            assert!(download_file(&ws, "A", &url, "arena/old").await.is_err());
            server.join().unwrap();
            assert_eq!(std::fs::read(ws.join("arena/old")).unwrap(), b"original");
        }
        assert_eq!(std::fs::read_dir(ws.join("arena")).unwrap().count(), 1);
        crate::testsup::cleanup(&ws);
    }
    #[tokio::test]
    async fn redirects_with_credentials_and_binary_fetch_are_rejected() {
        let (url, server) = serve(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/private?api_key=fake-secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(), || {});
        assert!(web_fetch(&serde_json::json!({"url":url})).await.is_err());
        server.join().unwrap();
        let (url, server) = serve(ok(&[255, 254]), || {});
        assert!(web_fetch(&serde_json::json!({"url":url})).await.is_err());
        server.join().unwrap();
        for bad in [
            "file:///tmp/secret",
            "https://user:pass@example.com",
            "https://example.com/?token=secret",
        ] {
            assert!(checked_url(bad).is_err());
        }
        assert!(!valid_slug("../escape"));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn download_rechecks_owner_after_network_await() {
        let ws = crate::testsup::world("download-recheck");
        std::fs::create_dir_all(ws.join("arena/link")).unwrap();
        let change = ws.clone();
        let (url, server) = serve(ok(b"do-not-leak"), move || {
            std::fs::remove_dir(change.join("arena/link")).unwrap();
            std::os::unix::fs::symlink(change.join("memory/B"), change.join("arena/link")).unwrap();
        });
        assert!(download_file(&ws, "A", &url, "arena/link/file")
            .await
            .is_err());
        server.join().unwrap();
        assert!(!ws.join("memory/B/file").exists());
        crate::testsup::cleanup(&ws);
    }
}
