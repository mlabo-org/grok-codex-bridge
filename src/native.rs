use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

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
pub(crate) const NATIVE_SEND_RETRY_LIMIT: usize = 3;
pub(crate) const NATIVE_RETRY_WALL_CLOCK: Duration = Duration::from_secs(60);
pub(crate) const NATIVE_RETRY_BACKOFF: [Duration; NATIVE_SEND_RETRY_LIMIT] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

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
        body: Vec<u8>,
    ) -> Result<reqwest::Response, NativeError> {
        let mut attempts = 0;
        self.post_with_started(
            suffix,
            incoming_headers,
            body,
            Instant::now(),
            &mut attempts,
        )
        .await
    }

    pub(crate) async fn post_with_started(
        &self,
        suffix: NativeApiPath,
        incoming_headers: &HeaderMap,
        body: Vec<u8>,
        started: Instant,
        attempts: &mut usize,
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
        loop {
            let attempt = *attempts;
            *attempts += 1;
            let Some(remaining) = NATIVE_RETRY_WALL_CLOCK.checked_sub(started.elapsed()) else {
                return Err(NativeError::DeadlineExceeded);
            };
            let result = tokio::time::timeout(
                remaining,
                self.client
                    .post(url.clone())
                    .headers(headers.clone())
                    .body(body.clone())
                    .send(),
            )
            .await;
            let result = match result {
                Ok(result) => result,
                Err(_) => return Err(NativeError::DeadlineExceeded),
            };
            match result {
                Ok(response)
                    if is_transient_status(response.status())
                        && attempt < NATIVE_SEND_RETRY_LIMIT =>
                {
                    let backoff =
                        retry_after_or_default(response.headers(), NATIVE_RETRY_BACKOFF[attempt]);
                    tracing::debug!(
                        route = suffix.as_str(),
                        status = response.status().as_u16(),
                        attempt = attempt + 1,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "Native upstream returned a transient status; retrying"
                    );
                    if started.elapsed().saturating_add(backoff) > NATIVE_RETRY_WALL_CLOCK {
                        tracing::warn!(
                            route = suffix.as_str(),
                            status = response.status().as_u16(),
                            error_class = "upstream_http_status",
                            attempt = attempt + 1,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "Native upstream remained unavailable after bounded recovery"
                        );
                        return Ok(response);
                    }
                    tokio::time::sleep(backoff).await;
                }
                Ok(response) => {
                    if is_transient_status(response.status()) {
                        tracing::warn!(
                            route = suffix.as_str(),
                            status = response.status().as_u16(),
                            error_class = "upstream_http_status",
                            attempt = attempt + 1,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "Native upstream remained unavailable after bounded recovery"
                        );
                    }
                    return Ok(response);
                }
                Err(error) => {
                    let error_class = native_transport_error_class(&error);
                    // With a replayable in-memory body, reqwest reports a peer
                    // closing before response headers as either connect,
                    // timeout, or request cancellation.
                    let retry = error.is_connect() || error.is_timeout() || error.is_request();
                    let Some(backoff) = NATIVE_RETRY_BACKOFF.get(attempt) else {
                        tracing::warn!(
                            route = suffix.as_str(),
                            error_class,
                            attempt = attempt + 1,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "Native upstream send failed; retry limit reached"
                        );
                        return Err(NativeError::Transport(error));
                    };
                    if !retry
                        || started.elapsed().saturating_add(*backoff) > NATIVE_RETRY_WALL_CLOCK
                    {
                        tracing::warn!(
                            route = suffix.as_str(),
                            error_class,
                            attempt = attempt + 1,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "Native upstream send failed; not retrying"
                        );
                        return Err(NativeError::Transport(error));
                    }
                    tracing::debug!(
                        route = suffix.as_str(),
                        error_class,
                        attempt = attempt + 1,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "Native upstream send failed; retrying"
                    );
                    tokio::time::sleep(*backoff).await;
                }
            }
        }
    }

    pub(crate) async fn post_stream(
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

fn is_transient_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 502..=504)
}

