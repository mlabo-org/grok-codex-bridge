use std::io::{self, Cursor, Read};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_ENCODING, CONTENT_TYPE, RETRY_AFTER};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::net::TcpListener;

use crate::catalog::ModelCatalog;
use crate::config::{CapabilityToken, RuntimeConfig};
use crate::credential::{CredentialError, CredentialStore};
use crate::grok::{GrokClient, GrokError, ResponsesTransportRequest};
use crate::lifecycle::PICKER_CALLER_HEADER;
use crate::native::{
    NativeClient, NativeError, NativeResponsesPath, NativeRouteState, is_hop_by_hop_response_header,
};
use crate::protocol::{NormalizedResponsesRequest, sanitize_unreplayable_history_for_native};

const MAX_RESPONSES_BODY_BYTES: usize = 16 * 1024 * 1024;
const DISABLE_ENVIRONMENT_VARIABLE: &str = "GROK_CODEX_BRIDGE_DISABLE";
const X_GROK_UPSTREAM_STATUS: &str = "x-grok-upstream-status";
const CREDENTIAL_RENEWAL_GRACE: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct ServiceState {
    capability: Arc<CapabilityToken>,
    catalog: ModelCatalog,
    responses: Option<Arc<ResponsesService>>,
    native: Option<Arc<NativeService>>,
    responses_disabled: bool,
}

pub fn build_router(config: RuntimeConfig, catalog: ModelCatalog) -> Router {
    let responses_disabled = responses_disabled_from_environment();
    let responses = if responses_disabled {
        None
    } else {
        match ResponsesService::production() {
            Ok(service) => Some(Arc::new(service)),
            Err(error) => {
                tracing::error!(
                    error_class = error.class(),
                    "Responses service is unavailable"
                );
                None
            }
        }
    };
    let native = native_service(&config).unwrap_or_else(|error| {
        tracing::error!(error_class = "native_route", %error, "Native route is unavailable");
        None
    });
    build_router_with_services(config, catalog, responses, native, responses_disabled)
}

#[cfg(test)]
fn build_router_with_responses(
    config: RuntimeConfig,
    catalog: ModelCatalog,
    responses_service: Option<Arc<ResponsesService>>,
    responses_disabled: bool,
) -> Router {
    build_router_with_services(config, catalog, responses_service, None, responses_disabled)
}

fn build_router_with_services(
    config: RuntimeConfig,
    catalog: ModelCatalog,
    responses_service: Option<Arc<ResponsesService>>,
    native_service: Option<Arc<NativeService>>,
    responses_disabled: bool,
) -> Router {
    let (_, capability) = config.into_server_parts();
    let state = ServiceState {
        capability: Arc::new(capability),
        catalog,
        responses: responses_service,
        native: native_service,
        responses_disabled,
    };

    Router::new()
        .route("/v1/healthz", get(picker_healthz))
        .route("/v1/models", get(picker_models))
        .route(
            "/v1/responses",
            get(picker_responses_websocket_not_supported).post(picker_responses),
        )
        .route("/v1/responses/compact", post(picker_responses_compact))
        .route("/_grok/{capability}/healthz", get(healthz))
        .route("/_grok/{capability}/v1/models", get(models))
        .route(
            "/_grok/{capability}/v1/responses",
            get(responses_websocket_not_supported).post(responses),
        )
        .route(
            "/_grok/{capability}/v1/responses/compact",
            post(responses_compact),
        )
        .fallback(not_found)
        .with_state(state)
}

pub struct BoundServer {
    bind: SocketAddr,
    listener: TcpListener,
    router: Router,
}

impl BoundServer {
    pub async fn serve(self) -> Result<(), ServerError> {
        tracing::info!(address = %self.bind, "loopback service started");
        axum::serve(self.listener, self.router)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(ServerError::Serve)
    }

    #[cfg(test)]
    fn local_addr(&self) -> SocketAddr {
        self.listener
            .local_addr()
            .expect("bound listener has address")
    }
}

pub async fn bind(
    config: RuntimeConfig,
    catalog: ModelCatalog,
) -> Result<BoundServer, ServerError> {
    let bind = config.bind();
    let native = native_service(&config).map_err(ServerError::NativeRoute)?;
    let responses_disabled = responses_disabled_from_environment();
    let responses = if responses_disabled {
        None
    } else {
        Some(Arc::new(ResponsesService::production()?))
    };
    let router = build_router_with_services(config, catalog, responses, native, responses_disabled);
    let listener = TcpListener::bind(bind).await.map_err(ServerError::Bind)?;
    Ok(BoundServer {
        bind,
        listener,
        router,
    })
}

pub async fn serve(config: RuntimeConfig, catalog: ModelCatalog) -> Result<(), ServerError> {
    bind(config, catalog).await?.serve().await
}

fn native_service(config: &RuntimeConfig) -> Result<Option<Arc<NativeService>>, NativeError> {
    let Some(route) = NativeRouteState::load_if_present(&config.grok().native_route_file())? else {
        return Ok(None);
    };
    let client = NativeClient::production(route.upstream())?;
    Ok(Some(Arc::new(NativeService { route, client })))
}

async fn healthz(State(state): State<ServiceState>, Path(capability): Path<String>) -> Response {
    if !state.capability.matches(&capability) {
        return StatusCode::NOT_FOUND.into_response();
    }

    tracing::debug!(route = "healthz", status = 200_u16, "request complete");
    Json(HealthResponse {
        status: "ok",
        service: "grok-codex-bridge",
        version: env!("CARGO_PKG_VERSION"),
    })
    .into_response()
}

async fn picker_healthz(State(state): State<ServiceState>, headers: HeaderMap) -> Response {
    if !picker_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    tracing::debug!(
        route = "picker_healthz",
        status = 200_u16,
        "request complete"
    );
    Json(HealthResponse {
        status: "ok",
        service: "grok-codex-bridge",
        version: env!("CARGO_PKG_VERSION"),
    })
    .into_response()
}

async fn models(State(state): State<ServiceState>, Path(capability): Path<String>) -> Response {
    if !state.capability.matches(&capability) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let response = state.catalog.response().await;
    tracing::debug!(route = "models", status = 200_u16, "request complete");
    Json(response).into_response()
}

async fn picker_models(State(state): State<ServiceState>, headers: HeaderMap) -> Response {
    if !picker_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let response = state.catalog.response().await;
    tracing::debug!(
        route = "picker_models",
        status = 200_u16,
        "request complete"
    );
    Json(response).into_response()
}

async fn responses(
    State(state): State<ServiceState>,
    Path(capability): Path<String>,
    request: Request<Body>,
) -> Response {
    route_responses(state, capability, request, NativeResponsesPath::Responses).await
}

async fn picker_responses(State(state): State<ServiceState>, request: Request<Body>) -> Response {
    route_picker_responses(state, request, NativeResponsesPath::Responses).await
}

async fn responses_compact(
    State(state): State<ServiceState>,
    Path(capability): Path<String>,
    request: Request<Body>,
) -> Response {
    route_responses(state, capability, request, NativeResponsesPath::Compact).await
}

async fn picker_responses_compact(
    State(state): State<ServiceState>,
    request: Request<Body>,
) -> Response {
    route_picker_responses(state, request, NativeResponsesPath::Compact).await
}

async fn responses_websocket_not_supported(
    State(state): State<ServiceState>,
    Path(capability): Path<String>,
) -> Response {
    if !state.capability.matches(&capability) {
        return StatusCode::NOT_FOUND.into_response();
    }
    StatusCode::UPGRADE_REQUIRED.into_response()
}

async fn picker_responses_websocket_not_supported(
    State(state): State<ServiceState>,
    headers: HeaderMap,
) -> Response {
    if !picker_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    StatusCode::UPGRADE_REQUIRED.into_response()
}

