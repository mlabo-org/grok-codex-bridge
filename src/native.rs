use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::Path;

use axum::http::header::{CONNECTION, CONTENT_LENGTH, HOST, TRANSFER_ENCODING};
use axum::http::{HeaderMap, HeaderName};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::lifecycle::PICKER_CALLER_HEADER;

const ROUTE_STATE_VERSION: u32 = 1;
const MAX_ROUTE_STATE_BYTES: u64 = 1024 * 1024;
const CHATGPT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex/";
const OPENAI_API_BASE_URL: &str = "https://api.openai.com/v1/";

/// The two first-party Responses origins selected by Codex 0.148 according to
/// its active authentication mode. Lifecycle input must capture which one was
/// effective; the bridge never guesses from credentials.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeUpstream {
    ChatgptCodex,
    OpenaiApi,
}

impl NativeUpstream {
    pub fn parse_base_url(value: &str) -> Result<Self, NativeError> {
        let parsed = Url::parse(value).map_err(|_| NativeError::UnsupportedUpstream)?;
        let canonical = parsed.as_str().trim_end_matches('/');
        match canonical {
            "https://chatgpt.com/backend-api/codex" => Ok(Self::ChatgptCodex),
            "https://api.openai.com/v1" => Ok(Self::OpenaiApi),
            _ => Err(NativeError::UnsupportedUpstream),
        }
    }

    #[must_use]
    pub fn base_url(self) -> &'static str {
        match self {
            Self::ChatgptCodex => CHATGPT_CODEX_BASE_URL,
            Self::OpenaiApi => OPENAI_API_BASE_URL,
        }
    }
}

/// Non-secret routing state published atomically with the merged picker
/// catalog. Only exact native slugs are admitted; missing or colliding rows are
/// rejected by the request router rather than inferred by prefix.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRouteState {
    version: u32,
    upstream: NativeUpstream,
    native_models: Vec<String>,
    #[serde(default)]
    mode: NativeRouteMode,
}

/// Determines whether the bridge may route admitted Grok slugs to Native
/// during a compatibility-only runtime. The default preserves the original
/// picker behavior for route-state files written before this field existed.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NativeRouteMode {
    #[default]
    GrokEnabled,
    NativeCompatibility {
        fallback_model: String,
    },
}

impl NativeRouteState {
    pub fn new(
        upstream: NativeUpstream,
        native_models: impl IntoIterator<Item = String>,
    ) -> Result<Self, NativeError> {
        let mut native_models = native_models.into_iter().collect::<Vec<_>>();
        native_models.sort_unstable();
        let state = Self {
            version: ROUTE_STATE_VERSION,
            upstream,
            native_models,
            mode: NativeRouteMode::GrokEnabled,
        };
        state.validate()?;
        Ok(state)
    }

    pub fn new_native_compatibility(
        upstream: NativeUpstream,
        native_models: impl IntoIterator<Item = String>,
        fallback_model: String,
    ) -> Result<Self, NativeError> {
        let mut native_models = native_models.into_iter().collect::<Vec<_>>();
        native_models.sort_unstable();
        let state = Self {
            version: ROUTE_STATE_VERSION,
            upstream,
            native_models,
            mode: NativeRouteMode::NativeCompatibility { fallback_model },
        };
        state.validate()?;
        Ok(state)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, NativeError> {
        let state: Self = serde_json::from_slice(bytes).map_err(NativeError::ParseState)?;
        state.validate()?;
        Ok(state)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, NativeError> {
        self.validate()?;
        serde_json::to_vec_pretty(self).map_err(NativeError::SerializeState)
    }

    pub fn load_if_present(path: &Path) -> Result<Option<Self>, NativeError> {
        let mut file = match open_read_only(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(NativeError::ReadState(error)),
        };
        let metadata = file.metadata().map_err(NativeError::ReadState)?;
        if !metadata.is_file() || metadata.len() > MAX_ROUTE_STATE_BYTES {
            return Err(NativeError::UnsafeState);
        }
        validate_private_permissions(&metadata)?;
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
        file.read_to_end(&mut bytes)
            .map_err(NativeError::ReadState)?;
        Self::from_json(&bytes).map(Some)
    }

    #[must_use]
    pub fn contains(&self, model: &str) -> bool {
        self.native_models
            .binary_search_by(|candidate| candidate.as_str().cmp(model))
            .is_ok()
    }

    #[must_use]
    pub fn upstream(&self) -> NativeUpstream {
        self.upstream
    }

    #[must_use]
    pub fn native_compatibility_fallback(&self) -> Option<&str> {
        match &self.mode {
            NativeRouteMode::GrokEnabled => None,
            NativeRouteMode::NativeCompatibility { fallback_model } => Some(fallback_model),
        }
    }

    fn validate(&self) -> Result<(), NativeError> {
        if self.version != ROUTE_STATE_VERSION || self.native_models.is_empty() {
            return Err(NativeError::InvalidState);
        }
        let mut seen = HashSet::with_capacity(self.native_models.len());
        for model in &self.native_models {
            if model.is_empty()
                || model.len() > 128
                || !model.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
                })
                || !seen.insert(model)
            {
                return Err(NativeError::InvalidState);
            }
        }
        if self.native_models.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(NativeError::InvalidState);
        }
        if let NativeRouteMode::NativeCompatibility { fallback_model } = &self.mode {
            if !self.contains(fallback_model) {
                return Err(NativeError::InvalidState);
            }
        }
        Ok(())
    }
}

