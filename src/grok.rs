use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
use reqwest::header::{ACCEPT, CONTENT_TYPE, ETAG, RETRY_AFTER};
use reqwest::{Client, StatusCode};
use serde_json::{Map, Value};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::credential::SessionCredential;
use crate::protocol::{
    NamespaceToolProjection, NormalizedResponsesRequest, ProtocolError, TextStreamValidator,
    ValidatedTextStreamEvent,
};

const OFFICIAL_INFERENCE_BASE: &str = "https://cli-chat-proxy.grok.com/v1/";
const MODELS_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CLIENT_MODE: &str = "headless";
// Match the first-party Grok CLI transport identity.  The bridge remains a
// separate local binary, but cli-chat-proxy gates request compatibility on
// this client family and its lockstepped version.
const CLIENT_IDENTIFIER: &str = "grok-shell";
// `x-grok-client-version` is a cli-chat-proxy compatibility gate, not this
// bridge's package version. Keep the truthful bridge identity in User-Agent
// and `x-grok-client-identifier`; this value tracks xAI's lockstepped
// `xai-grok-version` contract from the admitted Grok Build source snapshot.
const GROK_BUILD_COMPATIBILITY_VERSION: &str = "1.0.5";

#[derive(Clone)]
pub struct GrokClient {
    client: Client,
    base_url: Url,
}

impl GrokClient {
    pub fn production() -> Result<Self, GrokError> {
        let base_url = Url::parse(OFFICIAL_INFERENCE_BASE).expect("official Grok URL is static");
        Self::build(base_url, true)
    }

    fn build(base_url: Url, https_only: bool) -> Result<Self, GrokError> {
        let client = Client::builder()
            .use_rustls_tls()
            .https_only(https_only)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(format!("{CLIENT_IDENTIFIER}/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(GrokError::BuildClient)?;
        Ok(Self { client, base_url })
    }

    #[cfg(test)]
    pub(crate) fn for_test(base_url: Url) -> Result<Self, GrokError> {
        Self::build(base_url, false)
    }

    pub async fn fetch_models(
        &self,
        credential: &SessionCredential,
    ) -> Result<FetchModelsResult, GrokError> {
        let url = self
            .base_url
            .join("models")
            .map_err(GrokError::InvalidEndpoint)?;
        let request = self.authenticated(self.client.get(url), credential);
        let response = tokio::time::timeout(MODELS_TIMEOUT, request.send())
            .await
            .map_err(|_| GrokError::ModelsTimeout)?
            .map_err(GrokError::Transport)?;

        ensure_success(&response)?;
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let document: ModelsDocument = response.json().await.map_err(GrokError::DecodeModels)?;
        let mut models = Vec::with_capacity(document.data.len());
        for value in document.data {
            models.push(admit_model(value)?);
        }
        crate::catalog::validate_model_ids(models.iter().cloned())?;
        Ok(FetchModelsResult { models, etag })
    }

    pub async fn post_responses(
        &self,
        credential: Arc<SessionCredential>,
        request: ResponsesTransportRequest<'_>,
    ) -> Result<ResponsesByteStream, GrokError> {
        let model = request.body.model();
        let body = request.body.to_xai_value();
        let url = self
            .base_url
            .join("responses")
            .map_err(GrokError::InvalidEndpoint)?;
        let builder = self
            .authenticated(self.client.post(url), &credential)
            .header(ACCEPT, "text/event-stream")
            .header("x-grok-conv-id", request.conversation_id.to_string())
            .header("x-grok-req-id", request.request_id.to_string())
            .header("x-grok-session-id", request.conversation_id.to_string())
            .header("x-grok-turn-idx", request.turn_index.to_string())
            .header("x-grok-agent-id", request.agent_id.to_string())
            .header("x-grok-model-override", model)
            .json(&body);
        let response = builder.send().await.map_err(GrokError::Transport)?;
        ensure_success(&response)?;
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
        {
            return Err(GrokError::UnexpectedResponseContentType);
        }

        let stream = response
            .bytes_stream()
            .map(|item| item.map_err(GrokError::Stream));
        Ok(ResponsesByteStream {
            inner: Box::pin(stream),
            _credential: credential,
            namespace_projection: request.body.namespace_projection(),
        })
    }

    fn authenticated(
        &self,
        builder: reqwest::RequestBuilder,
        credential: &SessionCredential,
    ) -> reqwest::RequestBuilder {
        builder
            .bearer_auth(credential.token())
            .header("X-XAI-Token-Auth", "xai-grok-cli")
            .header("x-authenticateresponse", "authenticate-response")
            .header("x-userid", credential.user_id())
            .header("x-grok-user-id", credential.user_id())
            .header("x-grok-client-mode", CLIENT_MODE)
            .header("x-grok-client-identifier", CLIENT_IDENTIFIER)
            .header("x-grok-client-version", GROK_BUILD_COMPATIBILITY_VERSION)
    }
}

pub struct ResponsesTransportRequest<'a> {
    pub body: &'a NormalizedResponsesRequest,
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub agent_id: Uuid,
    pub turn_index: usize,
}

pub struct ResponsesByteStream {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, GrokError>> + Send>>,
    _credential: Arc<SessionCredential>,
    namespace_projection: NamespaceToolProjection,
}

impl Stream for ResponsesByteStream {
    type Item = Result<Bytes, GrokError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(context)
    }
}

