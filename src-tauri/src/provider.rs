//! OpenAI 兼容接入层(§4.1)+ 独立性可证伪条款(§4.3)。
//!
//! 独立性在这里落地的方式:
//! - 每个 agent 各持一个 `ProviderClient`(各自 `reqwest::Client`、独立连接池、独立请求头),
//!   没有共享 client,没有跨 agent 的 header 继承;
//! - `Authorization` 只从**本档案**的 api_key 构造;
//! - 上下文由 `context::build_messages` 生成,那里的隔离断言由本文件配合(见测试)。
//!
//! SSE 解析自研:`bytes_stream` + 行缓冲,处理跨包半行、多行 data、注释心跳、`[DONE]`、坏 UTF-8。

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::events::UsagePair;
use crate::validate::norm_host;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 流式增量回调:provider 每解析出一个 content 片段就**同步**调用它。
/// 引擎在里面直接落 SayDelta + 推 UI,不需要并发轮询 channel(那需要 tokio "time" feature 之外的东西,
/// 且 await 完才转发会让直播假死)。
pub type DeltaCb = Arc<dyn Fn(String) + Send + Sync>;

// ───────────────────────────── 类型 ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 原始 JSON 字符串(增量拼装完成后由工具层 parse)
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Msg {
    System(String),
    User(String),
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        call_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

pub struct ChatRequest {
    pub messages: Vec<Msg>,
    pub tools: Vec<ToolSchema>,
    pub temperature: f32,
    pub max_output_tokens: u32,
    pub delta_cb: Option<DeltaCb>,
}

impl ChatRequest {
    pub fn simple(messages: Vec<Msg>) -> Self {
        Self {
            messages,
            tools: Vec::new(),
            temperature: 0.7,
            max_output_tokens: 1024,
            delta_cb: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatResult {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: UsagePair,
    pub finish_reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderError {
    /// 网络/超时/5xx/429(可重试)
    Transient(String),
    /// 4xx:参数或鉴权问题(重试无意义)
    Fatal(String),
    /// 响应形状不对
    Malformed(String),
}

impl ProviderError {
    pub fn retryable(&self) -> bool {
        matches!(self, ProviderError::Transient(_))
    }
    pub fn message(&self) -> String {
        match self {
            ProviderError::Transient(m) | ProviderError::Fatal(m) | ProviderError::Malformed(m) => {
                m.clone()
            }
        }
    }
}

/// 引擎只认这个 trait(§7 可测性契约:假 provider 驱动全套状态机)。
pub trait Provider: Send + Sync {
    fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<ChatResult, ProviderError>>;
    fn label(&self) -> String;
    /// 独立性断言用:本 client 的凭证指纹(不可逆)
    fn key_fingerprint(&self) -> String;
    /// 独立性断言用:本 client 被调用过几次(压缩是否只走本方端点)
    fn call_count(&self) -> usize;
}

pub type SharedProvider = Arc<dyn Provider>;

// ───────────────────────────── SSE ─────────────────────────────

#[derive(Default)]
pub struct SseDecoder {
    buf: Vec<u8>,
    done: bool,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一段字节,返回本次可交付的 `data:` 载荷(已去前缀、已拼多行)。
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        if self.done {
            return out;
        }
        self.buf.extend_from_slice(bytes);
        while let Some(idx) = self.buf.iter().position(|b| *b == b'\n') {
            let line = self.buf[..idx].to_vec();
            self.buf.drain(..idx + 1);
            let s = String::from_utf8_lossy(&line)
                .trim_end_matches('\r')
                .to_string();
            if s.is_empty() || s.starts_with(':') {
                continue;
            }
            if let Some(rest) = s.strip_prefix("data:") {
                let payload = rest.trim();
                if payload == "[DONE]" {
                    self.done = true;
                    return out;
                }
                if !payload.is_empty() {
                    out.push(payload.to_string());
                }
            }
        }
        out
    }

    /// 流结束但 buf 里还剩半行(有的端点不发结尾换行)
    pub fn finish(&mut self) -> Vec<String> {
        if self.buf.is_empty() {
            return Vec::new();
        }
        let rest = std::mem::take(&mut self.buf);
        let s = String::from_utf8_lossy(&rest)
            .trim_end_matches('\r')
            .to_string();
        if self.done || !s.starts_with("data:") {
            return Vec::new();
        }
        let payload = s[5..].trim();
        if payload.is_empty() || payload == "[DONE]" {
            Vec::new()
        } else {
            vec![payload.to_string()]
        }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }
}

#[derive(Debug, Default)]
pub struct TurnAccumulator {
    pub text: String,
    calls: Vec<PartialCall>,
    pub usage: UsagePair,
    pub got_usage: bool,
    pub finish_reason: Option<String>,
}

/// 一次响应里允许的 tool_call 槽位上限(见 index 处的说明)。
/// 只用来挡恶意的巨大 index,不是业务限额 —— 业务限额在 engine 侧(每轮 8 次)。
const MAX_TOOL_CALL_SLOTS: usize = 64;

#[derive(Debug, Default)]
struct PartialCall {
    id: String,
    name: String,
    args: String,
}

impl TurnAccumulator {
    /// 解析一个 chunk;返回 content 增量(调用方负责转发给 delta 回调)。
    pub fn apply_chunk(&mut self, raw: &str) -> Vec<String> {
        let mut deltas = Vec::new();
        let Ok(v) = serde_json::from_str::<Value>(raw) else {
            return deltas;
        };
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
            let g = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            self.usage = UsagePair {
                input: g("prompt_tokens"),
                output: g("completion_tokens"),
                cached: u
                    .get("prompt_tokens_details")
                    .and_then(|d| d.get("cached_tokens"))
                    .and_then(Value::as_u64),
            };
            self.got_usage = true;
        }
        let Some(choice) = v
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .cloned()
        else {
            return deltas;
        };
        if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(fr.to_string());
        }
        // 非流式响应把正文放在 message 里,这里一并吃下
        let src = choice.get("delta").or_else(|| choice.get("message"));
        if let Some(t) = src.and_then(|d| d.get("content")).and_then(Value::as_str) {
            if !t.is_empty() {
                self.text.push_str(t);
                deltas.push(t.to_string());
            }
        }
        if let Some(calls) = src
            .and_then(|d| d.get("tool_calls"))
            .and_then(Value::as_array)
        {
            for tc in calls {
                let idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                // 上限不能省。
                //
                // idx 完全来自端点返回的 JSON,而下面这个 while 是按 idx 增长数组的:
                // index=4294967295 会要求约 300GB 内存(PartialCall 是 3 个 String),
                // 分配失败直接 abort —— 整个应用崩掉;index=u64::MAX 则近似死循环。
                // engine 的 MAX_TOOL_CALLS_PER_TURN=8 是在**解析之后**才生效的,救不了这里。
                // 一次流式响应里并发工具不会超过十来个,64 是很宽松的上限。
                if idx >= MAX_TOOL_CALL_SLOTS {
                    continue;
                }
                while self.calls.len() <= idx {
                    self.calls.push(PartialCall::default());
                }
                let slot = &mut self.calls[idx];
                if let Some(id) = tc.get("id").and_then(Value::as_str) {
                    slot.id = id.to_string();
                }
                if let Some(f) = tc.get("function") {
                    if let Some(n) = f.get("name").and_then(Value::as_str) {
                        if !n.is_empty() {
                            slot.name = n.to_string();
                        }
                    }
                    if let Some(a) = f.get("arguments").and_then(Value::as_str) {
                        slot.args.push_str(a);
                    }
                }
            }
        }
        deltas
    }

    /// 无正文时的输入侧估算(§KL-04:端点不给 usage 就粗估并标注)
    pub fn finalize(self, prompt_chars: usize) -> ChatResult {
        let mut usage = self.usage;
        if !self.got_usage {
            usage = UsagePair {
                input: estimate_tokens_by_len(prompt_chars),
                output: estimate_tokens_by_len(self.text.chars().count()),
                cached: None,
            };
        }
        let mut calls = Vec::new();
        for (i, c) in self.calls.into_iter().enumerate() {
            if c.name.is_empty() {
                continue;
            }
            calls.push(ToolCall {
                id: if c.id.is_empty() {
                    format!("call_{i}")
                } else {
                    c.id
                },
                name: c.name,
                arguments: c.args,
            });
        }
        ChatResult {
            text: self.text,
            tool_calls: calls,
            usage,
            finish_reason: self.finish_reason.unwrap_or_default(),
        }
    }
}

pub fn estimate_tokens_by_len(chars: usize) -> u64 {
    // 4 chars/token 粗估;CJK 偏密,取 chars/2 与 chars/4 的折中
    (chars as u64 + 3) / 3
}

// ───────────────────────────── 真实 provider ─────────────────────────────

pub struct ProviderClient {
    pub agent_id: String,
    base_url: String,
    api_key: String,
    model: String,
    http: reqwest::Client,
}

impl ProviderClient {
    /// 每个 agent 各调一次:**全新 Client**,独立连接池与默认头(Spec §4.3)。
    pub fn new(agent_id: &str, base_url: &str, api_key: &str, model: &str) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .tcp_nodelay(true)
            .build()
            .unwrap_or_default();
        Self {
            agent_id: agent_id.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            http,
        }
    }

    fn prompt_chars(&self, msgs: &[Msg]) -> usize {
        msgs.iter()
            .map(|m| match m {
                Msg::System(c) | Msg::User(c) => c.chars().count(),
                Msg::Tool { content, .. } => content.chars().count(),
                Msg::Assistant { text, tool_calls } => {
                    text.chars().count()
                        + tool_calls
                            .iter()
                            .map(|c| c.arguments.chars().count())
                            .sum::<usize>()
                }
            })
            .sum()
    }

    pub fn build_body(&self, req: &ChatRequest, stream: bool) -> Value {
        let mut msgs: Vec<Value> = Vec::new();
        for m in &req.messages {
            msgs.push(match m {
                Msg::System(c) => serde_json::json!({"role":"system","content":c}),
                Msg::User(c) => serde_json::json!({"role":"user","content":c}),
                Msg::Assistant { text, tool_calls } => {
                    let mut v = serde_json::Map::new();
                    v.insert("role".into(), serde_json::json!("assistant"));
                    if text.is_empty() {
                        v.insert("content".into(), Value::Null);
                    } else {
                        v.insert("content".into(), serde_json::json!(text));
                    }
                    if !tool_calls.is_empty() {
                        let tcs: Vec<Value> = tool_calls
                            .iter()
                            .map(|c| {
                                serde_json::json!({
                                    "id": c.id, "type": "function",
                                    "function": {"name": c.name, "arguments": c.arguments}
                                })
                            })
                            .collect();
                        v.insert("tool_calls".into(), serde_json::json!(tcs));
                    }
                    Value::Object(v)
                }
                Msg::Tool { call_id, content } => {
                    serde_json::json!({"role":"tool","tool_call_id":call_id,"content":content})
                }
            });
        }
        let mut body = serde_json::Map::new();
        body.insert("model".into(), serde_json::json!(self.model));
        body.insert("messages".into(), serde_json::json!(msgs));
        body.insert("temperature".into(), serde_json::json!(req.temperature));
        body.insert(
            "max_tokens".into(),
            serde_json::json!(req.max_output_tokens),
        );
        body.insert("stream".into(), serde_json::json!(stream));
        if stream {
            body.insert(
                "stream_options".into(),
                serde_json::json!({"include_usage": true}),
            );
        }
        if !req.tools.is_empty() {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {"name": t.name, "description": t.description, "parameters": t.parameters}
                    })
                })
                .collect();
            body.insert("tools".into(), serde_json::json!(tools));
            body.insert("tool_choice".into(), serde_json::json!("auto"));
        }
        Value::Object(body)
    }
}

