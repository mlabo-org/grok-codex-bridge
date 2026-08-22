use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use uuid::Uuid;

const MAX_CLIENT_METADATA_ENTRIES: usize = 64;
const MAX_CLIENT_METADATA_KEY_BYTES: usize = 256;
const MAX_CLIENT_METADATA_VALUE_BYTES: usize = 512 * 1024;
const MAX_CLIENT_METADATA_TOTAL_VALUE_BYTES: usize = 4 * 1024 * 1024;
const MAX_REASONING_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_ENCRYPTED_REASONING_BYTES: usize = 16 * 1024 * 1024;
const MAX_REASONING_PARTS: usize = 64;
const GROK_REASONING_ENVELOPE_PREFIX: &str = "grok-codex-bridge:v1:";
const NAMESPACE_DELIMITER: &str = "__";
const MAX_XAI_TOOLS: usize = 128;
const INTERNAL_MESSAGE_METADATA_FIELD: &str = "internal_chat_message_metadata_passthrough";
const MAX_TOOL_SCHEMA_DEPTH: usize = 8;
const MAX_TOOL_SCHEMA_LITERAL_DEPTH: usize = 32;
const TOOL_SCHEMA_UNION_KEYWORDS: &[&str] = &["anyOf", "oneOf", "allOf"];
const TOOL_SCHEMA_MAP_KEYWORDS: &[&str] =
    &["properties", "patternProperties", "$defs", "definitions"];