async fn route_responses(
    state: ServiceState,
    capability: String,
    request: Request<Body>,
    path: NativeResponsesPath,
) -> Response {
    if !state.capability.matches(&capability) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if state.responses_disabled && state.native.is_none() {
        return route_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "bridge_disabled",
            "server_error",
            "bridge_disabled",
            "Grok Responses routing is disabled",
        );
    }

    route_authorized_responses(state, request, path).await
}

async fn route_picker_responses(
    state: ServiceState,
    request: Request<Body>,
    path: NativeResponsesPath,
) -> Response {
    if !picker_authorized(&state, request.headers()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    route_authorized_responses(state, request, path).await
}

fn picker_authorized(state: &ServiceState, headers: &HeaderMap) -> bool {
    let candidate = headers
        .get(PICKER_CALLER_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    state.capability.matches(candidate)
}

async fn route_authorized_responses(
    state: ServiceState,
    request: Request<Body>,
    path: NativeResponsesPath,
) -> Response {
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_RESPONSES_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => {
            return route_error(
                StatusCode::BAD_REQUEST,
                "request_body",
                "invalid_request_error",
                "request_body_invalid",
                "Responses request body is invalid or too large",
            );
        }
    };
    let decoded = match decode_request_copy(&parts.headers, &body) {
        Ok(decoded) => decoded,
        Err(()) => {
            return route_error(
                StatusCode::BAD_REQUEST,
                "request_encoding",
                "invalid_request_error",
                "request_body_invalid",
                "Responses request body encoding is invalid or unsupported",
            );
        }
    };
    let envelope: RouteEnvelope = match serde_json::from_slice(&decoded) {
        Ok(envelope) => envelope,
        Err(_) => {
            return route_error(
                StatusCode::BAD_REQUEST,
                "request_json",
                "invalid_request_error",
                "invalid_json",
                "Responses request body must be valid JSON with a model",
            );
        }
    };
    let is_grok = state.catalog.contains(&envelope.model).await;
    let is_native = state
        .native
        .as_ref()
        .is_some_and(|native| native.route.contains(&envelope.model));
    match (is_native, is_grok) {
        (true, false) => {
            let native = state
                .native
                .expect("Native classifier requires a Native service");
            let body = match native_request_body(path, &parts.headers, body, &decoded) {
                Ok(body) => body,
                Err(()) => {
                    return route_error(
                        StatusCode::BAD_REQUEST,
                        "request_encoding",
                        "invalid_request_error",
                        "request_body_invalid",
                        "Responses request body encoding is invalid or unsupported",
                    );
                }
            };
            return native_response(native, path, &parts.headers, body).await;
        }
        (true, true) => {
            return route_error(
                StatusCode::BAD_REQUEST,
                "model_collision",
                "invalid_request_error",
                "model_route_ambiguous",
                "Requested model collides across Native and Grok catalogs",
            );
        }
        (false, false) => {
            return route_error(
                StatusCode::BAD_REQUEST,
                "unknown_model",
                "invalid_request_error",
                "model_not_admitted",
                "Requested model is not admitted by the current picker state",
            );
        }
        (false, true) => {}
    }

    if path == NativeResponsesPath::Compact {
        return route_error(
            StatusCode::BAD_REQUEST,
            "grok_compaction",
            "invalid_request_error",
            "unsupported_request",
            "Grok upstream does not expose an authoritative Responses compact contract",
        );
    }
    if state.responses_disabled {
        return route_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "bridge_disabled",
            "server_error",
            "bridge_disabled",
            "Grok Responses routing is disabled",
        );
    }

    let value: Value = match serde_json::from_slice(&decoded) {
        Ok(value) => value,
        Err(_) => {
            return route_error(
                StatusCode::BAD_REQUEST,
                "request_json",
                "invalid_request_error",
                "invalid_json",
                "Responses request body must be valid JSON",
            );
        }
    };
    let normalized = match NormalizedResponsesRequest::parse(value) {
        Ok(normalized) => normalized,
        Err(error) => {
            tracing::warn!(
                route = "responses",
                status = StatusCode::BAD_REQUEST.as_u16(),
                error_class = "request_protocol",
                protocol_error = %error,
                "request protocol rejected"
            );
            return route_error(
                StatusCode::BAD_REQUEST,
                "request_protocol",
                "invalid_request_error",
                "unsupported_request",
                "Responses request is invalid or unsupported",
            );
        }
    };
    let routing = match normalized.grok_routing_metadata() {
        Ok(routing) => routing,
        Err(error) => {
            tracing::warn!(
                route = "responses",
                status = StatusCode::BAD_REQUEST.as_u16(),
                error_class = "request_protocol",
                protocol_error = %error,
                "request protocol rejected"
            );
            return route_error(
                StatusCode::BAD_REQUEST,
                "request_protocol",
                "invalid_request_error",
                "unsupported_request",
                "Responses request is invalid or unsupported",
            );
        }
    };
    let Some(service) = state.responses else {
        return route_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "responses_service",
            "server_error",
            "service_unavailable",
            "Grok Responses service is unavailable",
        );
    };
    let credential_store = Arc::clone(&service.credentials);
    let credential_renewal_grace = service.credential_renewal_grace;
    let credential = match tokio::task::spawn_blocking(move || {
        credential_store.load_with_renewal_grace(credential_renewal_grace)
    })
    .await
    {
        Ok(Ok(credential)) => credential,
        Ok(Err(_)) => {
            return route_error(
                StatusCode::UNAUTHORIZED,
                "local_credential",
                "authentication_error",
                "grok_login_required",
                "Grok credential is unavailable; run the official Grok login flow",
            );
        }
        Err(_) => {
            return route_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "credential_task",
                "server_error",
                "service_unavailable",
                "Grok Responses service is unavailable",
            );
        }
    };

    let upstream = match service
        .client
        .post_responses(
            credential,
            ResponsesTransportRequest {
                body: &normalized,
                conversation_id: routing.conversation_id(),
                request_id: routing.request_id(),
                agent_id: routing.agent_id(),
                turn_index: routing.turn_index(),
            },
        )
        .await
    {
        Ok(stream) => stream,
        Err(error) => return upstream_error(error),
    };

    tracing::debug!(route = "responses", status = 200_u16, "request accepted");
    let stream = upstream.validated_text_events().map(|result| match result {
        Ok(event) => {
            let original = event.into_codex_value();
            let event_type = original
                .get("type")
                .and_then(Value::as_str)
                .expect("validated Responses events have a known type");
            Ok::<Bytes, io::Error>(Bytes::from(format!(
                "event: {event_type}\ndata: {original}\n\n"
            )))
        }
        Err(error) => {
            log_stream_error(&error);
            Err(io::Error::other("validated upstream stream terminated"))
        }
    });
    let mut response = Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response
}

async fn native_response(
    service: Arc<NativeService>,
    path: NativeResponsesPath,
    headers: &HeaderMap,
    body: Bytes,
) -> Response {
    let upstream = match service.client.post(path, headers, body).await {
        Ok(response) => response,
        Err(_) => {
            return route_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "native_transport",
                "server_error",
                "native_upstream_unavailable",
                "Native Codex Responses upstream is unavailable",
            );
        }
    };
    let status = upstream.status();
    let headers = upstream.headers().clone();
    let stream = upstream
        .bytes_stream()
        .map(|chunk| chunk.map_err(io::Error::other));
    let mut response = Body::from_stream(stream).into_response();
    *response.status_mut() = status;
    for (name, value) in &headers {
        if !is_hop_by_hop_response_header(name) {
            response.headers_mut().append(name.clone(), value.clone());
        }
    }
    response
}