impl Provider for ProviderClient {
    fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<ChatResult, ProviderError>> {
        Box::pin(async move {
            let stream = req.delta_cb.is_some();
            let body = self.build_body(&req, stream);
            let url = format!("{}/chat/completions", self.base_url);
            let resp = match self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => return Err(ProviderError::Transient(format!("连接失败:{e}"))),
            };
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                let brief: String = redact_secret(&text, &self.api_key)
                    .chars()
                    .take(400)
                    .collect();
                let msg = format!("HTTP {status}:{brief}");
                return Err(
                    if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        ProviderError::Transient(msg)
                    } else {
                        ProviderError::Fatal(msg)
                    },
                );
            }

            let mut acc = TurnAccumulator::default();
            if !stream {
                let v: Value = resp
                    .json()
                    .await
                    .map_err(|e| ProviderError::Malformed(format!("响应解析失败:{e}")))?;
                if v.get("choices").is_none() {
                    return Err(ProviderError::Malformed("响应缺少 choices".into()));
                }
                acc.apply_chunk(&v.to_string());
                return Ok(acc.finalize(self.prompt_chars(&req.messages)));
            }

            let mut stream = resp.bytes_stream();
            let mut dec = SseDecoder::new();
            while let Some(chunk) = stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => return Err(ProviderError::Transient(format!("流中断:{e}"))),
                };
                for payload in dec.feed(&bytes) {
                    for d in acc.apply_chunk(&payload) {
                        if let Some(cb) = &req.delta_cb {
                            cb(d);
                        }
                    }
                }
                if dec.is_done() {
                    break;
                }
            }
            for payload in dec.finish() {
                for d in acc.apply_chunk(&payload) {
                    if let Some(cb) = &req.delta_cb {
                        cb(d);
                    }
                }
            }
            let res = acc.finalize(self.prompt_chars(&req.messages));
            if res.text.trim().is_empty() && res.tool_calls.is_empty() {
                return Err(ProviderError::Malformed(
                    "空响应:既无文本也无工具调用".into(),
                ));
            }
            Ok(res)
        })
    }

    fn label(&self) -> String {
        format!("{}@{}", self.model, norm_host(&self.base_url))
    }
    fn key_fingerprint(&self) -> String {
        format!("{:016x}", fnv(&self.api_key))
    }
    fn call_count(&self) -> usize {
        0
    }
}

