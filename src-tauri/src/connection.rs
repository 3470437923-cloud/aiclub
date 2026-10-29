//! Minimal completion probe, independent of starting a session.
use crate::config::AgentProfile;
use crate::provider::{ChatRequest, Msg, Provider, ProviderClient};
use serde::Serialize;
use std::time::{Duration, Instant};

#[derive(Debug, Serialize)]
pub struct ConnectionCheck {
    pub model: String,
    pub ms: u64,
    pub text: String,
}

#[tauri::command]
pub async fn test_connection(profile: AgentProfile) -> Result<ConnectionCheck, String> {
    let p = ProviderClient::new(
        "connection-check",
        profile.base_url.trim(),
        profile.api_key.trim(),
        profile.model.trim(),
    );
    check_with(&profile, &p).await
}

async fn check_with(
    profile: &AgentProfile,
    provider: &dyn Provider,
) -> Result<ConnectionCheck, String> {
    if profile.base_url.trim().is_empty()
        || profile.api_key.trim().is_empty()
        || profile.model.trim().is_empty()
    {
        return Err("请先填写接口地址、API Key 和模型".into());
    }
    let url = reqwest::Url::parse(profile.base_url.trim()).map_err(|_| "接口地址无效")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("接口地址必须是有效的 HTTP(S) 地址，且不能包含凭证".into());
    }
    let start = Instant::now();
    let mut req = ChatRequest::simple(vec![Msg::User("Reply with exactly: OK".into())]);
    req.temperature = 0.0;
    req.max_output_tokens = 8;
    let response = tokio::time::timeout(Duration::from_secs(20), provider.chat(req))
        .await
        .map_err(|_| "连接测试超过 20 秒，请检查网络或模型状态".to_string())?
        .map_err(|e| e.message().replace(profile.api_key.trim(), "[已隐藏]"))?;
    if response.text.trim().is_empty() {
        return Err("接口已响应，但没有返回对话正文；请检查模型是否支持对话".into());
    }
    Ok(ConnectionCheck {
        model: profile.model.trim().into(),
        ms: start.elapsed().as_millis() as u64,
        text: response
            .text
            .replace(profile.api_key.trim(), "[已隐藏]")
            .chars()
            .take(80)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::UsagePair;
    use crate::provider::{BoxFuture, ChatResult, ProviderError};
    struct Probe {
        empty: bool,
    }
    impl Provider for Probe {
        fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<ChatResult, ProviderError>> {
            assert!(req.tools.is_empty());
            assert_eq!(req.max_output_tokens, 8);
            Box::pin(async move {
                Ok(ChatResult {
                    text: if self.empty { "" } else { "OK" }.into(),
                    tool_calls: vec![],
                    usage: UsagePair::default(),
                    finish_reason: "stop".into(),
                })
            })
        }
        fn label(&self) -> String {
            "probe".into()
        }
        fn key_fingerprint(&self) -> String {
            "probe".into()
        }
        fn call_count(&self) -> usize {
            0
        }
    }
    fn profile() -> AgentProfile {
        AgentProfile {
            base_url: "https://example.com/v1".into(),
            api_key: "test-key".into(),
            model: "test-model".into(),
            ..Default::default()
        }
    }
    #[tokio::test]
    async fn selected_model_can_be_checked_without_session() {
        let result = check_with(&profile(), &Probe { empty: false })
            .await
            .unwrap();
        assert_eq!(result.model, "test-model");
        assert_eq!(result.text, "OK");
    }
    #[tokio::test]
    async fn rejects_empty_response_and_incomplete_profile() {
        assert!(check_with(&profile(), &Probe { empty: true })
            .await
            .is_err());
        assert!(
            check_with(&AgentProfile::default(), &Probe { empty: false })
                .await
                .is_err()
        );
    }

    struct ResponseProbe {
        result: Result<ChatResult, ProviderError>,
    }
    impl Provider for ResponseProbe {
        fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<ChatResult, ProviderError>> {
            assert!(req.tools.is_empty());
            let result = self.result.clone();
            Box::pin(async move { result })
        }
        fn label(&self) -> String {
            "probe".into()
        }
        fn key_fingerprint(&self) -> String {
            "probe".into()
        }
        fn call_count(&self) -> usize {
            0
        }
    }
    #[tokio::test]
    async fn authentication_errors_and_echoes_never_expose_key() {
        let p = profile();
        let error = ResponseProbe {
            result: Err(ProviderError::Fatal(format!("401 invalid {}", p.api_key))),
        };
        let message = check_with(&p, &error).await.unwrap_err();
        assert!(message.contains("401"));
        assert!(!message.contains(&p.api_key));
        let echo = ResponseProbe {
            result: Ok(ChatResult {
                text: p.api_key.clone(),
                tool_calls: vec![],
                usage: UsagePair::default(),
                finish_reason: "stop".into(),
            }),
        };
        assert!(!check_with(&p, &echo)
            .await
            .unwrap()
            .text
            .contains(&p.api_key));
    }
    #[tokio::test]
    async fn invalid_schemes_and_url_credentials_are_rejected_before_request() {
        for url in [
            "file:///etc/passwd",
            "https://name:password@example.com",
            "not a URL",
        ] {
            let mut p = profile();
            p.base_url = url.into();
            assert!(check_with(&p, &Probe { empty: false }).await.is_err());
        }
    }
}