fn retry_after_or_default(headers: &HeaderMap, default: Duration) -> Duration {
    headers
        .get(axum::http::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(default)
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

    pub(crate) fn as_str(self) -> &'static str {
        self.relative_path()
    }
}

fn native_transport_error_class(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_request() {
        "request"
    } else {
        "transport"
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
    #[error("Native upstream retry deadline exceeded")]
    DeadlineExceeded,
    #[cfg(not(unix))]
    #[error("Native route-state permissions are unsupported on this platform")]
    UnsupportedPlatform,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

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

    #[tokio::test]
    async fn native_send_retries_connect_failures_and_reuses_the_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let server_hits = Arc::clone(&hits);
        let server = tokio::spawn(async move {
            for attempt in 0..=NATIVE_SEND_RETRY_LIMIT {
                let (mut stream, _) = listener.accept().await.unwrap();
                server_hits.fetch_add(1, Ordering::SeqCst);
                assert_eq!(read_request_body(&mut stream).await, b"same-body");
                if attempt == NATIVE_SEND_RETRY_LIMIT {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .unwrap();
                }
            }
        });
        let client = NativeClient::for_test(
            Url::parse(&format!("http://{address}/backend-api/codex/")).unwrap(),
        )
        .unwrap();
        let response = client
            .post(
                NativeApiPath::Responses,
                &HeaderMap::new(),
                b"same-body".to_vec(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), NATIVE_SEND_RETRY_LIMIT + 1);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn native_send_caps_continuous_connect_failures() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let client = NativeClient::for_test(
            Url::parse(&format!("http://{address}/backend-api/codex/")).unwrap(),
        )
        .unwrap();
        let started = Instant::now();
        let error = client
            .post(
                NativeApiPath::Responses,
                &HeaderMap::new(),
                b"same-body".to_vec(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, NativeError::Transport(_)));
        assert!(started.elapsed() <= Duration::from_secs(8));
    }

    #[tokio::test]
    async fn native_send_does_not_start_new_attempt_after_retry_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let accepted = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_ok()
        });
        let client = NativeClient::for_test(
            Url::parse(&format!("http://{address}/backend-api/codex/")).unwrap(),
        )
        .unwrap();
        let mut attempts = NATIVE_SEND_RETRY_LIMIT;
        let started = Instant::now() - NATIVE_RETRY_WALL_CLOCK;
        let error = client
            .post_with_started(
                NativeApiPath::Responses,
                &HeaderMap::new(),
                b"same-body".to_vec(),
                started,
                &mut attempts,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, NativeError::DeadlineExceeded));
        assert!(!accepted.await.unwrap());
    }

    #[test]
    fn native_retry_policy_uses_three_retries_and_one_two_four_second_backoff() {
        assert_eq!(NATIVE_SEND_RETRY_LIMIT, 3);
        assert_eq!(
            NATIVE_RETRY_BACKOFF,
            [
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ]
        );
        assert_eq!(NATIVE_RETRY_WALL_CLOCK, Duration::from_secs(60));
    }

    #[tokio::test]
    async fn native_send_retries_transient_http_status_and_preserves_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let server_hits = Arc::clone(&hits);
        let server = tokio::spawn(async move {
            for attempt in 0..=NATIVE_SEND_RETRY_LIMIT {
                let (mut stream, _) = listener.accept().await.unwrap();
                server_hits.fetch_add(1, Ordering::SeqCst);
                assert_eq!(read_request_body(&mut stream).await, b"same-body");
                let status = if attempt < NATIVE_SEND_RETRY_LIMIT {
                    "503 Service Unavailable"
                } else {
                    "200 OK"
                };
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let client = NativeClient::for_test(
            Url::parse(&format!("http://{address}/backend-api/codex/")).unwrap(),
        )
        .unwrap();
        let response = client
            .post(
                NativeApiPath::Responses,
                &HeaderMap::new(),
                b"same-body".to_vec(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), NATIVE_SEND_RETRY_LIMIT + 1);
        server.await.unwrap();
    }

    async fn read_request_body(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        let content_length = std::str::from_utf8(&head)
            .unwrap()
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap();
        let mut body = vec![0; content_length];
        stream.read_exact(&mut body).await.unwrap();
        body
    }
}