fn redact_secret(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "[已隐藏]")
    }
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

// ───────────────────────────── 连通探测 ─────────────────────────────

/// `GET {base}/models` —— 检测连接并拉模型清单。
///
/// 这里必须给**总超时**,不能只给 connect_timeout:
/// connect 成功但对方不再回字节(代理/NAT 黑洞、上游卡死)时,请求会永远挂着,
/// 设置页的「探测」按钮就一直是"探测中…",`testing` 状态再也回不来。
/// 整个调用用 20 秒封顶,足够慢速端点返回一张模型表。
pub async fn fetch_models(base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let endpoint = reqwest::Url::parse(base_url).map_err(|_| "接口地址无效")?;
    if !matches!(endpoint.scheme(), "http" | "https")
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err("接口地址必须为有效 HTTP(S) 地址且不能包含凭证".into());
    }
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "探测超时(20 秒):端点连上了但没有在时限内返回模型列表".to_string()
            } else {
                format!("连接失败:{e}")
            }
        })?;
    let status = resp.status();
    if !status.is_success() {
        let t: String = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect();
        return Err(format!("HTTP {status}:{}", redact_secret(&t, api_key)));
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("响应不是 JSON:{e}"))?;
    let arr = v
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| v.as_array().cloned())
        .unwrap_or_default();
    let mut ids: Vec<String> = arr
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