impl ResponsesByteStream {
    pub fn validated_text_events(self) -> ValidatedTextEventStream {
        let namespace_projection = self.namespace_projection.clone();
        let data = SseEofBoundaryStream::new(self).eventsource().map(|event| {
            event
                .map(|event| event.data)
                .map_err(|_| GrokError::InvalidSseFraming)
        });
        ValidatedTextEventStream {
            inner: Box::pin(data),
            validator: TextStreamValidator::new(),
            namespace_projection,
            finished: false,
        }
    }
}

/// Terminates a final unterminated SSE block when the upstream closes cleanly.
///
/// Grok can end the response immediately after the final `response.completed`
/// data line. `eventsource-stream` retains that block as incomplete and drops it
/// at EOF, whereas the Grok CLI-compatible parser used by codex-router drains
/// the remaining block. Appending one empty-line boundary preserves the same
/// behavior without treating a genuinely missing terminal event as success.
struct SseEofBoundaryStream<S> {
    inner: S,
    ended: bool,
}

impl<S> SseEofBoundaryStream<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            ended: false,
        }
    }
}

impl<S> Stream for SseEofBoundaryStream<S>
where
    S: Stream<Item = Result<Bytes, GrokError>> + Unpin,
{
    type Item = Result<Bytes, GrokError>;

    fn poll_next(
        self: Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.ended {
            return std::task::Poll::Ready(None);
        }

        match Pin::new(&mut this.inner).poll_next(context) {
            std::task::Poll::Ready(Some(Err(error))) => {
                this.ended = true;
                std::task::Poll::Ready(Some(Err(error)))
            }
            std::task::Poll::Ready(None) => {
                this.ended = true;
                std::task::Poll::Ready(Some(Ok(Bytes::from_static(b"\n\n"))))
            }
            poll => poll,
        }
    }
}

pub struct ValidatedTextEventStream {
    inner: Pin<Box<dyn Stream<Item = Result<String, GrokError>> + Send>>,
    validator: TextStreamValidator,
    namespace_projection: NamespaceToolProjection,
    finished: bool,
}

impl Stream for ValidatedTextEventStream {
    type Item = Result<ValidatedTextStreamEvent, GrokError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.finished {
            return std::task::Poll::Ready(None);
        }

