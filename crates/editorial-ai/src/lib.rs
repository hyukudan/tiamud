//! Optional editorial AI adapters, deliberately absent from Tiamud's server.

use std::fmt;
use std::io::Read;
use std::time::Duration;

use reqwest::Url;
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

pub const MAX_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
pub const MAX_OUTPUT_CHARS: usize = 16 * 1024;
pub const MAX_TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorialTask {
    DraftDescription,
    SuggestAltText,
    ReviewConsistency,
    FindSuspiciousReferences,
    BrainstormNpc,
    BrainstormQuest,
}

#[derive(Clone, Eq, PartialEq)]
pub enum ProviderConfig {
    None,
    Ollama {
        endpoint: String,
        model: String,
    },
    LlamaCpp {
        endpoint: String,
        model: String,
    },
    Compatible {
        endpoint: String,
        model: String,
        authorization: Option<String>,
    },
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::Ollama { endpoint, model } => formatter
                .debug_struct("Ollama")
                .field("endpoint", endpoint)
                .field("model", model)
                .finish(),
            Self::LlamaCpp { endpoint, model } => formatter
                .debug_struct("LlamaCpp")
                .field("endpoint", endpoint)
                .field("model", model)
                .finish(),
            Self::Compatible {
                endpoint, model, ..
            } => formatter
                .debug_struct("Compatible")
                .field("endpoint", endpoint)
                .field("model", model)
                .field("authorization", &"[redacted]")
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorialRequest {
    pub task: EditorialTask,
    pub input: String,
    pub context: Option<String>,
    pub max_output_chars: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanReview {
    pub reviewer: String,
    pub reviewed_at: String,
    pub accepted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorialSuggestion {
    pub provider: String,
    pub model: String,
    pub task: EditorialTask,
    pub text: String,
    pub provenance: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_review: Option<HumanReview>,
}

impl EditorialSuggestion {
    pub fn review(
        mut self,
        reviewer: &str,
        reviewed_at: &str,
        accepted: bool,
    ) -> Result<Self, EditorialError> {
        self.human_review = Some(HumanReview {
            reviewer: bounded_text(reviewer, "reviewer", 1024)?,
            reviewed_at: bounded_text(reviewed_at, "reviewed_at", 1024)?,
            accepted,
        });
        Ok(self)
    }

    pub fn is_publishable(&self) -> bool {
        self.human_review
            .as_ref()
            .is_some_and(|review| review.accepted)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AdapterLimits {
    pub timeout: Duration,
    pub connect_timeout: Duration,
    pub max_response_bytes: usize,
}

impl Default for AdapterLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(3),
            max_response_bytes: MAX_RESPONSE_BYTES,
        }
    }
}

#[derive(Debug, Error)]
pub enum EditorialError {
    #[error("editorial AI is disabled")]
    Disabled,
    #[error("invalid provider configuration: {0}")]
    InvalidProvider(String),
    #[error("editorial request exceeds its bounds")]
    InvalidRequest,
    #[error("provider request failed: {0}")]
    Transport(String),
    #[error("provider returned HTTP {0}")]
    Http(u16),
    #[error("provider response exceeded its byte limit")]
    ResponseTooLarge,
    #[error("provider response did not contain bounded suggestion text")]
    InvalidResponse,
    #[error("invalid review field {0}")]
    InvalidReview(&'static str),
}

pub struct EditorialAdapter {
    config: ProviderConfig,
    endpoint: Option<Url>,
    client: Option<Client>,
    limits: AdapterLimits,
}

impl EditorialAdapter {
    pub fn new(config: ProviderConfig, limits: AdapterLimits) -> Result<Self, EditorialError> {
        if limits.timeout.is_zero()
            || limits.timeout > MAX_TIMEOUT
            || limits.connect_timeout.is_zero()
            || limits.connect_timeout > MAX_CONNECT_TIMEOUT
            || limits.max_response_bytes == 0
            || limits.max_response_bytes > MAX_RESPONSE_BYTES
        {
            return Err(EditorialError::InvalidProvider(
                "adapter limits must be non-zero and bounded".to_owned(),
            ));
        }
        if matches!(config, ProviderConfig::None) {
            return Ok(Self {
                config,
                endpoint: None,
                client: None,
                limits,
            });
        }
        let (endpoint, local_only, model) = match &config {
            ProviderConfig::None => unreachable!(),
            ProviderConfig::Ollama { endpoint, model }
            | ProviderConfig::LlamaCpp { endpoint, model } => (endpoint, true, model),
            ProviderConfig::Compatible {
                endpoint, model, ..
            } => (endpoint, false, model),
        };
        validate_model(model)?;
        if let ProviderConfig::Compatible {
            authorization: Some(value),
            ..
        } = &config
        {
            reqwest::header::HeaderValue::from_str(value).map_err(|_| {
                EditorialError::InvalidProvider("invalid authorization header".to_owned())
            })?;
        }
        let endpoint = validate_endpoint(endpoint, local_only)?;
        let client = Client::builder()
            .timeout(limits.timeout)
            .connect_timeout(limits.connect_timeout)
            .redirect(Policy::none())
            .no_proxy()
            .build()
            .map_err(|error| EditorialError::InvalidProvider(error.to_string()))?;
        Ok(Self {
            config,
            endpoint: Some(endpoint),
            client: Some(client),
            limits,
        })
    }

    pub fn suggest(
        &self,
        request: &EditorialRequest,
    ) -> Result<EditorialSuggestion, EditorialError> {
        validate_request(request)?;
        let endpoint = self.endpoint.as_ref().ok_or(EditorialError::Disabled)?;
        let client = self.client.as_ref().ok_or(EditorialError::Disabled)?;
        let prompt = prompt(request);
        let (provider, model, url, body, authorization) = match &self.config {
            ProviderConfig::None => return Err(EditorialError::Disabled),
            ProviderConfig::Ollama { model, .. } => (
                "ollama",
                model,
                endpoint.join("api/generate"),
                json!({ "model": model, "prompt": prompt, "stream": false }),
                None,
            ),
            ProviderConfig::LlamaCpp { model, .. } => (
                "llama.cpp",
                model,
                endpoint.join("v1/chat/completions"),
                openai_body(model, &prompt),
                None,
            ),
            ProviderConfig::Compatible {
                model,
                authorization,
                ..
            } => (
                "compatible",
                model,
                endpoint.join("v1/chat/completions"),
                openai_body(model, &prompt),
                authorization.as_deref(),
            ),
        };
        let url = url.map_err(|error| EditorialError::InvalidProvider(error.to_string()))?;
        let mut builder = client.post(url).json(&body);
        if let Some(value) = authorization {
            builder = builder.header("authorization", value);
        }
        let response = builder
            .send()
            .map_err(|error| EditorialError::Transport(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(EditorialError::Http(status.as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > self.limits.max_response_bytes as u64)
        {
            return Err(EditorialError::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        response
            .take(self.limits.max_response_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| EditorialError::Transport(error.to_string()))?;
        if bytes.len() > self.limits.max_response_bytes {
            return Err(EditorialError::ResponseTooLarge);
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| EditorialError::InvalidResponse)?;
        let text = if provider == "ollama" {
            value.get("response").and_then(Value::as_str)
        } else {
            value
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
        }
        .ok_or(EditorialError::InvalidResponse)?
        .trim();
        if text.is_empty() || text.chars().count() > request.max_output_chars {
            return Err(EditorialError::InvalidResponse);
        }
        Ok(EditorialSuggestion {
            provider: provider.to_owned(),
            model: model.to_owned(),
            task: request.task,
            text: text.to_owned(),
            provenance: format!(
                "AI-assisted editorial draft from {provider}/{model}; human review required"
            ),
            human_review: None,
        })
    }
}

fn validate_endpoint(value: &str, local_only: bool) -> Result<Url, EditorialError> {
    let mut url =
        Url::parse(value).map_err(|error| EditorialError::InvalidProvider(error.to_string()))?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(EditorialError::InvalidProvider(
            "endpoint must not contain credentials, query, or fragment".to_owned(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| EditorialError::InvalidProvider("endpoint host is required".to_owned()))?;
    let loopback = host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || (local_only && !loopback)
    {
        return Err(EditorialError::InvalidProvider(
            "use HTTPS, or HTTP only on loopback; local providers require loopback".to_owned(),
        ));
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

fn validate_model(model: &str) -> Result<(), EditorialError> {
    if model.trim().is_empty()
        || model.len() > 256
        || model.chars().any(|character| character.is_control())
    {
        return Err(EditorialError::InvalidProvider(
            "invalid model name".to_owned(),
        ));
    }
    Ok(())
}

fn validate_request(request: &EditorialRequest) -> Result<(), EditorialError> {
    if request.input.trim().is_empty()
        || request.input.len() > MAX_INPUT_BYTES
        || request
            .context
            .as_ref()
            .is_some_and(|value| value.len() > MAX_INPUT_BYTES)
        || request.max_output_chars == 0
        || request.max_output_chars > MAX_OUTPUT_CHARS
    {
        return Err(EditorialError::InvalidRequest);
    }
    Ok(())
}

fn prompt(request: &EditorialRequest) -> String {
    format!(
        "Tiamud editorial task: {:?}. Return only a concise draft, never executable code.\nContext: {}\nInput:\n{}",
        request.task,
        request.context.as_deref().unwrap_or("none"),
        request.input
    )
}

fn openai_body(model: &str, prompt: &str) -> Value {
    json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt }],
        "stream": false
    })
}

fn bounded_text(
    value: &str,
    field: &'static str,
    maximum: usize,
) -> Result<String, EditorialError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > maximum
        || value.chars().any(|character| character.is_control())
    {
        return Err(EditorialError::InvalidReview(field));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_is_the_default_safe_failure_and_never_builds_a_client() {
        let adapter =
            EditorialAdapter::new(ProviderConfig::None, AdapterLimits::default()).unwrap();
        let request = EditorialRequest {
            task: EditorialTask::SuggestAltText,
            input: "A lantern square.".to_owned(),
            context: None,
            max_output_chars: 200,
        };
        assert!(matches!(
            adapter.suggest(&request),
            Err(EditorialError::Disabled)
        ));
    }

    #[test]
    fn endpoints_are_fail_closed_and_secrets_are_redacted() {
        assert!(
            EditorialAdapter::new(
                ProviderConfig::Ollama {
                    endpoint: "http://example.com:11434".to_owned(),
                    model: "small".to_owned(),
                },
                AdapterLimits::default(),
            )
            .is_err()
        );
        assert!(
            EditorialAdapter::new(
                ProviderConfig::Compatible {
                    endpoint: "https://models.example/v1/../".to_owned(),
                    model: "small".to_owned(),
                    authorization: Some("Bearer secret".to_owned()),
                },
                AdapterLimits::default(),
            )
            .is_ok()
        );
        let debug = format!(
            "{:?}",
            ProviderConfig::Compatible {
                endpoint: "https://models.example/".to_owned(),
                model: "small".to_owned(),
                authorization: Some("Bearer secret".to_owned()),
            }
        );
        assert!(!debug.contains("Bearer secret"));
    }

    #[test]
    fn suggestions_require_explicit_positive_human_review() {
        let suggestion = EditorialSuggestion {
            provider: "fixture".to_owned(),
            model: "none".to_owned(),
            task: EditorialTask::DraftDescription,
            text: "Rain crosses the stones.".to_owned(),
            provenance: "fixture".to_owned(),
            human_review: None,
        };
        assert!(!suggestion.is_publishable());
        let rejected = suggestion
            .clone()
            .review("Editor", "2026-07-15T00:00:00Z", false)
            .unwrap();
        assert!(!rejected.is_publishable());
        let accepted = suggestion
            .review("Editor", "2026-07-15T00:00:00Z", true)
            .unwrap();
        assert!(accepted.is_publishable());
    }

    #[test]
    fn request_and_review_fields_are_bounded() {
        let request = EditorialRequest {
            task: EditorialTask::BrainstormQuest,
            input: "x".repeat(MAX_INPUT_BYTES + 1),
            context: None,
            max_output_chars: 20,
        };
        assert!(matches!(
            validate_request(&request),
            Err(EditorialError::InvalidRequest)
        ));
        let suggestion = EditorialSuggestion {
            provider: "fixture".to_owned(),
            model: "none".to_owned(),
            task: EditorialTask::BrainstormQuest,
            text: "draft".to_owned(),
            provenance: "fixture".to_owned(),
            human_review: None,
        };
        assert!(suggestion.review("\n", "date", true).is_err());
    }

    #[test]
    fn adapter_timeouts_cannot_exceed_authoring_bounds() {
        let excessive_request_timeout = AdapterLimits {
            timeout: MAX_TIMEOUT + Duration::from_secs(1),
            ..AdapterLimits::default()
        };
        assert!(EditorialAdapter::new(ProviderConfig::None, excessive_request_timeout).is_err());
        assert!(matches!(
            EditorialAdapter::new(
                ProviderConfig::Ollama {
                    endpoint: "http://127.0.0.1:11434/".to_owned(),
                    model: "fixture".to_owned(),
                },
                excessive_request_timeout,
            ),
            Err(EditorialError::InvalidProvider(_))
        ));

        let excessive_connect_timeout = AdapterLimits {
            connect_timeout: MAX_CONNECT_TIMEOUT + Duration::from_secs(1),
            ..AdapterLimits::default()
        };
        assert!(matches!(
            EditorialAdapter::new(
                ProviderConfig::Compatible {
                    endpoint: "https://models.example/".to_owned(),
                    model: "fixture".to_owned(),
                    authorization: None,
                },
                excessive_connect_timeout,
            ),
            Err(EditorialError::InvalidProvider(_))
        ));
    }
}