// ───────────────────────────── 假 provider(测试 / 冒烟) ─────────────────────────────

pub struct ScriptedProvider {
    pub agent_id: String,
    steps: Mutex<Vec<Result<ChatResult, ProviderError>>>,
    label: String,
    key_fp: String,
    calls: Mutex<usize>,
    pub prompts: Mutex<Vec<Vec<Msg>>>,
    pub tool_names: Mutex<Vec<Vec<String>>>,
}

impl ScriptedProvider {
    pub fn new(
        agent_id: &str,
        label: &str,
        key: &str,
        steps: Vec<Result<ChatResult, ProviderError>>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            steps: Mutex::new(steps),
            label: label.into(),
            key_fp: format!("{:016x}", fnv(key)),
            calls: Mutex::new(0),
            prompts: Mutex::new(Vec::new()),
            tool_names: Mutex::new(Vec::new()),
        }
    }
    /// 按标点切片,模拟流式
    fn emit_pieces(r: &ChatResult, cb: &Option<DeltaCb>) {
        let Some(cb) = cb else { return };
        for piece in r.text.split_inclusive(['。', '！', '？', '\n', '，']) {
            if !piece.is_empty() {
                cb(piece.to_string());
            }
        }
    }
}

impl Provider for ScriptedProvider {
    fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<ChatResult, ProviderError>> {
        Box::pin(async move {
            *self.calls.lock().unwrap() += 1;
            self.prompts.lock().unwrap().push(req.messages.clone());
            self.tool_names
                .lock()
                .unwrap()
                .push(req.tools.iter().map(|t| t.name.clone()).collect());
            let mut q = self.steps.lock().unwrap();
            let step = match if q.is_empty() {
                None
            } else {
                Some(q.remove(0))
            } {
                Some(s) => s,
                None => Ok(ChatResult {
                    text: String::new(),
                    tool_calls: vec![],
                    usage: UsagePair::default(),
                    finish_reason: "stop".into(),
                }),
            };
            match step {
                Ok(r) => {
                    Self::emit_pieces(&r, &req.delta_cb);
                    Ok(r)
                }
                Err(e) => Err(e),
            }
        })
    }
    fn label(&self) -> String {
        self.label.clone()
    }
    fn key_fingerprint(&self) -> String {
        self.key_fp.clone()
    }
    fn call_count(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn provider_errors_redact_entire_key_before_truncation() {
        let secret = "test-private-credential";
        let message = super::redact_secret(&format!("401 invalid {secret}"), secret);
        assert!(message.contains("401"));
        assert!(!message.contains(secret));
        assert_eq!(super::redact_secret("unchanged", ""), "unchanged");
    }

    #[tokio::test]
    async fn model_probe_rejects_non_http_and_embedded_credentials() {
        for endpoint in [
            "file:///tmp/key",
            "https://user:password@example.com",
            "bad",
        ] {
            assert!(super::fetch_models(endpoint, "fake-key").await.is_err());
        }
    }

    use super::*;
    use serde_json::json;

    fn chunk(content: Option<&str>, fr: Option<&str>) -> String {
        let mut delta = serde_json::Map::new();
        if let Some(x) = content {
            delta.insert("content".into(), json!(x));
        }
        let mut c = serde_json::Map::new();
        c.insert("delta".into(), Value::Object(delta));
        if let Some(f) = fr {
            c.insert("finish_reason".into(), json!(f));
        }
        json!({"choices":[c],"usage":{"prompt_tokens":7,"completion_tokens":3}}).to_string()
    }

    #[test]
    fn sse_byte_by_byte_split_still_yields_both_payloads() {
        let mut d = SseDecoder::new();
        let full = chunk(Some("你好"), None);
        let bytes = format!("data: {full}\n\ndata: {full}\n");
        let mut got = Vec::new();
        for b in bytes.as_bytes() {
            got.extend(d.feed(std::slice::from_ref(b)));
        }
        got.extend(d.finish());
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(!d.is_done());
    }

    #[test]
    fn sse_done_and_heartbeat() {
        let mut d = SseDecoder::new();
        assert!(d.feed(b": heartbeat\n\n").is_empty());
        let out = d.feed(b"data: [DONE]\n\ndata: {\"x\":1}\n");
        assert!(out.is_empty(), "[DONE] 之后应停止");
        assert!(d.is_done());
        assert!(d.finish().is_empty());
    }

    #[test]
    fn sse_bad_utf8_is_lossy_not_panic() {
        let mut d = SseDecoder::new();
        let mut bytes = b"data: {\"a\":\"".to_vec();
        bytes.push(0xFF);
        bytes.push(0xFE);
        bytes.extend(b"\"}\n\n");
        assert_eq!(d.feed(&bytes).len(), 1);
    }

    #[test]
    fn tail_line_without_newline_is_flushed() {
        let mut d = SseDecoder::new();
        let payload = chunk(Some("尾行"), None);
        assert!(d.feed(format!("data: {payload}").as_bytes()).is_empty());
        let out = d.finish();
        assert_eq!(out.len(), 1, "{out:?}");
    }

    #[test]
    fn tool_calls_incremental_by_index() {
        let mut acc = TurnAccumulator::default();
        acc.apply_chunk(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c0","function":{"name":"read","arguments":""}}]}}]}"#);
        acc.apply_chunk(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\""}}]}}]}"#);
        acc.apply_chunk(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"arena/x.md\"}"}}]}}]}"#);
        acc.apply_chunk(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#);
        let r = acc.finalize(100);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "read");
        assert_eq!(r.tool_calls[0].arguments, r#"{"path":"arena/x.md"}"#);
        assert_eq!(r.finish_reason, "tool_calls");
    }

    #[test]
    fn usage_missing_falls_back_to_estimate() {
        let mut acc = TurnAccumulator::default();
        acc.apply_chunk(
            r#"{"choices":[{"delta":{"content":"一二三四五"},"finish_reason":"stop"}]}"#,
        );
        let r = acc.finalize(2000);
        assert!(r.usage.input > 0 && r.usage.output > 0, "{:?}", r.usage);
        assert_eq!(r.text, "一二三四五");
    }

    #[test]
    fn nonstream_message_shape_is_parsed() {
        let mut acc = TurnAccumulator::default();
        acc.apply_chunk(
            r#"{"choices":[{"message":{"role":"assistant","content":"直接答案"},"finish_reason":"stop"}],"usage":{"prompt_tokens":9,"completion_tokens":4}}"#,
        );
        let r = acc.finalize(50);
        assert_eq!(r.text, "直接答案");
        assert_eq!((r.usage.input, r.usage.output), (9, 4));
    }

    #[test]
    fn body_shape_and_tools() {
        let p = ProviderClient::new("A", "https://api.x.com/", "k", "m");
        let req = ChatRequest {
            messages: vec![
                Msg::System("s".into()),
                Msg::User("u".into()),
                Msg::Assistant {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "read".into(),
                        arguments: "{}".into(),
                    }],
                },
                Msg::Tool {
                    call_id: "c1".into(),
                    content: "ok".into(),
                },
            ],
            tools: vec![ToolSchema {
                name: "read".into(),
                description: "d".into(),
                parameters: json!({"type":"object"}),
            }],
            temperature: 0.9,
            max_output_tokens: 256,
            delta_cb: None,
        };
        let body = p.build_body(&req, true);
        assert_eq!(body["model"], "m");
        assert_eq!(body["stream"], json!(true));
        assert!(body.get("stream_options").is_some());
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[2]["role"], "assistant");
        assert!(
            msgs[2]["content"].is_null(),
            "空文本 assistant 必须 content:null"
        );
        assert_eq!(msgs[2]["tool_calls"][0]["function"]["name"], "read");
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(body["tools"][0]["function"]["name"], "read");
        // 非流式不发 stream_options
        let b2 = p.build_body(&ChatRequest::simple(vec![Msg::User("hi".into())]), false);
        assert!(b2.get("stream_options").is_none());
        assert!(b2.get("tools").is_none());
    }

    #[test]
    fn each_client_is_independent() {
        let a = ProviderClient::new("A", "https://api.x.com", "key-a", "m1");
        let b = ProviderClient::new("B", "https://api.y.com", "key-b", "m2");
        assert_ne!(a.key_fingerprint(), b.key_fingerprint());
        assert_eq!(a.label(), "m1@api.x.com");
        assert_eq!(b.label(), "m2@api.y.com");
    }

    #[tokio::test]
    async fn scripted_serves_queue_then_silence() {
        let sp = ScriptedProvider::new(
            "A",
            "s",
            "k",
            vec![Ok(ChatResult {
                text: "甲:你好,我是 DeepSeek 的模型。".into(),
                tool_calls: vec![],
                usage: UsagePair {
                    input: 1,
                    output: 1,
                    cached: None,
                },
                finish_reason: "stop".into(),
            })],
        );
        let got = Arc::new(Mutex::new(String::new()));
        let g = got.clone();
        let req = ChatRequest {
            messages: vec![Msg::User("hi".into())],
            tools: vec![],
            temperature: 0.5,
            max_output_tokens: 8,
            delta_cb: Some(Arc::new(move |s: String| {
                *g.lock().unwrap() += &s;
            })),
        };
        let r = sp.chat(req).await.unwrap();
        assert_eq!(r.text, "甲:你好,我是 DeepSeek 的模型。");
        assert_eq!(*got.lock().unwrap(), r.text, "流式增量拼起来应等于全文");
        let r2 = sp.chat(ChatRequest::simple(vec![])).await.unwrap();
        assert!(r2.text.is_empty() && r2.tool_calls.is_empty());
        assert_eq!(sp.call_count(), 2);
    }

    #[test]
    fn error_classification() {
        assert!(ProviderError::Transient("x".into()).retryable());
        assert!(!ProviderError::Fatal("x".into()).retryable());
        assert_eq!(
            ProviderError::Fatal("bad key".to_string()).message(),
            "bad key"
        );
    }
}