        loop {
            match self.inner.as_mut().poll_next(context) {
                std::task::Poll::Ready(Some(Ok(data))) if data.trim().is_empty() => continue,
                std::task::Poll::Ready(Some(Ok(data))) => match self.validator.accept_data(&data) {
                    Ok(mut event) => {
                        event.restore_namespaced_tool_calls(&self.namespace_projection);
                        return std::task::Poll::Ready(Some(Ok(event)));
                    }
                    Err(error) => {
                        self.finished = true;
                        log_rejected_sse_event(&data, &error);
                        return std::task::Poll::Ready(Some(Err(GrokError::Protocol(error))));
                    }
                },
                std::task::Poll::Ready(Some(Err(error))) => {
                    self.finished = true;
                    return std::task::Poll::Ready(Some(Err(error)));
                }
                std::task::Poll::Ready(None) => match self.validator.finish() {
                    Ok(()) => {
                        if let Some(mut event) = self.validator.synthetic_completed_on_eof() {
                            let response_id = event.original()["response"]["id"].as_str();
                            tracing::warn!(
                                route = "responses",
                                response_id,
                                "synthesizing response.completed after upstream EOF"
                            );
                            event.restore_namespaced_tool_calls(&self.namespace_projection);
                            self.finished = true;
                            return std::task::Poll::Ready(Some(Ok(event)));
                        }
                        self.finished = true;
                        return std::task::Poll::Ready(None);
                    }
                    Err(error) => {
                        self.finished = true;
                        return std::task::Poll::Ready(Some(Err(GrokError::Protocol(error))));
                    }
                },
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}

fn log_rejected_sse_event(data: &str, error: &ProtocolError) {
    let value = serde_json::from_str::<Value>(data).ok();
    let sse_event_type = value
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str);
    let item_type = value
        .as_ref()
        .and_then(|value| value.get("item"))
        .and_then(|item| item.get("type"))
        .and_then(Value::as_str);
    tracing::warn!(
        route = "responses",
        sse_event_type,
        item_type,
        protocol_error = %error,
        "upstream SSE event rejected"
    );
}

pub struct FetchModelsResult {
    pub models: Vec<String>,
    pub etag: Option<String>,
}

#[derive(serde::Deserialize)]
struct ModelsDocument {
    data: Vec<Value>,
}

fn admit_model(value: Value) -> Result<String, GrokError> {
    let object = value.as_object().ok_or(GrokError::InvalidModelEntry)?;
    let meta = object.get("_meta").and_then(Value::as_object);
    let model = string_field(object, "model")
        .or_else(|| string_field(object, "modelId"))
        .or_else(|| string_field(object, "id"))
        .or_else(|| meta.and_then(|fields| string_field(fields, "model")))
        .or_else(|| meta.and_then(|fields| string_field(fields, "modelId")))
        .ok_or(GrokError::InvalidModelEntry)?;
    let backend = string_field(object, "apiBackend")
        .or_else(|| string_field(object, "api_backend"))
        .or_else(|| meta.and_then(|fields| string_field(fields, "apiBackend")))
        .or_else(|| meta.and_then(|fields| string_field(fields, "api_backend")))
        .ok_or(GrokError::UnconfirmedResponsesModel)?;
    if backend != "responses" {
        return Err(GrokError::UnconfirmedResponsesModel);
    }
    if bool_field(object, meta, "hidden", "hidden").unwrap_or(false) {
        return Err(GrokError::HiddenModelEntry);
    }
    if !bool_field(object, meta, "supportedInApi", "supported_in_api").unwrap_or(true) {
        return Err(GrokError::UnsupportedApiModelEntry);
    }

    let base_url = string_field(object, "baseUrl")
        .or_else(|| string_field(object, "base_url"))
        .or_else(|| meta.and_then(|fields| string_field(fields, "baseUrl")))
        .or_else(|| meta.and_then(|fields| string_field(fields, "base_url")));
    if base_url
        .is_some_and(|url| normalize_base_url(&url) != normalize_base_url(OFFICIAL_INFERENCE_BASE))
    {
        return Err(GrokError::AlternateModelOrigin);
    }
    Ok(model)
}

fn string_field(fields: &Map<String, Value>, name: &str) -> Option<String> {
    fields
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn bool_field(
    object: &Map<String, Value>,
    meta: Option<&Map<String, Value>>,
    camel: &str,
    snake: &str,
) -> Option<bool> {
    object
        .get(camel)
        .or_else(|| object.get(snake))
        .or_else(|| meta.and_then(|fields| fields.get(camel)))
        .or_else(|| meta.and_then(|fields| fields.get(snake)))
        .and_then(Value::as_bool)
}

fn normalize_base_url(value: &str) -> &str {
    value.trim_end_matches('/')
}

fn ensure_success(response: &reqwest::Response) -> Result<(), GrokError> {
    match response.status() {
        status if status.is_success() => Ok(()),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            Err(GrokError::AuthenticationRejected {
                upstream_status: response.status().as_u16(),
            })
        }
        StatusCode::TOO_MANY_REQUESTS => Err(GrokError::RateLimited {
            retry_after_seconds: response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok()),
        }),
        status => Err(GrokError::UpstreamStatus(status.as_u16())),
    }
}