const TOOL_SCHEMA_LIST_KEYWORDS: &[&str] = &["anyOf", "oneOf", "allOf", "prefixItems"];
const TOOL_SCHEMA_CHILD_KEYWORDS: &[&str] = &[
    "items",
    "additionalProperties",
    "contains",
    "not",
    "if",
    "then",
    "else",
    "propertyNames",
];

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("responses request must be a JSON object")]
    RequestNotObject,
    #[error("responses field `{0}` is missing")]
    MissingRequestField(&'static str),
    #[error("responses field `{0}` is invalid")]
    InvalidRequestField(&'static str),
    #[error("responses input item type is unsupported in this phase")]
    UnsupportedInputItem,
    #[error("responses content type is unsupported in this phase")]
    UnsupportedContent,
    #[error("responses image URL is invalid or unsupported")]
    InvalidImageUrl,
    #[error("responses tool kind or field is unsupported in this phase")]
    UnsupportedTools,
    #[error("responses tool name must be unique")]
    DuplicateToolName,
    #[error("responses request exceeds the upstream tool limit")]
    TooManyTools,
    #[error("responses tool choice is unsupported in this phase")]
    UnsupportedToolChoice,
    #[error("responses specific tool choice does not reference an admitted tool")]
    UnknownToolChoice,
    #[error("responses function call arguments must encode a JSON object")]
    InvalidFunctionArguments,
    #[error("responses function output has no earlier matching call")]
    UnmatchedFunctionOutput,
    #[error("SSE data must be a valid JSON object")]
    InvalidSsePayload,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedResponsesRequest {
    model: String,
    instructions: Option<String>,
    input: Vec<InputItem>,
    tools: ToolsField,
    tool_choice: ToolChoice,
    parallel_tool_calls: bool,
    reasoning: OptionalJson,
    store: bool,
    stream: bool,
    stream_options: OptionalJson,
    include: Vec<String>,
    service_tier: OptionalJson,
    prompt_cache_key: OptionalJson,
    // These are Codex-side state/transport fields. They are validated as
    // scalar strings so a current request can be replayed, then terminate at
    // the bridge boundary because this route sends the full input history.
    _prompt_cache_retention: OptionalJson,
    _previous_response_id: OptionalJson,
    text: OptionalJson,
    _client_metadata: ClientMetadata,
    namespace_projection: NamespaceToolProjection,
}

#[derive(Debug, Clone, PartialEq)]
enum InputItem {
    Message(TextMessage),
    Reasoning(PriorReasoning),
    FunctionCall(FunctionCall),
    FunctionCallOutput(FunctionCallOutput),
    ToolSearchCall(ToolSearchCall),
    ToolSearchOutput(ToolSearchOutput),
    ForeignAgentMessage,
    ForeignCustomToolCall,
    ForeignCustomToolCallOutput,
}

#[derive(Debug, Clone, PartialEq)]
struct PriorReasoning {
    id: String,
    summary: Vec<String>,
    content: PriorReasoningContent,
    encrypted_content: PriorEncryptedContent,
}

#[derive(Debug, Clone, PartialEq)]
enum PriorReasoningContent {
    Absent,
    Null,
    Parts(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
enum PriorEncryptedContent {
    Null,
    Grok(String),
    Foreign(String),
}

#[derive(Debug, Clone, PartialEq)]
struct FunctionCall {
    name: String,
    arguments: String,
    call_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeToolName {
    namespace: String,
    name: String,
}

/// Request-local map between Codex's native namespace tool identity and the
/// flat function identity accepted by the Grok Responses endpoint.
///
/// Namespace strings themselves can contain `__`, so response restoration
/// always uses this exact map and never tries to split a provider name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NamespaceToolProjection {
    provider_to_native: HashMap<String, NativeToolName>,
    native_to_provider: HashMap<(String, String), String>,
    tool_search_provider_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct FunctionCallOutput {
    call_id: String,
    output: FunctionCallOutputBody,
}

#[derive(Debug, Clone, PartialEq)]
struct ToolSearchCall {
    call_id: String,
    arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
struct ToolSearchOutput {
    call_id: String,
    tools: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
struct TextMessage {
    role: MessageRole,
    content: Vec<MessageContent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageRole {
    User,
    Developer,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
enum MessageContent {
    Input(InputContent),
    OutputText(String),
}

#[derive(Debug, Clone, PartialEq)]
enum InputContent {
    InputText(String),
    InputImage(InputImage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputImage {
    image_url: String,
    detail: Option<ImageDetail>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageDetail {
    Auto,
    Low,
    High,
    Original,
}

#[derive(Debug, Clone, PartialEq)]
enum FunctionCallOutputBody {
    Text(String),
    Content(Vec<InputContent>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ToolChoice {
    Auto,
    None,
    Required,
    Function(String),
}

#[derive(Debug, Clone, PartialEq)]
enum ToolsField {
    Absent,
    Null,
    List(Vec<ProjectedTool>),
}

#[derive(Debug, Clone, PartialEq)]
enum ProjectedTool {
    Function(FunctionTool),
    HostedWebSearch,
}

#[derive(Debug, Clone, PartialEq)]
struct FunctionTool {
    name: String,
    description: String,
    parameters: Map<String, Value>,
    strict: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
enum OptionalJson {
    Absent,
    Present(Value),
}

#[derive(Debug, Clone, PartialEq)]
enum ClientMetadata {
    Absent,
    Values(Vec<(String, String)>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrokRoutingMetadata {
    conversation_id: Uuid,
    request_id: Uuid,
    agent_id: Uuid,
    turn_index: usize,
}

impl GrokRoutingMetadata {
    pub fn conversation_id(self) -> Uuid {
        self.conversation_id
    }

    pub fn request_id(self) -> Uuid {
        self.request_id
    }

    pub fn agent_id(self) -> Uuid {
        self.agent_id
    }

    pub fn turn_index(self) -> usize {
        self.turn_index
    }
}

impl NormalizedResponsesRequest {
    pub fn parse(value: Value) -> Result<Self, ProtocolError> {
        let object = value.as_object().ok_or(ProtocolError::RequestNotObject)?;
        // Codex adds transport and tracing metadata as the request crosses
        // versions.  The bridge is a projection boundary: unknown fields are
        // deliberately ignored and only fields needed by xAI are serialized
        // below.  In particular, do not make the upstream contract a closed
        // world copy of the Responses schema.

        let model = required_nonempty_string(object, "model")?;
        let instructions = optional_string(object, "instructions")?;
        let (mut tools, mut namespace_projection) = parse_tools(object.get("tools"))?;
        let input_values = required_array(object, "input")?;
        // Discovery output is part of the request history.  Project it before
        // parsing later function calls so a namespace discovered in an earlier
        // tool_search turn is available to the same request's replayed call.
        merge_discovered_tool_search_specs(&mut tools, &mut namespace_projection, input_values)?;
        let input = parse_input(input_values, &mut namespace_projection)?;
        let tool_choice = parse_tool_choice(required_value(object, "tool_choice")?)?;
        validate_tool_choice(&tool_choice, &tools)?;
        let parallel_tool_calls = required_bool(object, "parallel_tool_calls")?;
        let reasoning = optional_object(object, "reasoning")?;
        let store = required_bool(object, "store")?;
        if store {
            return Err(ProtocolError::InvalidRequestField("store"));
        }
        let stream = required_bool(object, "stream")?;
        if !stream {
            return Err(ProtocolError::InvalidRequestField("stream"));
        }
        let stream_options = optional_object(object, "stream_options")?;
        let include = required_string_array(object, "include")?;
        let service_tier = optional_scalar_string(object, "service_tier")?;
        let prompt_cache_key = optional_scalar_string(object, "prompt_cache_key")?;
        let prompt_cache_retention = optional_scalar_string(object, "prompt_cache_retention")?;
        let previous_response_id = optional_scalar_string(object, "previous_response_id")?;
        let text = optional_object(object, "text")?;
        let client_metadata = parse_client_metadata(object)?;

        Ok(Self {
            model,
            instructions,
            input,
            tools,
            tool_choice,
            parallel_tool_calls,
            reasoning,
            store,
            stream,
            stream_options,
            include,
            service_tier,
            prompt_cache_key,
            _prompt_cache_retention: prompt_cache_retention,
            _previous_response_id: previous_response_id,
            text,
            _client_metadata: client_metadata,
            namespace_projection,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn namespace_projection(&self) -> NamespaceToolProjection {
        self.namespace_projection.clone()
    }

    pub fn grok_routing_metadata(&self) -> Result<GrokRoutingMetadata, ProtocolError> {
        // Routing is deliberately independent from Codex transport metadata.
        // The full replay determines the stable conversation; each wire
        // request gets fresh request and agent identities.
        let conversation_id = self.derived_conversation_id();
        let request_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();
        // Grok's prompt index advances once per prompt, not once per wire-level
        // user item. Codex can emit consecutive synthetic environment-context
        // and actual-prompt user items for one prompt, while model output marks
        // the boundary before the next prompt block.
        // Full-history requests are always the first wire turn from Grok's
        // perspective.  Deriving this from the replay tail caused old tool
        // search failures and synthetic user blocks to change routing.
        let turn_index = 1;
        Ok(GrokRoutingMetadata {
            conversation_id,
            request_id,
            agent_id,
            turn_index,
        })
    }

    fn derived_conversation_id(&self) -> Uuid {
        // codex-router anchors direct Responses requests on the opening
        // instruction/message pair.  Never let later replay items change the
        // conversation UUID when a follow-up contains additional history.
        let mut anchor = Vec::with_capacity(2);
        if let Some(instructions) = &self.instructions {
            anchor.push(serde_json::json!({
                "type": "message",
                "role": "developer",
                "content": [{"type": "input_text", "text": instructions}]
            }));
        }
        if let Some(first) = self.input.iter().find_map(InputItem::to_grok_value) {
            anchor.push(first);
        }
        if anchor.is_empty() {
            return Uuid::new_v4();
        }

        // xAI routes its cache by conversation UUID. Hash only the opening
        // replay items, not the growing tail, so every full-history turn of
        // one Codex conversation lands on the same upstream conversation.
        let digest = Sha256::digest(Value::Array(anchor).to_string().as_bytes());
        let mut bytes = [0_u8; 16];
        let byte_count = bytes.len();
        bytes.copy_from_slice(&digest[..byte_count]);
        // Use UUID v5/ RFC 9562-compatible variant bits for a deterministic
        // identifier while keeping the value in the UUID header contract.
        bytes[6] = (bytes[6] & 0x0f) | 0x50;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Uuid::from_bytes(bytes)
    }

    pub fn to_xai_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("model".into(), Value::String(self.model.clone()));
        if let Some(instructions) = &self.instructions {
            object.insert("instructions".into(), Value::String(instructions.clone()));
        }
        object.insert(
            "input".into(),
            Value::Array(
                self.input
                    .iter()
                    .filter_map(InputItem::to_grok_value)
                    .collect(),
            ),
        );
        let projected_tools = project_tools_for_xai(&self.tools);
        if let Some(tools) = projected_tools {
            let include_tool_choice = match &tools {
                Value::Array(items) => !items.is_empty(),
                _ => false,
            };
            object.insert("tools".into(), tools);
            // xAI rejects `tool_choice` when no projected tool is present, even
            // though Codex requires and validates the field on its side.
            if include_tool_choice {
                object.insert("tool_choice".into(), self.tool_choice.to_value());
            }
        }
        object.insert(
            "parallel_tool_calls".into(),
            Value::Bool(self.parallel_tool_calls),
        );
        insert_optional(&mut object, "reasoning", &self.reasoning);
        object.insert("store".into(), Value::Bool(self.store));
        object.insert("stream".into(), Value::Bool(self.stream));
        insert_optional(&mut object, "stream_options", &self.stream_options);
        object.insert(
            "include".into(),
            Value::Array(self.include.iter().cloned().map(Value::String).collect()),
        );
        insert_optional(&mut object, "service_tier", &self.service_tier);
        insert_optional(&mut object, "prompt_cache_key", &self.prompt_cache_key);
        insert_optional(&mut object, "text", &self.text);
        // `prompt_cache_retention` and `previous_response_id` belong to the
        // Codex-side stateful transport. This bridge intentionally sends a
        // stateless full-history request to Grok, so neither field is valid
        // upstream and both terminate here.
        // `client_metadata` is a Codex transport/session field. The current xAI
        // Responses request has no corresponding field, so validated metadata
        // intentionally terminates at the bridge boundary.
        Value::Object(object)
    }

    pub fn into_xai_value(self) -> Value {
        self.to_xai_value()
    }
}

impl TryFrom<Value> for NormalizedResponsesRequest {
    type Error = ProtocolError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl InputItem {
    fn parse(
        value: &Value,
        namespace_projection: &mut NamespaceToolProjection,
    ) -> Result<Self, ProtocolError> {
        let object = value
            .as_object()
            .ok_or(ProtocolError::InvalidRequestField("input"))?;
        match required_string(object, "type")? {
            "message" => {
                reject_unknown_keys(
                    object,
                    &[
                        "type",
                        "id",
                        "role",
                        "content",
                        "phase",
                        INTERNAL_MESSAGE_METADATA_FIELD,
                    ],
                )?;
                validate_internal_message_metadata(object)?;
                Ok(Self::Message(TextMessage::parse(object)?))
            }
            "reasoning" => {
                reject_unknown_keys(
                    object,
                    &[
                        "type",
                        "id",
                        "summary",
                        "content",
                        "encrypted_content",
                        INTERNAL_MESSAGE_METADATA_FIELD,
                    ],
                )?;
                validate_internal_message_metadata(object)?;
                Ok(Self::Reasoning(PriorReasoning::parse(object)?))
            }
            "agent_message" => {
                reject_unknown_keys(
                    object,
                    &[
                        "type",
                        "id",
                        "author",
                        "recipient",
                        "content",
                        INTERNAL_MESSAGE_METADATA_FIELD,
                    ],
                )?;
                validate_internal_message_metadata(object)?;
                match parse_agent_message(object)? {
                    Some(message) => Ok(Self::Message(message)),
                    None => Ok(Self::ForeignAgentMessage),
                }
            }
            "function_call" => {
                reject_unknown_keys(
                    object,
                    &[
                        "type",
                        "id",
                        "name",
                        "namespace",
                        "arguments",
                        "call_id",
                        INTERNAL_MESSAGE_METADATA_FIELD,
                    ],
                )?;
                validate_internal_message_metadata(object)?;
                Ok(Self::FunctionCall(FunctionCall::parse(
                    object,
                    namespace_projection,
                )?))
            }
            "function_call_output" => {
                reject_unknown_keys(
                    object,
                    &[
                        "type",
                        "id",
                        "call_id",
                        "output",
                        INTERNAL_MESSAGE_METADATA_FIELD,
                    ],
                )?;
                validate_internal_message_metadata(object)?;
                Ok(Self::FunctionCallOutput(FunctionCallOutput::parse(object)?))
            }
            "tool_search_call" => {
                reject_unknown_keys(
                    object,
                    &[
                        "type",
                        "id",
                        "call_id",
                        "execution",
                        "arguments",
                        INTERNAL_MESSAGE_METADATA_FIELD,
                    ],
                )?;
                optional_nonempty_string(object, "id")?;
                validate_internal_message_metadata(object)?;
                if required_string(object, "execution")? != "client" {
                    return Err(ProtocolError::InvalidRequestField("execution"));
                }
                let mut arguments = required_value(object, "arguments")?.clone();
                if !arguments.is_object() {
                    return Err(ProtocolError::InvalidFunctionArguments);
                }
                canonicalize_integer_valued_numbers(&mut arguments);
                Ok(Self::ToolSearchCall(ToolSearchCall {
                    call_id: required_nonempty_string(object, "call_id")?.to_owned(),
                    arguments,
                }))
            }
            "tool_search_output" => {
                reject_unknown_keys(object, &["type", "call_id", "status", "execution", "tools"])?;
                if required_string(object, "status")? != "completed"
                    || required_string(object, "execution")? != "client"
                {
                    return Err(ProtocolError::InvalidRequestField("tool_search_output"));
                }
                let tools = required_array(object, "tools")?.to_vec();
                if tools.iter().any(|tool| !tool.is_object()) {
                    return Err(ProtocolError::UnsupportedTools);
                }
                Ok(Self::ToolSearchOutput(ToolSearchOutput {
                    call_id: required_nonempty_string(object, "call_id")?.to_owned(),
                    tools,
                }))
            }
            "custom_tool_call" => {
                reject_unknown_keys(
                    object,
                    &[
                        "type",
                        "id",
                        "status",
                        "call_id",
                        "name",
                        "namespace",
                        "input",
                        INTERNAL_MESSAGE_METADATA_FIELD,
                    ],
                )?;
                validate_internal_message_metadata(object)?;
                optional_nonempty_string(object, "id")?;
                match object.get("status") {
                    None | Some(Value::Null) => {}
                    Some(Value::String(status)) if status == "completed" => {}
                    _ => return Err(ProtocolError::InvalidRequestField("status")),
                }
                required_nonempty_string(object, "call_id")?;
                required_nonempty_string(object, "name")?;
                validate_optional_nullable_nonempty_string(object, "namespace")?;
                required_string(object, "input")?;
                Ok(Self::ForeignCustomToolCall)
            }
            "custom_tool_call_output" => {
                reject_unknown_keys(
                    object,
                    &[
                        "type",
                        "id",
                        "call_id",
                        "name",
                        "output",
                        INTERNAL_MESSAGE_METADATA_FIELD,
                    ],
                )?;
                validate_internal_message_metadata(object)?;
                optional_nonempty_string(object, "id")?;
                required_nonempty_string(object, "call_id")?;
                validate_optional_nullable_nonempty_string(object, "name")?;
                validate_foreign_custom_tool_output(required_value(object, "output")?)?;
                Ok(Self::ForeignCustomToolCallOutput)
            }
            _ => Err(ProtocolError::UnsupportedInputItem),
        }
    }

    fn to_grok_value(&self) -> Option<Value> {
        match self {
            Self::Message(message) => Some(message.to_value()),
            Self::Reasoning(reasoning)
                if matches!(reasoning.encrypted_content, PriorEncryptedContent::Grok(_)) =>
            {
                Some(reasoning.to_value())
            }
            Self::Reasoning(_)
            | Self::ForeignAgentMessage
            | Self::ForeignCustomToolCall
            | Self::ForeignCustomToolCallOutput => None,
            Self::FunctionCall(call) => Some(call.to_value()),
            Self::FunctionCallOutput(output) => Some(output.to_value()),
            Self::ToolSearchCall(call) => Some(serde_json::json!({
                "type": "function_call",
                "name": "tool_search",
                "arguments": call.arguments.to_string(),
                "call_id": call.call_id,
            })),
            Self::ToolSearchOutput(output) => Some(serde_json::json!({
                "type": "function_call_output",
                "call_id": output.call_id,
                "output": serde_json::json!({"tools": output.tools}).to_string(),
            })),
        }
    }
}

fn parse_agent_message(object: &Map<String, Value>) -> Result<Option<TextMessage>, ProtocolError> {
    optional_nonempty_string(object, "id")?;
    required_nonempty_string(object, "author")?;
    required_nonempty_string(object, "recipient")?;

    let mut text_parts = Vec::new();
    let mut contains_encrypted_content = false;
    for part in required_array(object, "content")? {
        let part = part
            .as_object()
            .ok_or(ProtocolError::InvalidRequestField("content"))?;
        match required_string(part, "type")? {
            "input_text" => {
                reject_unknown_keys(part, &["type", "text"])?;
                text_parts.push(required_string(part, "text")?);
            }
            "encrypted_content" => {
                reject_unknown_keys(part, &["type", "encrypted_content"])?;
                if required_nonempty_string(part, "encrypted_content")?.len()
                    > MAX_ENCRYPTED_REASONING_BYTES
                {
                    return Err(ProtocolError::InvalidRequestField("encrypted_content"));
                }
                contains_encrypted_content = true;
            }
            _ => return Err(ProtocolError::UnsupportedContent),
        }
    }

    if contains_encrypted_content {
        return Ok(None);
    }
    let text = text_parts.join("\n");
    if text.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(TextMessage {
        role: MessageRole::Assistant,
        content: vec![MessageContent::OutputText(text)],
    }))
}

fn validate_foreign_custom_tool_output(value: &Value) -> Result<(), ProtocolError> {
    match value {
        Value::String(_) => Ok(()),
        Value::Array(parts) => {
            for part in parts {
                InputContent::parse(part)?;
            }
            Ok(())
        }
        _ => Err(ProtocolError::InvalidRequestField("output")),
    }
}

fn validate_internal_message_metadata(object: &Map<String, Value>) -> Result<(), ProtocolError> {
    let Some(metadata) = object.get(INTERNAL_MESSAGE_METADATA_FIELD) else {
        return Ok(());
    };
    let metadata = metadata
        .as_object()
        .ok_or(ProtocolError::InvalidRequestField("input"))?;
    if reject_unknown_keys_for(metadata, &["turn_id", "create_time", "executed_tool_calls"])
        .is_err()
    {
        return Err(ProtocolError::InvalidRequestField("input"));
    }
    if let Some(turn_id) = metadata.get("turn_id")
        && turn_id.as_str().is_none_or(str::is_empty)
    {
        return Err(ProtocolError::InvalidRequestField("input"));
    }
    if let Some(create_time) = metadata.get("create_time")
        && create_time
            .as_f64()
            .is_none_or(|seconds| !seconds.is_finite() || seconds < 0.0)
    {
        return Err(ProtocolError::InvalidRequestField("input"));
    }
    if let Some(executed_tool_calls) = metadata.get("executed_tool_calls") {
        let calls = executed_tool_calls
            .as_array()
            .ok_or(ProtocolError::InvalidRequestField("input"))?;
        for call in calls {
            let call = call
                .as_object()
                .ok_or(ProtocolError::InvalidRequestField("input"))?;
            if reject_unknown_keys_for(call, &["name", "arguments"]).is_err()
                || required_nonempty_string(call, "name").is_err()
                || !call.contains_key("arguments")
            {
                return Err(ProtocolError::InvalidRequestField("input"));
            }
        }
    }
    Ok(())
}

impl PriorReasoning {
    fn parse(object: &Map<String, Value>) -> Result<Self, ProtocolError> {
        let summary =
            parse_prior_reasoning_parts(required_array(object, "summary")?, "summary_text")?;
        let content = match object.get("content") {
            None => PriorReasoningContent::Absent,
            Some(Value::Null) => PriorReasoningContent::Null,
            Some(Value::Array(parts)) => {
                PriorReasoningContent::Parts(parse_prior_reasoning_parts(parts, "reasoning_text")?)
            }
            Some(_) => return Err(ProtocolError::InvalidRequestField("content")),
        };
        let encrypted_content = match required_value(object, "encrypted_content")? {
            Value::Null => PriorEncryptedContent::Null,
            Value::String(value) if !value.is_empty() => {
                if value.len() > MAX_ENCRYPTED_REASONING_BYTES {
                    return Err(ProtocolError::InvalidRequestField("encrypted_content"));
                }
                match value.strip_prefix(GROK_REASONING_ENVELOPE_PREFIX) {
                    Some(value) if !value.is_empty() => {
                        PriorEncryptedContent::Grok(value.to_owned())
                    }
                    Some(_) => {
                        return Err(ProtocolError::InvalidRequestField("encrypted_content"));
                    }
                    None => PriorEncryptedContent::Foreign(value.clone()),
                }
            }
            _ => return Err(ProtocolError::InvalidRequestField("encrypted_content")),
        };
        Ok(Self {
            id: required_nonempty_string(object, "id")?,
            summary,
            content,
            encrypted_content,
        })
    }

    fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("type".into(), Value::String("reasoning".into()));
        object.insert("id".into(), Value::String(self.id.clone()));
        object.insert(
            "summary".into(),
            Value::Array(
                self.summary
                    .iter()
                    .map(|text| serde_json::json!({"type": "summary_text", "text": text}))
                    .collect(),
            ),
        );
        match &self.content {
            // Codex replays an absent optional reasoning payload as explicit
            // null. Grok Build's typed optional field omits that key upstream.
            PriorReasoningContent::Absent | PriorReasoningContent::Null => {}
            PriorReasoningContent::Parts(parts) => {
                object.insert(
                    "content".into(),
                    Value::Array(
                        parts
                            .iter()
                            .map(|text| serde_json::json!({"type": "reasoning_text", "text": text}))
                            .collect(),
                    ),
                );
            }
        }
        object.insert(
            "encrypted_content".into(),
            match &self.encrypted_content {
                PriorEncryptedContent::Null => Value::Null,
                PriorEncryptedContent::Grok(value) | PriorEncryptedContent::Foreign(value) => {
                    Value::String(value.clone())
                }
            },
        );
        Value::Object(object)
    }
}

/// Removes provider-bound replay state before a `store: false` mixed-provider
/// request is forwarded to Native GPT.
///
/// Bridge-enveloped or explicitly null reasoning identifies Grok history. In
/// that case the complete reasoning family and provider-owned item IDs are not
/// portable to the Native Responses store, while function `call_id` values are
/// retained so Codex can preserve tool call/output pairs. Native tool-search
/// IDs are only replayable with Codex's `tsc_` prefix, so malformed or foreign
/// IDs are removed even when older history lacks reasoning provenance.
/// Native-only GPT sessions otherwise remain byte-for-byte intact.
pub fn sanitize_unreplayable_history_for_native(request: &mut Value) -> bool {
    let Some(object) = request.as_object_mut() else {
        return false;
    };
    if object.get("store").and_then(Value::as_bool) != Some(false) {
        return false;
    }
    let Some(input) = object.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let contains_grok_reasoning = input.iter().any(|item| {
        let Some(item) = item.as_object() else {
            return false;
        };
        if item.get("type").and_then(Value::as_str) != Some("reasoning") {
            return false;
        }
        match item.get("encrypted_content") {
            Some(Value::Null) => true,
            Some(Value::String(value)) => value.starts_with(GROK_REASONING_ENVELOPE_PREFIX),
            _ => false,
        }
    });
    let mut changed = false;
    if contains_grok_reasoning {
        let original_len = input.len();
        input.retain(|item| {
            item.as_object()
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str)
                != Some("reasoning")
        });
        changed = input.len() != original_len;
    }

    for item in input {
        let Some(item) = item.as_object_mut() else {
            continue;
        };
        let Some(item_type) = item.get("type").and_then(Value::as_str) else {
            continue;
        };
        let remove_provider_id = contains_grok_reasoning
            && matches!(
                item_type,
                "message" | "function_call" | "function_call_output"
            );
        let remove_invalid_tool_search_id = item_type == "tool_search_call"
            && item
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.starts_with("tsc_"));
        if (remove_provider_id || remove_invalid_tool_search_id) && item.remove("id").is_some() {
            changed = true;
        }
    }
    changed
}

fn parse_prior_reasoning_parts(
    values: &[Value],
    expected_type: &'static str,
) -> Result<Vec<String>, ProtocolError> {
    if values.len() > MAX_REASONING_PARTS {
        return Err(ProtocolError::InvalidRequestField("reasoning"));
    }
    let mut total_bytes = 0usize;
    values
        .iter()
        .map(|value| {
            let part = value
                .as_object()
                .ok_or(ProtocolError::InvalidRequestField("reasoning"))?;
            if reject_unknown_keys_for(part, &["type", "text"]).is_err()
                || required_string(part, "type")? != expected_type
            {
                return Err(ProtocolError::InvalidRequestField("reasoning"));
            }
            let text = required_string(part, "text")?.to_owned();
            total_bytes = total_bytes
                .checked_add(text.len())
                .ok_or(ProtocolError::InvalidRequestField("reasoning"))?;
            if total_bytes > MAX_REASONING_TEXT_BYTES {
                return Err(ProtocolError::InvalidRequestField("reasoning"));
            }
            Ok(text)
        })
        .collect()
}


fn canonicalize_integer_valued_numbers(value: &mut Value) {
    match value {
        Value::Number(number) => {
            if let Some(canonical) = integer_valued_json_number(number) {
                *number = canonical;
            }
        }
        Value::Array(values) => {
            for value in values {
                canonicalize_integer_valued_numbers(value);
            }
        }
        Value::Object(object) => {
            for value in object.values_mut() {
                canonicalize_integer_valued_numbers(value);
            }
        }
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}

fn integer_valued_json_number(number: &serde_json::Number) -> Option<serde_json::Number> {
    if number.is_i64() || number.is_u64() {
        return None;
    }
    let float = number.as_f64()?;
    if !float.is_finite() {
        return None;
    }
    if float < i64::MIN as f64 || float > i64::MAX as f64 {
        return None;
    }
    let integer = float as i64;
    if integer as f64 != float {
        return None;
    }
    Some(serde_json::Number::from(integer))
}

fn canonicalize_json_object_argument_string(arguments: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(arguments).ok()?;
    if !value.is_object() {
        return None;
    }
    let original = value.clone();
    canonicalize_integer_valued_numbers(&mut value);
    if value == original {
        None
    } else {
        Some(value.to_string())
    }
}

fn canonicalize_arguments_field(object: &mut Map<String, Value>) {
    match object.get("arguments") {
        Some(Value::String(arguments)) => {
            if let Some(canonical) = canonicalize_json_object_argument_string(arguments) {
                object.insert("arguments".into(), Value::String(canonical));
            }
        }
        Some(Value::Object(_) | Value::Array(_)) => {
            if let Some(arguments) = object.get_mut("arguments") {
                canonicalize_integer_valued_numbers(arguments);
            }
        }
        _ => {}
    }
}

fn canonicalize_function_call_item_arguments(item: &mut Value) {
    let Some(object) = item.as_object_mut() else {
        return;
    };
    if object.get("type").and_then(Value::as_str) != Some("function_call") {
        return;
    }
    canonicalize_arguments_field(object);
}

fn canonicalize_response_event_arguments(event: &mut Value) {
    let Some(object) = event.as_object_mut() else {
        return;
    };
    match object.get("type").and_then(Value::as_str) {
        Some("response.output_item.added" | "response.output_item.done") => {
            if let Some(item) = object.get_mut("item") {
                canonicalize_function_call_item_arguments(item);
            }
        }
        Some("response.completed") => {
            if let Some(output) = object
                .get_mut("response")
                .and_then(Value::as_object_mut)
                .and_then(|response| response.get_mut("output"))
                .and_then(Value::as_array_mut)
            {
                for item in output {
                    canonicalize_function_call_item_arguments(item);
                }
            }
        }
        Some("response.function_call_arguments.done") => {
            canonicalize_arguments_field(object);
        }
        _ => {}
    }
}

impl FunctionCall {
    fn parse(
        object: &Map<String, Value>,
        namespace_projection: &mut NamespaceToolProjection,
    ) -> Result<Self, ProtocolError> {
        // Grok Build deliberately omits the response item id when replaying a
        // tool call. Validate Codex's output-only id, but do not forward it.
        optional_nonempty_string(object, "id")?;
        let arguments = required_string(object, "arguments")?.to_owned();
        let mut parsed: Value = serde_json::from_str(&arguments)
            .map_err(|_| ProtocolError::InvalidFunctionArguments)?;
        if !parsed.is_object() {
            return Err(ProtocolError::InvalidFunctionArguments);
        }
        let original_arguments = parsed.clone();
        canonicalize_integer_valued_numbers(&mut parsed);
        let arguments = if parsed == original_arguments {
            arguments
        } else {
            parsed.to_string()
        };
        let native_name = required_nonempty_string(object, "name")?;
        let name = match optional_nonempty_string(object, "namespace")? {
            Some(namespace) => namespace_projection
                .ensure_provider_name(&namespace, &native_name)?,
            None => native_name,
        };
        Ok(Self {
            name,
            arguments,
            call_id: required_nonempty_string(object, "call_id")?,
        })
    }

    fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("type".into(), Value::String("function_call".into()));
        object.insert("name".into(), Value::String(self.name.clone()));
        object.insert("arguments".into(), Value::String(self.arguments.clone()));
        object.insert("call_id".into(), Value::String(self.call_id.clone()));
        Value::Object(object)
    }
}

impl FunctionCallOutput {
    fn parse(object: &Map<String, Value>) -> Result<Self, ProtocolError> {
        // The official replay shape likewise omits a tool result item id.
        optional_nonempty_string(object, "id")?;
        Ok(Self {
            call_id: required_nonempty_string(object, "call_id")?,
            output: FunctionCallOutputBody::parse(required_value(object, "output")?)?,
        })
    }

    fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("type".into(), Value::String("function_call_output".into()));
        object.insert("call_id".into(), Value::String(self.call_id.clone()));
        object.insert("output".into(), self.output.to_value());
        Value::Object(object)
    }
}

impl FunctionCallOutputBody {
    fn parse(value: &Value) -> Result<Self, ProtocolError> {
        match value {
            Value::String(text) => Ok(Self::Text(text.clone())),
            Value::Array(parts) if !parts.is_empty() => Ok(Self::Content(
                parts
                    .iter()
                    .map(InputContent::parse)
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            _ => Err(ProtocolError::InvalidRequestField("output")),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Text(text) => Value::String(text.clone()),
            Self::Content(parts) => {
                Value::Array(parts.iter().map(InputContent::to_value).collect())
            }
        }
    }
}

impl TextMessage {
    fn parse(object: &Map<String, Value>) -> Result<Self, ProtocolError> {
        // Grok Build reconstructs messages as easy input messages without the
        // response item id. Keep validating the field while stripping it from
        // the compaction-bound replay prefix.
        optional_nonempty_string(object, "id")?;
        let role = match required_string(object, "role")? {
            "user" => MessageRole::User,
            "developer" => MessageRole::Developer,
            "assistant" => MessageRole::Assistant,
            _ => return Err(ProtocolError::InvalidRequestField("role")),
        };
        match object.get("phase") {
            None => {}
            Some(Value::String(phase))
                if role == MessageRole::Assistant
                    && matches!(phase.as_str(), "commentary" | "final_answer") => {}
            _ => return Err(ProtocolError::InvalidRequestField("phase")),
        }
        let content = required_array(object, "content")?
            .iter()
            .map(MessageContent::parse)
            .collect::<Result<Vec<_>, _>>()?;

        let valid_role_content = content.iter().all(|part| match (role, part) {
            (MessageRole::User | MessageRole::Developer, MessageContent::Input(_)) => true,
            (MessageRole::Assistant, MessageContent::OutputText(_)) => true,
            _ => false,
        });
        if !valid_role_content {
            return Err(ProtocolError::InvalidRequestField("content"));
        }

        Ok(Self { role, content })
    }

    fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("type".into(), Value::String("message".into()));
        object.insert(
            "role".into(),
            Value::String(
                match self.role {
                    MessageRole::User => "user",
                    MessageRole::Developer => "developer",
                    MessageRole::Assistant => "assistant",
                }
                .into(),
            ),
        );
        let content = match self.role {
            // Grok Build folds response output text into the next turn's easy
            // assistant message. The compaction blob is bound to that replay
            // shape, so Codex's output_text array must not cross upstream.
            MessageRole::Assistant => Value::String(
                self.content
                    .iter()
                    .filter_map(|part| match part {
                        MessageContent::OutputText(text) => Some(text.as_str()),
                        MessageContent::Input(_) => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            MessageRole::User | MessageRole::Developer => {
                Value::Array(self.content.iter().map(MessageContent::to_value).collect())
            }
        };
        object.insert("content".into(), content);
        Value::Object(object)
    }
}

impl MessageContent {
    fn parse(value: &Value) -> Result<Self, ProtocolError> {
        let object = value
            .as_object()
            .ok_or(ProtocolError::InvalidRequestField("content"))?;
        match required_string(object, "type")? {
            "input_text" | "input_image" => Ok(Self::Input(InputContent::parse(value)?)),
            "output_text" => {
                reject_unknown_keys(object, &["type", "text"])?;
                Ok(Self::OutputText(
                    required_string(object, "text")?.to_owned(),
                ))
            }
            _ => Err(ProtocolError::UnsupportedContent),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Input(content) => content.to_value(),
            Self::OutputText(text) => {
                serde_json::json!({"type": "output_text", "text": text})
            }
        }
    }
}

impl InputContent {
    fn parse(value: &Value) -> Result<Self, ProtocolError> {
        let object = value
            .as_object()
            .ok_or(ProtocolError::InvalidRequestField("content"))?;
        match required_string(object, "type")? {
            "input_text" => {
                if reject_unknown_keys_for(object, &["type", "text"]).is_err() {
                    return Err(ProtocolError::UnsupportedContent);
                }
                Ok(Self::InputText(required_string(object, "text")?.to_owned()))
            }
            "input_image" => Ok(Self::InputImage(InputImage::parse(object)?)),
            _ => Err(ProtocolError::UnsupportedContent),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::InputText(text) => {
                serde_json::json!({"type": "input_text", "text": text})
            }
            Self::InputImage(image) => image.to_value(),
        }
    }
}

impl InputImage {
    fn parse(object: &Map<String, Value>) -> Result<Self, ProtocolError> {
        if reject_unknown_keys_for(object, &["type", "image_url", "detail"]).is_err() {
            return Err(ProtocolError::UnsupportedContent);
        }
        let image_url = required_string(object, "image_url")?.to_owned();
        if !is_supported_image_url(&image_url) {
            return Err(ProtocolError::InvalidImageUrl);
        }
        let detail = match object.get("detail") {
            None => None,
            Some(Value::String(value)) => Some(ImageDetail::parse(value)?),
            Some(_) => return Err(ProtocolError::InvalidRequestField("detail")),
        };
        Ok(Self { image_url, detail })
    }

    fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("type".into(), Value::String("input_image".into()));
        object.insert("image_url".into(), Value::String(self.image_url.clone()));
        if let Some(detail) = self.detail {
            object.insert("detail".into(), Value::String(detail.as_str().into()));
        }
        Value::Object(object)
    }
}

impl ImageDetail {
    fn parse(value: &str) -> Result<Self, ProtocolError> {
        match value {
            "auto" => Ok(Self::Auto),
            "low" => Ok(Self::Low),
            "high" => Ok(Self::High),
            "original" => Ok(Self::Original),
            _ => Err(ProtocolError::InvalidRequestField("detail")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Low => "low",
            Self::High => "high",
            Self::Original => "original",
        }
    }
}

fn is_supported_image_url(value: &str) -> bool {
    if let Some(data) = value.strip_prefix("data:image/") {
        return is_valid_image_data_url(data);
    }

    let authority = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .and_then(|rest| rest.split(['/', '?', '#']).next());
    if authority.is_none_or(str::is_empty) {
        return false;
    }

    match url::Url::parse(value) {
        Ok(url) => matches!(url.scheme(), "http" | "https") && url.host().is_some(),
        Err(_) => false,
    }
}

fn is_valid_image_data_url(data: &str) -> bool {
    let Some((subtype, payload)) = data.split_once(";base64,") else {
        return false;
    };
    !subtype.is_empty() && subtype.bytes().all(is_mime_subtype_byte) && is_valid_base64(payload)
}

fn is_mime_subtype_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
        )
}

fn is_valid_base64(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return false;
    }

    let padding = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 {
        return false;
    }
    let content_len = bytes.len() - padding;
    if content_len == 0
        || bytes[..content_len]
            .iter()
            .any(|byte| base64_value(*byte).is_none())
        || bytes[..content_len].contains(&b'=')
    {
        return false;
    }

    let remainder = content_len % 4;
    if remainder == 1
        || (padding > 0
            && (bytes.len() % 4 != 0 || !matches!((remainder, padding), (2, 2) | (3, 1))))
    {
        return false;
    }

    match remainder {
        2 => base64_value(bytes[content_len - 1]).is_some_and(|value| value & 0x0f == 0),
        3 => base64_value(bytes[content_len - 1]).is_some_and(|value| value & 0x03 == 0),
        _ => true,
    }
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn parse_input(
    values: &[Value],
    namespace_projection: &mut NamespaceToolProjection,
) -> Result<Vec<InputItem>, ProtocolError> {
    let mut calls = HashSet::new();
    let mut outputs = HashSet::new();
    let mut reasoning_ids = HashSet::new();
    let mut parsed = Vec::with_capacity(values.len());
    for value in values {
        // A failed client-side tool search can leave an empty output item in
        // the replay (and, in older Codex builds, an output whose call_id no
        // longer has a matching call).  Such an item is not replayable by any
        // provider.  Drop only that item; preserve all valid text and tool
        // history so the next turn remains useful.
        let Some(raw_object) = value.as_object() else {
            continue;
        };
        let raw_type = raw_object.get("type").and_then(Value::as_str);
        if matches!(raw_type, Some("function_call_output" | "tool_search_output")) {
            let call_id = raw_object.get("call_id").and_then(Value::as_str);
            if call_id.is_none_or(str::is_empty) || !calls.contains(call_id.unwrap()) {
                continue;
            }
        }
        let item = match InputItem::parse(value, namespace_projection) {
            Ok(item) => item,
            // Unknown history items are Codex-owned and have no xAI
            // projection.  Ignore them instead of rejecting the request.
            Err(ProtocolError::UnsupportedInputItem) => continue,
            Err(error) => return Err(error),
        };
        match &item {
            InputItem::Reasoning(reasoning)
                if matches!(reasoning.encrypted_content, PriorEncryptedContent::Grok(_)) =>
            {
                reasoning_ids.insert(reasoning.id.clone());
            }
            // Native GPT and legacy untagged reasoning are provider-bound and
            // are removed before the Grok request is serialized. They must not
            // impose Grok's assistant-output ordering on a mixed-model history.
            InputItem::Reasoning(_) => {}
            InputItem::FunctionCall(call) => {
                calls.insert(call.call_id.clone());
            }
            InputItem::ToolSearchCall(call) => {
                calls.insert(call.call_id.clone());
            }
            InputItem::FunctionCallOutput(output) => {
                if !calls.contains(&output.call_id) {
                    return Err(ProtocolError::UnmatchedFunctionOutput);
                }
                if !outputs.insert(output.call_id.clone()) {
                    continue;
                }
            }
            InputItem::ToolSearchOutput(output) => {
                if !calls.contains(&output.call_id) {
                    return Err(ProtocolError::UnmatchedFunctionOutput);
                }
                if !outputs.insert(output.call_id.clone()) {
                    continue;
                }
            }
            // Provider-bound agent messages are not Grok tool-loop state.
            InputItem::ForeignAgentMessage => {}
            // Native custom tools are executed and recorded by the Codex
            // harness. They are complete foreign history, not Grok tool-loop
            // state, so validation terminates at the bridge boundary.
            InputItem::ForeignCustomToolCall | InputItem::ForeignCustomToolCallOutput => {}
            InputItem::Message(message) => match message.role {
                MessageRole::User | MessageRole::Developer | MessageRole::Assistant => {}
            },
        }
        parsed.push(item);
    }
    Ok(parsed)
}

impl NamespaceToolProjection {
    fn provider_name(&self, namespace: &str, name: &str) -> Option<&str> {
        self.native_to_provider
            .get(&(namespace.to_owned(), name.to_owned()))
            .map(String::as_str)
    }

    fn ensure_provider_name(
        &mut self,
        namespace: &str,
        name: &str,
    ) -> Result<String, ProtocolError> {
        if let Some(provider_name) = self.provider_name(namespace, name) {
            return Ok(provider_name.to_owned());
        }

        // Codex may replay a namespaced function call after its current
        // request has dropped the namespace's tool declaration (for example,
        // when a deferred tool search is no longer present). The provider
        // projection is still deterministic from the native pair, so retain
        // the history instead of rejecting the whole request.
        let provider_name = format!("{namespace}{NAMESPACE_DELIMITER}{name}");
        if let Some(existing) = self.provider_to_native.get(&provider_name)
            && (existing.namespace != namespace || existing.name != name)
        {
            return Err(ProtocolError::DuplicateToolName);
        }
        self.insert(
            provider_name.clone(),
            namespace.to_owned(),
            name.to_owned(),
        )?;
        Ok(provider_name)
    }

    fn insert(
        &mut self,
        provider_name: String,
        namespace: String,
        name: String,
    ) -> Result<(), ProtocolError> {
        let native_key = (namespace.clone(), name.clone());
        if self.native_to_provider.contains_key(&native_key)
            || self.provider_to_native.contains_key(&provider_name)
        {
            return Err(ProtocolError::DuplicateToolName);
        }
        self.native_to_provider
            .insert(native_key, provider_name.clone());
        self.provider_to_native
            .insert(provider_name, NativeToolName { namespace, name });
        Ok(())
    }

    fn restore_function_item(&self, value: &mut Value) {
        let Some(item) = value.as_object_mut() else {
            return;
        };
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return;
        }
        let Some(provider_name) = item.get("name").and_then(Value::as_str) else {
            return;
        };
        let Some(native) = self.provider_to_native.get(provider_name) else {
            return;
        };
        item.insert("name".into(), Value::String(native.name.clone()));
        item.insert("namespace".into(), Value::String(native.namespace.clone()));
    }

    fn restore_response_event(&self, value: &mut Value) {
        canonicalize_response_event_arguments(value);
        if self.provider_to_native.is_empty() && self.tool_search_provider_name.is_none() {
            return;
        }
        let Some(event) = value.as_object_mut() else {
            return;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_item.added") => {
                if let Some(item) = event.get_mut("item") {
                    self.restore_function_item(item);
                }
            }
            Some("response.output_item.done") => {
                if let Some(item) = event.get_mut("item") {
                    if !self.restore_tool_search_item(item) {
                        self.restore_function_item(item);
                    }
                }
            }
            Some("response.completed") => {
                let Some(output) = event
                    .get_mut("response")
                    .and_then(Value::as_object_mut)
                    .and_then(|response| response.get_mut("output"))
                    .and_then(Value::as_array_mut)
                else {
                    return;
                };
                for item in output {
                    if !self.restore_tool_search_item(item) {
                        self.restore_function_item(item);
                    }
                }
            }
            _ => {}
        }
    }

    fn restore_tool_search_item(&self, item: &mut Value) -> bool {
        let Some(provider_name) = self.tool_search_provider_name.as_deref() else {
            return false;
        };
        let Some(object) = item.as_object_mut() else {
            return false;
        };
        if object.get("type").and_then(Value::as_str) != Some("function_call")
            || object.get("name").and_then(Value::as_str) != Some(provider_name)
        {
            return false;
        }
        let Some(arguments) = object.get("arguments").and_then(Value::as_str) else {
            return false;
        };
        let Ok(mut arguments) = serde_json::from_str::<Value>(arguments) else {
            return false;
        };
        if !arguments.is_object() {
            return false;
        }
        canonicalize_integer_valued_numbers(&mut arguments);
        object.insert("type".into(), Value::String("tool_search_call".into()));
        object.insert("execution".into(), Value::String("client".into()));
        object.insert("arguments".into(), arguments);
        object.remove("name");
        object.remove("id");
        object.remove("status");
        true
    }
}

fn parse_tools(
    value: Option<&Value>,
) -> Result<(ToolsField, NamespaceToolProjection), ProtocolError> {
    match value {
        None => Ok((ToolsField::Absent, NamespaceToolProjection::default())),
        Some(Value::Null) => Ok((ToolsField::Null, NamespaceToolProjection::default())),
        Some(Value::Array(tools)) => {
            let mut provider_names = HashSet::new();
            let mut has_hosted_web_search = false;
            let mut namespace_projection = NamespaceToolProjection::default();
            let mut parsed = Vec::with_capacity(tools.len());
            for value in tools {
                let object = value.as_object().ok_or(ProtocolError::UnsupportedTools)?;
                match required_string(object, "type")? {
                    "function" => {
                        let tool = FunctionTool::parse(value)?;
                        if !provider_names.insert(tool.name.clone()) {
                            return Err(ProtocolError::DuplicateToolName);
                        }
                        parsed.push(ProjectedTool::Function(tool));
                    }
                    "tool_search" => {
                        if reject_unknown_keys_for(
                            object,
                            &["type", "execution", "description", "parameters"],
                        )
                        .is_err()
                            || required_string(object, "execution")? != "client"
                        {
                            return Err(ProtocolError::UnsupportedTools);
                        }
                        let parameters = required_value(object, "parameters")?
                            .as_object()
                            .ok_or(ProtocolError::InvalidRequestField("parameters"))?
                            .clone();
                        let provider_name = "tool_search".to_owned();
                        if !provider_names.insert(provider_name.clone()) {
                            return Err(ProtocolError::DuplicateToolName);
                        }
                        namespace_projection.tool_search_provider_name =
                            Some(provider_name.clone());
                        parsed.push(ProjectedTool::Function(FunctionTool {
                            name: provider_name,
                            description: required_string(object, "description")?.to_owned(),
                            parameters: provider_tool_schema(&parameters),
                            strict: Some(false),
                        }));
                    }
                    "namespace" => {
                        if reject_unknown_keys_for(
                            object,
                            &["type", "name", "description", "tools"],
                        )
                        .is_err()
                        {
                            return Err(ProtocolError::UnsupportedTools);
                        }
                        let namespace = required_nonempty_string(object, "name")?;
                        required_string(object, "description")?;
                        let children = required_array(object, "tools")?;
                        let mut native_names = HashSet::new();
                        for child in children {
                            let mut tool = FunctionTool::parse(child)?;
                            if !native_names.insert(tool.name.clone()) {
                                return Err(ProtocolError::DuplicateToolName);
                            }
                            let native_name = tool.name.clone();
                            let provider_name =
                                format!("{namespace}{NAMESPACE_DELIMITER}{native_name}");
                            if !provider_names.insert(provider_name.clone()) {
                                return Err(ProtocolError::DuplicateToolName);
                            }
                            namespace_projection.insert(
                                provider_name.clone(),
                                namespace.clone(),
                                native_name,
                            )?;
                            tool.name = provider_name;
                            parsed.push(ProjectedTool::Function(tool));
                        }
                    }
                    "web_search" => {
                        if reject_unknown_keys_for(object, &["type", "external_web_access"])
                            .is_err()
                            || !matches!(object.get("external_web_access"), Some(Value::Bool(_)))
                        {
                            return Err(ProtocolError::UnsupportedTools);
                        }
                        if has_hosted_web_search {
                            return Err(ProtocolError::DuplicateToolName);
                        }
                        has_hosted_web_search = true;
                        // Codex distinguishes cached/live access with an OpenAI-only
                        // field. Grok's Responses proxy exposes the same hosted tool
                        // as a bare type tag, so the provider projection ends that
                        // transport detail here instead of forwarding it upstream.
                        parsed.push(ProjectedTool::HostedWebSearch);
                    }
                    _ => return Err(ProtocolError::UnsupportedTools),
                }
            }
            if parsed.len() > MAX_XAI_TOOLS {
                return Err(ProtocolError::TooManyTools);
            }
            Ok((ToolsField::List(parsed), namespace_projection))
        }
        Some(_) => Err(ProtocolError::UnsupportedTools),
    }
}

fn merge_discovered_tool_search_specs(
    tools: &mut ToolsField,
    projection: &mut NamespaceToolProjection,
    input: &[Value],
) -> Result<(), ProtocolError> {
    for item in input {
        let Some(object) = item.as_object() else {
            continue;
        };
        if object.get("type").and_then(Value::as_str) != Some("tool_search_output") {
            continue;
        }
        let output = object
            .get("tools")
            .and_then(Value::as_array)
            .ok_or(ProtocolError::InvalidRequestField("tool_search_output"))?;
        for value in output.iter().cloned().map(strip_defer_loading) {
            let (discovered_tools, discovered_projection) =
                parse_tools(Some(&Value::Array(vec![value])))?;
            let ToolsField::List(additions) = discovered_tools else {
                unreachable!();
            };
            let ToolsField::List(existing) = tools else {
                return Err(ProtocolError::InvalidRequestField("tool_search_output"));
            };
            for addition in additions {
                merge_discovered_tool(existing, projection, addition, &discovered_projection)?;
            }
        }
    }
    Ok(())
}

fn merge_discovered_tool(
    existing: &mut Vec<ProjectedTool>,
    projection: &mut NamespaceToolProjection,
    addition: ProjectedTool,
    discovered_projection: &NamespaceToolProjection,
) -> Result<(), ProtocolError> {
    let ProjectedTool::Function(candidate) = &addition else {
        if existing.contains(&addition) {
            return Ok(());
        }
        if existing.len() >= MAX_XAI_TOOLS {
            return Err(ProtocolError::TooManyTools);
        }
        existing.push(addition);
        return Ok(());
    };

    let candidate_native = discovered_projection
        .provider_to_native
        .get(&candidate.name);
    let existing_native = projection.provider_to_native.get(&candidate.name);
    let candidate_is_tool_search = discovered_projection.tool_search_provider_name.as_deref()
        == Some(candidate.name.as_str());
    let existing_is_tool_search = projection.tool_search_provider_name.as_deref()
        == Some(candidate.name.as_str());

    if let Some(current) = existing.iter().find_map(|tool| match tool {
        ProjectedTool::Function(tool) if tool.name == candidate.name => Some(tool),
        _ => None,
    }) {
        if current == candidate
            && existing_native == candidate_native
            && existing_is_tool_search == candidate_is_tool_search
        {
            return Ok(());
        }
        return Err(ProtocolError::DuplicateToolName);
    }

    if existing_native.is_some() || existing_is_tool_search {
        return Err(ProtocolError::DuplicateToolName);
    }
    if let Some(native) = candidate_native {
        projection.insert(
            candidate.name.clone(),
            native.namespace.clone(),
            native.name.clone(),
        )?;
    }
    if candidate_is_tool_search {
        if projection.tool_search_provider_name.is_some() {
            return Err(ProtocolError::DuplicateToolName);
        }
        projection.tool_search_provider_name = Some(candidate.name.clone());
    }
    if existing.len() >= MAX_XAI_TOOLS {
        return Err(ProtocolError::TooManyTools);
    }
    existing.push(addition);
    Ok(())
}

fn strip_defer_loading(value: Value) -> Value {
    let Some(mut object) = value.as_object().cloned() else {
        return value;
    };
    object.remove("defer_loading");
    if let Some(Value::Array(children)) = object.get_mut("tools") {
        *children = children.drain(..).map(strip_defer_loading).collect();
    }
    Value::Object(object)
}

impl FunctionTool {
    fn parse(value: &Value) -> Result<Self, ProtocolError> {
        let object = value.as_object().ok_or(ProtocolError::UnsupportedTools)?;
        if required_string(object, "type")? != "function" {
            return Err(ProtocolError::UnsupportedTools);
        }
        if reject_unknown_keys_for(
            object,
            &["type", "name", "description", "parameters", "strict"],
        )
        .is_err()
        {
            return Err(ProtocolError::UnsupportedTools);
        }
        let parameters = required_value(object, "parameters")?
            .as_object()
            .ok_or(ProtocolError::InvalidRequestField("parameters"))?
            .clone();
        let strict = match object.get("strict") {
            None => None,
            Some(Value::Bool(value)) => Some(*value),
            Some(_) => return Err(ProtocolError::InvalidRequestField("strict")),
        };
        Ok(Self {
            name: required_nonempty_string(object, "name")?,
            description: required_string(object, "description")?.to_owned(),
            parameters,
            strict,
        })
    }

    fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("type".into(), Value::String("function".into()));
        object.insert("name".into(), Value::String(self.name.clone()));
        object.insert(
            "description".into(),
            Value::String(self.description.clone()),
        );
        object.insert(
            "parameters".into(),
            Value::Object(provider_tool_schema(&self.parameters)),
        );
        if let Some(strict) = self.strict {
            object.insert("strict".into(), Value::Bool(strict));
        }
        Value::Object(object)
    }
}

/// xAI rejects a whole Responses request when even one function parameter
/// schema has a union or nullable-object root. Codex's app tool catalog
/// includes such a schema for `codex_app__automation_update`. This is the
/// provider-only projection: Codex parsing and the native route keep the
/// original schema intact.
fn provider_tool_schema(schema: &Map<String, Value>) -> Map<String, Value> {
    let normalized = normalize_schema_literals(schema, 0);
    if !has_root_union(&normalized) && !has_nullable_object_root(&normalized) {
        return normalized;
    }
    object_root_tool_schema(&normalized)
}

fn normalize_schema_literals(schema: &Map<String, Value>, depth: usize) -> Map<String, Value> {
    if depth > MAX_TOOL_SCHEMA_LITERAL_DEPTH {
        return schema.clone();
    }

    let mut next = schema.clone();
    let declared_types = declared_schema_types(schema);
    if !declared_types.is_empty() {
        if let Some(Value::Array(values)) = schema.get("enum") {
            let kept: Vec<Value> = values
                .iter()
                .filter(|value| matches_declared_schema_type(value, &declared_types))
                .cloned()
                .collect();
            if kept.len() != values.len() {
                if kept.is_empty() {
                    next.remove("enum");
                } else {
                    next.insert("enum".into(), Value::Array(kept));
                }
            }
        }
        if let Some(value) = schema.get("const") {
            if !matches_declared_schema_type(value, &declared_types) {
                next.remove("const");
            }
        }
    }

    for key in TOOL_SCHEMA_MAP_KEYWORDS {
        let Some(children) = schema.get(*key).and_then(Value::as_object) else {
            continue;
        };
        let rewritten = children
            .iter()
            .map(|(name, child)| (name.clone(), normalize_schema_value(child, depth + 1)))
            .collect();
        if &rewritten != children {
            next.insert((*key).into(), Value::Object(rewritten));
        }
    }

    for key in TOOL_SCHEMA_LIST_KEYWORDS {
        let Some(Value::Array(children)) = schema.get(*key) else {
            continue;
        };
        let rewritten: Vec<Value> = children
            .iter()
            .map(|child| normalize_schema_value(child, depth + 1))
            .collect();
        if &rewritten != children {
            next.insert((*key).into(), Value::Array(rewritten));
        }
    }

    for key in TOOL_SCHEMA_CHILD_KEYWORDS {
        let Some(child) = schema.get(*key) else {
            continue;
        };
        match child {
            Value::Array(children) => {
                let rewritten: Vec<Value> = children
                    .iter()
                    .map(|child| normalize_schema_value(child, depth + 1))
                    .collect();
                if &rewritten != children {
                    next.insert((*key).into(), Value::Array(rewritten));
                }
            }
            Value::Object(_) => {
                let rewritten = normalize_schema_value(child, depth + 1);
                if &rewritten != child {
                    next.insert((*key).into(), rewritten);
                }
            }
            _ => {}
        }
    }

    next
}

fn normalize_schema_value(value: &Value, depth: usize) -> Value {
    match value.as_object() {
        Some(schema) => Value::Object(normalize_schema_literals(schema, depth)),
        None => value.clone(),
    }
}

fn declared_schema_types(schema: &Map<String, Value>) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(value)) => vec![value.as_str()],
        Some(Value::Array(values)) => values.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn matches_declared_schema_type(value: &Value, types: &[&str]) -> bool {
    let actual = match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
    };
    types.contains(&actual) || (actual == "integer" && types.contains(&"number"))
}

fn has_root_union(schema: &Map<String, Value>) -> bool {
    TOOL_SCHEMA_UNION_KEYWORDS
        .iter()
        .any(|key| matches!(schema.get(*key), Some(Value::Array(_))))
}

fn has_nullable_object_root(schema: &Map<String, Value>) -> bool {
    matches!(schema.get("type"), Some(Value::Array(types)) if types.iter().any(|value| value == "object"))
}

fn object_root_tool_schema(schema: &Map<String, Value>) -> Map<String, Value> {
    let mut seen_refs = HashSet::new();
    let mut branches = Vec::new();
    collect_object_schema_branches(schema, schema, &mut seen_refs, 0, true, &mut branches);

    let mut properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let root_required = required_schema_values(schema);
    let union_branches: Vec<&Map<String, Value>> = branches
        .iter()
        .filter_map(|branch| (!branch.is_root).then_some(&branch.schema))
        .collect();
    for branch in &branches {
        let Some(branch_properties) = branch.schema.get("properties").and_then(Value::as_object)
        else {
            continue;
        };
        for (name, property) in branch_properties {
            properties
                .entry(name.clone())
                .or_insert_with(|| property.clone());
        }
    }

    let mut required = root_required;
    if let Some((first, rest)) = union_branches.split_first() {
        let mut shared = required_schema_values(first);
        for branch in rest {
            let branch_required = required_schema_values(branch);
            shared.retain(|name| branch_required.contains(name));
        }
        for name in shared {
            if !required.contains(&name) {
                required.push(name);
            }
        }
    }

    let mut rewritten = Map::new();
    for key in ["$schema", "$defs", "definitions"] {
        if let Some(value) = schema.get(key) {
            rewritten.insert(key.into(), value.clone());
        }
    }
    if let Some(Value::String(description)) = schema.get("description") {
        rewritten.insert("description".into(), Value::String(description.clone()));
    }
    rewritten.insert("type".into(), Value::String("object".into()));
    rewritten.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        rewritten.insert("required".into(), Value::Array(required));
    }
    let additional_properties =
        if !union_branches.is_empty() || !schema.contains_key("additionalProperties") {
            Value::Bool(true)
        } else {
            schema
                .get("additionalProperties")
                .cloned()
                .expect("checked above")
        };
    rewritten.insert("additionalProperties".into(), additional_properties);
    rewritten
}

#[derive(Debug)]
struct ObjectSchemaBranch {
    schema: Map<String, Value>,
    is_root: bool,
}

fn collect_object_schema_branches(
    schema: &Map<String, Value>,
    root: &Map<String, Value>,
    seen_refs: &mut HashSet<String>,
    depth: usize,
    is_root: bool,
    branches: &mut Vec<ObjectSchemaBranch>,
) {
    if depth > MAX_TOOL_SCHEMA_DEPTH {
        return;
    }
    if let Some(Value::String(reference)) = schema.get("$ref") {
        if !seen_refs.insert(reference.clone()) {
            return;
        }
        if let Some(resolved) = resolve_local_schema_ref(reference, root) {
            collect_object_schema_branches(&resolved, root, seen_refs, depth + 1, false, branches);
        }
        return;
    }
    for key in TOOL_SCHEMA_UNION_KEYWORDS {
        let Some(Value::Array(children)) = schema.get(*key) else {
            continue;
        };
        for child in children {
            if let Some(child) = child.as_object() {
                collect_object_schema_branches(child, root, seen_refs, depth + 1, false, branches);
            }
        }
    }
    if matches!(schema.get("type"), Some(Value::String(kind)) if kind == "object")
        || matches!(schema.get("properties"), Some(Value::Object(_)))
    {
        branches.push(ObjectSchemaBranch {
            schema: schema.clone(),
            is_root,
        });
    }
}

fn resolve_local_schema_ref(
    reference: &str,
    root: &Map<String, Value>,
) -> Option<Map<String, Value>> {
    let pointer = reference.strip_prefix("#/")?;
    let mut segments = pointer.split('/').peekable();
    let mut current = root;
    while let Some(raw_segment) = segments.next() {
        let segment = raw_segment.replace("~1", "/").replace("~0", "~");
        let value = current.get(&segment)?;
        if segments.peek().is_none() {
            return value.as_object().cloned();
        }
        current = value.as_object()?;
    }
    None
}

fn required_schema_values(schema: &Map<String, Value>) -> Vec<Value> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|values| values.iter().cloned().collect())
        .unwrap_or_default()
}

impl ToolChoice {
    fn to_value(&self) -> Value {
        match self {
            Self::Auto => Value::String("auto".into()),
            Self::None => Value::String("none".into()),
            Self::Required => Value::String("required".into()),
            Self::Function(name) => serde_json::json!({"type": "function", "name": name}),
        }
    }
}

fn parse_tool_choice(value: &Value) -> Result<ToolChoice, ProtocolError> {
    match value {
        Value::String(choice) => match choice.as_str() {
            "auto" => Ok(ToolChoice::Auto),
            "none" => Ok(ToolChoice::None),
            "required" => Ok(ToolChoice::Required),
            _ => Err(ProtocolError::UnsupportedToolChoice),
        },
        Value::Object(object) => {
            if reject_unknown_keys_for(object, &["type", "name"]).is_err()
                || required_string(object, "type")? != "function"
            {
                return Err(ProtocolError::UnsupportedToolChoice);
            }
            Ok(ToolChoice::Function(required_nonempty_string(
                object, "name",
            )?))
        }
        _ => Err(ProtocolError::UnsupportedToolChoice),
    }
}

fn validate_tool_choice(choice: &ToolChoice, tools: &ToolsField) -> Result<(), ProtocolError> {
    let admitted = match tools {
        ToolsField::List(tools) => tools.as_slice(),
        ToolsField::Absent | ToolsField::Null => &[],
    };
    match choice {
        ToolChoice::Required if admitted.is_empty() => Err(ProtocolError::UnknownToolChoice),
        ToolChoice::Function(name)
            if !admitted.iter().any(
                |tool| matches!(tool, ProjectedTool::Function(tool) if tool.name == *name),
            ) =>
        {
            Err(ProtocolError::UnknownToolChoice)
        }
        _ => Ok(()),
    }
}

fn project_tools_for_xai(tools: &ToolsField) -> Option<Value> {
    match tools {
        ToolsField::Absent => None,
        ToolsField::Null => Some(Value::Null),
        ToolsField::List(tools) => Some(Value::Array(
            tools.iter().filter_map(project_tool_for_xai).collect(),
        )),
    }
}

fn project_tool_for_xai(tool: &ProjectedTool) -> Option<Value> {
    match tool {
        ProjectedTool::HostedWebSearch => Some(serde_json::json!({"type": "web_search"})),
        ProjectedTool::Function(tool) => {
            let projected = tool.to_value();
            if tool.name == "tool_search" {
                return Some(projected);
            }
            let Some(parameters) = projected.get("parameters") else {
                return Some(projected);
            };
            match xai_incompatible_tool_schema(parameters) {
                Some(reason) => {
                    tracing::warn!(
                        tool = %tool.name,
                        reason,
                        "omitting xAI-incompatible function tool from provider projection"
                    );
                    None
                }
                None => Some(projected),
            }
        }
    }
}

/// xAI rejects the whole Responses request when even one function schema is
/// illegal. Nested defects cannot be rewritten into a faithful contract, so
/// the provider projection omits that function instead of poisoning spawn and
/// other valid tools. Codex parsing and the native route keep the original
/// catalog.
fn xai_incompatible_tool_schema(schema: &Value) -> Option<&'static str> {
    walk_xai_tool_schema(schema, schema, 0, false)
}

fn walk_xai_tool_schema(
    node: &Value,
    root: &Value,
    depth: usize,
    property_schema: bool,
) -> Option<&'static str> {
    if depth > MAX_TOOL_SCHEMA_LITERAL_DEPTH {
        return None;
    }
    match node {
        Value::Bool(_) if property_schema => Some("boolean_property_schema"),
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("$ref") {
                if resolve_json_pointer(root, reference).is_none() {
                    return Some("unresolvable_ref");
                }
            }
            for key in ["anyOf", "oneOf", "enum"] {
                if matches!(map.get(key), Some(Value::Array(values)) if values.is_empty()) {
                    return Some("empty_union");
                }
            }
            if map.contains_key("maxContains") || map.contains_key("minContains") {
                return Some("contains_constraint");
            }
            if matches!(map.get("items"), Some(Value::Array(_))) {
                return Some("tuple_items");
            }
            for (key, child) in map {
                if *key == "properties" || *key == "patternProperties" {
                    if let Value::Object(properties) = child {
                        for property in properties.values() {
                            if let Some(reason) =
                                walk_xai_tool_schema(property, root, depth + 1, true)
                            {
                                return Some(reason);
                            }
                        }
                        continue;
                    }
                }
                if let Some(reason) = walk_xai_tool_schema(child, root, depth + 1, false) {
                    return Some(reason);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(reason) = walk_xai_tool_schema(item, root, depth + 1, false) {
                    return Some(reason);
                }
            }
            None
        }
        _ => None,
    }
}

fn resolve_json_pointer<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let pointer = reference.strip_prefix('#')?;
    if pointer.is_empty() {
        return Some(root);
    }
    let pointer = pointer.strip_prefix('/')?;
    let mut current = root;
    for raw_segment in pointer.split('/') {
        let segment = raw_segment.replace("~1", "/").replace("~0", "~");
        current = match current {
            Value::Object(map) => map.get(&segment)?,
            Value::Array(items) => {
                let index = segment.parse::<usize>().ok()?;
                items.get(index)?
            }
            _ => return None,
        };
    }
    Some(current)
}

fn insert_optional(object: &mut Map<String, Value>, key: &str, field: &OptionalJson) {
    if let OptionalJson::Present(value) = field {
        object.insert(key.into(), value.clone());
    }
}

fn reject_unknown_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), ProtocolError> {
    let _ = (object, allowed);
    Ok(())
}

fn reject_unknown_keys_for(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), ()> {
    if object
        .keys()
        .any(|key| !allowed.iter().any(|allowed| key == allowed))
    {
        Err(())
    } else {
        Ok(())
    }
}

fn required_value<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a Value, ProtocolError> {
    object
        .get(key)
        .ok_or(ProtocolError::MissingRequestField(key))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, ProtocolError> {
    required_value(object, key)?
        .as_str()
        .ok_or(ProtocolError::InvalidRequestField(key))
}

fn required_nonempty_string(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<String, ProtocolError> {
    let value = required_string(object, key)?;
    if value.trim().is_empty() {
        return Err(ProtocolError::InvalidRequestField(key));
    }
    Ok(value.to_owned())
}

fn required_bool(object: &Map<String, Value>, key: &'static str) -> Result<bool, ProtocolError> {
    required_value(object, key)?
        .as_bool()
        .ok_or(ProtocolError::InvalidRequestField(key))
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a [Value], ProtocolError> {
    required_value(object, key)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or(ProtocolError::InvalidRequestField(key))
}

fn optional_string(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<String>, ProtocolError> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(ProtocolError::InvalidRequestField(key)),
    }
}

fn optional_nonempty_string(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<String>, ProtocolError> {
    match optional_string(object, key)? {
        Some(value) if value.trim().is_empty() => Err(ProtocolError::InvalidRequestField(key)),
        value => Ok(value),
    }
}

fn validate_optional_nullable_nonempty_string(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<(), ProtocolError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(()),
        _ => Err(ProtocolError::InvalidRequestField(key)),
    }
}

fn optional_object(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<OptionalJson, ProtocolError> {
    match object.get(key) {
        None => Ok(OptionalJson::Absent),
        Some(value @ (Value::Null | Value::Object(_))) => Ok(OptionalJson::Present(value.clone())),
        Some(_) => Err(ProtocolError::InvalidRequestField(key)),
    }
}

fn optional_scalar_string(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<OptionalJson, ProtocolError> {
    match object.get(key) {
        None => Ok(OptionalJson::Absent),
        Some(value @ (Value::Null | Value::String(_))) => Ok(OptionalJson::Present(value.clone())),
        Some(_) => Err(ProtocolError::InvalidRequestField(key)),
    }
}

fn parse_client_metadata(object: &Map<String, Value>) -> Result<ClientMetadata, ProtocolError> {
    let Some(value) = object.get("client_metadata") else {
        return Ok(ClientMetadata::Absent);
    };
    let values = value
        .as_object()
        .ok_or(ProtocolError::InvalidRequestField("client_metadata"))?;
    if values.len() > MAX_CLIENT_METADATA_ENTRIES {
        return Err(ProtocolError::InvalidRequestField("client_metadata"));
    }
    let mut total_value_bytes = 0usize;
    let mut parsed = Vec::with_capacity(values.len());
    for (key, value) in values {
        if key.is_empty() || key.len() > MAX_CLIENT_METADATA_KEY_BYTES {
            return Err(ProtocolError::InvalidRequestField("client_metadata"));
        }
        let value = value
            .as_str()
            .ok_or(ProtocolError::InvalidRequestField("client_metadata"))?;
        if value.len() > MAX_CLIENT_METADATA_VALUE_BYTES {
            return Err(ProtocolError::InvalidRequestField("client_metadata"));
        }
        total_value_bytes = total_value_bytes
            .checked_add(value.len())
            .ok_or(ProtocolError::InvalidRequestField("client_metadata"))?;
        if total_value_bytes > MAX_CLIENT_METADATA_TOTAL_VALUE_BYTES {
            return Err(ProtocolError::InvalidRequestField("client_metadata"));
        }
        parsed.push((key.clone(), value.to_owned()));
    }
    Ok(ClientMetadata::Values(parsed))
}

fn required_string_array(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<Vec<String>, ProtocolError> {
    required_array(object, key)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or(ProtocolError::InvalidRequestField(key))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextStreamState {
    AwaitingCreated,
    Streaming,
    Completed,
    Failed,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextStreamEventKind {
    ResponseCreated {
        response_id: String,
    },
    ResponseInProgress {
        response_id: String,
    },
    OutputItemAdded {
        item_id: String,
        output_index: u64,
    },
    ContentPartAdded {
        item_id: String,
        output_index: u64,
        content_index: u64,
        text: String,
    },
    OutputTextDelta {
        item_id: String,
        output_index: u64,
        content_index: u64,
        delta: String,
    },
    OutputTextDone {
        item_id: String,
        output_index: u64,
        content_index: u64,
        text: String,
    },
    ContentPartDone {
        item_id: String,
        output_index: u64,
        content_index: u64,
        text: String,
    },
    OutputItemDone {
        item_id: String,
        output_index: u64,
        text: String,
    },
    FunctionCallAdded {
        item_id: String,
        output_index: u64,
        call_id: String,
        name: String,
    },
    FunctionCallArgumentsDelta {
        item_id: String,
        output_index: u64,
        delta: String,
    },
    FunctionCallArgumentsDone {
        item_id: String,
        output_index: u64,
        arguments: String,
    },
    FunctionCallItemDone {
        item_id: String,
        output_index: u64,
        call_id: String,
        name: String,
        arguments: String,
    },
    ReasoningItemAdded {
        item_id: String,
        output_index: u64,
    },
    ReasoningSummaryPartAdded {
        item_id: String,
        output_index: u64,
        summary_index: u64,
        text: String,
    },
    ReasoningSummaryTextDelta {
        item_id: String,
        output_index: u64,
        summary_index: u64,
        delta: String,
    },
    ReasoningSummaryTextDone {
        item_id: String,
        output_index: u64,
        summary_index: u64,
        text: String,
    },
    ReasoningSummaryPartDone {
        item_id: String,
        output_index: u64,
        summary_index: u64,
        text: String,
    },
    ReasoningTextDelta {
        item_id: String,
        output_index: u64,
        content_index: u64,
        delta: String,
    },
    ReasoningTextDone {
        item_id: String,
        output_index: u64,
        content_index: u64,
        text: String,
    },
    ReasoningItemDone {
        item_id: String,
        output_index: u64,
        encrypted_content: Option<String>,
    },
    OpaqueItemAdded {
        item_id: String,
        output_index: u64,
        item_type: String,
    },
    OpaqueItemDone {
        item_id: String,
        output_index: u64,
        item_type: String,
    },
    ResponseFailed {
        response_id: String,
    },
    ResponseIncomplete {
        response_id: String,
    },
    Passthrough {
        event_type: String,
    },
    ResponseCompleted {
        response_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedTextStreamEvent {
    sequence_number: u64,
    kind: TextStreamEventKind,
    original: Value,
}

impl ValidatedTextStreamEvent {
    pub fn sequence_number(&self) -> u64 {
        self.sequence_number
    }

    pub fn kind(&self) -> &TextStreamEventKind {
        &self.kind
    }

    pub fn original(&self) -> &Value {
        &self.original
    }

    pub fn into_original(self) -> Value {
        self.original
    }

    /// Project one validated Grok event onto the Codex-facing Responses
    /// boundary while marking provider-bound encrypted reasoning state.
    ///
    /// OpenAI-compatible reasoning ciphertext is not portable across
    /// providers. Codex persists completed reasoning items and may replay them
    /// after a model-picker change, so the bridge wraps Grok's opaque value
    /// with provenance. A later Grok request unwraps it, while a native GPT
    /// request removes the complete foreign reasoning item before forwarding.
    pub fn into_codex_value(mut self) -> Value {
        mark_provider_bound_reasoning(&mut self.original);
        self.original
    }

    pub(crate) fn restore_namespaced_tool_calls(
        &mut self,
        namespace_projection: &NamespaceToolProjection,
    ) {
        namespace_projection.restore_response_event(&mut self.original);
    }
}

fn mark_provider_bound_reasoning(event: &mut Value) {
    let Some(event) = event.as_object_mut() else {
        return;
    };
    match event.get("type").and_then(Value::as_str) {
        Some("response.output_item.done") => {
            if let Some(item) = event.get_mut("item").and_then(Value::as_object_mut) {
                mark_reasoning_ciphertext(item);
            }
        }
        Some("response.completed") => {
            if let Some(output) = event
                .get_mut("response")
                .and_then(Value::as_object_mut)
                .and_then(|response| response.get_mut("output"))
                .and_then(Value::as_array_mut)
            {
                for item in output.iter_mut().filter_map(Value::as_object_mut) {
                    mark_reasoning_ciphertext(item);
                }
            }
        }
        _ => {}
    }
}

fn mark_reasoning_ciphertext(item: &mut Map<String, Value>) {
    if item.get("type").and_then(Value::as_str) != Some("reasoning") {
        return;
    }
    if let Some(Value::String(value)) = item.get_mut("encrypted_content")
        && !value.starts_with(GROK_REASONING_ENVELOPE_PREFIX)
    {
        value.insert_str(0, GROK_REASONING_ENVELOPE_PREFIX);
    }
}

#[derive(Debug, Clone)]
pub struct TextStreamValidator {
    state: TextStreamState,
    last_sequence: Option<u64>,
    response_id: Option<String>,
}
impl Default for TextStreamValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl TextStreamValidator {
    pub fn new() -> Self {
        Self {
            state: TextStreamState::AwaitingCreated,
            last_sequence: None,
            response_id: None,
        }
    }

    pub fn state(&self) -> TextStreamState {
        self.state
    }

    pub fn is_completed(&self) -> bool {
        self.state == TextStreamState::Completed
    }

    fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            TextStreamState::Completed | TextStreamState::Failed | TextStreamState::Incomplete
        )
    }

    pub fn finish(&self) -> Result<(), ProtocolError> {
        // The provider may close after its terminal payload without emitting
        // every Responses lifecycle marker.  The transport has already
        // forwarded all useful events, so EOF is not a schema violation.
        Ok(())
    }

    /// Codex treats a Responses SSE close without `response.completed` as a
    /// transport failure. Grok can end after the last useful item, so the
    /// bridge supplies only that lifecycle marker. This is not a substitute
    /// for `response.failed` / `response.incomplete`, and it does not invent
    /// output items that were never streamed.
    pub fn synthetic_completed_on_eof(&mut self) -> Option<ValidatedTextStreamEvent> {
        if self.is_terminal() || self.state == TextStreamState::AwaitingCreated {
            return None;
        }
        let response_id = self
            .response_id
            .clone()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| format!("resp_{}", Uuid::new_v4()));
        let sequence_number = self.last_sequence.map_or(0, |n| n.saturating_add(1));
        let original = serde_json::json!({
            "type": "response.completed",
            "sequence_number": sequence_number,
            "response": {
                "id": response_id,
                "status": "completed",
                "output": []
            }
        });
        self.state = TextStreamState::Completed;
        self.last_sequence = Some(sequence_number);
        self.response_id = Some(response_id.clone());
        Some(ValidatedTextStreamEvent {
            sequence_number,
            kind: TextStreamEventKind::ResponseCompleted { response_id },
            original,
        })
    }

    pub fn accept_data(&mut self, data: &str) -> Result<ValidatedTextStreamEvent, ProtocolError> {
        let value: Value =
            serde_json::from_str(data).map_err(|_| ProtocolError::InvalidSsePayload)?;
        self.accept_value(value)
    }

    pub fn accept_value(
        &mut self,
        value: Value,
    ) -> Result<ValidatedTextStreamEvent, ProtocolError> {
        return self.accept_value_permissive(value);
    }

    /// Convert only the event families understood by Codex.  xAI emits
    /// auxiliary lifecycle/tool-search events that are not part of the
    /// Responses surface; they must pass through without becoming a bridge
    /// protocol failure.  No cross-event ordering or replay state is needed
    /// for a full-history request.
    fn accept_value_permissive(
        &mut self,
        value: Value,
    ) -> Result<ValidatedTextStreamEvent, ProtocolError> {
        let object = value.as_object().ok_or(ProtocolError::InvalidSsePayload)?;
        let event_type = object.get("type").and_then(Value::as_str).unwrap_or("");
        let sequence_number = object
            .get("sequence_number")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| self.last_sequence.map_or(0, |n| n + 1));
        self.last_sequence = Some(sequence_number);
        let response = object.get("response").and_then(Value::as_object);
        let item = object.get("item").and_then(Value::as_object);
        let string = |source: Option<&Map<String, Value>>, key: &str| {
            source.and_then(|map| map.get(key)).and_then(Value::as_str).unwrap_or("").to_owned()
        };
        let id = string(item, "id");
        let item_id = if id.is_empty() { string(Some(object), "item_id") } else { id };
        let output_index = object.get("output_index").and_then(Value::as_u64).unwrap_or(0);
        let content_index = object.get("content_index").and_then(Value::as_u64).unwrap_or(0);
        let summary_index = object.get("summary_index").and_then(Value::as_u64).unwrap_or(0);
        let delta = string(Some(object), "delta");
        let text = string(Some(object), "text");
        let response_id = string(response, "id");
        if !response_id.is_empty() {
            self.response_id.get_or_insert_with(|| response_id.clone());
        }
        let kind = match event_type {
            "response.created" => TextStreamEventKind::ResponseCreated { response_id },
            "response.in_progress" => TextStreamEventKind::ResponseInProgress { response_id },
            "response.output_item.added" => TextStreamEventKind::OutputItemAdded { item_id, output_index },
            "response.content_part.added" => TextStreamEventKind::ContentPartAdded { item_id, output_index, content_index, text },
            "response.output_text.delta" => TextStreamEventKind::OutputTextDelta { item_id, output_index, content_index, delta },
            "response.output_text.done" => TextStreamEventKind::OutputTextDone { item_id, output_index, content_index, text },
            "response.content_part.done" => TextStreamEventKind::ContentPartDone { item_id, output_index, content_index, text },
            "response.output_item.done" => TextStreamEventKind::OutputItemDone { item_id, output_index, text },
            "response.function_call_arguments.delta" => TextStreamEventKind::FunctionCallArgumentsDelta { item_id, output_index, delta },
            "response.function_call_arguments.done" => TextStreamEventKind::FunctionCallArgumentsDone { item_id, output_index, arguments: string(Some(object), "arguments") },
            "response.reasoning_summary_part.added" => TextStreamEventKind::ReasoningSummaryPartAdded { item_id, output_index, summary_index, text },
            "response.reasoning_summary_text.delta" => TextStreamEventKind::ReasoningSummaryTextDelta { item_id, output_index, summary_index, delta },
            "response.reasoning_summary_text.done" => TextStreamEventKind::ReasoningSummaryTextDone { item_id, output_index, summary_index, text },
            "response.reasoning_summary_part.done" => TextStreamEventKind::ReasoningSummaryPartDone { item_id, output_index, summary_index, text },
            "response.reasoning_text.delta" => TextStreamEventKind::ReasoningTextDelta { item_id, output_index, content_index, delta },
            "response.reasoning_text.done" => TextStreamEventKind::ReasoningTextDone { item_id, output_index, content_index, text },
            "response.function_call.added" => TextStreamEventKind::FunctionCallAdded { item_id, output_index, call_id: string(item, "call_id"), name: string(item, "name") },
            "response.reasoning.added" => TextStreamEventKind::ReasoningItemAdded { item_id, output_index },
            "response.reasoning.done" => TextStreamEventKind::ReasoningItemDone { item_id, output_index, encrypted_content: item.and_then(|i| i.get("encrypted_content")).and_then(Value::as_str).map(str::to_owned) },
            "response.completed" => TextStreamEventKind::ResponseCompleted { response_id },
            "response.failed" => TextStreamEventKind::ResponseFailed { response_id },
            "response.incomplete" => TextStreamEventKind::ResponseIncomplete { response_id },
            other => TextStreamEventKind::Passthrough { event_type: other.to_owned() },
        };
        self.state = match &kind {
            TextStreamEventKind::ResponseCompleted { .. } => TextStreamState::Completed,
            TextStreamEventKind::ResponseFailed { .. } => TextStreamState::Failed,
            TextStreamEventKind::ResponseIncomplete { .. } => TextStreamState::Incomplete,
            _ => TextStreamState::Streaming,
        };
        Ok(ValidatedTextStreamEvent { sequence_number, kind, original: value })
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request_fixture() -> Value {
        json!({
            "model": "grok-4.6",
            "instructions": "Keep  two spaces.\nNo rewrite.",
            "input": [
                {"type":"message", "id":"msg_user_1", "role":"user", "content":[
                    {"type":"input_text", "text":"first\n"},
                    {"type":"input_text", "text":"  second"}
                ]},
                {"type":"message", "role":"assistant", "content":[
                    {"type":"output_text", "text":"prior answer"}
                ]}
            ],
            "tools": [], "tool_choice":"auto", "parallel_tool_calls":false,
            "reasoning":{"effort":"high", "summary":"auto"}, "store":false, "stream":true,
            "stream_options":{"include_obfuscation":false},
            "include":["reasoning.encrypted_content"], "service_tier":"default",
            "prompt_cache_key":"cache-key", "text":{"verbosity":"medium"},
            "client_metadata":{"originator":"codex", "kind":"agent"}
        })
    }

    #[test]
    fn codex_state_fields_are_validated_and_not_forwarded_to_grok() {
        let mut original = request_fixture();
        original["prompt_cache_retention"] = json!("24h");
        original["previous_response_id"] = json!("resp_previous");
        let normalized = NormalizedResponsesRequest::parse(original).unwrap();
        let upstream = normalized.to_xai_value();
        assert!(upstream.get("prompt_cache_retention").is_none());
        assert!(upstream.get("previous_response_id").is_none());

        let mut malformed = request_fixture();
        malformed["previous_response_id"] = json!(42);
        assert_eq!(
            NormalizedResponsesRequest::parse(malformed).unwrap_err(),
            ProtocolError::InvalidRequestField("previous_response_id")
        );
    }

    #[test]
    fn tool_search_is_projected_as_one_provider_function() {
        let mut request = request_fixture();
        request["tools"] = json!([{
            "type": "tool_search",
            "execution": "client",
            "description": "Search deferred tools.",
            "parameters": {
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            }
        }]);
        let normalized = NormalizedResponsesRequest::parse(request).unwrap();
        assert_eq!(
            normalized.to_xai_value()["tools"].as_array().unwrap().len(),
            1
        );
        assert_eq!(normalized.to_xai_value()["tools"][0]["name"], "tool_search");
    }

    #[test]
    fn discovered_tool_search_namespace_is_replayed_and_projected() {
        let mut request = request_fixture();
        request["tools"] = json!([{
            "type": "tool_search",
            "execution": "client",
            "description": "Search deferred tools.",
            "parameters": {"type": "object", "properties": {}}
        }]);
        request["input"] = json!([
            {
                "type": "tool_search_call", "id": "ts_1", "call_id": "search-1",
                "execution": "client", "arguments": {"query": "calendar"},
                "internal_chat_message_metadata_passthrough": {"turn_id": "turn-1"}
            },
            {"type": "tool_search_output", "call_id": "search-1", "status": "completed", "execution": "client", "tools": [{
                "type": "namespace", "name": "mcp__calendar", "description": "Calendar tools.", "tools": [{
                    "type": "function", "name": "create", "description": "Create event.",
                    "parameters": {"type": "object", "properties": {}}, "defer_loading": true
                }]
            }]},
            {"type": "function_call", "call_id": "call-1", "name": "create", "namespace": "mcp__calendar", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "call-1", "output": "ok"}
        ]);
        let upstream = NormalizedResponsesRequest::parse(request)
            .unwrap()
            .to_xai_value();
        assert_eq!(upstream["tools"].as_array().unwrap().len(), 2);
        assert_eq!(upstream["tools"][1]["name"], "mcp__calendar__create");
        assert_eq!(upstream["input"][2]["name"], "mcp__calendar__create");
        assert_eq!(upstream["input"][0]["name"], "tool_search");
        assert!(upstream["input"][0].get("id").is_none());
        assert!(
            upstream["input"][0]
                .get(INTERNAL_MESSAGE_METADATA_FIELD)
                .is_none()
        );
    }

    #[test]
    fn xai_projection_omits_unresolvable_ref_without_dropping_spawn_agent() {
        let mut request = request_fixture();
        request["tools"] = json!([{
            "type": "tool_search",
            "execution": "client",
            "description": "Search deferred tools.",
            "parameters": {"type": "object", "properties": {"query": {"type": "string"}}}
        }]);
        request["input"] = json!([
            {
                "type": "tool_search_call",
                "call_id": "search-1",
                "execution": "client",
                "arguments": {"query": "spawn subagent official Codex multi-agent tools"}
            },
            {
                "type": "tool_search_output",
                "call_id": "search-1",
                "status": "completed",
                "execution": "client",
                "tools": [
                    {
                        "type": "namespace",
                        "name": "multi_agent_v1",
                        "description": "Tools for spawning and managing sub-agents.",
                        "tools": [{
                            "type": "function",
                            "name": "spawn_agent",
                            "description": "Spawn a sub-agent.",
                            "strict": false,
                            "defer_loading": true,
                            "parameters": {
                                "type": "object",
                                "properties": {"message": {"type": "string"}},
                                "additionalProperties": false
                            }
                        }]
                    },
                    {
                        "type": "namespace",
                        "name": "mcp__agent_video_studio",
                        "description": "Tools in the mcp__agent_video_studio namespace.",
                        "tools": [
                            {
                                "type": "function",
                                "name": "inspect_auto_edit_plan",
                                "description": "Validate an inline avs.auto-edit-plan.v1 from Codex.",
                                "strict": false,
                                "defer_loading": true,
                                "parameters": {
                                    "type": "object",
                                    "properties": {
                                        "plan": {
                                            "type": "object",
                                            "properties": {
                                                "metadata": {
                                                    "$ref": "#/properties/plan/properties/operations/items/anyOf/1/properties/clips/items/properties/properties/properties/metadata"
                                                },
                                                "operations": {
                                                    "type": "array",
                                                    "items": {}
                                                }
                                            },
                                            "additionalProperties": false
                                        }
                                    },
                                    "required": ["plan"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "type": "function",
                                "name": "plan_silence_removal",
                                "description": "Build a silence-removal plan for Codex.",
                                "strict": false,
                                "defer_loading": true,
                                "parameters": {
                                    "type": "object",
                                    "properties": {
                                        "metadata": {
                                            "type": "object",
                                            "properties": {},
                                            "additionalProperties": {
                                                "anyOf": [
                                                    {"type": "string"},
                                                    {
                                                        "type": "array",
                                                        "items": {
                                                            "$ref": "#/properties/metadata/additionalProperties"
                                                        }
                                                    },
                                                    {
                                                        "type": "object",
                                                        "properties": {},
                                                        "additionalProperties": {
                                                            "$ref": "#/properties/metadata/additionalProperties"
                                                        }
                                                    }
                                                ]
                                            }
                                        }
                                    },
                                    "additionalProperties": false
                                }
                            }
                        ]
                    }
                ]
            }
        ]);

        assert_eq!(
            upstream_tool_names(&request),
            vec![
                "tool_search",
                "multi_agent_v1__spawn_agent",
                "mcp__agent_video_studio__plan_silence_removal"
            ]
        );
    }

    #[test]
    fn xai_projection_omits_documented_request_killing_schemas_and_keeps_resolvable_refs() {
        let mut request = request_fixture();
        request["tools"] = json!([
            {
                "type": "function",
                "name": "boolean_property",
                "description": "Illegal boolean property schema.",
                "parameters": {
                    "type": "object",
                    "properties": {"flag": true}
                }
            },
            {
                "type": "function",
                "name": "empty_any_of",
                "description": "Illegal empty anyOf.",
                "parameters": {
                    "type": "object",
                    "properties": {"value": {"anyOf": []}}
                }
            },
            {
                "type": "function",
                "name": "tuple_items",
                "description": "Illegal tuple items.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pair": {"type": "array", "items": [{"type": "string"}, {"type": "number"}]}
                    }
                }
            },
            {
                "type": "function",
                "name": "resolvable_ref",
                "description": "Legal local $ref.",
                "parameters": {
                    "type": "object",
                    "properties": {"item": {"$ref": "#/$defs/item"}},
                    "$defs": {
                        "item": {
                            "type": "object",
                            "properties": {"id": {"type": "string"}}
                        }
                    }
                }
            }
        ]);

        assert_eq!(upstream_tool_names(&request), vec!["resolvable_ref"]);
    }

    fn upstream_tool_names(request: &Value) -> Vec<String> {
        NormalizedResponsesRequest::parse(request.clone())
            .unwrap()
            .to_xai_value()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_owned))
            .collect()
    }

    #[test]
    fn discovered_tool_search_namespace_rediscovery_is_idempotent() {
        let mut request = request_fixture();
        request["tools"] = json!([{
            "type": "tool_search",
            "execution": "client",
            "description": "Search deferred tools.",
            "parameters": {"type": "object", "properties": {}}
        }]);
        request["input"] = json!([
            {
                "type": "tool_search_call", "call_id": "search-1",
                "execution": "client", "arguments": {"query": "Computer History"}
            },
            {"type": "tool_search_output", "call_id": "search-1", "status": "completed", "execution": "client", "tools": [{
                "type": "namespace", "name": "mcp__computer_history", "description": "History tools.",
                "tools": [
                    {
                        "type": "function", "name": "computer_history_status", "description": "Status.",
                        "strict": false, "parameters": {"type": "object", "properties": {}, "additionalProperties": false},
                        "defer_loading": true
                    },
                    {
                        "type": "function", "name": "computer_history_pause", "description": "Pause.",
                        "strict": false, "parameters": {"type": "object", "properties": {}, "additionalProperties": false},
                        "defer_loading": true
                    }
                ]
            }]},
            {
                "type": "tool_search_call", "call_id": "search-2",
                "execution": "client", "arguments": {"query": "computer_history_status"}
            },
            {"type": "tool_search_output", "call_id": "search-2", "status": "completed", "execution": "client", "tools": [{
                "type": "namespace", "name": "mcp__computer_history", "description": "History tools.",
                "tools": [{
                    "type": "function", "name": "computer_history_status", "description": "Status.",
                    "strict": false, "parameters": {"type": "object", "properties": {}, "additionalProperties": false}
                }]
            }]}
        ]);

        let upstream = NormalizedResponsesRequest::parse(request)
            .unwrap()
            .to_xai_value();
        let names = upstream["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "tool_search",
                "mcp__computer_history__computer_history_status",
                "mcp__computer_history__computer_history_pause"
            ]
        );
    }

    #[test]
    fn discovered_tool_search_conflicting_rediscovery_fails_closed() {
        let mut request = request_fixture();
        request["tools"] = json!([{
            "type": "tool_search", "execution": "client",
            "description": "Search deferred tools.",
            "parameters": {"type": "object", "properties": {}}
        }]);
        request["input"] = json!([
            {
                "type": "tool_search_call", "call_id": "search-1",
                "execution": "client", "arguments": {"query": "status"}
            },
            {"type": "tool_search_output", "call_id": "search-1", "status": "completed", "execution": "client", "tools": [{
                "type": "namespace", "name": "mcp__history", "description": "History tools.",
                "tools": [{
                    "type": "function", "name": "status", "description": "Status.",
                    "parameters": {"type": "object", "properties": {}}
                }]
            }]},
            {
                "type": "tool_search_call", "call_id": "search-2",
                "execution": "client", "arguments": {"query": "status"}
            },
            {"type": "tool_search_output", "call_id": "search-2", "status": "completed", "execution": "client", "tools": [{
                "type": "namespace", "name": "mcp__history", "description": "History tools.",
                "tools": [{
                    "type": "function", "name": "status", "description": "Conflicting status.",
                    "parameters": {"type": "object", "properties": {}}
                }]
            }]}
        ]);

        assert_eq!(
            NormalizedResponsesRequest::parse(request).unwrap_err(),
            ProtocolError::DuplicateToolName
        );
    }

    #[test]
    fn tool_search_sse_function_call_is_restored_to_native_shape() {
        let mut request = request_fixture();
        request["tools"] = json!([{
            "type": "tool_search", "execution": "client",
            "description": "Search deferred tools.",
            "parameters": {"type": "object", "properties": {}}
        }]);
        let projection = NormalizedResponsesRequest::parse(request)
            .unwrap()
            .namespace_projection();
        let mut event = json!({
            "type": "response.output_item.done", "sequence_number": 1,
            "output_index": 0, "item": {
                "type": "function_call", "id": "fc-1", "call_id": "search-1",
                "name": "tool_search", "arguments": "{\"query\":\"calendar\"}", "status": "completed"
            }
        });
        projection.restore_response_event(&mut event);
        assert_eq!(event["item"]["type"], "tool_search_call");
        assert_eq!(event["item"]["execution"], "client");
        assert_eq!(event["item"]["arguments"]["query"], "calendar");
        assert!(event["item"].get("name").is_none());
    }

    #[test]
    fn integer_valued_json_floats_become_integers() {
        let mut value = json!({
            "limit": 8.0,
            "nested": {"waitMs": 120000.0, "ratio": 0.25},
            "flags": [1.0, 1.5, -2.0]
        });
        canonicalize_integer_valued_numbers(&mut value);
        assert_eq!(value["limit"].as_i64(), Some(8));
        assert_eq!(value["nested"]["waitMs"].as_i64(), Some(120000));
        assert_eq!(value["nested"]["ratio"].as_f64(), Some(0.25));
        assert!(value["nested"]["ratio"].as_i64().is_none());
        assert_eq!(value["flags"][0].as_i64(), Some(1));
        assert_eq!(value["flags"][1].as_f64(), Some(1.5));
        assert!(value["flags"][1].as_i64().is_none());
        assert_eq!(value["flags"][2].as_i64(), Some(-2));
    }

    #[test]
    fn tool_search_float_limit_is_restored_as_json_integer() {
        let mut request = request_fixture();
        request["tools"] = json!([{
            "type": "tool_search", "execution": "client",
            "description": "Search deferred tools.",
            "parameters": {"type": "object", "properties": {}}
        }]);
        let projection = NormalizedResponsesRequest::parse(request)
            .unwrap()
            .namespace_projection();
        let mut event = json!({
            "type": "response.output_item.done", "sequence_number": 1,
            "output_index": 0, "item": {
                "type": "function_call", "id": "fc-1", "call_id": "search-1",
                "name": "tool_search",
                "arguments": "{\"limit\":8.0,\"query\":\"generate_image_grid\"}",
                "status": "completed"
            }
        });
        projection.restore_response_event(&mut event);
        assert_eq!(event["item"]["type"], "tool_search_call");
        assert_eq!(event["item"]["arguments"]["limit"].as_i64(), Some(8));
        assert_eq!(event["item"]["arguments"]["query"], "generate_image_grid");
    }

    #[test]
    fn host_tool_float_arguments_are_restored_as_json_integers() {
        let mut event = json!({
            "type": "response.output_item.done", "sequence_number": 1,
            "output_index": 0, "item": {
                "type": "function_call", "id": "fc-1", "call_id": "call-1",
                "name": "write_stdin",
                "arguments": "{\"session_id\":42337.0,\"chars\":\"x\",\"yield_time_ms\":250.0}"
            }
        });
        NamespaceToolProjection::default().restore_response_event(&mut event);
        let parsed: Value = serde_json::from_str(
            event["item"]["arguments"].as_str().expect("function arguments"),
        )
        .unwrap();
        assert_eq!(parsed["session_id"].as_i64(), Some(42337));
        assert_eq!(parsed["yield_time_ms"].as_i64(), Some(250));
        assert_eq!(parsed["chars"], "x");
    }

    #[test]
    fn function_call_arguments_done_float_is_canonicalized() {
        let mut event = json!({
            "type": "response.function_call_arguments.done",
            "item_id": "fc-1",
            "output_index": 0,
            "arguments": "{\"limit\":12.0,\"query\":\"grid\",\"temperature\":0.7}"
        });
        NamespaceToolProjection::default().restore_response_event(&mut event);
        let parsed: Value =
            serde_json::from_str(event["arguments"].as_str().expect("done arguments")).unwrap();
        assert_eq!(parsed["limit"].as_i64(), Some(12));
        assert_eq!(parsed["query"], "grid");
        assert_eq!(parsed["temperature"].as_f64(), Some(0.7));
        assert!(parsed["temperature"].as_i64().is_none());
    }

    #[test]
    fn tool_search_replay_float_limit_is_projected_as_json_integer() {
        let mut request = request_fixture();
        request["tools"] = json!([{
            "type": "tool_search", "execution": "client",
            "description": "Search deferred tools.",
            "parameters": {"type": "object", "properties": {}}
        }]);
        request["input"] = json!([
            {"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}]},
            {"type":"tool_search_call","call_id":"search_1","execution":"client","arguments":{"limit":15.0,"query":"rust"}}
        ]);
        let projected = NormalizedResponsesRequest::parse(request)
            .unwrap()
            .to_xai_value();
        let call = projected["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "tool_search")
            .expect("projected tool_search call");
        let parsed: Value =
            serde_json::from_str(call["arguments"].as_str().expect("projected arguments")).unwrap();
        assert_eq!(parsed["limit"].as_i64(), Some(15));
        assert_eq!(parsed["query"], "rust");
    }

    #[test]
    fn provider_tool_limit_fails_closed_before_upstream() {
        let mut request = request_fixture();
        request["tools"] = Value::Array(
            (0..129)
                .map(|index| {
                    json!({
                        "type": "function", "name": format!("tool_{index}"), "description": "",
                        "parameters": {"type": "object", "properties": {}}
                    })
                })
                .collect(),
        );
        assert_eq!(
            NormalizedResponsesRequest::parse(request).unwrap_err(),
            ProtocolError::TooManyTools
        );
    }

    #[test]
    fn upstream_omits_tool_choice_for_absent_null_and_empty_tools() {
        for tools in [None, Some(Value::Null), Some(json!([]))] {
            let mut request = request_fixture();
            match tools {
                None => {
                    request.as_object_mut().unwrap().remove("tools");
                }
                Some(tools) => request["tools"] = tools,
            }

            let upstream = NormalizedResponsesRequest::parse(request)
                .unwrap()
                .to_xai_value();
            assert!(upstream.get("tool_choice").is_none());
            assert_eq!(upstream["parallel_tool_calls"], false);
        }
    }

    #[test]
    fn codex_openai_message_metadata_is_validated_and_ends_at_grok_boundary() {
        let mut request = request_fixture();
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

        let upstream = NormalizedResponsesRequest::parse(request)
            .unwrap()
            .to_xai_value();
        assert_eq!(
            upstream["input"],
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
    }

    fn two_turn_reasoning_request() -> Value {
        let mut request = request_fixture();
        request["input"] = json!([
            {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "run the marker"}]
            },
            {
                "type": "reasoning",
                "id": "reasoning_turn_1",
                "summary": [{"type": "summary_text", "text": "Use the shell tool."}],
                "content": [{"type": "reasoning_text", "text": "Need the marker value."}],
                "encrypted_content": format!("{GROK_REASONING_ENVELOPE_PREFIX}opaque-encrypted-turn-state")
            },
            {
                "type": "message", "id": "msg_turn_1", "role": "assistant",
                "content": [{"type": "output_text", "text": "Checking."}]
            },
            {
                "type": "function_call", "id": "fc_turn_1", "name": "shell",
                "arguments": "{\"command\":\"cat ./tool-marker.txt\"}",
                "call_id": "call_turn_1"
            },
            {
                "type": "function_call_output", "call_id": "call_turn_1",
                "id": "fco_turn_1",
                "output": "marker-value"
            }
        ]);
        request
    }

    #[test]
    fn model_switch_replays_only_bridge_enveloped_reasoning_to_grok() {
        let original = two_turn_reasoning_request();
        let normalized = NormalizedResponsesRequest::parse(original.clone()).unwrap();
        let mut expected = original;
        expected.as_object_mut().unwrap().remove("client_metadata");
        expected.as_object_mut().unwrap().remove("tool_choice");
        expected["input"][1]["encrypted_content"] = json!("opaque-encrypted-turn-state");
        expected["input"][2].as_object_mut().unwrap().remove("id");
        expected["input"][3].as_object_mut().unwrap().remove("id");
        expected["input"][4].as_object_mut().unwrap().remove("id");
        expected["input"][2]["content"] = json!("Checking.");
        assert_eq!(normalized.to_xai_value(), expected);
        assert_eq!(
            normalized.to_xai_value()["input"][1]["encrypted_content"],
            "opaque-encrypted-turn-state"
        );
        assert_eq!(
            normalized.to_xai_value()["input"][1]["id"],
            "reasoning_turn_1"
        );
        assert_eq!(
            normalized.to_xai_value()["input"][1]["content"],
            json!([{"type": "reasoning_text", "text": "Need the marker value."}])
        );
        assert_eq!(
            normalized.to_xai_value()["input"][3]["call_id"],
            "call_turn_1"
        );

        let mut no_reasoning_text = two_turn_reasoning_request();
        no_reasoning_text["input"][1]["content"] = Value::Null;
        let normalized = NormalizedResponsesRequest::parse(no_reasoning_text).unwrap();
        assert!(
            normalized.to_xai_value()["input"][1]
                .as_object()
                .unwrap()
                .get("content")
                .is_none()
        );

        let mut foreign_reasoning = two_turn_reasoning_request();
        foreign_reasoning["input"][1]["encrypted_content"] = json!("native-gpt-ciphertext");
        let normalized = NormalizedResponsesRequest::parse(foreign_reasoning).unwrap();
        assert_eq!(
            normalized.to_xai_value()["input"].as_array().unwrap().len(),
            4
        );
        assert!(
            normalized.to_xai_value()["input"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["type"] != "reasoning")
        );
    }

    #[test]
    fn model_switch_native_only_history_stays_byte_exact_without_foreign_replay_state() {
        let mut native_only = json!({
            "store": false,
            "input": [
                {
                    "type": "reasoning", "id": "rs_native",
                    "summary": [], "encrypted_content": "native-gpt-ciphertext"
                },
                {
                    "type": "message", "id": "msg_native", "role": "user",
                    "content": [{"type": "input_text", "text": "continue"}]
                },
                {
                    "type": "tool_search_call", "id": "tsc_native",
                    "call_id": "search-native", "execution": "client",
                    "arguments": {"query": "status"}
                }
            ]
        });
        let original = native_only.clone();
        assert!(!sanitize_unreplayable_history_for_native(&mut native_only));
        assert_eq!(native_only, original);
    }

    #[test]
    fn model_switch_mixed_provider_native_replay_drops_ids_and_keeps_tool_pairs() {
        let mut mixed = two_turn_reasoning_request();
        mixed["input"].as_array_mut().unwrap().push(json!({
            "type": "tool_search_call", "id": "msg_foreign_tool_search",
            "call_id": "search-1", "execution": "client",
            "arguments": {"limit": 8, "query": "generate_image_grid"}
        }));
        mixed["input"].as_array_mut().unwrap().push(json!({
            "type": "tool_search_call", "id": "tsc_native_tool_search",
            "call_id": "search-2", "execution": "client",
            "arguments": {"limit": 8, "query": "codex_image_grid"}
        }));

        assert!(sanitize_unreplayable_history_for_native(&mut mixed));
        let input = mixed["input"].as_array().unwrap();
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
        let function_call = input
            .iter()
            .find(|item| item["type"] == "function_call")
            .unwrap();
        let function_output = input
            .iter()
            .find(|item| item["type"] == "function_call_output")
            .unwrap();
        assert_eq!(function_call["call_id"], "call_turn_1");
        assert_eq!(function_output["call_id"], "call_turn_1");
        let searches = input
            .iter()
            .filter(|item| item["type"] == "tool_search_call")
            .collect::<Vec<_>>();
        assert_eq!(searches[0].get("id"), None);
        assert_eq!(searches[0]["call_id"], "search-1");
        assert_eq!(searches[1]["id"], "tsc_native_tool_search");
    }

    #[test]
    fn model_switch_legacy_foreign_tool_search_id_is_removed_without_reasoning_provenance() {
        let mut request = json!({
            "store": false,
            "input": [{
                "type": "tool_search_call",
                "id": "msg_e2a2848a-4ab3-9328-a1b4-c2318babb894",
                "call_id": "search-1",
                "execution": "client",
                "arguments": {"limit": 8, "query": "generate_image_grid"}
            }]
        });
        assert!(sanitize_unreplayable_history_for_native(&mut request));
        assert!(request["input"][0].get("id").is_none());
        assert_eq!(request["input"][0]["call_id"], "search-1");
    }

    #[test]
    fn model_switch_ignores_foreign_reasoning_order_before_grok() {
        let mut request = request_fixture();
        request["input"] = json!([
            {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "first"}]
            },
            {
                "type": "reasoning", "id": "rs_native_1",
                "summary": [], "encrypted_content": "native-gpt-ciphertext-1"
            },
            {
                "type": "message", "role": "assistant",
                "phase": "commentary",
                "content": [{"type": "output_text", "text": "working"}]
            },
            {
                "type": "custom_tool_call",
                "id": "ctc_native_1",
                "status": "completed",
                "call_id": "call_native_1",
                "name": "exec",
                "namespace": null,
                "input": "const result = await tools.exec_command({cmd: \"pwd\"});",
                "internal_chat_message_metadata_passthrough": {
                    "turn_id": "turn-native-1",
                    "create_time": 1.0
                }
            },
            {
                "type": "custom_tool_call_output",
                "id": "ctco_native_1",
                "call_id": "call_native_1",
                "name": null,
                "output": [{"type": "input_text", "text": "workspace"}],
                "internal_chat_message_metadata_passthrough": {
                    "turn_id": "turn-native-1",
                    "create_time": 2.0
                }
            },
            {
                "type": "reasoning", "id": "rs_native_2",
                "summary": [], "encrypted_content": "native-gpt-ciphertext-2"
            },
            {
                "type": "message", "role": "assistant",
                "phase": "final_answer",
                "content": [{"type": "output_text", "text": "done"}]
            },
            {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "continue with Grok"}]
            }
        ]);

        let upstream = NormalizedResponsesRequest::parse(request)
            .unwrap()
            .to_xai_value();
        let input = upstream["input"].as_array().unwrap();
        assert_eq!(input.len(), 4);
        assert!(input.iter().all(|item| item["type"] != "reasoning"));
        assert!(input.iter().all(|item| item["type"] != "custom_tool_call"));
        assert!(
            input
                .iter()
                .all(|item| item["type"] != "custom_tool_call_output")
        );
        assert!(input.iter().all(|item| item.get("phase").is_none()));
        assert_eq!(input[1]["content"], "working");
        assert_eq!(input[2]["content"], "done");
        assert_eq!(input[3]["content"][0]["text"], "continue with Grok");
    }

    #[test]
    fn model_switch_preserves_plaintext_and_drops_encrypted_cli_agent_messages() {
        let mut request = request_fixture();
        request["prompt_cache_key"] = json!("11111111-1111-4111-8111-111111111111");
        request["client_metadata"] = json!({
            "session_id": "11111111-1111-4111-8111-111111111111",
            "turn_id": "22222222-2222-4222-8222-222222222222",
            "x-codex-installation-id": "33333333-3333-4333-8333-333333333333"
        });
        request["input"] = json!([
            {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "first"}]
            },
            {
                "type": "agent_message",
                "id": "amsg_foreign_1",
                "author": "/root",
                "recipient": "/root/worker",
                "content": [
                    {"type": "input_text", "text": "Message Type: NEW_TASK\nPayload:\n"},
                    {"type": "encrypted_content", "encrypted_content": "native-gpt-agent-state"}
                ],
                "internal_chat_message_metadata_passthrough": {
                    "turn_id": "turn-agent-1",
                    "create_time": 1.0
                }
            },
            {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "second"}]
            },
            {
                "type": "agent_message",
                "id": "amsg_plaintext_1",
                "author": "/root/worker",
                "recipient": "/root",
                "content": [{
                    "type": "input_text",
                    "text": "Message Type: FINAL_ANSWER\nPayload:\nworker result"
                }],
                "internal_chat_message_metadata_passthrough": {
                    "turn_id": "turn-agent-2",
                    "create_time": 2.0
                }
            },
            {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "continue with Grok"}]
            }
        ]);

        let normalized = NormalizedResponsesRequest::parse(request).unwrap();
        assert_eq!(normalized.grok_routing_metadata().unwrap().turn_index(), 1);
        let upstream = normalized.to_xai_value();
        let input = upstream["input"].as_array().unwrap();
        assert_eq!(input.len(), 4);
        assert!(input.iter().all(|item| item["type"] != "agent_message"));
        assert_eq!(input[0]["content"][0]["text"], "first");
        assert_eq!(input[1]["content"][0]["text"], "second");
        assert_eq!(
            input[2]["content"],
            "Message Type: FINAL_ANSWER\nPayload:\nworker result"
        );
        assert_eq!(input[3]["content"][0]["text"], "continue with Grok");
    }

    #[test]
    fn prior_reasoning_rejects_malformed_oversize_unknown_and_bad_order() {
        let mut wrong_content = two_turn_reasoning_request();
        wrong_content["input"][1]["content"][0]["type"] = json!("text");
        assert_eq!(
            NormalizedResponsesRequest::parse(wrong_content).unwrap_err(),
            ProtocolError::InvalidRequestField("reasoning")
        );

        let mut unknown = two_turn_reasoning_request();
        unknown["input"][1]["status"] = json!("completed");
        assert!(NormalizedResponsesRequest::parse(unknown).is_ok());

        let mut oversized = two_turn_reasoning_request();
        oversized["input"][1]["encrypted_content"] =
            json!("x".repeat(MAX_ENCRYPTED_REASONING_BYTES + 1));
        assert_eq!(
            NormalizedResponsesRequest::parse(oversized).unwrap_err(),
            ProtocolError::InvalidRequestField("encrypted_content")
        );

    }

    #[test]
    fn current_codex_client_metadata_is_validated_and_not_forwarded_to_xai() {
        let mut original = request_fixture();
        original["prompt_cache_key"] = json!("11111111-1111-4111-8111-111111111111");
        original["client_metadata"] = json!({
            "x-codex-window-id": "window-id",
            "root_turn_id": "root-turn-id",
            "turn_id": "22222222-2222-4222-8222-222222222222",
            "x-codex-turn-metadata": "{\"request_kind\":\"turn\"}",
            "x-codex-installation-id": "33333333-3333-4333-8333-333333333333",
            "session_id": "11111111-1111-4111-8111-111111111111",
            "thread_id": "thread-id"
        });
        let normalized = NormalizedResponsesRequest::parse(original).unwrap();
        let routing = normalized.grok_routing_metadata().unwrap();
        assert_ne!(routing.conversation_id(), Uuid::nil());
        assert_ne!(routing.request_id(), Uuid::nil());
        assert_ne!(routing.agent_id(), Uuid::nil());
        assert_eq!(routing.turn_index(), 1);
        assert!(
            normalized
                .to_xai_value()
                .as_object()
                .unwrap()
                .get("client_metadata")
                .is_none()
        );
    }

    #[test]
    fn routing_metadata_uses_a_stable_history_anchor_when_codex_ids_are_absent() {
        let mut original = request_fixture();
        original.as_object_mut().unwrap().remove("prompt_cache_key");
        original.as_object_mut().unwrap().remove("client_metadata");

        let first = NormalizedResponsesRequest::parse(original.clone())
            .unwrap()
            .grok_routing_metadata()
            .unwrap();
        let second = NormalizedResponsesRequest::parse(original)
            .unwrap()
            .grok_routing_metadata()
            .unwrap();

        assert_eq!(first.conversation_id(), second.conversation_id());
        assert_eq!(first.turn_index(), 1);
        assert_ne!(first.conversation_id(), Uuid::nil());
    }

    #[test]
    fn grok_routing_ids_use_opening_history_and_constant_turn_index() {
        let mut current = request_fixture();
        current["prompt_cache_key"] = json!("11111111-1111-4111-8111-111111111111");
        current["client_metadata"] = json!({
            "session_id": "11111111-1111-4111-8111-111111111111",
            "turn_id": "22222222-2222-4222-8222-222222222222",
            "x-codex-installation-id": "33333333-3333-4333-8333-333333333333"
        });
        current["input"] = json!([
            {"type":"message","role":"developer","content":[{"type":"input_text","text":"policy"}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"environment"}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"one"}]},
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"two"}]}
        ]);
        let normalized = NormalizedResponsesRequest::parse(current.clone()).unwrap();
        assert_eq!(normalized.grok_routing_metadata().unwrap().turn_index(), 1);

        current["prompt_cache_key"] = json!("not-a-uuid");
        assert_eq!(
            NormalizedResponsesRequest::parse(current.clone())
                .unwrap()
                .grok_routing_metadata()
                .unwrap()
                .turn_index(),
            1
        );

        current["client_metadata"]["turn_id"] = json!("not-a-uuid");
        let routing = NormalizedResponsesRequest::parse(current)
            .unwrap()
            .grok_routing_metadata()
            .unwrap();
        assert_eq!(routing.turn_index(), 1);
        assert_ne!(routing.request_id(), routing.agent_id());
    }

    #[test]
    fn malformed_client_metadata_fails_closed() {
        let mut non_string = request_fixture();
        non_string["client_metadata"]["turn_id"] = json!(42);
        assert_eq!(
            NormalizedResponsesRequest::parse(non_string).unwrap_err(),
            ProtocolError::InvalidRequestField("client_metadata")
        );

        let mut null_metadata = request_fixture();
        null_metadata["client_metadata"] = Value::Null;
        assert_eq!(
            NormalizedResponsesRequest::parse(null_metadata).unwrap_err(),
            ProtocolError::InvalidRequestField("client_metadata")
        );

        let mut empty_key = request_fixture();
        empty_key["client_metadata"] = json!({"": "value"});
        assert_eq!(
            NormalizedResponsesRequest::parse(empty_key).unwrap_err(),
            ProtocolError::InvalidRequestField("client_metadata")
        );

        let mut too_many = request_fixture();
        let mut metadata = Map::new();
        for index in 0..=MAX_CLIENT_METADATA_ENTRIES {
            metadata.insert(format!("key-{index}"), Value::String("value".into()));
        }
        too_many["client_metadata"] = Value::Object(metadata);
        assert_eq!(
            NormalizedResponsesRequest::parse(too_many).unwrap_err(),
            ProtocolError::InvalidRequestField("client_metadata")
        );
    }

    #[test]
    fn request_rejects_unknown_nontext_and_tool_surfaces() {
        let mut unknown = request_fixture();
        unknown["unverified"] = json!(true);
        assert!(NormalizedResponsesRequest::parse(unknown).is_ok());

        let mut tools = request_fixture();
        tools["tools"] = json!([{"type": "web_search"}]);
        assert_eq!(
            NormalizedResponsesRequest::parse(tools).unwrap_err(),
            ProtocolError::UnsupportedTools
        );

        let mut choice = request_fixture();
        choice["tool_choice"] = json!("required");
        assert_eq!(
            NormalizedResponsesRequest::parse(choice).unwrap_err(),
            ProtocolError::UnknownToolChoice
        );
    }

    #[test]
    fn failed_tool_search_replay_keeps_text_and_omits_empty_output() {
        let mut request = request_fixture();
        request["input"] = json!([
            {"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}],"history_metadata":{"opaque":true}},
            {"type":"tool_search_call","call_id":"search_1","execution":"client","arguments":{"limit":15.0,"query":"rust"}},
            {"type":"function_call_output","call_id":"","output":"tool parser error"},
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"still continue"}]}
        ]);
        let normalized = NormalizedResponsesRequest::parse(request).unwrap();
        let projected = normalized.to_xai_value();
        let input = projected["input"].as_array().unwrap();
        assert!(input.iter().any(|item| item["type"] == "message"));
        assert!(input.iter().all(|item| item.get("call_id") != Some(&json!(""))));
    }

    #[test]
    fn request_enforces_safe_transport_and_identifiers() {
        let mut not_streaming = request_fixture();
        not_streaming["stream"] = json!(false);
        assert_eq!(
            NormalizedResponsesRequest::parse(not_streaming).unwrap_err(),
            ProtocolError::InvalidRequestField("stream")
        );

        let mut stored = request_fixture();
        stored["store"] = json!(true);
        assert_eq!(
            NormalizedResponsesRequest::parse(stored).unwrap_err(),
            ProtocolError::InvalidRequestField("store")
        );

        let mut empty_id = request_fixture();
        empty_id["input"][0]["id"] = json!("  ");
        assert_eq!(
            NormalizedResponsesRequest::parse(empty_id).unwrap_err(),
            ProtocolError::InvalidRequestField("id")
        );
    }

    #[test]
    fn message_images_round_trip_exact_url_data_and_detail() {
        let mut url_image = request_fixture();
        url_image["input"][0]["content"] = json!([
            {"type": "input_text", "text": "before"},
            {
                "type": "input_image",
                "image_url": "https://images.example.invalid/a%20b.png?sig=a%2Fb#kept",
                "detail": "original"
            },
            {"type": "input_text", "text": "after"}
        ]);
        let normalized = NormalizedResponsesRequest::parse(url_image.clone()).unwrap();
        url_image.as_object_mut().unwrap().remove("client_metadata");
        url_image.as_object_mut().unwrap().remove("tool_choice");
        url_image["input"][0].as_object_mut().unwrap().remove("id");
        url_image["input"][1]["content"] = json!("prior answer");
        assert_eq!(normalized.to_xai_value(), url_image);

        let mut data_image = request_fixture();
        data_image["input"][0]["content"] = json!([{
            "type": "input_image",
            "image_url": "data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=",
            "detail": "low"
        }]);
        let normalized = NormalizedResponsesRequest::parse(data_image.clone()).unwrap();
        data_image
            .as_object_mut()
            .unwrap()
            .remove("client_metadata");
        data_image.as_object_mut().unwrap().remove("tool_choice");
        data_image["input"][0].as_object_mut().unwrap().remove("id");
        data_image["input"][1]["content"] = json!("prior answer");
        assert_eq!(normalized.into_xai_value(), data_image);
    }

    fn function_request_fixture() -> Value {
        json!({
            "model": "grok-4.6",
            "instructions": "Use the declared functions exactly.",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "inspect both"}]
                },
                {
                    "type": "function_call",
                    "id": "fc_history_1",
                    "name": "read_file",
                    "call_id": "call_read_1",
                    "arguments": "{\"path\": \"README.md\", \"line\": 2}"
                },
                {
                    "type": "function_call",
                    "id": "fc_history_2",
                    "name": "search",
                    "call_id": "call_search_1",
                    "arguments": "{ \"query\" : \"Rust bridge\" }"
                },
                {
                    "type": "function_call_output",
                    "id": "fco_history_1",
                    "call_id": "call_read_1",
                    "output": "line one\nline two"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_search_1",
                    "output": "match A\nmatch B"
                }
            ],
            "tools": [
                {
                    "type": "function",
                    "name": "read_file",
                    "description": "Read one file",
                    "parameters": {
                        "type": "object",
                        "properties": {"path": {"type": "string"}, "line": {"type": "integer"}},
                        "required": ["path"]
                    },
                    "strict": true
                },
                {
                    "type": "function",
                    "name": "search",
                    "description": "Search text",
                    "parameters": {
                        "type": "object",
                        "properties": {"query": {"type": "string"}}
                    }
                }
            ],
            "tool_choice": "required",
            "parallel_tool_calls": true,
            "reasoning": null,
            "store": false,
            "stream": true,
            "include": []
        })
    }

    #[test]
    fn function_request_round_trips_tools_calls_results_and_choices() {
        let mut required = function_request_fixture();
        let normalized = NormalizedResponsesRequest::parse(required.clone()).unwrap();
        required["input"][1].as_object_mut().unwrap().remove("id");
        required["input"][2].as_object_mut().unwrap().remove("id");
        required["input"][3].as_object_mut().unwrap().remove("id");
        assert_eq!(normalized.to_xai_value(), required);
        assert_eq!(normalized.to_xai_value()["tool_choice"], "required");

        let mut specific = function_request_fixture();
        specific["tool_choice"] = json!({"type": "function", "name": "search"});
        let normalized = NormalizedResponsesRequest::parse(specific.clone()).unwrap();
        specific["input"][1].as_object_mut().unwrap().remove("id");
        specific["input"][2].as_object_mut().unwrap().remove("id");
        specific["input"][3].as_object_mut().unwrap().remove("id");
        assert_eq!(normalized.into_xai_value(), specific);
    }

    #[test]
    fn codex_namespace_functions_are_flattened_replayed_and_restored_by_exact_map() {
        let mut request = request_fixture();
        request["input"] = json!([
            {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "君は誰だ？"}]
            },
            {
                "type": "function_call",
                "id": "fc_history_1",
                "namespace": "mcp__demo",
                "name": "ping",
                "call_id": "call_ping_1",
                "arguments": "{}"
            },
            {
                "type": "function_call_output",
                "call_id": "call_ping_1",
                "output": "pong"
            }
        ]);
        request["tools"] = json!([{
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
        }]);

        let normalized = NormalizedResponsesRequest::parse(request).unwrap();
        let upstream = normalized.to_xai_value();
        assert_eq!(upstream["tools"][0]["type"], "function");
        assert_eq!(upstream["tools"][0]["name"], "mcp__demo__ping");
        assert_eq!(upstream["input"][1]["name"], "mcp__demo__ping");
        assert!(
            upstream["input"][1]
                .as_object()
                .unwrap()
                .get("namespace")
                .is_none()
        );

        let projection = normalized.namespace_projection();
        let mut added = json!({
            "type": "response.output_item.added",
            "sequence_number": 1,
            "output_index": 0,
            "item": {
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "mcp__demo__ping",
                "arguments": ""
            }
        });
        projection.restore_response_event(&mut added);
        assert_eq!(added["item"]["name"], "ping");
        assert_eq!(added["item"]["namespace"], "mcp__demo");

        let mut completed = json!({
            "type": "response.completed",
            "sequence_number": 5,
            "response": {
                "id": "resp_1",
                "output": [{
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "mcp__demo__ping",
                    "arguments": "{}"
                }]
            }
        });
        projection.restore_response_event(&mut completed);
        assert_eq!(completed["response"]["output"][0]["name"], "ping");
        assert_eq!(completed["response"]["output"][0]["namespace"], "mcp__demo");
    }

    #[test]
    fn stale_namespaced_history_is_flattened_when_current_tools_omit_namespace() {
        let mut request = request_fixture();
        request["input"] = json!([
            {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "continue"}]
            },
            {
                "type": "function_call",
                "namespace": "mcp__stale",
                "name": "read_file",
                "call_id": "call_stale_1",
                "arguments": "{\"path\":\"README.md\"}"
            },
            {
                "type": "function_call_output",
                "call_id": "call_stale_1",
                "output": "line one"
            }
        ]);
        request["tools"] = json!([]);

        let normalized = NormalizedResponsesRequest::parse(request).unwrap();
        let upstream = normalized.to_xai_value();
        assert_eq!(upstream["input"][1]["name"], "mcp__stale__read_file");
        assert!(upstream["input"][1].get("namespace").is_none());

        let projection = normalized.namespace_projection();
        let mut event = json!({
            "type": "response.output_item.done",
            "sequence_number": 1,
            "output_index": 0,
            "item": {
                "type": "function_call",
                "call_id": "call_stale_2",
                "name": "mcp__stale__read_file",
                "arguments": "{}"
            }
        });
        projection.restore_response_event(&mut event);
        assert_eq!(event["item"]["name"], "read_file");
        assert_eq!(event["item"]["namespace"], "mcp__stale");
    }

    #[test]
    fn xai_projection_normalizes_union_rooted_app_tool_schemas_without_mutating_clean_tools() {
        let mut request = request_fixture();
        request["tools"] = json!([
            {
                "type": "namespace",
                "name": "codex_app",
                "description": "Codex app tools.",
                "tools": [{
                    "type": "function",
                    "name": "automation_update",
                    "description": "Create, update, view, or delete an automation.",
                    "parameters": {
                        "$schema": "https://json-schema.org/draft/2020-12/schema",
                        "$defs": {
                            "create": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["create", true]},
                                    "payload": {"type": "string", "const": true}
                                },
                                "required": ["action", "payload"]
                            },
                            "delete": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["delete"]},
                                    "id": {"type": "string"}
                                },
                                "required": ["action", "id"]
                            }
                        },
                        "description": "Automation command.",
                        "type": "object",
                        "properties": {"request_id": {"type": "string"}},
                        "required": ["request_id"],
                        "additionalProperties": false,
                        "oneOf": [
                            {"$ref": "#/$defs/create"},
                            {"$ref": "#/$defs/delete"}
                        ]
                    }
                }, {
                    "type": "function",
                    "name": "clean",
                    "description": "A normal MCP tool.",
                    "parameters": {
                        "type": "object",
                        "properties": {"path": {"type": "string", "enum": ["README.md"]}},
                        "required": ["path"],
                        "additionalProperties": false
                    }
                }]
            }
        ]);

        let upstream = NormalizedResponsesRequest::parse(request)
            .unwrap()
            .to_xai_value();
        assert_eq!(upstream["tools"][0]["name"], "codex_app__automation_update");
        assert_eq!(
            upstream["tools"][0]["parameters"],
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$defs": {
                    "create": {
                        "type": "object",
                        "properties": {
                            "action": {"type": "string", "enum": ["create"]},
                            "payload": {"type": "string"}
                        },
                        "required": ["action", "payload"]
                    },
                    "delete": {
                        "type": "object",
                        "properties": {
                            "action": {"type": "string", "enum": ["delete"]},
                            "id": {"type": "string"}
                        },
                        "required": ["action", "id"]
                    }
                },
                "description": "Automation command.",
                "type": "object",
                "properties": {
                    "request_id": {"type": "string"},
                    "action": {"type": "string", "enum": ["create"]},
                    "payload": {"type": "string"},
                    "id": {"type": "string"}
                },
                "required": ["request_id", "action"],
                "additionalProperties": true
            })
        );
        assert_eq!(upstream["tools"][1]["name"], "codex_app__clean");
        assert_eq!(
            upstream["tools"][1]["parameters"],
            json!({
                "type": "object",
                "properties": {"path": {"type": "string", "enum": ["README.md"]}},
                "required": ["path"],
                "additionalProperties": false
            })
        );
    }

    #[test]
    fn codex_cached_web_search_is_projected_to_grok_hosted_search() {
        let mut request = request_fixture();
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

        let upstream = NormalizedResponsesRequest::parse(request)
            .unwrap()
            .to_xai_value();
        assert_eq!(
            upstream["tools"],
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
    }

    #[test]
    fn namespace_projection_fails_closed_on_flat_name_collision() {
        let mut request = request_fixture();
        request["tools"] = json!([
            {
                "type": "function",
                "name": "mcp__demo__ping",
                "description": "plain",
                "parameters": {"type": "object", "properties": {}}
            },
            {
                "type": "namespace",
                "name": "mcp__demo",
                "description": "demo",
                "tools": [{
                    "type": "function",
                    "name": "ping",
                    "description": "nested",
                    "strict": false,
                    "parameters": {"type": "object", "properties": {}}
                }]
            }
        ]);
        assert_eq!(
            NormalizedResponsesRequest::parse(request).unwrap_err(),
            ProtocolError::DuplicateToolName
        );
    }

    #[test]
    fn function_output_round_trips_ordered_text_and_images() {
        let mut request = function_request_fixture();
        request["input"][3]["output"] = json!([
            {"type": "input_text", "text": "screenshot follows\n"},
            {
                "type": "input_image",
                "image_url": "data:image/png;base64,iVBORw0KGgo=",
                "detail": "high"
            },
            {"type": "input_text", "text": "\nexact tail"},
            {
                "type": "input_image",
                "image_url": "http://127.0.0.1:4321/frame.png"
            }
        ]);

        let normalized = NormalizedResponsesRequest::parse(request.clone()).unwrap();
        request["input"][1].as_object_mut().unwrap().remove("id");
        request["input"][2].as_object_mut().unwrap().remove("id");
        request["input"][3].as_object_mut().unwrap().remove("id");
        assert_eq!(normalized.to_xai_value(), request);
    }

    #[test]
    fn function_output_string_regression_round_trips_exactly() {
        let mut request = function_request_fixture();
        let normalized = NormalizedResponsesRequest::parse(request.clone()).unwrap();
        assert_eq!(
            normalized.to_xai_value()["input"][3]["output"],
            "line one\nline two"
        );
        request["input"][1].as_object_mut().unwrap().remove("id");
        request["input"][2].as_object_mut().unwrap().remove("id");
        request["input"][3].as_object_mut().unwrap().remove("id");
        assert_eq!(normalized.to_xai_value(), request);
    }

    #[test]
    fn function_request_drops_unreplayable_call_result_integrity() {
        let mut duplicate_call = function_request_fixture();
        duplicate_call["input"][2]["call_id"] = json!("call_read_1");
        assert!(NormalizedResponsesRequest::parse(duplicate_call).is_ok());

        let mut unmatched = function_request_fixture();
        unmatched["input"][3]["call_id"] = json!("call_missing");
        let unmatched = NormalizedResponsesRequest::parse(unmatched).unwrap();
        assert!(unmatched.to_xai_value()["input"]
            .as_array().unwrap()
            .iter().all(|item| item.get("call_id") != Some(&json!("call_missing"))));

        let mut duplicate_output = function_request_fixture();
        duplicate_output["input"][4]["call_id"] = json!("call_read_1");
        assert!(NormalizedResponsesRequest::parse(duplicate_output).is_ok());

        let mut malformed = function_request_fixture();
        malformed["input"][1]["arguments"] = json!("{not-json}");
        assert_eq!(
            NormalizedResponsesRequest::parse(malformed).unwrap_err(),
            ProtocolError::InvalidFunctionArguments
        );

        let mut non_object = function_request_fixture();
        non_object["input"][1]["arguments"] = json!("[1,2]");
        assert_eq!(
            NormalizedResponsesRequest::parse(non_object).unwrap_err(),
            ProtocolError::InvalidFunctionArguments
        );
    }

    #[test]
    fn request_rejects_malformed_or_unsupported_image_content() {
        for image_url in [
            "",
            "file:///tmp/image.png",
            "https:///missing-host.png",
            "data:image/;base64,QUJD",
            "data:image/png;base64,",
            "data:image/png;base64,A",
            "data:image/png;base64,%%%",
            "data:image/png;charset=utf-8;base64,QUJD",
        ] {
            let mut request = request_fixture();
            request["input"][0]["content"][0] =
                json!({"type": "input_image", "image_url": image_url});
            assert_eq!(
                NormalizedResponsesRequest::parse(request).unwrap_err(),
                ProtocolError::InvalidImageUrl,
                "unexpected result for {image_url}"
            );
        }

        let mut invalid_detail = request_fixture();
        invalid_detail["input"][0]["content"][0] = json!({
            "type": "input_image",
            "image_url": "https://example.invalid/a.png",
            "detail": "medium"
        });
        assert_eq!(
            NormalizedResponsesRequest::parse(invalid_detail).unwrap_err(),
            ProtocolError::InvalidRequestField("detail")
        );

        let mut unknown_image_field = request_fixture();
        unknown_image_field["input"][0]["content"][0] = json!({
            "type": "input_image",
            "image_url": "https://example.invalid/a.png",
            "file_id": "file_1"
        });
        assert_eq!(
            NormalizedResponsesRequest::parse(unknown_image_field).unwrap_err(),
            ProtocolError::UnsupportedContent
        );

        let mut assistant_image = request_fixture();
        assistant_image["input"][1]["content"][0] = json!({
            "type": "input_image",
            "image_url": "https://example.invalid/a.png"
        });
        assert_eq!(
            NormalizedResponsesRequest::parse(assistant_image).unwrap_err(),
            ProtocolError::InvalidRequestField("content")
        );

        let mut empty_output = function_request_fixture();
        empty_output["input"][3]["output"] = json!([]);
        assert_eq!(
            NormalizedResponsesRequest::parse(empty_output).unwrap_err(),
            ProtocolError::InvalidRequestField("output")
        );

        for unsupported in [
            json!({"type": "output_text", "text": "not input content"}),
            json!({"type": "input_audio", "audio_url": "data:audio/wav;base64,AAAA"}),
            json!({"type": "input_text", "text": "ok", "extra": true}),
        ] {
            let mut request = function_request_fixture();
            request["input"][3]["output"] = json!([unsupported]);
            assert_eq!(
                NormalizedResponsesRequest::parse(request).unwrap_err(),
                ProtocolError::UnsupportedContent
            );
        }
    }

    #[test]
    fn function_request_rejects_unsupported_or_ambiguous_tools() {
        let mut duplicate_tool = function_request_fixture();
        duplicate_tool["tools"][1]["name"] = json!("read_file");
        assert_eq!(
            NormalizedResponsesRequest::parse(duplicate_tool).unwrap_err(),
            ProtocolError::DuplicateToolName
        );

        let mut unknown_field = function_request_fixture();
        unknown_field["tools"][0]["defer_loading"] = json!(true);
        assert_eq!(
            NormalizedResponsesRequest::parse(unknown_field).unwrap_err(),
            ProtocolError::UnsupportedTools
        );

        let mut unknown_choice = function_request_fixture();
        unknown_choice["tool_choice"] = json!({"type": "function", "name": "missing"});
        assert_eq!(
            NormalizedResponsesRequest::parse(unknown_choice).unwrap_err(),
            ProtocolError::UnknownToolChoice
        );
    }

    fn text_events() -> Vec<Value> {
        let added_item = json!({
            "type": "message",
            "id": "msg_1",
            "role": "assistant",
            "status": "in_progress",
            "content": []
        });
        let done_item = json!({
            "type": "message",
            "id": "msg_1",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": "Hello 世界", "annotations": []}]
        });
        vec![
            json!({
                "type": "response.created",
                "sequence_number": 0,
                "response": {"id": "resp_1", "status": "in_progress", "output": []}
            }),
            json!({
                "type": "response.output_item.added",
                "sequence_number": 1,
                "output_index": 0,
                "item": added_item
            }),
            json!({
                "type": "response.content_part.added",
                "sequence_number": 2,
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0,
                "part": {"type": "output_text", "text": "", "annotations": []}
            }),
            json!({
                "type": "response.output_text.delta",
                "sequence_number": 3,
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0,
                "delta": "Hello "
            }),
            json!({
                "type": "response.output_text.delta",
                "sequence_number": 4,
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0,
                "delta": "世界"
            }),
            json!({
                "type": "response.output_text.done",
                "sequence_number": 5,
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0,
                "text": "Hello 世界"
            }),
            json!({
                "type": "response.content_part.done",
                "sequence_number": 6,
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0,
                "part": {"type": "output_text", "text": "Hello 世界", "annotations": []}
            }),
            json!({
                "type": "response.output_item.done",
                "sequence_number": 7,
                "output_index": 0,
                "item": done_item.clone()
            }),
            json!({
                "type": "response.completed",
                "sequence_number": 8,
                "response": {
                    "id": "resp_1",
                    "status": "completed",
                    "output": [done_item]
                }
            }),
        ]
    }

    #[test]
    fn unknown_sse_event_is_passthrough_and_does_not_terminate() {
        let mut validator = TextStreamValidator::new();
        let event = validator.accept_value(json!({
            "type":"response.xai_auxiliary", "payload":{"opaque":true}
        })).unwrap();
        assert_eq!(event.kind(), &TextStreamEventKind::Passthrough {
            event_type: "response.xai_auxiliary".into()
        });
        let text = validator.accept_value(json!({
            "type":"response.output_text.delta", "item_id":"m1", "delta":"ok"
        })).unwrap();
        assert!(matches!(text.kind(), TextStreamEventKind::OutputTextDelta { .. }));
        assert!(validator.finish().is_ok());
    }

    #[test]
    fn permissive_sse_projects_text_function_reasoning_and_terminal_events() {
        let mut validator = TextStreamValidator::new();
        let cases = [
            ("response.output_text.delta", TextStreamEventKind::OutputTextDelta {
                item_id: "m1".into(), output_index: 0, content_index: 0, delta: "hi".into()
            }),
            ("response.function_call_arguments.delta", TextStreamEventKind::FunctionCallArgumentsDelta {
                item_id: "f1".into(), output_index: 1, delta: "{}".into()
            }),
            ("response.reasoning_text.delta", TextStreamEventKind::ReasoningTextDelta {
                item_id: "r1".into(), output_index: 2, content_index: 0, delta: "why".into()
            }),
            ("response.completed", TextStreamEventKind::ResponseCompleted { response_id: "resp".into() }),
        ];
        for (event_type, expected) in cases {
            let value = match event_type {
                "response.completed" => json!({"type":event_type,"response":{"id":"resp"}}),
                "response.function_call_arguments.delta" => json!({"type":event_type,"item_id":"f1","output_index":1,"delta":"{}"}),
                "response.reasoning_text.delta" => json!({"type":event_type,"item_id":"r1","output_index":2,"content_index":0,"delta":"why"}),
                _ => json!({"type":event_type,"item_id":"m1","output_index":0,"content_index":0,"delta":"hi"}),
            };
            let actual = validator.accept_value(value).unwrap();
            assert_eq!(actual.kind(), &expected);
        }
        assert!(validator.is_completed());
    }

    #[test]
    fn complete_text_stream_validates_and_preserves_delta_text() {
        let mut validator = TextStreamValidator::new();
        let mut deltas = String::new();
        for original in text_events() {
            let encoded = original.to_string();
            let event = validator.accept_data(&encoded).unwrap();
            assert_eq!(event.original(), &original);
            if let TextStreamEventKind::OutputTextDelta { delta, .. } = event.kind() {
                deltas.push_str(delta);
            }
        }
        assert_eq!(deltas, "Hello 世界");
        assert_eq!(validator.state(), TextStreamState::Completed);
        assert!(validator.finish().is_ok());
    }

    #[test]
    fn eof_without_terminal_event_synthesizes_response_completed() {
        let mut validator = TextStreamValidator::new();
        validator
            .accept_value(json!({
                "type": "response.created",
                "sequence_number": 0,
                "response": {"id": "resp_1", "output": []}
            }))
            .unwrap();
        validator
            .accept_value(json!({
                "type": "response.output_item.done",
                "sequence_number": 1,
                "output_index": 0,
                "item": {
                    "type": "reasoning",
                    "id": "rs_1",
                    "status": "completed"
                }
            }))
            .unwrap();
        assert!(!validator.is_completed());
        let event = validator.synthetic_completed_on_eof().unwrap();
        assert_eq!(
            event.kind(),
            &TextStreamEventKind::ResponseCompleted {
                response_id: "resp_1".into()
            }
        );
        assert_eq!(event.original()["type"], "response.completed");
        assert_eq!(event.original()["response"]["id"], "resp_1");
        assert_eq!(event.original()["response"]["status"], "completed");
        assert_eq!(event.original()["response"]["output"], json!([]));
        assert!(validator.is_completed());
        assert!(validator.synthetic_completed_on_eof().is_none());
    }

    #[test]
    fn eof_after_upstream_completed_does_not_synthesize_another_terminal() {
        let mut validator = TextStreamValidator::new();
        validator
            .accept_value(json!({
                "type": "response.completed",
                "response": {"id": "resp"}
            }))
            .unwrap();
        assert!(validator.synthetic_completed_on_eof().is_none());
    }

    #[test]
    fn eof_before_any_sse_event_does_not_synthesize_completed() {
        let mut validator = TextStreamValidator::new();
        assert!(validator.synthetic_completed_on_eof().is_none());
    }

}