/// Narrow reverse transport for first-party Codex Responses traffic. It never
/// receives Grok credentials and does no JSON or SSE transformation.
#[derive(Clone)]
pub(crate) struct NativeClient {
    client: reqwest::Client,
    base_url: Url,
}

impl NativeClient {
    pub(crate) fn production(upstream: NativeUpstream) -> Result<Self, NativeError> {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .https_only(true)
            .build()
            .map_err(NativeError::ConstructClient)?;
        Ok(Self {
            client,
            base_url: Url::parse(upstream.base_url())
                .expect("authoritative Native upstream URL must parse"),
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(base_url: Url) -> Result<Self, NativeError> {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .map_err(NativeError::ConstructClient)?;
        Ok(Self { client, base_url })
    }

    pub(crate) async fn post(
        &self,
        suffix: NativeApiPath,
        incoming_headers: &HeaderMap,
        body: reqwest::Body,
    ) -> Result<reqwest::Response, NativeError> {
        let url = self
            .base_url
            .join(suffix.relative_path())
            .map_err(|_| NativeError::UnsupportedUpstream)?;
        let mut headers = HeaderMap::new();
        for (name, value) in incoming_headers {
            if !is_hop_by_hop_request_header(name) && name.as_str() != PICKER_CALLER_HEADER {
                headers.append(name.clone(), value.clone());
            }
        }
        self.client
            .post(url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(NativeError::Transport)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeApiPath {
    Responses,
    Compact,
    ImagesGenerations,
    ImagesEdits,
    AlphaSearch,
}

impl NativeApiPath {
    fn relative_path(self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::Compact => "responses/compact",
            Self::ImagesGenerations => "images/generations",
            Self::ImagesEdits => "images/edits",
            Self::AlphaSearch => "alpha/search",
        }
    }
}

pub(crate) fn is_hop_by_hop_response_header(name: &HeaderName) -> bool {
    name == CONNECTION || name == TRANSFER_ENCODING
}

fn is_hop_by_hop_request_header(name: &HeaderName) -> bool {
    name == HOST || name == CONTENT_LENGTH || name == CONNECTION || name == TRANSFER_ENCODING
}

#[cfg(unix)]
fn open_read_only(path: &Path) -> Result<fs::File, std::io::Error> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_read_only(path: &Path) -> Result<fs::File, std::io::Error> {
    fs::OpenOptions::new().read(true).open(path)
}

#[cfg(unix)]
fn validate_private_permissions(metadata: &fs::Metadata) -> Result<(), NativeError> {
    use std::os::unix::fs::PermissionsExt;

    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(NativeError::UnsafeState);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_permissions(_metadata: &fs::Metadata) -> Result<(), NativeError> {
    Err(NativeError::UnsupportedPlatform)
}

#[derive(Debug, Error)]
pub enum NativeError {
    #[error("Native upstream must be the exact Codex 0.148 first-party Responses base URL")]
    UnsupportedUpstream,
    #[error("Native route state is invalid")]
    InvalidState,
    #[error("Native route state is malformed")]
    ParseState(#[source] serde_json::Error),
    #[error("failed to serialize Native route state")]
    SerializeState(#[source] serde_json::Error),
    #[error("failed to read Native route state")]
    ReadState(#[source] std::io::Error),
    #[error("Native route state is not a private regular file")]
    UnsafeState,
    #[error("failed to construct Native upstream client")]
    ConstructClient(#[source] reqwest::Error),
    #[error("Native upstream request failed")]
    Transport(#[source] reqwest::Error),
    #[cfg(not(unix))]
    #[error("Native route-state permissions are unsupported on this platform")]
    UnsupportedPlatform,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_state_requires_exact_sorted_unique_native_models() {
        let state = NativeRouteState::new(
            NativeUpstream::ChatgptCodex,
            ["gpt-5.6-sol".to_owned(), "gpt-5.4".to_owned()],
        )
        .unwrap();
        assert!(state.contains("gpt-5.6-sol"));
        assert!(!state.contains("gpt-5.6"));
        assert_eq!(
            NativeRouteState::from_json(&state.to_json().unwrap()).unwrap(),
            state
        );

        assert!(
            NativeRouteState::new(
                NativeUpstream::OpenaiApi,
                ["gpt-5.4".to_owned(), "gpt-5.4".to_owned()]
            )
            .is_err()
        );
    }

    #[test]
    fn native_upstream_accepts_only_codex_owned_first_party_bases() {
        assert_eq!(
            NativeUpstream::parse_base_url("https://chatgpt.com/backend-api/codex").unwrap(),
            NativeUpstream::ChatgptCodex
        );
        assert_eq!(
            NativeUpstream::parse_base_url("https://api.openai.com/v1/").unwrap(),
            NativeUpstream::OpenaiApi
        );
        assert!(NativeUpstream::parse_base_url("https://example.com/v1").is_err());
    }

    #[test]
    fn native_compatibility_route_requires_and_serializes_a_native_fallback() {
        let state = NativeRouteState::new_native_compatibility(
            NativeUpstream::ChatgptCodex,
            ["gpt-native".to_owned()],
            "gpt-native".to_owned(),
        )
        .unwrap();
        assert_eq!(state.native_compatibility_fallback(), Some("gpt-native"));
        assert_eq!(
            NativeRouteState::from_json(&state.to_json().unwrap()).unwrap(),
            state
        );
        assert!(
            NativeRouteState::new_native_compatibility(
                NativeUpstream::ChatgptCodex,
                ["gpt-native".to_owned()],
                "missing-native".to_owned(),
            )
            .is_err()
        );
    }
}