fn decode_request_copy(headers: &HeaderMap, raw: &[u8]) -> Result<Vec<u8>, ()> {
    let Some(encoding) = headers.get(CONTENT_ENCODING) else {
        return Ok(raw.to_vec());
    };
    match encoding.to_str().map_err(|_| ())?.trim() {
        "identity" => Ok(raw.to_vec()),
        "zstd" => {
            let decoder = zstd::stream::read::Decoder::new(Cursor::new(raw)).map_err(|_| ())?;
            let mut decoded = Vec::new();
            decoder
                .take((MAX_RESPONSES_BODY_BYTES + 1) as u64)
                .read_to_end(&mut decoded)
                .map_err(|_| ())?;
            if decoded.len() > MAX_RESPONSES_BODY_BYTES {
                return Err(());
            }
            Ok(decoded)
        }
        _ => Err(()),
    }
}

fn native_request_body(
    path: NativeResponsesPath,
    headers: &HeaderMap,
    original: Bytes,
    decoded: &[u8],
) -> Result<Bytes, ()> {
    if path != NativeResponsesPath::Responses {
        return Ok(original);
    }
    let mut request: Value = serde_json::from_slice(decoded).map_err(|_| ())?;
    let removed_prompt_cache_retention = request
        .as_object_mut()
        .and_then(|request| request.remove("prompt_cache_retention"))
        .is_some();
    let removed_previous_response_id = request
        .as_object_mut()
        .and_then(|request| request.remove("previous_response_id"))
        .is_some();
    let sanitized_unreplayable_history = sanitize_unreplayable_history_for_native(&mut request);
    if !removed_prompt_cache_retention
        && !removed_previous_response_id
        && !sanitized_unreplayable_history
    {
        return Ok(original);
    }
    let serialized = serde_json::to_vec(&request).map_err(|_| ())?;
    let encoded = match headers
        .get(CONTENT_ENCODING)
        .map(|value| value.to_str().map(str::trim))
        .transpose()
        .map_err(|_| ())?
    {
        None | Some("identity") => serialized,
        Some("zstd") => zstd::stream::encode_all(Cursor::new(serialized), 0).map_err(|_| ())?,
        Some(_) => return Err(()),
    };
    Ok(Bytes::from(encoded))
}

#[derive(Deserialize)]
struct RouteEnvelope {
    model: String,
}

fn stream_error_class(error: &GrokError) -> &'static str {
    match error {
        GrokError::Protocol(_) => "upstream_stream_protocol",
        GrokError::InvalidSseFraming => "upstream_stream_framing",
        GrokError::Stream(_) => "upstream_stream_transport",
        _ => "upstream_stream_other",
    }
}

fn log_stream_error(error: &GrokError) {
    let error_class = stream_error_class(error);
    if let GrokError::Protocol(protocol_error) = error {
        tracing::warn!(
            route = "responses",
            status = 502_u16,
            error_class,
            protocol_error = %protocol_error,
            "Responses stream terminated"
        );
    } else {
        tracing::warn!(
            route = "responses",
            status = 502_u16,
            error_class,
            "Responses stream terminated"
        );
    }
}

fn responses_disabled_from_environment() -> bool {
    std::env::var_os(DISABLE_ENVIRONMENT_VARIABLE).is_some_and(|value| value == "1")
}

#[derive(Clone)]
struct ResponsesService {
    credentials: Arc<CredentialStore>,
    client: GrokClient,
    credential_renewal_grace: Duration,
}

#[derive(Clone)]
struct NativeService {
    route: NativeRouteState,
    client: NativeClient,
}

impl ResponsesService {
    fn production() -> Result<Self, ServerError> {
        Ok(Self {
            credentials: Arc::new(
                CredentialStore::from_environment().map_err(ServerError::CredentialSource)?,
            ),
            client: GrokClient::production().map_err(ServerError::UpstreamClient)?,
            credential_renewal_grace: CREDENTIAL_RENEWAL_GRACE,
        })
    }
}

fn route_error(
    status: StatusCode,
    error_class: &'static str,
    error_type: &'static str,
    code: &'static str,
    message: &'static str,
) -> Response {
    tracing::warn!(
        route = "responses",
        status = status.as_u16(),
        error_class,
        "request failed"
    );
    (
        status,
        Json(ErrorEnvelope {
            error: ErrorBody {
                error_type,
                code,
                message,
            },
        }),
    )
        .into_response()
}

fn upstream_error(error: GrokError) -> Response {
    let error_class = error.response_boundary_class();
    let diagnostic_upstream_status = error.upstream_status();
    let upstream_status = match &error {
        GrokError::UpstreamStatus(status) => Some(*status),
        _ => None,
    };
    let (status, error_type, code, message, retry_after) = match error {
        GrokError::AuthenticationRejected { .. } => (
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "grok_login_required",
            "xAI rejected the Grok credential; run the official Grok login flow",
            None,
        ),
        GrokError::RateLimited {
            retry_after_seconds,
        } => (
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_error",
            "rate_limited",
            "xAI rate limited the Grok request",
            retry_after_seconds,
        ),
        GrokError::Transport(_) | GrokError::UpstreamStatus(500..=599) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "upstream_unavailable",
            "xAI Responses service is temporarily unavailable",
            None,
        ),
        GrokError::UnexpectedResponseContentType => (
            StatusCode::BAD_GATEWAY,
            "server_error",
            "invalid_upstream_response",
            "xAI returned an invalid Responses stream",
            None,
        ),
        _ => (
            StatusCode::BAD_GATEWAY,
            "server_error",
            "invalid_upstream_response",
            "xAI returned an invalid Responses response",
            None,
        ),
    };
    tracing::warn!(
        route = "responses",
        status = status.as_u16(),
        error_class,
        upstream_status = ?diagnostic_upstream_status,
        "request failed"
    );
    let mut response = (
        status,
        Json(ErrorEnvelope {
            error: ErrorBody {
                error_type,
                code,
                message,
            },
        }),
    )
        .into_response();
    if let Some(seconds) = retry_after
        && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
    {
        response.headers_mut().insert(RETRY_AFTER, value);
    }
    if let Some(status) = upstream_status
        && let Ok(value) = HeaderValue::from_str(&status.to_string())
    {
        response.headers_mut().insert(X_GROK_UPSTREAM_STATUS, value);
    }
    response
}

async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