#[derive(Debug, Error)]
pub enum GrokError {
    #[error("failed to construct the origin-locked xAI client")]
    BuildClient(#[source] reqwest::Error),
    #[error("failed to construct an xAI endpoint URL")]
    InvalidEndpoint(#[source] url::ParseError),
    #[error("xAI transport failed")]
    Transport(#[source] reqwest::Error),
    #[error("xAI model catalog request exceeded its bounded timeout")]
    ModelsTimeout,
    #[error("xAI rejected the session credential; run the official Grok login flow")]
    AuthenticationRejected { upstream_status: u16 },
    #[error("xAI rate limited the request")]
    RateLimited { retry_after_seconds: Option<u64> },
    #[error("xAI returned unsuccessful status {0}")]
    UpstreamStatus(u16),
    #[error("xAI model catalog response is not valid JSON")]
    DecodeModels(#[source] reqwest::Error),
    #[error("xAI model catalog contains an invalid entry")]
    InvalidModelEntry,
    #[error("xAI model catalog entry is not explicitly backed by Responses")]
    UnconfirmedResponsesModel,
    #[error("xAI model catalog entry is hidden")]
    HiddenModelEntry,
    #[error("xAI model catalog entry is not supported by the API")]
    UnsupportedApiModelEntry,
    #[error("xAI model catalog entry selects an alternate inference origin")]
    AlternateModelOrigin,
    #[error(transparent)]
    Catalog(#[from] crate::catalog::CatalogError),
    #[error("xAI Responses transport did not return text/event-stream")]
    UnexpectedResponseContentType,
    #[error("xAI Responses stream framing is invalid")]
    InvalidSseFraming,
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("xAI Responses stream failed")]
    Stream(#[source] reqwest::Error),
}

impl GrokError {
    /// Safe classification for the HTTP response boundary. It deliberately
    /// excludes upstream headers and bodies.
    pub(crate) fn response_boundary_class(&self) -> &'static str {
        match self {
            Self::AuthenticationRejected { .. }
            | Self::RateLimited { .. }
            | Self::UpstreamStatus(_) => "upstream_http_status",
            Self::UnexpectedResponseContentType => "upstream_content_type",
            _ => "upstream_response",
        }
    }

    pub(crate) fn upstream_status(&self) -> Option<u16> {
        match self {
            Self::AuthenticationRejected { upstream_status } => Some(*upstream_status),
            Self::RateLimited { .. } => Some(StatusCode::TOO_MANY_REQUESTS.as_u16()),
            Self::UpstreamStatus(status) => Some(*status),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{HeaderMap, Response};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use futures_util::StreamExt;
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::*;
    use crate::credential::test_session_credential;
    use crate::protocol::NormalizedResponsesRequest;

    async fn start(router: Router) -> (Url, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (Url::parse(&format!("http://{address}/v1/")).unwrap(), task)
    }

    #[tokio::test]
    async fn response_completed_without_trailing_sse_boundary_is_flushed() {
        let item_added = json!({
            "type": "message", "id": "msg_1", "role": "assistant",
            "status": "in_progress", "content": []
        });
        let item_done = json!({
            "type": "message", "id": "msg_1", "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": "ok", "annotations": []}]
        });
        let events = [
            json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_1","output":[]}}),
            json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":item_added}),
            json!({"type":"response.content_part.added","sequence_number":2,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
            json!({"type":"response.output_text.delta","sequence_number":3,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"ok"}),
            json!({"type":"response.output_text.done","sequence_number":4,"item_id":"msg_1","output_index":0,"content_index":0,"text":"ok"}),
            json!({"type":"response.content_part.done","sequence_number":5,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"ok","annotations":[]}}),
            json!({"type":"response.output_item.done","sequence_number":6,"output_index":0,"item":item_done.clone()}),
            json!({"type":"response.completed","sequence_number":7,"response":{"id":"resp_1","output":[item_done]}}),
        ];
        let stream_body = events
            .into_iter()
            .enumerate()
            .map(|(index, event)| {
                if index == 7 {
                    format!("data: {event}")
                } else {
                    format!("data: {event}\n\n")
                }
            })
            .collect::<String>();
        let stream = ResponsesByteStream {
            inner: Box::pin(futures_util::stream::iter([Ok(Bytes::from(stream_body))])),
            _credential: Arc::new(test_session_credential("stream-secret", "user-1")),
            namespace_projection: NamespaceToolProjection::default(),
        };

        let events = stream
            .validated_text_events()
            .map(|event| event.unwrap())
            .collect::<Vec<_>>()
            .await;

        assert!(matches!(
            events.last().map(ValidatedTextStreamEvent::kind),
            Some(crate::protocol::TextStreamEventKind::ResponseCompleted { .. })
        ));
    }

    #[tokio::test]
    async fn unterminated_stream_synthesizes_response_completed_at_eof() {
        let item_done = json!({
            "type": "reasoning",
            "id": "rs_1",
            "status": "completed",
            "summary": []
        });
        let events = [
            json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_1","output":[]}}),
            json!({"type":"response.output_item.done","sequence_number":1,"output_index":0,"item":item_done}),
        ];
        let stream_body = events
            .into_iter()
            .map(|event| {
                format!(
                    "data: {event}

"
                )
            })
            .collect::<String>();
        let stream = ResponsesByteStream {
            inner: Box::pin(futures_util::stream::iter([Ok(Bytes::from(stream_body))])),
            _credential: Arc::new(test_session_credential("stream-secret", "user-1")),
            namespace_projection: NamespaceToolProjection::default(),
        };

        let events = stream
            .validated_text_events()
            .map(|event| event.unwrap())
            .collect::<Vec<_>>()
            .await;

        assert_eq!(events.len(), 3);
        assert!(matches!(
            events.last().map(ValidatedTextStreamEvent::kind),
            Some(crate::protocol::TextStreamEventKind::ResponseCompleted { response_id })
                if response_id == "resp_1"
        ));
        assert_eq!(
            events.last().unwrap().original()["response"]["status"],
            "completed"
        );
    }

    #[tokio::test]
    async fn blank_sse_event_after_completed_does_not_fail_the_stream() {
        let item_done = json!({
            "type": "message", "id": "msg_1", "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": "ok", "annotations": []}]
        });
        let events = [
            json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_1","output":[]}}),
            json!({"type":"response.output_item.done","sequence_number":1,"output_index":0,"item":item_done.clone()}),
            json!({"type":"response.completed","sequence_number":2,"response":{"id":"resp_1","output":[item_done]}}),
        ];
        let stream_body = events
            .into_iter()
            .map(|event| {
                format!(
                    "data: {event}

"
                )
            })
            .collect::<String>()
            + "

";
        let stream = ResponsesByteStream {
            inner: Box::pin(futures_util::stream::iter([Ok(Bytes::from(stream_body))])),
            _credential: Arc::new(test_session_credential("stream-secret", "user-1")),
            namespace_projection: NamespaceToolProjection::default(),
        };

        let events = stream
            .validated_text_events()
            .map(|event| event.unwrap())
            .collect::<Vec<_>>()
            .await;

        assert!(matches!(
            events.last().map(ValidatedTextStreamEvent::kind),
            Some(crate::protocol::TextStreamEventKind::ResponseCompleted { .. })
        ));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    crate::protocol::TextStreamEventKind::ResponseCompleted { .. }
                ))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn official_responses_models_and_future_slugs_are_admitted() {
        async fn models(headers: HeaderMap) -> impl IntoResponse {
            assert_eq!(headers["authorization"], "Bearer test-session-secret");
            assert_eq!(headers["x-xai-token-auth"], "xai-grok-cli");
            assert_eq!(headers["x-grok-client-mode"], "headless");
            (
                [(ETAG, "catalog-47")],
                Json(json!({"data":[
                    {"id":"grok-4.7","apiBackend":"responses"},
                    {"model":"grok-4.6","api_backend":"responses","baseUrl":"https://cli-chat-proxy.grok.com/v1"}
                ]})),
            )
        }
        let (base, task) = start(Router::new().route("/v1/models", get(models))).await;
        let client = GrokClient::for_test(base).unwrap();
        let credential = test_session_credential("test-session-secret", "user-1");
        let fetched = client.fetch_models(&credential).await.unwrap();
        assert_eq!(fetched.models, ["grok-4.7", "grok-4.6"]);
        assert_eq!(fetched.etag.as_deref(), Some("catalog-47"));
        task.abort();
    }

    #[tokio::test]
    async fn redirects_are_not_followed_and_auth_failures_are_typed() {
        async fn redirect(State(hits): State<Arc<AtomicUsize>>) -> impl IntoResponse {
            hits.fetch_add(1, Ordering::SeqCst);
            StatusCode::OK
        }
        let hits = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route(
                "/v1/models",
                get(|| async { (StatusCode::FOUND, [("location", "/redirected")]) }),
            )
            .route("/redirected", get(redirect))
            .with_state(Arc::clone(&hits));
        let (base, task) = start(router).await;
        let client = GrokClient::for_test(base).unwrap();
        let credential = test_session_credential("secret", "user-1");
        assert!(matches!(
            client.fetch_models(&credential).await,
            Err(GrokError::UpstreamStatus(302))
        ));
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        task.abort();
    }

    #[tokio::test]
    async fn upstream_http_boundary_errors_have_safe_static_classes() {
        let non_success =
            Router::new().route("/v1/responses", post(|| async { StatusCode::IM_A_TEAPOT }));
        let (base, task) = start(non_success).await;
        let client = GrokClient::for_test(base).unwrap();
        let credential = Arc::new(test_session_credential("stream-secret", "user-1"));
        let body = NormalizedResponsesRequest::parse(json!({
            "model": "grok-4.7", "input": [], "tools": [], "tool_choice": "auto",
            "parallel_tool_calls": false, "store": false, "stream": true, "include": []
        }))
        .unwrap();
        let request = ResponsesTransportRequest {
            body: &body,
            conversation_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            turn_index: 0,
        };
        let error = match client.post_responses(credential, request).await {
            Err(error) => error,
            Ok(_) => panic!("non-success response must not start an SSE stream"),
        };
        assert_eq!(error.response_boundary_class(), "upstream_http_status");
        assert_eq!(error.upstream_status(), Some(418));
        assert!(matches!(error, GrokError::UpstreamStatus(418)));
        task.abort();

        let invalid_content_type = Router::new().route(
            "/v1/responses",
            post(|| async { (StatusCode::OK, [(CONTENT_TYPE, "application/json")]) }),
        );
        let (base, task) = start(invalid_content_type).await;
        let client = GrokClient::for_test(base).unwrap();
        let credential = Arc::new(test_session_credential("stream-secret", "user-1"));
        let body = NormalizedResponsesRequest::parse(json!({
            "model": "grok-4.7", "input": [], "tools": [], "tool_choice": "auto",
            "parallel_tool_calls": false, "store": false, "stream": true, "include": []
        }))
        .unwrap();
        let error = match client
            .post_responses(
                credential,
                ResponsesTransportRequest {
                    body: &body,
                    conversation_id: Uuid::new_v4(),
                    request_id: Uuid::new_v4(),
                    agent_id: Uuid::new_v4(),
                    turn_index: 0,
                },
            )
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("invalid Content-Type must not start an SSE stream"),
        };
        assert_eq!(error.response_boundary_class(), "upstream_content_type");
        assert_eq!(error.upstream_status(), None);
        assert!(matches!(error, GrokError::UnexpectedResponseContentType));
        task.abort();
    }

    #[tokio::test]
    async fn responses_transport_requires_sse_and_keeps_credential_alive() {
        async fn responses(headers: HeaderMap, Json(body): Json<Value>) -> Response<Body> {
            assert_eq!(headers["authorization"], "Bearer stream-secret");
            assert_eq!(headers["x-grok-model-override"], "grok-4.7");
            assert_eq!(
                headers["x-grok-client-version"],
                GROK_BUILD_COMPATIBILITY_VERSION
            );
            assert_eq!(headers["x-grok-client-identifier"], CLIENT_IDENTIFIER);
            assert_eq!(headers["x-grok-turn-idx"], "1");
            assert_eq!(
                headers["x-grok-agent-id"],
                "33333333-3333-4333-8333-333333333333"
            );
            assert_eq!(
                headers["user-agent"],
                format!("{CLIENT_IDENTIFIER}/{}", env!("CARGO_PKG_VERSION"))
            );
            assert_eq!(body["stream"], true);
            let item_added = json!({
                "type": "message", "id": "msg_1", "role": "assistant",
                "status": "in_progress", "content": []
            });
            let item_done = json!({
                "type": "message", "id": "msg_1", "role": "assistant",
                "status": "completed",
                "content": [{"type": "output_text", "text": "ok", "annotations": []}]
            });
            let events = [
                json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_1","output":[]}}),
                json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":item_added}),
                json!({"type":"response.content_part.added","sequence_number":2,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
                json!({"type":"response.output_text.delta","sequence_number":3,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"ok"}),
                json!({"type":"response.output_text.done","sequence_number":4,"item_id":"msg_1","output_index":0,"content_index":0,"text":"ok"}),
                json!({"type":"response.content_part.done","sequence_number":5,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"ok","annotations":[]}}),
                json!({"type":"response.output_item.done","sequence_number":6,"output_index":0,"item":item_done.clone()}),
                json!({"type":"response.completed","sequence_number":7,"response":{"id":"resp_1","output":[item_done]}}),
            ];
            let stream_body = events
                .into_iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>();
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "text/event-stream; charset=utf-8")
                .body(Body::from(stream_body))
                .unwrap()
        }
        let (base, task) = start(Router::new().route("/v1/responses", post(responses))).await;
        let client = GrokClient::for_test(base).unwrap();
        let credential = Arc::new(test_session_credential("stream-secret", "user-1"));
        let body = NormalizedResponsesRequest::parse(json!({
            "model": "grok-4.7",
            "instructions": "answer clearly",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}]
            }],
            "tools": [],
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "reasoning": null,
            "store": false,
            "stream": true,
            "include": []
        }))
        .unwrap();
        let stream = client
            .post_responses(
                credential,
                ResponsesTransportRequest {
                    body: &body,
                    conversation_id: Uuid::new_v4(),
                    request_id: Uuid::new_v4(),
                    agent_id: Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap(),
                    turn_index: 1,
                },
            )
            .await
            .unwrap();
        let mut events = stream.validated_text_events();
        let mut deltas = String::new();
        while let Some(event) = events.next().await {
            if let crate::protocol::TextStreamEventKind::OutputTextDelta { delta, .. } =
                event.unwrap().kind()
            {
                deltas.push_str(delta);
            }
        }
        assert_eq!(deltas, "ok");
        task.abort();
    }

    #[tokio::test]
    async fn responses_transport_projects_codex_namespace_tools_in_both_directions() {
        async fn responses(Json(body): Json<Value>) -> Response<Body> {
            assert_eq!(body["tools"][0]["type"], "function");
            assert_eq!(body["tools"][0]["name"], "mcp__demo__ping");
            let added = json!({
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "mcp__demo__ping",
                "arguments": "",
                "status": "in_progress"
            });
            let done = json!({
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "mcp__demo__ping",
                "arguments": "{}",
                "status": "completed"
            });
            let events = [
                json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_1","output":[]}}),
                json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":added}),
                json!({"type":"response.function_call_arguments.delta","sequence_number":2,"item_id":"fc_1","output_index":0,"delta":"{}"}),
                json!({"type":"response.function_call_arguments.done","sequence_number":3,"item_id":"fc_1","output_index":0,"arguments":"{}"}),
                json!({"type":"response.output_item.done","sequence_number":4,"output_index":0,"item":done.clone()}),
                json!({"type":"response.completed","sequence_number":5,"response":{"id":"resp_1","output":[done]}}),
            ];
            let stream_body = events
                .into_iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>();
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "text/event-stream; charset=utf-8")
                .body(Body::from(stream_body))
                .unwrap()
        }

        let (base, task) = start(Router::new().route("/v1/responses", post(responses))).await;
        let client = GrokClient::for_test(base).unwrap();
        let body = NormalizedResponsesRequest::parse(json!({
            "model": "grok-4.6",
            "instructions": "Use the admitted tool.",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "ping"}]
            }],
            "tools": [{
                "type": "namespace",
                "name": "mcp__demo",
                "description": "Tools in the mcp__demo namespace.",
                "tools": [{
                    "type": "function",
                    "name": "ping",
                    "description": "",
                    "strict": false,
                    "parameters": {"type": "object", "properties": {}}
                }]
            }],
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "reasoning": {"effort": "xhigh", "summary": "auto"},
            "store": false,
            "stream": true,
            "include": ["reasoning.encrypted_content"]
        }))
        .unwrap();
        let stream = client
            .post_responses(
                Arc::new(test_session_credential("stream-secret", "user-1")),
                ResponsesTransportRequest {
                    body: &body,
                    conversation_id: Uuid::new_v4(),
                    request_id: Uuid::new_v4(),
                    agent_id: Uuid::new_v4(),
                    turn_index: 1,
                },
            )
            .await
            .unwrap();
        let events = stream
            .validated_text_events()
            .map(|event| event.unwrap().into_original())
            .collect::<Vec<_>>()
            .await;
        assert_eq!(events[1]["item"]["name"], "ping");
        assert_eq!(events[1]["item"]["namespace"], "mcp__demo");
        assert_eq!(events[4]["item"]["name"], "ping");
        assert_eq!(events[4]["item"]["namespace"], "mcp__demo");
        assert_eq!(events[5]["response"]["output"][0]["name"], "ping");
        assert_eq!(events[5]["response"]["output"][0]["namespace"], "mcp__demo");
        task.abort();
    }
}