#[derive(Serialize)]
struct HealthResponse<'a> {
    status: &'a str,
    service: &'a str,
    version: &'a str,
}

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    #[serde(rename = "type")]
    error_type: &'a str,
    code: &'a str,
    message: &'a str,
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown requested");
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("failed to resolve the official Grok credential source")]
    CredentialSource(#[source] CredentialError),
    #[error("failed to construct the origin-locked Grok client")]
    UpstreamClient(#[source] GrokError),
    #[error("failed to load or construct the Native Codex route")]
    NativeRoute(#[source] NativeError),
    #[error("failed to bind loopback service")]
    Bind(#[source] std::io::Error),
    #[error("loopback service failed")]
    Serve(#[source] std::io::Error),
}

impl ServerError {
    fn class(&self) -> &'static str {
        match self {
            Self::CredentialSource(_) => "credential_source",
            Self::UpstreamClient(_) => "upstream_client",
            Self::NativeRoute(_) => "native_route",
            Self::Bind(_) => "bind",
            Self::Serve(_) => "serve",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path as FsPath, PathBuf};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use axum::extract::State as AxumState;
    use axum::http::HeaderMap;
    use axum::routing::post as upstream_post;
    use futures_util::StreamExt;
    use serde_json::{Value, json};
    use tokio::net::TcpStream;
    use tokio::task::JoinHandle;
    use tower::ServiceExt;
    use url::Url;
    use uuid::Uuid;

    use super::*;

    const TOKEN: &str = "abcdefghijklmnopqrstuvwxyz_12345";

    fn runtime_config(directory: &FsPath) -> RuntimeConfig {
        let token_path = directory.join("caller-token");
        fs::write(&token_path, TOKEN).unwrap();
        fs::set_permissions(&token_path, fs::Permissions::from_mode(0o600)).unwrap();
        let config_path = directory.join("bridge.toml");
        fs::write(
            &config_path,
            format!(
                "version = 1\n\n[server]\nbind = \"127.0.0.1:4545\"\ncapability_token_file = {:?}\n\n[grok]\ncatalog_cache_file = {:?}\nrefresh_on_start = false\n",
                token_path.display().to_string(),
                directory.join("models.json").display().to_string()
            ),
        )
        .unwrap();
        RuntimeConfig::load(&config_path).unwrap()
    }

    #[tokio::test]
    async fn listener_binds_before_a_delayed_startup_refresh_can_complete() {
        let directory = tempfile::tempdir().unwrap();
        let token_path = directory.path().join("caller-token");
        fs::write(&token_path, TOKEN).unwrap();
        fs::set_permissions(&token_path, fs::Permissions::from_mode(0o600)).unwrap();
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let config_path = directory.path().join("bridge.toml");
        fs::write(
            &config_path,
            format!(
                "version = 1\n\n[server]\nbind = \"127.0.0.1:{port}\"\ncapability_token_file = {:?}\n\n[grok]\ncatalog_cache_file = {:?}\nrefresh_on_start = true\n",
                token_path.display().to_string(),
                directory.path().join("models.json").display().to_string()
            ),
        )
        .unwrap();
        let config = RuntimeConfig::load(&config_path).unwrap();
        let server = bind(config, ModelCatalog::bootstrap().unwrap())
            .await
            .unwrap();
        let address = server.local_addr();

        // A startup refresh may remain pending here; the concrete loopback
        // listener is already accepting TCP connections independently of it.
        let refresh = tokio::sync::oneshot::channel::<()>();
        let (_release, pending) = refresh;
        let _connection = TcpStream::connect(address).await.unwrap();
        assert!(!_connection.peer_addr().unwrap().ip().is_unspecified());
        drop(pending);
        drop(server);
    }

    fn write_auth(directory: &FsPath) -> PathBuf {
        let auth_path = directory.join("auth.json");
        fs::write(
            &auth_path,
            br#"{"https://auth.x.ai::current-client":{"key":"mock-session-secret","auth_mode":"oidc","create_time":"2026-08-01T00:00:00Z","user_id":"user-1","expires_at":"2099-01-01T00:00:00Z","oidc_issuer":"https://auth.x.ai"}}"#,
        )
        .unwrap();
        fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o600)).unwrap();
        auth_path
    }

    fn request_body(model: &str) -> Value {
        json!({
            "model": model,
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
            "include": [],
            "prompt_cache_key": "11111111-1111-4111-8111-111111111111",
            "client_metadata": {
                "session_id": "11111111-1111-4111-8111-111111111111",
                "turn_id": "22222222-2222-4222-8222-222222222222",
                "x-codex-installation-id": "33333333-3333-4333-8333-333333333333"
            }
        })
    }

    async fn send(app: Router, capability: &str, body: impl Into<Body>) -> Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/_grok/{capability}/v1/responses"))
                .body(body.into())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn send_with_headers(
        app: Router,
        capability: &str,
        headers: &[(&str, &str)],
        body: impl Into<Body>,
    ) -> Response {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/_grok/{capability}/v1/responses"));
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        app.oneshot(builder.body(body.into()).unwrap())
            .await
            .unwrap()
    }

    async fn send_picker_with_headers(
        app: Router,
        headers: &[(&str, &str)],
        body: impl Into<Body>,
    ) -> Response {
        let mut builder = Request::builder().method("POST").uri("/v1/responses");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        app.oneshot(builder.body(body.into()).unwrap())
            .await
            .unwrap()
    }

    fn test_app(
        config: RuntimeConfig,
        catalog: ModelCatalog,
        credentials: CredentialStore,
        client: GrokClient,
    ) -> Router {
        test_app_with_renewal_grace(
            config,
            catalog,
            credentials,
            client,
            Duration::ZERO,
        )
    }

    fn test_app_with_renewal_grace(
        config: RuntimeConfig,
        catalog: ModelCatalog,
        credentials: CredentialStore,
        client: GrokClient,
        credential_renewal_grace: Duration,
    ) -> Router {
        build_router_with_responses(
            config,
            catalog,
            Some(Arc::new(ResponsesService {
                credentials: Arc::new(credentials),
                client,
                credential_renewal_grace,
            })),
            false,
        )
    }

    #[derive(Clone, Copy)]
    enum MockReply {
        Valid,
        Reasoning,
        Unauthorized,
        RateLimited,
        Unavailable,
        InvalidStream,
        FailedStream,
    }

    struct MockState {
        reply: MockReply,
        hits: AtomicUsize,
        observed: Mutex<Vec<(HeaderMap, Value)>>,
    }

    async fn mock_responses(
        AxumState(state): AxumState<Arc<MockState>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        state.hits.fetch_add(1, Ordering::SeqCst);
        state.observed.lock().unwrap().push((headers, body));
        match state.reply {
            MockReply::Valid => sse_response(valid_text_events()),
            MockReply::Reasoning => sse_response(valid_reasoning_events()),
            MockReply::Unauthorized => StatusCode::UNAUTHORIZED.into_response(),
            MockReply::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                [(RETRY_AFTER, "17")],
                "rate limited",
            )
                .into_response(),
            MockReply::Unavailable => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            MockReply::InvalidStream => sse_response(format!(
                "data: {}\n\ndata: {}\n\n",
                json!({
                    "type": "response.created",
                    "sequence_number": 0,
                    "response": {"id": "resp_1", "output": []}
                }),
                json!({
                    "type": "response.created",
                    "sequence_number": 0,
                    "response": {"id": "resp_other", "output": []}
                })
            )),
            MockReply::FailedStream => sse_response(format!(
                "data: {}\n\ndata: {}\n\n",
                json!({
                    "type": "response.created",
                    "sequence_number": 0,
                    "response": {"id": "resp_1", "output": []}
                }),
                json!({
                    "type": "response.failed",
                    "sequence_number": 1,
                    "response": {
                        "id": "resp_1",
                        "status": "failed",
                        "error": {"code": "server_error", "message": "stopped"}
                    }
                })
            )),
        }
    }

    fn sse_response(events: String) -> Response {
        let mut response = Body::from(events).into_response();
        response.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
        response
    }

    fn valid_text_events() -> String {
        let item_added = json!({
            "type": "message", "id": "msg_1", "role": "assistant",
            "status": "in_progress", "content": []
        });
        let item_done = json!({
            "type": "message", "id": "msg_1", "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": "ok", "annotations": []}]
        });
        [
            json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_1","output":[]}}),
            json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":item_added}),
            json!({"type":"response.content_part.added","sequence_number":2,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
            json!({"type":"response.output_text.delta","sequence_number":3,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"ok"}),
            json!({"type":"response.output_text.done","sequence_number":4,"item_id":"msg_1","output_index":0,"content_index":0,"text":"ok"}),
            json!({"type":"response.content_part.done","sequence_number":5,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"ok","annotations":[]}}),
            json!({"type":"response.output_item.done","sequence_number":6,"output_index":0,"item":item_done.clone()}),
            json!({"type":"response.completed","sequence_number":7,"response":{"id":"resp_1","output":[item_done]}}),
        ]
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
    }

    fn valid_reasoning_events() -> String {
        let reasoning_done = json!({
            "type": "reasoning",
            "id": "reasoning_1",
            "summary": [{"type": "summary_text", "text": "Plan."}],
            "encrypted_content": "provider-bound-ciphertext",
            "status": "completed"
        });
        [
            json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_1","output":[]}}),
            json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"reasoning","id":"reasoning_1","summary":[],"status":"in_progress"}}),
            json!({"type":"response.reasoning_summary_part.added","sequence_number":2,"item_id":"reasoning_1","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":""}}),
            json!({"type":"response.reasoning_summary_text.delta","sequence_number":3,"item_id":"reasoning_1","output_index":0,"summary_index":0,"delta":"Plan."}),
            json!({"type":"response.reasoning_summary_text.done","sequence_number":4,"item_id":"reasoning_1","output_index":0,"summary_index":0,"text":"Plan."}),
            json!({"type":"response.reasoning_summary_part.done","sequence_number":5,"item_id":"reasoning_1","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":"Plan."}}),
            json!({"type":"response.output_item.done","sequence_number":6,"output_index":0,"item":reasoning_done.clone()}),
            json!({"type":"response.completed","sequence_number":7,"response":{"id":"resp_1","output":[reasoning_done]}}),
        ]
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
    }

    async fn start_mock(reply: MockReply) -> (GrokClient, Arc<MockState>, JoinHandle<()>) {
        let state = Arc::new(MockState {
            reply,
            hits: AtomicUsize::new(0),
            observed: Mutex::new(Vec::new()),
        });
        let router = Router::new()
            .route("/v1/responses", upstream_post(mock_responses))
            .with_state(Arc::clone(&state));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let client =
            GrokClient::for_test(Url::parse(&format!("http://{address}/v1/")).unwrap()).unwrap();
        (client, state, task)
    }

    struct NativeMockState {
        hits: AtomicUsize,
        observed: Mutex<Vec<(HeaderMap, Vec<u8>)>>,
        response: Vec<u8>,
    }

    async fn mock_native_responses(
        AxumState(state): AxumState<Arc<NativeMockState>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        state.hits.fetch_add(1, Ordering::SeqCst);
        state
            .observed
            .lock()
            .unwrap()
            .push((headers, body.to_vec()));
        let mut response = Body::from(state.response.clone()).into_response();
        response.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
        response
            .headers_mut()
            .insert("x-native-proof", HeaderValue::from_static("raw"));
        response
    }

    async fn start_native_mock() -> (NativeClient, Arc<NativeMockState>, JoinHandle<()>) {
        let response = b"event: response.created\ndata: {\"type\":\"response.created\"}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\"}\n\n".to_vec();
        let state = Arc::new(NativeMockState {
            hits: AtomicUsize::new(0),
            observed: Mutex::new(Vec::new()),
            response,
        });
        let router = Router::new()
            .route(
                "/backend-api/codex/responses",
                upstream_post(mock_native_responses),
            )
            .with_state(Arc::clone(&state));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let client = NativeClient::for_test(
            Url::parse(&format!("http://{address}/backend-api/codex/")).unwrap(),
        )
        .unwrap();
        (client, state, task)
    }

    #[tokio::test]
    async fn native_route_preserves_compressed_request_auth_and_sse_bytes() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_native_mock().await;
        let route = NativeRouteState::new(
            crate::native::NativeUpstream::ChatgptCodex,
            ["gpt-native".to_owned()],
        )
        .unwrap();
        let app = build_router_with_services(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            None,
            Some(Arc::new(NativeService { route, client })),
            true,
        );
        let raw = br#"{ "future": {"opaque":true}, "model" : "gpt-native", "input":[] }"#;
        let compressed = zstd::stream::encode_all(Cursor::new(raw), 3).unwrap();

        let response = send_with_headers(
            app.clone(),
            TOKEN,
            &[
                ("content-encoding", "zstd"),
                ("content-type", "application/json"),
                ("authorization", "Bearer native-caller-secret"),
                ("chatgpt-account-id", "native-account"),
                ("x-codex-turn-metadata", "opaque-metadata"),
            ],
            compressed.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-native-proof"], "raw");
        let response_body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(response_body.as_ref(), mock.response.as_slice());
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
        let observed = mock.observed.lock().unwrap();
        let (headers, upstream_body) = &observed[0];
        assert_eq!(upstream_body, &compressed);
        assert_eq!(headers[CONTENT_ENCODING], "zstd");
        assert_eq!(headers["authorization"], "Bearer native-caller-secret");
        assert_eq!(headers["chatgpt-account-id"], "native-account");
        assert_eq!(headers["x-codex-turn-metadata"], "opaque-metadata");
        assert!(headers.get("x-grok-model-override").is_none());
        drop(observed);

        let websocket = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/_grok/{TOKEN}/v1/responses"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(websocket.status(), StatusCode::UPGRADE_REQUIRED);
        task.abort();
    }

    #[tokio::test]
    async fn native_route_removes_stateless_transport_fields() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_native_mock().await;
        let route = NativeRouteState::new(
            crate::native::NativeUpstream::ChatgptCodex,
            ["gpt-native".to_owned()],
        )
        .unwrap();
        let app = build_router_with_services(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            None,
            Some(Arc::new(NativeService { route, client })),
            true,
        );
        let mut request = request_body("gpt-native");
        request["prompt_cache_retention"] = json!("24h");
        request["previous_response_id"] = json!("resp_grok_store_false");
        let compressed = zstd::stream::encode_all(Cursor::new(request.to_string()), 3).unwrap();

        let response = send_with_headers(
            app,
            TOKEN,
            &[
                ("content-encoding", "zstd"),
                ("content-type", "application/json"),
                ("authorization", "Bearer native-caller-secret"),
            ],
            compressed,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let observed = mock.observed.lock().unwrap();
        let (headers, upstream_body) = &observed[0];
        assert_eq!(headers[CONTENT_ENCODING], "zstd");
        assert_eq!(headers["authorization"], "Bearer native-caller-secret");
        let decoded = zstd::stream::decode_all(Cursor::new(upstream_body)).unwrap();
        let upstream: Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(upstream.get("prompt_cache_retention"), None);
        assert_eq!(upstream.get("previous_response_id"), None);
        assert_eq!(
            upstream["prompt_cache_key"],
            "11111111-1111-4111-8111-111111111111"
        );
        drop(observed);
        task.abort();
    }

    #[tokio::test]
    async fn model_switch_native_route_sanitizes_replay_and_keeps_call_pairs() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_native_mock().await;
        let route = NativeRouteState::new(
            crate::native::NativeUpstream::ChatgptCodex,
            ["gpt-native".to_owned()],
        )
        .unwrap();
        let app = build_router_with_services(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            None,
            Some(Arc::new(NativeService { route, client })),
            true,
        );
        let mut request = request_body("gpt-native");
        request["input"] = json!([
            {
                "type": "message", "id": "msg_user", "role": "user",
                "content": [{"type": "input_text", "text": "use the plugin"}]
            },
            {
                "type": "reasoning", "id": "rs_grok",
                "summary": [], "encrypted_content": "grok-codex-bridge:v1:opaque"
            },
            {
                "type": "message", "id": "msg_grok", "role": "assistant",
                "content": [{"type": "output_text", "text": "running"}]
            },
            {
                "type": "function_call", "id": "fc_grok", "name": "mcp__demo__ping",
                "arguments": "{}", "call_id": "call-1"
            },
            {
                "type": "function_call_output", "id": "fco_grok",
                "call_id": "call-1", "output": "pong"
            },
            {
                "type": "tool_search_call",
                "id": "msg_e2a2848a-4ab3-9328-a1b4-c2318babb894",
                "call_id": "search-1", "execution": "client",
                "arguments": {"limit": 8, "query": "generate_image_grid"}
            },
            {
                "type": "tool_search_call", "id": "tsc_native",
                "call_id": "search-2", "execution": "client",
                "arguments": {"limit": 8, "query": "codex_image_grid"}
            }
        ]);
        let compressed = zstd::stream::encode_all(Cursor::new(request.to_string()), 3).unwrap();

        let response = send_with_headers(
            app,
            TOKEN,
            &[
                ("content-encoding", "zstd"),
                ("content-type", "application/json"),
                ("authorization", "Bearer native-caller-secret"),
            ],
            compressed,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let observed = mock.observed.lock().unwrap();
        let decoded = zstd::stream::decode_all(Cursor::new(&observed[0].1)).unwrap();
        let upstream: Value = serde_json::from_slice(&decoded).unwrap();
        let input = upstream["input"].as_array().unwrap();
        assert!(input.iter().all(|item| item["type"] != "reasoning"));
        assert!(
            input
                .iter()
                .filter(|item| matches!(
                    item["type"].as_str(),
                    Some("message" | "function_call" | "function_call_output")
                ))
                .all(|item| item.get("id").is_none())
        );
        let call = input
            .iter()
            .find(|item| item["type"] == "function_call")
            .unwrap();
        let output = input
            .iter()
            .find(|item| item["type"] == "function_call_output")
            .unwrap();
        assert_eq!(call["call_id"], "call-1");
        assert_eq!(output["call_id"], "call-1");
        let searches = input
            .iter()
            .filter(|item| item["type"] == "tool_search_call")
            .collect::<Vec<_>>();
        assert_eq!(searches[0].get("id"), None);
        assert_eq!(searches[0]["call_id"], "search-1");
        assert_eq!(searches[1]["id"], "tsc_native");
        drop(observed);
        task.abort();
    }

    #[tokio::test]
    async fn health_models_and_wrong_capability_do_not_read_credentials_or_hit_upstream() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let missing_auth = temporary.path().join("missing-auth.json");
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(missing_auth).unwrap(),
            client,
        );

        let health = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/_grok/{TOKEN}/healthz"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);
        let health_body = to_bytes(health.into_body(), 4096).await.unwrap();
        let health_json: Value = serde_json::from_slice(&health_body).unwrap();
        assert_eq!(health_json["status"], "ok");
        assert_eq!(health_json["service"], "grok-codex-bridge");

        let models = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/_grok/{TOKEN}/v1/models"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(models.status(), StatusCode::OK);
        let models_body = to_bytes(models.into_body(), 4096).await.unwrap();
        let models_json: Value = serde_json::from_slice(&models_body).unwrap();
        assert_eq!(models_json["data"][0]["id"], "grok-4.6");
        assert_eq!(models_json["data"][1]["id"], "grok-4.5");

        let wrong_capability =
            send(app, "not-the-token", request_body("grok-4.6").to_string()).await;
        assert_eq!(wrong_capability.status(), StatusCode::NOT_FOUND);
        assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
        task.abort();
    }

    #[tokio::test]
    async fn disabled_responses_fail_closed_before_body_credentials_or_upstream() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let app = build_router_with_responses(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            Some(Arc::new(ResponsesService {
                credentials: Arc::new(
                    CredentialStore::new(temporary.path().join("missing-auth.json")).unwrap(),
                ),
                client,
                credential_renewal_grace: Duration::ZERO,
            })),
            true,
        );

        let health = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/_grok/{TOKEN}/healthz"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);

        let models = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/_grok/{TOKEN}/v1/models"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(models.status(), StatusCode::OK);

        let wrong_capability = send(app.clone(), "not-the-token", "{").await;
        assert_eq!(wrong_capability.status(), StatusCode::NOT_FOUND);

        let disabled = send(app, TOKEN, "{").await;
        assert_eq!(disabled.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(disabled.into_body(), 4096).await.unwrap();
        let error: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"]["type"], "server_error");
        assert_eq!(error["error"]["code"], "bridge_disabled");
        assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
        task.abort();
    }

    #[tokio::test]
    async fn valid_text_request_reaches_upstream_and_returns_validated_sse() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(write_auth(temporary.path())).unwrap(),
            client,
        );
        let mut request = request_body("grok-4.6");
        request["prompt_cache_retention"] = json!("24h");
        request["previous_response_id"] = json!("resp_previous");

        let response = send_with_headers(
            app,
            TOKEN,
            &[
                ("authorization", "Bearer native-caller-secret"),
                ("chatgpt-account-id", "native-account"),
            ],
            request.to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "text/event-stream; charset=utf-8"
        );
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(body.contains("event: response.created\ndata: {"));
        assert!(body.contains("event: response.output_text.delta\ndata: {"));
        assert!(body.contains("event: response.completed\ndata: {"));
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
        let observed = mock.observed.lock().unwrap();
        let (headers, upstream_body) = &observed[0];
        assert_eq!(headers["authorization"], "Bearer mock-session-secret");
        assert!(headers.get("chatgpt-account-id").is_none());
        assert_eq!(headers["x-grok-model-override"], "grok-4.6");
        assert_eq!(headers["x-grok-conv-id"], headers["x-grok-session-id"]);
        assert!(Uuid::parse_str(headers["x-grok-conv-id"].to_str().unwrap()).is_ok());
        assert!(Uuid::parse_str(headers["x-grok-req-id"].to_str().unwrap()).is_ok());
        assert_eq!(headers["x-grok-turn-idx"], "1");
        assert!(Uuid::parse_str(headers["x-grok-agent-id"].to_str().unwrap()).is_ok());
        let mut expected_upstream = request;
        let expected_object = expected_upstream.as_object_mut().unwrap();
        expected_object.remove("client_metadata");
        expected_object.remove("tool_choice");
        expected_object.remove("prompt_cache_retention");
        expected_object.remove("previous_response_id");
        assert_eq!(upstream_body, &expected_upstream);
        drop(observed);
        task.abort();
    }

    #[tokio::test]
    async fn missing_codex_routing_metadata_uses_a_stable_history_anchor() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(write_auth(temporary.path())).unwrap(),
            client,
        );

        let mut request = request_body("grok-4.6");
        request.as_object_mut().unwrap().remove("prompt_cache_key");
        request.as_object_mut().unwrap().remove("client_metadata");
        let response = send(app, TOKEN, request.to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), 64 * 1024).await.unwrap();

        let observed = mock.observed.lock().unwrap();
        assert_eq!(observed.len(), 1);
        let headers = &observed[0].0;
        assert_eq!(headers["x-grok-conv-id"], headers["x-grok-session-id"]);
        assert_eq!(headers["x-grok-turn-idx"], "1");
        assert!(Uuid::parse_str(headers["x-grok-conv-id"].to_str().unwrap()).is_ok());
        assert!(observed[0].1.get("client_metadata").is_none());
        drop(observed);
        task.abort();
    }

    #[tokio::test]
    async fn model_switch_marks_grok_reasoning_for_provider_safe_replay() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Reasoning).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(write_auth(temporary.path())).unwrap(),
            client,
        );

        let response = send(app, TOKEN, request_body("grok-4.6").to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let body = std::str::from_utf8(&body).unwrap();

        let events = body
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|data| serde_json::from_str::<Value>(data).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 8);
        assert_eq!(events[6]["item"]["type"], "reasoning");
        assert_eq!(events[6]["item"]["summary"][0]["text"], "Plan.");
        assert_eq!(
            events[6]["item"]["encrypted_content"],
            "grok-codex-bridge:v1:provider-bound-ciphertext"
        );
        assert_eq!(events[7]["response"]["output"][0]["type"], "reasoning");
        assert_eq!(
            events[7]["response"]["output"][0]["summary"][0]["text"],
            "Plan."
        );
        assert_eq!(
            events[7]["response"]["output"][0]["encrypted_content"],
            "grok-codex-bridge:v1:provider-bound-ciphertext"
        );
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
        task.abort();
    }

    #[tokio::test]
    async fn model_switch_removes_grok_reasoning_before_native_gpt() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_native_mock().await;
        let route = NativeRouteState::new(
            crate::native::NativeUpstream::ChatgptCodex,
            ["gpt-native".to_owned()],
        )
        .unwrap();
        let app = build_router_with_services(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            None,
            Some(Arc::new(NativeService { route, client })),
            true,
        );
        let mut request = request_body("gpt-native");
        request["input"] = json!([
            {
                "type": "reasoning", "id": "rs_grok",
                "summary": [], "encrypted_content": "grok-codex-bridge:v1:grok-ciphertext"
            },
            {
                "type": "reasoning", "id": "rs_legacy_grok",
                "summary": [], "encrypted_content": null
            },
            {
                "type": "reasoning", "id": "rs_native",
                "summary": [], "encrypted_content": "native-gpt-ciphertext"
            },
            {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "continue"}]
            }
        ]);
        let compressed = zstd::stream::encode_all(Cursor::new(request.to_string()), 3).unwrap();

        let response = send_with_headers(
            app,
            TOKEN,
            &[
                ("content-encoding", "zstd"),
                ("content-type", "application/json"),
                ("authorization", "Bearer native-caller-secret"),
            ],
            compressed,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let observed = mock.observed.lock().unwrap();
        let (headers, upstream_body) = &observed[0];
        assert_eq!(headers[CONTENT_ENCODING], "zstd");
        let decoded = zstd::stream::decode_all(Cursor::new(upstream_body)).unwrap();
        let upstream: Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(upstream["input"].as_array().unwrap().len(), 1);
        assert_eq!(upstream["input"][0]["content"][0]["text"], "continue");
        drop(observed);
        task.abort();
    }

    #[tokio::test]
    async fn picker_header_authenticates_v1_1_without_a_capability_url_and_preserves_v1_0_path() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(write_auth(temporary.path())).unwrap(),
            client,
        );

        let accepted = send_picker_with_headers(
            app.clone(),
            &[(PICKER_CALLER_HEADER, TOKEN)],
            request_body("grok-4.6").to_string(),
        )
        .await;
        assert_eq!(accepted.status(), StatusCode::OK);
        to_bytes(accepted.into_body(), 64 * 1024).await.unwrap();

        let legacy = send(app.clone(), TOKEN, request_body("grok-4.6").to_string()).await;
        assert_eq!(legacy.status(), StatusCode::OK);
        to_bytes(legacy.into_body(), 64 * 1024).await.unwrap();

        let missing =
            send_picker_with_headers(app.clone(), &[], request_body("grok-4.6").to_string()).await;
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let wrong = send_picker_with_headers(
            app,
            &[(PICKER_CALLER_HEADER, "not-the-token")],
            request_body("grok-4.6").to_string(),
        )
        .await;
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(mock.hits.load(Ordering::SeqCst), 2);
        task.abort();
    }

    #[tokio::test]
    async fn codex_cached_web_search_reaches_grok_as_bare_hosted_tool() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(write_auth(temporary.path())).unwrap(),
            client,
        );
        let mut request = request_body("grok-4.6");
        request["tools"] = json!([
            {
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
            },
            {
                "type": "web_search",
                "external_web_access": false
            }
        ]);

        let response = send(app, TOKEN, request.to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), 64 * 1024).await.unwrap();

        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
        let observed = mock.observed.lock().unwrap();
        assert_eq!(
            observed[0].1["tools"],
            json!([
                {
                    "type": "function",
                    "name": "mcp__demo__ping",
                    "description": "",
                    "strict": false,
                    "parameters": {"type": "object", "properties": {}}
                },
                {"type": "web_search"}
            ])
        );
        drop(observed);
        task.abort();
    }

    #[tokio::test]
    async fn codex_openai_message_metadata_is_not_sent_to_grok() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(write_auth(temporary.path())).unwrap(),
            client,
        );
        let mut request = request_body("grok-4.6");
        request["input"] = json!([
            {
                "type": "message",
                "id": "msg_context",
                "role": "developer",
                "content": [{"type": "input_text", "text": "context"}],
                "internal_chat_message_metadata_passthrough": {
                    "turn_id": "11111111-1111-4111-8111-111111111111",
                    "create_time": 1787079600.125
                }
            },
            {
                "type": "message",
                "id": "msg_user",
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}],
                "internal_chat_message_metadata_passthrough": {
                    "turn_id": "11111111-1111-4111-8111-111111111111",
                    "create_time": 1787079600.25
                }
            }
        ]);

        let response = send_with_headers(
            app,
            TOKEN,
            &[("authorization", "Bearer native-caller-secret")],
            request.to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), 64 * 1024).await.unwrap();

        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
        let observed = mock.observed.lock().unwrap();
        let (headers, upstream_body) = &observed[0];
        assert_eq!(headers["authorization"], "Bearer mock-session-secret");
        assert_ne!(headers["authorization"], "Bearer native-caller-secret");
        assert_eq!(
            upstream_body["input"],
            json!([
                {
                    "type": "message",
                    "role": "developer",
                    "content": [{"type": "input_text", "text": "context"}]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "hello"}]
                }
            ])
        );
        drop(observed);
        task.abort();
    }

    #[tokio::test]
    async fn codex_tool_followup_reuses_official_grok_conversation_and_turn_headers() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(write_auth(temporary.path())).unwrap(),
            client,
        );

        let first = request_body("grok-4.6");
        let first_response = send(app.clone(), TOKEN, first.to_string()).await;
        assert_eq!(first_response.status(), StatusCode::OK);
        to_bytes(first_response.into_body(), 64 * 1024)
            .await
            .unwrap();

        let mut second = request_body("grok-4.6");
        second["input"] = json!([
            {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "hello"}]
            },
            {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "run the marker"}]
            },
            {
                "type": "reasoning", "id": "reasoning_1",
                "summary": [{"type": "summary_text", "text": "Use the tool."}],
                "content": null,
                "encrypted_content": "opaque-state"
            },
            {
                "type": "message", "id": "msg_1", "role": "assistant",
                "content": [{"type": "output_text", "text": "Checking."}]
            },
            {
                "type": "function_call", "id": "fc_1", "name": "shell",
                "arguments": "{\"command\":\"true\"}", "call_id": "call_1"
            },
            {
                "type": "function_call_output", "id": "fco_1", "call_id": "call_1",
                "output": "Exit code: 0"
            }
        ]);
        let second_response = send(app, TOKEN, second.to_string()).await;
        assert_eq!(second_response.status(), StatusCode::OK);
        to_bytes(second_response.into_body(), 64 * 1024)
            .await
            .unwrap();

        let observed = mock.observed.lock().unwrap();
        assert_eq!(observed.len(), 2);
        let first_headers = &observed[0].0;
        let second_headers = &observed[1].0;
        for name in ["x-grok-conv-id", "x-grok-session-id"] {
            assert_eq!(first_headers[name], second_headers[name]);
        }
        assert_ne!(first_headers["x-grok-req-id"], second_headers["x-grok-req-id"]);
        assert_eq!(first_headers["x-grok-turn-idx"], "1");
        assert_eq!(second_headers["x-grok-turn-idx"], "1");
        assert_ne!(first_headers["x-grok-agent-id"], second_headers["x-grok-agent-id"]);
        assert!(
            observed
                .iter()
                .all(|(_, body)| body.get("client_metadata").is_none())
        );
        let replay = observed[1].1["input"].as_array().unwrap();
        assert_eq!(replay.len(), 5);
        assert_eq!(replay[0]["content"][0]["text"], "hello");
        assert_eq!(replay[1]["content"][0]["text"], "run the marker");
        assert!(replay[2].get("id").is_none());
        assert_eq!(replay[3]["call_id"], "call_1");
        assert!(replay[4].get("id").is_none());
        assert_eq!(replay[4]["call_id"], "call_1");
        drop(observed);
        task.abort();
    }

    #[tokio::test]
    async fn codex_routing_metadata_does_not_block_upstream() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(write_auth(temporary.path())).unwrap(),
            client,
        );

        let mut malformed_prompt = request_body("grok-4.6");
        malformed_prompt["prompt_cache_key"] = json!("not-a-uuid");
        let response = send(app.clone(), TOKEN, malformed_prompt.to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);

        let mut mismatched = request_body("grok-4.6");
        mismatched["client_metadata"]["session_id"] = json!("33333333-3333-4333-8333-333333333333");
        let response = send(app.clone(), TOKEN, mismatched.to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);

        let mut malformed_turn = request_body("grok-4.6");
        malformed_turn["client_metadata"]["turn_id"] = json!("not-a-uuid");
        let response = send(app.clone(), TOKEN, malformed_turn.to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);

        let mut malformed_agent = request_body("grok-4.6");
        malformed_agent["client_metadata"]["x-codex-installation-id"] = json!("not-a-uuid");
        let response = send(app, TOKEN, malformed_agent.to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);

        assert_eq!(mock.hits.load(Ordering::SeqCst), 4);
        task.abort();
    }

    #[tokio::test]
    async fn malformed_and_unknown_model_requests_stop_before_credential_or_upstream() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(temporary.path().join("missing-auth.json")).unwrap(),
            client,
        );

        let malformed = send(app.clone(), TOKEN, "{").await;
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        let malformed_body = to_bytes(malformed.into_body(), 4096).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&malformed_body).unwrap()["error"]["code"],
            "invalid_json"
        );

        let unknown = send(app, TOKEN, request_body("grok-9.9").to_string()).await;
        assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
        let unknown_body = to_bytes(unknown.into_body(), 4096).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&unknown_body).unwrap()["error"]["code"],
            "model_not_admitted"
        );
        assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
        task.abort();
    }

    #[tokio::test]
    async fn missing_local_credential_maps_to_login_safe_401() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(temporary.path().join("missing-auth.json")).unwrap(),
            client,
        );
        let response = send(app, TOKEN, request_body("grok-4.6").to_string()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
            "grok_login_required"
        );
        assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
        task.abort();
    }

    #[tokio::test]
    async fn expired_local_credential_waits_for_official_renewal_before_upstream() {
        let temporary = tempfile::tempdir().unwrap();
        let auth_path = temporary.path().join("auth.json");
        fs::write(
            &auth_path,
            br#"{"https://auth.x.ai::current-client":{"key":"expired","auth_mode":"oidc","create_time":"2019-01-01T00:00:00Z","user_id":"user-1","expires_at":"2020-01-01T00:00:00Z","oidc_issuer":"https://auth.x.ai"}}"#,
        )
        .unwrap();
        fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o600)).unwrap();

        let (client, mock, task) = start_mock(MockReply::Valid).await;
        let app = test_app_with_renewal_grace(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(auth_path.clone()).unwrap(),
            client,
            Duration::from_secs(1),
        );

        let renewed_path = temporary.path().join("auth.json.renewed");
        let renew = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            fs::write(
                &renewed_path,
                br#"{"https://auth.x.ai::current-client":{"key":"renewed-session-secret","auth_mode":"oidc","create_time":"2026-08-21T00:00:00Z","user_id":"user-1","expires_at":"2099-01-01T00:00:00Z","oidc_issuer":"https://auth.x.ai"}}"#,
            )
            .unwrap();
            fs::set_permissions(&renewed_path, fs::Permissions::from_mode(0o600)).unwrap();
            fs::rename(renewed_path, auth_path).unwrap();
        });

        let response = send(app, TOKEN, request_body("grok-4.6").to_string()).await;
        renew.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            mock.observed.lock().unwrap()[0].0["authorization"],
            "Bearer renewed-session-secret"
        );
        task.abort();
    }

    #[tokio::test]
    async fn upstream_http_boundary_auth_rate_limit_and_server_errors_are_typed() {
        for (reply, expected_status, expected_code, retry_after) in [
            (
                MockReply::Unauthorized,
                StatusCode::UNAUTHORIZED,
                "grok_login_required",
                None,
            ),
            (
                MockReply::RateLimited,
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                Some("17"),
            ),
            (
                MockReply::Unavailable,
                StatusCode::SERVICE_UNAVAILABLE,
                "upstream_unavailable",
                None,
            ),
        ] {
            let temporary = tempfile::tempdir().unwrap();
            let (client, mock, task) = start_mock(reply).await;
            let app = test_app(
                runtime_config(temporary.path()),
                ModelCatalog::bootstrap().unwrap(),
                CredentialStore::new(write_auth(temporary.path())).unwrap(),
                client,
            );

            let response = send(app, TOKEN, request_body("grok-4.6").to_string()).await;
            assert_eq!(response.status(), expected_status);
            assert_eq!(
                response
                    .headers()
                    .get(X_GROK_UPSTREAM_STATUS)
                    .and_then(|value| value.to_str().ok()),
                matches!(reply, MockReply::Unavailable).then_some("500")
            );
            assert_eq!(
                response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|value| value.to_str().ok()),
                retry_after
            );
            let body = to_bytes(response.into_body(), 4096).await.unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
                expected_code
            );
            assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
            task.abort();
        }
    }

    #[tokio::test]
    async fn unknown_upstream_event_is_forwarded_through_permissive_boundary() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::InvalidStream).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(write_auth(temporary.path())).unwrap(),
            client,
        );

        let response = send(app, TOKEN, request_body("grok-4.6").to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        let valid = body.next().await.unwrap().unwrap();
        let valid = std::str::from_utf8(&valid).unwrap();
        assert!(valid.contains("event: response.created"));
        assert!(valid.contains("resp_other") || valid.contains("response.created"));
        while let Some(chunk) = body.next().await {
            assert!(chunk.is_ok());
        }
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
        task.abort();
    }

    #[tokio::test]
    async fn failed_upstream_event_is_forwarded_as_a_terminal_sse_event() {
        let temporary = tempfile::tempdir().unwrap();
        let (client, mock, task) = start_mock(MockReply::FailedStream).await;
        let app = test_app(
            runtime_config(temporary.path()),
            ModelCatalog::bootstrap().unwrap(),
            CredentialStore::new(write_auth(temporary.path())).unwrap(),
            client,
        );

        let response = send(app, TOKEN, request_body("grok-4.6").to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        let created = body.next().await.unwrap().unwrap();
        assert!(
            std::str::from_utf8(&created)
                .unwrap()
                .contains("event: response.created")
        );
        let failed = body.next().await.unwrap().unwrap();
        let failed = std::str::from_utf8(&failed).unwrap();
        assert!(failed.contains("event: response.failed"));
        assert!(failed.contains("\"status\":\"failed\""));
        assert!(body.next().await.is_none());
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
        task.abort();
    }

    #[test]
    fn upstream_http_boundary_and_stream_error_classifiers_are_distinct() {
        assert_eq!(
            stream_error_class(&GrokError::Protocol(
                crate::protocol::ProtocolError::InvalidSsePayload
            )),
            "upstream_stream_protocol"
        );
        assert_eq!(
            stream_error_class(&GrokError::InvalidSseFraming),
            "upstream_stream_framing"
        );

        let transport = reqwest::Client::new()
            .get("not a valid absolute URL")
            .build()
            .unwrap_err();
        assert_eq!(
            stream_error_class(&GrokError::Stream(transport)),
            "upstream_stream_transport"
        );
        assert_eq!(
            stream_error_class(&GrokError::ModelsTimeout),
            "upstream_stream_other"
        );
    }

    #[test]
    fn upstream_http_boundary_classifier_is_safe_and_distinct() {
        assert_eq!(
            GrokError::UpstreamStatus(418).response_boundary_class(),
            "upstream_http_status"
        );
        assert_eq!(
            GrokError::UnexpectedResponseContentType.response_boundary_class(),
            "upstream_content_type"
        );
        assert_eq!(
            stream_error_class(&GrokError::Protocol(
                crate::protocol::ProtocolError::InvalidSsePayload
            )),
            "upstream_stream_protocol"
        );
    }
}
