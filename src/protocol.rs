use serde_json::{Map, Value};
use std::collections::HashSet;
use thiserror::Error;
use uuid::Uuid;

const REQUEST_FIELDS: &[&str] = &[
    "model",
    "instructions",
    "input",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "reasoning",
    "store",
    "stream",
    "stream_options",
    "include",
    "service_tier",
    "prompt_cache_key",
    "text",
    "client_metadata",
];

const MAX_CLIENT_METADATA_ENTRIES: usize = 64;
const MAX_CLIENT_METADATA_KEY_BYTES: usize = 256;
const MAX_CLIENT_METADATA_VALUE_BYTES: usize = 512 * 1024;
const MAX_CLIENT_METADATA_TOTAL_VALUE_BYTES: usize = 4 * 1024 * 1024;
const MAX_REASONING_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_ENCRYPTED_REASONING_BYTES: usize = 16 * 1024 * 1024;
const MAX_REASONING_PARTS: usize = 64;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("responses request must be a JSON object")]
    RequestNotObject,
    #[error("responses request contains an unsupported top-level field")]
    UnsupportedTopLevelField,
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
    #[error("responses tool choice is unsupported in this phase")]
    UnsupportedToolChoice,
    #[error("responses specific tool choice does not reference an admitted tool")]
    UnknownToolChoice,
    #[error("responses function call identifier must be unique")]
    DuplicateCallId,
    #[error("responses function call arguments must encode a JSON object")]
    InvalidFunctionArguments,
    #[error("responses function output has no earlier matching call")]
    UnmatchedFunctionOutput,
    #[error("responses function output is duplicated")]
    DuplicateFunctionOutput,
    #[error("SSE data must be a valid JSON object")]
    InvalidSsePayload,
    #[error("SSE event family is unsupported in this phase")]
    UnsupportedSseEvent,
    #[error("SSE event field `{0}` is missing or invalid")]
    InvalidSseField(&'static str),
    #[error("SSE sequence_number must strictly increase")]
    NonIncreasingSequence,
    #[error("SSE event violates the text lifecycle")]
    InvalidSseOrder,
    #[error("SSE event followed a terminal event")]
    EventAfterCompletion,
    #[error("SSE response identifier changed")]
    ResponseIdMismatch,
    #[error("SSE item identifier changed")]
    ItemIdMismatch,
    #[error("SSE output item identifier must be unique")]
    DuplicateItemId,
    #[error("SSE output index changed")]
    OutputIndexMismatch,
    #[error("SSE output indices must be unique and contiguous")]
    InvalidOutputIndexOrder,
    #[error("SSE content index changed")]
    ContentIndexMismatch,
    #[error("SSE completed text does not match streamed text")]
    TextMismatch,
    #[error("SSE function call identifier changed")]
    CallIdMismatch,
    #[error("SSE function name changed")]
    FunctionNameMismatch,
    #[error("SSE function arguments do not match streamed arguments")]
    FunctionArgumentsMismatch,
    #[error("SSE reasoning content does not match streamed reasoning")]
    ReasoningMismatch,
    #[error("SSE stream ended before response.completed")]
    StreamNotCompleted,
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
    text: OptionalJson,
    _client_metadata: ClientMetadata,
}

#[derive(Debug, Clone, PartialEq)]
enum InputItem {
    Message(TextMessage),
    Reasoning(PriorReasoning),
    FunctionCall(FunctionCall),
    FunctionCallOutput(FunctionCallOutput),
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
    Opaque(String),
}

#[derive(Debug, Clone, PartialEq)]
struct FunctionCall {
    name: String,
    arguments: String,
    call_id: String,
}

#[derive(Debug, Clone, PartialEq)]
struct FunctionCallOutput {
    call_id: String,
    output: FunctionCallOutputBody,
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
    List(Vec<FunctionTool>),
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
        let allowed: HashSet<&str> = REQUEST_FIELDS.iter().copied().collect();
        if object.keys().any(|key| !allowed.contains(key.as_str())) {
            return Err(ProtocolError::UnsupportedTopLevelField);
        }

        let model = required_nonempty_string(object, "model")?;
        let instructions = optional_string(object, "instructions")?;
        let input = parse_input(required_array(object, "input")?)?;
        let tools = parse_tools(object.get("tools"))?;
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
            text,
            _client_metadata: client_metadata,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn grok_routing_metadata(&self) -> Result<GrokRoutingMetadata, ProtocolError> {
        let conversation_id = match &self.prompt_cache_key {
            OptionalJson::Present(Value::String(value)) => Uuid::parse_str(value)
                .map_err(|_| ProtocolError::InvalidRequestField("prompt_cache_key"))?,
            _ => return Err(ProtocolError::InvalidRequestField("prompt_cache_key")),
        };
        let ClientMetadata::Values(metadata) = &self._client_metadata else {
            return Err(ProtocolError::InvalidRequestField("client_metadata"));
        };
        let value = |key: &str| {
            metadata
                .iter()
                .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
        };
        let session_id = value("session_id")
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(ProtocolError::InvalidRequestField("client_metadata"))?;
        if session_id != conversation_id {
            return Err(ProtocolError::InvalidRequestField("client_metadata"));
        }
        let request_id = value("turn_id")
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(ProtocolError::InvalidRequestField("client_metadata"))?;
        let agent_id = value("x-codex-installation-id")
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(ProtocolError::InvalidRequestField("client_metadata"))?;
        // Grok's prompt index advances once per prompt, not once per wire-level
        // user item. Codex can emit consecutive synthetic environment-context
        // and actual-prompt user items for one prompt, while model output marks
        // the boundary before the next prompt block.
        let mut turn_index = 0usize;
        let mut in_user_block = false;
        for item in &self.input {
            match item {
                InputItem::Message(TextMessage {
                    role: MessageRole::User,
                    ..
                }) => {
                    if !in_user_block {
                        turn_index += 1;
                    }
                    in_user_block = true;
                }
                InputItem::Message(TextMessage {
                    role: MessageRole::Developer,
                    ..
                }) => {}
                InputItem::Message(TextMessage {
                    role: MessageRole::Assistant,
                    ..
                })
                | InputItem::Reasoning(_)
                | InputItem::FunctionCall(_)
                | InputItem::FunctionCallOutput(_) => in_user_block = false,
            }
        }
        if turn_index == 0 {
            return Err(ProtocolError::InvalidRequestField("input"));
        }
        Ok(GrokRoutingMetadata {
            conversation_id,
            request_id,
            agent_id,
            turn_index,
        })
    }

    pub fn to_xai_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("model".into(), Value::String(self.model.clone()));
        if let Some(instructions) = &self.instructions {
            object.insert("instructions".into(), Value::String(instructions.clone()));
        }
        object.insert(
            "input".into(),
            Value::Array(self.input.iter().map(InputItem::to_value).collect()),
        );
        insert_tools(&mut object, &self.tools);
        object.insert("tool_choice".into(), self.tool_choice.to_value());
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
    fn parse(value: &Value) -> Result<Self, ProtocolError> {
        let object = value
            .as_object()
            .ok_or(ProtocolError::InvalidRequestField("input"))?;
        match required_string(object, "type")? {
            "message" => {
                reject_unknown_keys(object, &["type", "id", "role", "content"])?;
                Ok(Self::Message(TextMessage::parse(object)?))
            }
            "reasoning" => {
                reject_unknown_keys(
                    object,
                    &["type", "id", "summary", "content", "encrypted_content"],
                )?;
                Ok(Self::Reasoning(PriorReasoning::parse(object)?))
            }
            "function_call" => {
                reject_unknown_keys(object, &["type", "id", "name", "arguments", "call_id"])?;
                Ok(Self::FunctionCall(FunctionCall::parse(object)?))
            }
            "function_call_output" => {
                reject_unknown_keys(object, &["type", "id", "call_id", "output"])?;
                Ok(Self::FunctionCallOutput(FunctionCallOutput::parse(object)?))
            }
            _ => Err(ProtocolError::UnsupportedInputItem),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Message(message) => message.to_value(),
            Self::Reasoning(reasoning) => reasoning.to_value(),
            Self::FunctionCall(call) => call.to_value(),
            Self::FunctionCallOutput(output) => output.to_value(),
        }
    }
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
                PriorEncryptedContent::Opaque(value.clone())
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
                PriorEncryptedContent::Opaque(value) => Value::String(value.clone()),
            },
        );
        Value::Object(object)
    }
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

impl FunctionCall {
    fn parse(object: &Map<String, Value>) -> Result<Self, ProtocolError> {
        // Grok Build deliberately omits the response item id when replaying a
        // tool call. Validate Codex's output-only id, but do not forward it.
        optional_nonempty_string(object, "id")?;
        let arguments = required_string(object, "arguments")?.to_owned();
        let parsed: Value = serde_json::from_str(&arguments)
            .map_err(|_| ProtocolError::InvalidFunctionArguments)?;
        if !parsed.is_object() {
            return Err(ProtocolError::InvalidFunctionArguments);
        }
        Ok(Self {
            name: required_nonempty_string(object, "name")?,
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

fn parse_input(values: &[Value]) -> Result<Vec<InputItem>, ProtocolError> {
    let mut calls = HashSet::new();
    let mut outputs = HashSet::new();
    let mut reasoning_ids = HashSet::new();
    let mut assistant_output_started = false;
    let mut parsed = Vec::with_capacity(values.len());
    for value in values {
        let item = InputItem::parse(value)?;
        match &item {
            InputItem::Reasoning(reasoning) => {
                if assistant_output_started || calls.len() != outputs.len() {
                    return Err(ProtocolError::InvalidRequestField("input"));
                }
                if !reasoning_ids.insert(reasoning.id.clone()) {
                    return Err(ProtocolError::DuplicateItemId);
                }
                assistant_output_started = true;
            }
            InputItem::FunctionCall(call) => {
                if !calls.insert(call.call_id.clone()) {
                    return Err(ProtocolError::DuplicateCallId);
                }
                assistant_output_started = true;
            }
            InputItem::FunctionCallOutput(output) => {
                if !calls.contains(&output.call_id) {
                    return Err(ProtocolError::UnmatchedFunctionOutput);
                }
                if !outputs.insert(output.call_id.clone()) {
                    return Err(ProtocolError::DuplicateFunctionOutput);
                }
                if calls.len() == outputs.len() {
                    assistant_output_started = false;
                }
            }
            InputItem::Message(message) => match message.role {
                MessageRole::User | MessageRole::Developer => {
                    if calls.len() != outputs.len() {
                        return Err(ProtocolError::InvalidRequestField("input"));
                    }
                    assistant_output_started = false;
                }
                MessageRole::Assistant => {
                    assistant_output_started = true;
                }
            },
        }
        parsed.push(item);
    }
    Ok(parsed)
}

fn parse_tools(value: Option<&Value>) -> Result<ToolsField, ProtocolError> {
    match value {
        None => Ok(ToolsField::Absent),
        Some(Value::Null) => Ok(ToolsField::Null),
        Some(Value::Array(tools)) => {
            let mut names = HashSet::new();
            let mut parsed = Vec::with_capacity(tools.len());
            for tool in tools {
                let tool = FunctionTool::parse(tool)?;
                if !names.insert(tool.name.clone()) {
                    return Err(ProtocolError::DuplicateToolName);
                }
                parsed.push(tool);
            }
            Ok(ToolsField::List(parsed))
        }
        Some(_) => Err(ProtocolError::UnsupportedTools),
    }
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
        object.insert("parameters".into(), Value::Object(self.parameters.clone()));
        if let Some(strict) = self.strict {
            object.insert("strict".into(), Value::Bool(strict));
        }
        Value::Object(object)
    }
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
        ToolChoice::Function(name) if !admitted.iter().any(|tool| tool.name == *name) => {
            Err(ProtocolError::UnknownToolChoice)
        }
        _ => Ok(()),
    }
}

fn insert_tools(object: &mut Map<String, Value>, tools: &ToolsField) {
    match tools {
        ToolsField::Absent => {}
        ToolsField::Null => {
            object.insert("tools".into(), Value::Null);
        }
        ToolsField::List(tools) => {
            object.insert(
                "tools".into(),
                Value::Array(tools.iter().map(FunctionTool::to_value).collect()),
            );
        }
    }
}

fn insert_optional(object: &mut Map<String, Value>, key: &str, field: &OptionalJson) {
    if let OptionalJson::Present(value) = field {
        object.insert(key.into(), value.clone());
    }
}

fn reject_unknown_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), ProtocolError> {
    if reject_unknown_keys_for(object, allowed).is_err() {
        return Err(ProtocolError::InvalidRequestField("input"));
    }
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
    Created,
    InProgress,
    OutputItemStarted,
    ContentStarted,
    Streaming,
    OutputTextDone,
    FunctionArgumentsDone,
    ReasoningStarted,
    ReasoningStreaming,
    ReasoningDone,
    ContentDone,
    OutputItemDone,
    Completed,
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
}

#[derive(Debug, Clone)]
pub struct TextStreamValidator {
    state: TextStreamState,
    last_sequence: Option<u64>,
    response_id: Option<String>,
    item_id: Option<String>,
    output_index: Option<u64>,
    content_index: Option<u64>,
    text: String,
    message_stage: Option<MessageItemStage>,
    reasoning_item: Option<ReasoningStreamItem>,
    function_items: Vec<FunctionStreamItem>,
}

#[derive(Debug, Clone)]
struct ReasoningStreamItem {
    item_id: String,
    output_index: u64,
    summaries: Vec<ReasoningTextSegment>,
    content: Vec<ReasoningTextSegment>,
    encrypted_content: Option<String>,
    explicitly_added: bool,
    stage: ReasoningItemStage,
}

#[derive(Debug, Clone)]
struct ReasoningTextSegment {
    text: String,
    stage: ReasoningSegmentStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReasoningSegmentStage {
    Seeded,
    PartAdded,
    Streaming,
    TextDone,
    PartDone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReasoningItemStage {
    Added,
    Streaming,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageItemStage {
    Added,
    ContentStarted,
    Streaming,
    TextDone,
    ContentDone,
    Done,
}

#[derive(Debug, Clone)]
struct FunctionStreamItem {
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
    stage: FunctionItemStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FunctionItemStage {
    Added,
    Streaming,
    ArgumentsDone,
    Done,
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
            item_id: None,
            output_index: None,
            content_index: None,
            text: String::new(),
            message_stage: None,
            reasoning_item: None,
            function_items: Vec::new(),
        }
    }

    pub fn state(&self) -> TextStreamState {
        self.state
    }

    pub fn is_completed(&self) -> bool {
        self.state == TextStreamState::Completed
    }

    pub fn finish(&self) -> Result<(), ProtocolError> {
        if self.is_completed() {
            Ok(())
        } else {
            Err(ProtocolError::StreamNotCompleted)
        }
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
        let object = value.as_object().ok_or(ProtocolError::InvalidSsePayload)?;
        if self.state == TextStreamState::Completed {
            return Err(ProtocolError::EventAfterCompletion);
        }

        let sequence_number = event_u64(object, "sequence_number")?;
        if self
            .last_sequence
            .is_some_and(|last| sequence_number <= last)
        {
            return Err(ProtocolError::NonIncreasingSequence);
        }

        let event_type = event_string(object, "type")?;
        let (kind, next_state) = match event_type {
            "response.created" => self.validate_created(object)?,
            "response.in_progress" => self.validate_in_progress(object)?,
            "response.output_item.added" => self.validate_output_item_added(object)?,
            "response.content_part.added" => self.validate_content_part_added(object)?,
            "response.output_text.delta" => self.validate_output_text_delta(object)?,
            "response.output_text.done" => self.validate_output_text_done(object)?,
            "response.content_part.done" => self.validate_content_part_done(object)?,
            "response.function_call_arguments.delta" => {
                self.validate_function_arguments_delta(object)?
            }
            "response.function_call_arguments.done" => {
                self.validate_function_arguments_done(object)?
            }
            "response.reasoning_summary_part.added" => {
                self.validate_reasoning_summary_part_added(object)?
            }
            "response.reasoning_summary_text.delta" => {
                self.validate_reasoning_summary_text_delta(object)?
            }
            "response.reasoning_summary_text.done" => {
                self.validate_reasoning_summary_text_done(object)?
            }
            "response.reasoning_summary_part.done" => {
                self.validate_reasoning_summary_part_done(object)?
            }
            "response.reasoning_text.delta" => self.validate_reasoning_text_delta(object)?,
            "response.reasoning_text.done" => self.validate_reasoning_text_done(object)?,
            "response.output_item.done" => self.validate_output_item_done(object)?,
            "response.completed" => self.validate_completed(object)?,
            _ => return Err(ProtocolError::UnsupportedSseEvent),
        };

        self.last_sequence = Some(sequence_number);
        self.state = next_state;
        Ok(ValidatedTextStreamEvent {
            sequence_number,
            kind,
            original: value,
        })
    }

    fn validate_created(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        self.require_state(&[TextStreamState::AwaitingCreated])?;
        let response = event_object(object, "response")?;
        let response_id = event_nonempty_string(response, "id")?.to_owned();
        let output = event_array(response, "output")?;
        if !output.is_empty() {
            return Err(ProtocolError::InvalidSseField("response.output"));
        }
        self.response_id = Some(response_id.clone());
        Ok((
            TextStreamEventKind::ResponseCreated { response_id },
            TextStreamState::Created,
        ))
    }

    fn validate_in_progress(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        self.require_state(&[TextStreamState::Created])?;
        reject_sse_unknown_keys(object, &["type", "sequence_number", "response"])?;
        let response = event_object(object, "response")?;
        let response_id = event_nonempty_string(response, "id")?.to_owned();
        if self.response_id.as_deref() != Some(response_id.as_str()) {
            return Err(ProtocolError::ResponseIdMismatch);
        }
        if event_string(response, "status")? != "in_progress" {
            return Err(ProtocolError::InvalidSseField("response.status"));
        }
        if !event_array(response, "output")?.is_empty() {
            return Err(ProtocolError::InvalidSseField("response.output"));
        }
        Ok((
            TextStreamEventKind::ResponseInProgress { response_id },
            TextStreamState::InProgress,
        ))
    }

    fn validate_output_item_added(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        let output_index = event_u64(object, "output_index")?;
        let item = event_object(object, "item")?;
        match event_string(item, "type")? {
            "message" => {
                self.require_state(&[
                    TextStreamState::Created,
                    TextStreamState::InProgress,
                    TextStreamState::OutputItemDone,
                    TextStreamState::ReasoningStreaming,
                    TextStreamState::ReasoningDone,
                ])?;
                self.require_reasoning_ready_for_next_item()?;
                let expected_index = u64::from(self.reasoning_item.is_some());
                if !self.function_items.is_empty() || output_index != expected_index {
                    return Err(ProtocolError::InvalidOutputIndexOrder);
                }
                let (item_id, text) = validate_sse_message(item, MessageShape::Added)?;
                if !text.is_empty() {
                    return Err(ProtocolError::InvalidSseField("item.content"));
                }
                if self
                    .reasoning_item
                    .as_ref()
                    .is_some_and(|reasoning| reasoning.item_id == item_id)
                {
                    return Err(ProtocolError::DuplicateItemId);
                }
                self.item_id = Some(item_id.clone());
                self.output_index = Some(output_index);
                self.message_stage = Some(MessageItemStage::Added);
                Ok((
                    TextStreamEventKind::OutputItemAdded {
                        item_id,
                        output_index,
                    },
                    TextStreamState::OutputItemStarted,
                ))
            }
            "function_call" => self.validate_function_item_added(item, output_index),
            "reasoning" => self.validate_reasoning_item_added(item, output_index),
            _ => Err(ProtocolError::UnsupportedSseEvent),
        }
    }

    fn validate_reasoning_item_added(
        &mut self,
        item: &Map<String, Value>,
        output_index: u64,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        self.require_state(&[TextStreamState::Created, TextStreamState::InProgress])?;
        if self.reasoning_item.is_some()
            || self.item_id.is_some()
            || !self.function_items.is_empty()
            || output_index != 0
        {
            return Err(ProtocolError::InvalidOutputIndexOrder);
        }
        let parsed = validate_sse_reasoning_item(item, ReasoningItemShape::Added)?;
        self.reasoning_item = Some(ReasoningStreamItem {
            item_id: parsed.item_id.clone(),
            output_index,
            summaries: parsed
                .summaries
                .into_iter()
                .map(|text| ReasoningTextSegment {
                    text,
                    stage: ReasoningSegmentStage::Seeded,
                })
                .collect(),
            content: parsed
                .content
                .into_iter()
                .map(|text| ReasoningTextSegment {
                    text,
                    stage: ReasoningSegmentStage::Seeded,
                })
                .collect(),
            encrypted_content: None,
            explicitly_added: true,
            stage: ReasoningItemStage::Added,
        });
        Ok((
            TextStreamEventKind::ReasoningItemAdded {
                item_id: parsed.item_id,
                output_index,
            },
            TextStreamState::ReasoningStarted,
        ))
    }

    fn validate_function_item_added(
        &mut self,
        item: &Map<String, Value>,
        output_index: u64,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        if self.response_id.is_none() {
            return Err(ProtocolError::InvalidSseOrder);
        }
        self.require_reasoning_ready_for_next_item()?;
        let expected_index = self.function_items.len() as u64
            + u64::from(self.reasoning_item.is_some())
            + u64::from(self.item_id.is_some());
        if output_index != expected_index {
            return Err(ProtocolError::InvalidOutputIndexOrder);
        }
        let parsed = validate_sse_function_call(item)?;
        if !parsed.arguments.is_empty() {
            return Err(ProtocolError::FunctionArgumentsMismatch);
        }
        if self.function_items.iter().any(|existing| {
            existing.item_id == parsed.item_id || existing.call_id == parsed.call_id
        }) {
            return Err(ProtocolError::DuplicateCallId);
        }
        if self.item_id.as_deref() == Some(parsed.item_id.as_str())
            || self
                .reasoning_item
                .as_ref()
                .is_some_and(|reasoning| reasoning.item_id == parsed.item_id)
        {
            return Err(ProtocolError::DuplicateItemId);
        }
        self.function_items.push(FunctionStreamItem {
            item_id: parsed.item_id.clone(),
            call_id: parsed.call_id.clone(),
            name: parsed.name.clone(),
            arguments: String::new(),
            stage: FunctionItemStage::Added,
        });
        Ok((
            TextStreamEventKind::FunctionCallAdded {
                item_id: parsed.item_id,
                output_index,
                call_id: parsed.call_id,
                name: parsed.name,
            },
            TextStreamState::OutputItemStarted,
        ))
    }

    fn validate_content_part_added(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        self.require_message_stage(&[MessageItemStage::Added])?;
        let (item_id, output_index, content_index) = self.validate_item_coordinates(object)?;
        let part = event_object(object, "part")?;
        let text = validate_output_text_part(part)?;
        self.content_index = Some(content_index);
        self.text = text.clone();
        self.message_stage = Some(MessageItemStage::ContentStarted);
        Ok((
            TextStreamEventKind::ContentPartAdded {
                item_id,
                output_index,
                content_index,
                text,
            },
            TextStreamState::ContentStarted,
        ))
    }

    fn validate_output_text_delta(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        self.require_message_stage(&[
            MessageItemStage::ContentStarted,
            MessageItemStage::Streaming,
        ])?;
        let (item_id, output_index, content_index) = self.validate_item_coordinates(object)?;
        let delta = event_string(object, "delta")?.to_owned();
        self.text.push_str(&delta);
        self.message_stage = Some(MessageItemStage::Streaming);
        Ok((
            TextStreamEventKind::OutputTextDelta {
                item_id,
                output_index,
                content_index,
                delta,
            },
            TextStreamState::Streaming,
        ))
    }

    fn validate_output_text_done(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        self.require_message_stage(&[
            MessageItemStage::ContentStarted,
            MessageItemStage::Streaming,
        ])?;
        let (item_id, output_index, content_index) = self.validate_item_coordinates(object)?;
        let text = event_string(object, "text")?.to_owned();
        self.require_matching_text(&text)?;
        self.message_stage = Some(MessageItemStage::TextDone);
        Ok((
            TextStreamEventKind::OutputTextDone {
                item_id,
                output_index,
                content_index,
                text,
            },
            TextStreamState::OutputTextDone,
        ))
    }

    fn validate_content_part_done(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        self.require_message_stage(&[MessageItemStage::TextDone])?;
        let (item_id, output_index, content_index) = self.validate_item_coordinates(object)?;
        let part = event_object(object, "part")?;
        let text = validate_output_text_part(part)?;
        self.require_matching_text(&text)?;
        self.message_stage = Some(MessageItemStage::ContentDone);
        Ok((
            TextStreamEventKind::ContentPartDone {
                item_id,
                output_index,
                content_index,
                text,
            },
            TextStreamState::ContentDone,
        ))
    }

    fn validate_function_arguments_delta(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        let output_index = event_u64(object, "output_index")?;
        let item_id = event_nonempty_string(object, "item_id")?.to_owned();
        let delta = event_string(object, "delta")?.to_owned();
        let item = self.function_item_mut(output_index)?;
        if item.item_id != item_id {
            return Err(ProtocolError::ItemIdMismatch);
        }
        if !matches!(
            item.stage,
            FunctionItemStage::Added | FunctionItemStage::Streaming
        ) {
            return Err(ProtocolError::InvalidSseOrder);
        }
        item.arguments.push_str(&delta);
        item.stage = FunctionItemStage::Streaming;
        Ok((
            TextStreamEventKind::FunctionCallArgumentsDelta {
                item_id,
                output_index,
                delta,
            },
            TextStreamState::Streaming,
        ))
    }

    fn validate_function_arguments_done(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        let output_index = event_u64(object, "output_index")?;
        let item_id = event_nonempty_string(object, "item_id")?.to_owned();
        let arguments = event_string(object, "arguments")?.to_owned();
        validate_arguments_object(&arguments)?;
        let item = self.function_item_mut(output_index)?;
        if item.item_id != item_id {
            return Err(ProtocolError::ItemIdMismatch);
        }
        if !matches!(
            item.stage,
            FunctionItemStage::Added | FunctionItemStage::Streaming
        ) {
            return Err(ProtocolError::InvalidSseOrder);
        }
        if item.arguments != arguments {
            return Err(ProtocolError::FunctionArgumentsMismatch);
        }
        item.stage = FunctionItemStage::ArgumentsDone;
        Ok((
            TextStreamEventKind::FunctionCallArgumentsDone {
                item_id,
                output_index,
                arguments,
            },
            TextStreamState::FunctionArgumentsDone,
        ))
    }

    fn validate_reasoning_summary_part_added(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        reject_sse_unknown_keys(
            object,
            &[
                "type",
                "sequence_number",
                "item_id",
                "output_index",
                "summary_index",
                "part",
            ],
        )?;
        let (item_id, output_index) = reasoning_event_coordinates(object)?;
        let summary_index = event_u64(object, "summary_index")?;
        let part = event_object(object, "part")?;
        let text = validate_reasoning_text_part(part, "summary_text")?;
        let reasoning = self.reasoning_item_mut(&item_id, output_index, false)?;
        let index = reasoning_index(summary_index)?;
        if index == reasoning.summaries.len() {
            reasoning.summaries.push(ReasoningTextSegment {
                text: text.clone(),
                stage: ReasoningSegmentStage::PartAdded,
            });
        } else {
            let segment = reasoning
                .summaries
                .get_mut(index)
                .ok_or(ProtocolError::ContentIndexMismatch)?;
            if segment.stage != ReasoningSegmentStage::Seeded || segment.text != text {
                return Err(ProtocolError::InvalidSseOrder);
            }
            segment.stage = ReasoningSegmentStage::PartAdded;
        }
        reasoning.stage = ReasoningItemStage::Streaming;
        Ok((
            TextStreamEventKind::ReasoningSummaryPartAdded {
                item_id,
                output_index,
                summary_index,
                text,
            },
            TextStreamState::ReasoningStreaming,
        ))
    }

    fn validate_reasoning_summary_text_delta(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        reject_sse_unknown_keys(
            object,
            &[
                "type",
                "sequence_number",
                "item_id",
                "output_index",
                "summary_index",
                "delta",
            ],
        )?;
        let (item_id, output_index) = reasoning_event_coordinates(object)?;
        let summary_index = event_u64(object, "summary_index")?;
        let delta = event_string(object, "delta")?.to_owned();
        let reasoning = self.reasoning_item_mut(&item_id, output_index, true)?;
        let segment = reasoning_segment_mut(
            &mut reasoning.summaries,
            summary_index,
            ReasoningSegmentKind::Summary,
        )?;
        if !matches!(
            segment.stage,
            ReasoningSegmentStage::Seeded
                | ReasoningSegmentStage::PartAdded
                | ReasoningSegmentStage::Streaming
        ) {
            return Err(ProtocolError::InvalidSseOrder);
        }
        append_reasoning_text(&mut segment.text, &delta)?;
        segment.stage = ReasoningSegmentStage::Streaming;
        reasoning.stage = ReasoningItemStage::Streaming;
        Ok((
            TextStreamEventKind::ReasoningSummaryTextDelta {
                item_id,
                output_index,
                summary_index,
                delta,
            },
            TextStreamState::ReasoningStreaming,
        ))
    }

    fn validate_reasoning_summary_text_done(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        reject_sse_unknown_keys(
            object,
            &[
                "type",
                "sequence_number",
                "item_id",
                "output_index",
                "summary_index",
                "text",
            ],
        )?;
        let (item_id, output_index) = reasoning_event_coordinates(object)?;
        let summary_index = event_u64(object, "summary_index")?;
        let text = event_string(object, "text")?.to_owned();
        validate_reasoning_text_size(&text)?;
        let reasoning = self.reasoning_item_mut(&item_id, output_index, false)?;
        let segment = reasoning_segment_mut(
            &mut reasoning.summaries,
            summary_index,
            ReasoningSegmentKind::Summary,
        )?;
        if !matches!(
            segment.stage,
            ReasoningSegmentStage::Seeded
                | ReasoningSegmentStage::PartAdded
                | ReasoningSegmentStage::Streaming
        ) {
            return Err(ProtocolError::InvalidSseOrder);
        }
        if !segment.text.is_empty() && segment.text != text {
            return Err(ProtocolError::ReasoningMismatch);
        }
        segment.text = text.clone();
        segment.stage = ReasoningSegmentStage::TextDone;
        reasoning.stage = ReasoningItemStage::Streaming;
        Ok((
            TextStreamEventKind::ReasoningSummaryTextDone {
                item_id,
                output_index,
                summary_index,
                text,
            },
            TextStreamState::ReasoningStreaming,
        ))
    }

    fn validate_reasoning_summary_part_done(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        reject_sse_unknown_keys(
            object,
            &[
                "type",
                "sequence_number",
                "item_id",
                "output_index",
                "summary_index",
                "part",
            ],
        )?;
        let (item_id, output_index) = reasoning_event_coordinates(object)?;
        let summary_index = event_u64(object, "summary_index")?;
        let part = event_object(object, "part")?;
        let text = validate_reasoning_text_part(part, "summary_text")?;
        let reasoning = self.reasoning_item_mut(&item_id, output_index, false)?;
        let index = reasoning_index(summary_index)?;
        let segment = reasoning
            .summaries
            .get_mut(index)
            .ok_or(ProtocolError::ContentIndexMismatch)?;
        if segment.stage != ReasoningSegmentStage::TextDone || segment.text != text {
            return Err(ProtocolError::ReasoningMismatch);
        }
        segment.stage = ReasoningSegmentStage::PartDone;
        Ok((
            TextStreamEventKind::ReasoningSummaryPartDone {
                item_id,
                output_index,
                summary_index,
                text,
            },
            TextStreamState::ReasoningStreaming,
        ))
    }

    fn validate_reasoning_text_delta(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        reject_sse_unknown_keys(
            object,
            &[
                "type",
                "sequence_number",
                "item_id",
                "output_index",
                "content_index",
                "delta",
            ],
        )?;
        let (item_id, output_index) = reasoning_event_coordinates(object)?;
        let content_index = event_u64(object, "content_index")?;
        let delta = event_string(object, "delta")?.to_owned();
        let reasoning = self.reasoning_item_mut(&item_id, output_index, true)?;
        let segment = reasoning_segment_mut(
            &mut reasoning.content,
            content_index,
            ReasoningSegmentKind::Content,
        )?;
        if !matches!(
            segment.stage,
            ReasoningSegmentStage::Seeded | ReasoningSegmentStage::Streaming
        ) {
            return Err(ProtocolError::InvalidSseOrder);
        }
        append_reasoning_text(&mut segment.text, &delta)?;
        segment.stage = ReasoningSegmentStage::Streaming;
        reasoning.stage = ReasoningItemStage::Streaming;
        Ok((
            TextStreamEventKind::ReasoningTextDelta {
                item_id,
                output_index,
                content_index,
                delta,
            },
            TextStreamState::ReasoningStreaming,
        ))
    }

    fn validate_reasoning_text_done(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        reject_sse_unknown_keys(
            object,
            &[
                "type",
                "sequence_number",
                "item_id",
                "output_index",
                "content_index",
                "text",
            ],
        )?;
        let (item_id, output_index) = reasoning_event_coordinates(object)?;
        let content_index = event_u64(object, "content_index")?;
        let text = event_string(object, "text")?.to_owned();
        validate_reasoning_text_size(&text)?;
        let reasoning = self.reasoning_item_mut(&item_id, output_index, false)?;
        let segment = reasoning_segment_mut(
            &mut reasoning.content,
            content_index,
            ReasoningSegmentKind::Content,
        )?;
        if !matches!(
            segment.stage,
            ReasoningSegmentStage::Seeded | ReasoningSegmentStage::Streaming
        ) {
            return Err(ProtocolError::InvalidSseOrder);
        }
        if !segment.text.is_empty() && segment.text != text {
            return Err(ProtocolError::ReasoningMismatch);
        }
        segment.text = text.clone();
        segment.stage = ReasoningSegmentStage::TextDone;
        reasoning.stage = ReasoningItemStage::Streaming;
        Ok((
            TextStreamEventKind::ReasoningTextDone {
                item_id,
                output_index,
                content_index,
                text,
            },
            TextStreamState::ReasoningStreaming,
        ))
    }

    fn validate_output_item_done(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        let output_index = event_u64(object, "output_index")?;
        let item = event_object(object, "item")?;
        match event_string(item, "type")? {
            "message" => {
                self.require_message_stage(&[MessageItemStage::ContentDone])?;
                self.require_output_index(output_index)?;
                let (item_id, text) = validate_sse_message(item, MessageShape::Done)?;
                self.require_item_id(&item_id)?;
                self.require_matching_text(&text)?;
                self.message_stage = Some(MessageItemStage::Done);
                Ok((
                    TextStreamEventKind::OutputItemDone {
                        item_id,
                        output_index,
                        text,
                    },
                    TextStreamState::OutputItemDone,
                ))
            }
            "function_call" => self.validate_function_item_done(item, output_index),
            "reasoning" => self.validate_reasoning_item_done(item, output_index),
            _ => Err(ProtocolError::UnsupportedSseEvent),
        }
    }

    fn validate_reasoning_item_done(
        &mut self,
        value: &Map<String, Value>,
        output_index: u64,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        let parsed = validate_sse_reasoning_item(value, ReasoningItemShape::Done)?;
        let reasoning = self.reasoning_item_mut(&parsed.item_id, output_index, false)?;
        if reasoning.stage == ReasoningItemStage::Done {
            return Err(ProtocolError::InvalidSseOrder);
        }
        if reasoning.explicitly_added
            && (reasoning.summaries.iter().any(|segment| {
                !matches!(
                    segment.stage,
                    ReasoningSegmentStage::TextDone | ReasoningSegmentStage::PartDone
                )
            }) || reasoning
                .content
                .iter()
                .any(|segment| segment.stage != ReasoningSegmentStage::TextDone))
        {
            return Err(ProtocolError::InvalidSseOrder);
        }
        require_matching_reasoning(reasoning, &parsed)?;
        reasoning.encrypted_content = parsed.encrypted_content.clone();
        reasoning.stage = ReasoningItemStage::Done;
        Ok((
            TextStreamEventKind::ReasoningItemDone {
                item_id: parsed.item_id,
                output_index,
                encrypted_content: parsed.encrypted_content,
            },
            TextStreamState::ReasoningDone,
        ))
    }

    fn validate_function_item_done(
        &mut self,
        value: &Map<String, Value>,
        output_index: u64,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        let parsed = validate_sse_function_call(value)?;
        validate_arguments_object(&parsed.arguments)?;
        let item = self.function_item_mut(output_index)?;
        if item.stage != FunctionItemStage::ArgumentsDone {
            return Err(ProtocolError::InvalidSseOrder);
        }
        require_matching_function(item, &parsed)?;
        item.stage = FunctionItemStage::Done;
        Ok((
            TextStreamEventKind::FunctionCallItemDone {
                item_id: parsed.item_id,
                output_index,
                call_id: parsed.call_id,
                name: parsed.name,
                arguments: parsed.arguments,
            },
            TextStreamState::OutputItemDone,
        ))
    }

    fn validate_completed(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<(TextStreamEventKind, TextStreamState), ProtocolError> {
        let response = event_object(object, "response")?;
        let response_id = event_nonempty_string(response, "id")?.to_owned();
        if self.response_id.as_deref() != Some(response_id.as_str()) {
            return Err(ProtocolError::ResponseIdMismatch);
        }
        let output = event_array(response, "output")?;
        let mut output_offset = 0usize;
        if let Some(expected) = &self.reasoning_item {
            if expected.explicitly_added && expected.stage != ReasoningItemStage::Done {
                return Err(ProtocolError::InvalidSseOrder);
            }
            let item = output
                .first()
                .and_then(Value::as_object)
                .ok_or(ProtocolError::InvalidSseField("response.output"))?;
            let parsed = validate_sse_reasoning_item(item, ReasoningItemShape::Done)?;
            require_matching_reasoning(expected, &parsed)?;
            output_offset = 1;
        }

        if self.item_id.is_some() {
            self.require_message_stage(&[MessageItemStage::Done])?;
            let item = output
                .get(output_offset)
                .and_then(Value::as_object)
                .ok_or(ProtocolError::InvalidSseField("response.output"))?;
            let (item_id, text) = validate_sse_message(item, MessageShape::Done)?;
            self.require_item_id(&item_id)?;
            self.require_matching_text(&text)?;
            output_offset += 1;
        }

        if !self.function_items.is_empty() {
            if self
                .function_items
                .iter()
                .any(|item| item.stage != FunctionItemStage::Done)
            {
                return Err(ProtocolError::InvalidSseOrder);
            }
            if output.len() != output_offset + self.function_items.len() {
                return Err(ProtocolError::InvalidSseField("response.output"));
            }
            for (value, expected) in output[output_offset..].iter().zip(&self.function_items) {
                let object = value
                    .as_object()
                    .ok_or(ProtocolError::InvalidSseField("response.output"))?;
                let parsed = validate_sse_function_call(object)?;
                validate_arguments_object(&parsed.arguments)?;
                require_matching_function(expected, &parsed)?;
            }
        } else {
            if output.len() != output_offset
                || (self.reasoning_item.is_none() && self.item_id.is_none())
            {
                return Err(ProtocolError::InvalidSseField("response.output"));
            }
        }
        Ok((
            TextStreamEventKind::ResponseCompleted { response_id },
            TextStreamState::Completed,
        ))
    }

    fn function_item_mut(
        &mut self,
        output_index: u64,
    ) -> Result<&mut FunctionStreamItem, ProtocolError> {
        let prefix = u64::from(self.reasoning_item.is_some()) + u64::from(self.item_id.is_some());
        let index = output_index
            .checked_sub(prefix)
            .and_then(|index| usize::try_from(index).ok())
            .ok_or(ProtocolError::OutputIndexMismatch)?;
        self.function_items
            .get_mut(index)
            .ok_or(ProtocolError::OutputIndexMismatch)
    }

    fn reasoning_item_mut(
        &mut self,
        item_id: &str,
        output_index: u64,
        allow_implicit: bool,
    ) -> Result<&mut ReasoningStreamItem, ProtocolError> {
        if self.reasoning_item.is_none() {
            if !allow_implicit
                || !matches!(
                    self.state,
                    TextStreamState::Created | TextStreamState::InProgress
                )
                || self.item_id.is_some()
                || !self.function_items.is_empty()
                || output_index != 0
            {
                return Err(ProtocolError::InvalidSseOrder);
            }
            self.reasoning_item = Some(ReasoningStreamItem {
                item_id: item_id.to_owned(),
                output_index,
                summaries: Vec::new(),
                content: Vec::new(),
                encrypted_content: None,
                explicitly_added: false,
                stage: ReasoningItemStage::Streaming,
            });
        }
        let item = self.reasoning_item.as_mut().expect("reasoning item exists");
        if item.item_id != item_id {
            return Err(ProtocolError::ItemIdMismatch);
        }
        if item.output_index != output_index {
            return Err(ProtocolError::OutputIndexMismatch);
        }
        if item.stage == ReasoningItemStage::Done {
            return Err(ProtocolError::InvalidSseOrder);
        }
        Ok(item)
    }

    fn require_reasoning_ready_for_next_item(&self) -> Result<(), ProtocolError> {
        if let Some(reasoning) = &self.reasoning_item
            && reasoning.explicitly_added
            && reasoning.stage != ReasoningItemStage::Done
        {
            return Err(ProtocolError::InvalidSseOrder);
        }
        Ok(())
    }

    fn validate_item_coordinates(
        &self,
        object: &Map<String, Value>,
    ) -> Result<(String, u64, u64), ProtocolError> {
        let item_id = event_nonempty_string(object, "item_id")?.to_owned();
        self.require_item_id(&item_id)?;
        let output_index = event_u64(object, "output_index")?;
        self.require_output_index(output_index)?;
        let content_index = event_u64(object, "content_index")?;
        if let Some(expected) = self.content_index {
            if content_index != expected {
                return Err(ProtocolError::ContentIndexMismatch);
            }
        }
        Ok((item_id, output_index, content_index))
    }

    fn require_state(&self, states: &[TextStreamState]) -> Result<(), ProtocolError> {
        if states.contains(&self.state) {
            Ok(())
        } else {
            Err(ProtocolError::InvalidSseOrder)
        }
    }

    fn require_message_stage(&self, stages: &[MessageItemStage]) -> Result<(), ProtocolError> {
        if self
            .message_stage
            .is_some_and(|stage| stages.contains(&stage))
        {
            Ok(())
        } else {
            Err(ProtocolError::InvalidSseOrder)
        }
    }

    fn require_item_id(&self, actual: &str) -> Result<(), ProtocolError> {
        if self.item_id.as_deref() == Some(actual) {
            Ok(())
        } else {
            Err(ProtocolError::ItemIdMismatch)
        }
    }

    fn require_output_index(&self, actual: u64) -> Result<(), ProtocolError> {
        if self.output_index == Some(actual) {
            Ok(())
        } else {
            Err(ProtocolError::OutputIndexMismatch)
        }
    }

    fn require_matching_text(&self, actual: &str) -> Result<(), ProtocolError> {
        if self.text == actual {
            Ok(())
        } else {
            Err(ProtocolError::TextMismatch)
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ReasoningItemShape {
    Added,
    Done,
}

#[derive(Debug)]
struct ParsedSseReasoningItem {
    item_id: String,
    summaries: Vec<String>,
    content: Vec<String>,
    encrypted_content: Option<String>,
}

fn validate_sse_reasoning_item(
    item: &Map<String, Value>,
    shape: ReasoningItemShape,
) -> Result<ParsedSseReasoningItem, ProtocolError> {
    if event_string(item, "type")? != "reasoning" {
        return Err(ProtocolError::UnsupportedSseEvent);
    }
    reject_sse_unknown_keys(
        item,
        &[
            "type",
            "id",
            "summary",
            "content",
            "encrypted_content",
            "status",
        ],
    )?;
    if let Some(status) = item.get("status") {
        let status = status
            .as_str()
            .ok_or(ProtocolError::InvalidSseField("item.status"))?;
        let valid = match shape {
            ReasoningItemShape::Added => status == "in_progress",
            ReasoningItemShape::Done => status == "completed",
        };
        if !valid {
            return Err(ProtocolError::InvalidSseField("item.status"));
        }
    }
    let summaries = parse_reasoning_parts(event_array(item, "summary")?, "summary_text")?;
    let content = match item.get("content") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(values)) => parse_reasoning_parts(values, "reasoning_text")?,
        Some(_) => return Err(ProtocolError::InvalidSseField("item.content")),
    };
    let encrypted_content = match item.get("encrypted_content") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if !value.is_empty() => {
            if value.len() > MAX_ENCRYPTED_REASONING_BYTES {
                return Err(ProtocolError::InvalidSseField("item.encrypted_content"));
            }
            Some(value.clone())
        }
        Some(_) => return Err(ProtocolError::InvalidSseField("item.encrypted_content")),
    };
    if matches!(shape, ReasoningItemShape::Added)
        && (encrypted_content.is_some()
            || summaries.iter().any(|text| !text.is_empty())
            || content.iter().any(|text| !text.is_empty()))
    {
        return Err(ProtocolError::InvalidSseOrder);
    }
    Ok(ParsedSseReasoningItem {
        item_id: event_nonempty_string(item, "id")?.to_owned(),
        summaries,
        content,
        encrypted_content,
    })
}

fn parse_reasoning_parts(
    values: &[Value],
    expected_type: &'static str,
) -> Result<Vec<String>, ProtocolError> {
    values
        .iter()
        .map(|value| {
            let part = value
                .as_object()
                .ok_or(ProtocolError::InvalidSseField("item.reasoning_part"))?;
            validate_reasoning_text_part(part, expected_type)
        })
        .collect()
}

fn validate_reasoning_text_part(
    part: &Map<String, Value>,
    expected_type: &'static str,
) -> Result<String, ProtocolError> {
    reject_sse_unknown_keys(part, &["type", "text"])?;
    if event_string(part, "type")? != expected_type {
        return Err(ProtocolError::UnsupportedSseEvent);
    }
    let text = event_string(part, "text")?.to_owned();
    validate_reasoning_text_size(&text)?;
    Ok(text)
}

fn reasoning_event_coordinates(
    object: &Map<String, Value>,
) -> Result<(String, u64), ProtocolError> {
    Ok((
        event_nonempty_string(object, "item_id")?.to_owned(),
        event_u64(object, "output_index")?,
    ))
}

#[derive(Debug, Clone, Copy)]
enum ReasoningSegmentKind {
    Summary,
    Content,
}

fn reasoning_segment_mut(
    segments: &mut Vec<ReasoningTextSegment>,
    index: u64,
    _kind: ReasoningSegmentKind,
) -> Result<&mut ReasoningTextSegment, ProtocolError> {
    let index = reasoning_index(index)?;
    if index == segments.len() {
        segments.push(ReasoningTextSegment {
            text: String::new(),
            stage: ReasoningSegmentStage::Seeded,
        });
    }
    segments
        .get_mut(index)
        .ok_or(ProtocolError::ContentIndexMismatch)
}

fn reasoning_index(index: u64) -> Result<usize, ProtocolError> {
    usize::try_from(index).map_err(|_| ProtocolError::ContentIndexMismatch)
}

fn append_reasoning_text(target: &mut String, delta: &str) -> Result<(), ProtocolError> {
    let new_len = target
        .len()
        .checked_add(delta.len())
        .ok_or(ProtocolError::InvalidSseField("reasoning.text"))?;
    if new_len > MAX_REASONING_TEXT_BYTES {
        return Err(ProtocolError::InvalidSseField("reasoning.text"));
    }
    target.push_str(delta);
    Ok(())
}

fn validate_reasoning_text_size(text: &str) -> Result<(), ProtocolError> {
    if text.len() > MAX_REASONING_TEXT_BYTES {
        Err(ProtocolError::InvalidSseField("reasoning.text"))
    } else {
        Ok(())
    }
}

fn require_matching_reasoning(
    expected: &ReasoningStreamItem,
    actual: &ParsedSseReasoningItem,
) -> Result<(), ProtocolError> {
    if expected.item_id != actual.item_id {
        return Err(ProtocolError::ItemIdMismatch);
    }
    if expected
        .summaries
        .iter()
        .map(|segment| segment.text.as_str())
        .ne(actual.summaries.iter().map(String::as_str))
        || expected
            .content
            .iter()
            .map(|segment| segment.text.as_str())
            .ne(actual.content.iter().map(String::as_str))
        || expected
            .encrypted_content
            .as_ref()
            .is_some_and(|encrypted| actual.encrypted_content.as_ref() != Some(encrypted))
    {
        return Err(ProtocolError::ReasoningMismatch);
    }
    Ok(())
}

fn reject_sse_unknown_keys(
    object: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), ProtocolError> {
    reject_unknown_keys_for(object, allowed).map_err(|_| ProtocolError::UnsupportedSseEvent)
}

#[derive(Debug, Clone, Copy)]
enum MessageShape {
    Added,
    Done,
}

#[derive(Debug)]
struct ParsedSseFunctionCall {
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
}

fn validate_sse_function_call(
    item: &Map<String, Value>,
) -> Result<ParsedSseFunctionCall, ProtocolError> {
    if event_string(item, "type")? != "function_call" {
        return Err(ProtocolError::UnsupportedSseEvent);
    }
    if reject_unknown_keys_for(
        item,
        &["type", "id", "call_id", "name", "arguments", "status"],
    )
    .is_err()
    {
        return Err(ProtocolError::UnsupportedSseEvent);
    }
    if let Some(status) = item.get("status")
        && !status.is_string()
    {
        return Err(ProtocolError::InvalidSseField("item.status"));
    }
    Ok(ParsedSseFunctionCall {
        item_id: event_nonempty_string(item, "id")?.to_owned(),
        call_id: event_nonempty_string(item, "call_id")?.to_owned(),
        name: event_nonempty_string(item, "name")?.to_owned(),
        arguments: event_string(item, "arguments")?.to_owned(),
    })
}

fn validate_arguments_object(arguments: &str) -> Result<(), ProtocolError> {
    let value: Value =
        serde_json::from_str(arguments).map_err(|_| ProtocolError::InvalidFunctionArguments)?;
    if value.is_object() {
        Ok(())
    } else {
        Err(ProtocolError::InvalidFunctionArguments)
    }
}

fn require_matching_function(
    expected: &FunctionStreamItem,
    actual: &ParsedSseFunctionCall,
) -> Result<(), ProtocolError> {
    if expected.item_id != actual.item_id {
        return Err(ProtocolError::ItemIdMismatch);
    }
    if expected.call_id != actual.call_id {
        return Err(ProtocolError::CallIdMismatch);
    }
    if expected.name != actual.name {
        return Err(ProtocolError::FunctionNameMismatch);
    }
    if expected.arguments != actual.arguments {
        return Err(ProtocolError::FunctionArgumentsMismatch);
    }
    Ok(())
}

fn validate_sse_message(
    item: &Map<String, Value>,
    shape: MessageShape,
) -> Result<(String, String), ProtocolError> {
    if event_string(item, "type")? != "message" {
        return Err(ProtocolError::UnsupportedSseEvent);
    }
    if event_string(item, "role")? != "assistant" {
        return Err(ProtocolError::InvalidSseField("item.role"));
    }
    let item_id = event_nonempty_string(item, "id")?.to_owned();
    let content = event_array(item, "content")?;
    match shape {
        MessageShape::Added if content.is_empty() => Ok((item_id, String::new())),
        MessageShape::Done if content.len() == 1 => {
            let part = content[0]
                .as_object()
                .ok_or(ProtocolError::InvalidSseField("item.content"))?;
            Ok((item_id, validate_output_text_part(part)?))
        }
        _ => Err(ProtocolError::InvalidSseField("item.content")),
    }
}

fn validate_output_text_part(part: &Map<String, Value>) -> Result<String, ProtocolError> {
    if event_string(part, "type")? != "output_text" {
        return Err(ProtocolError::UnsupportedSseEvent);
    }
    Ok(event_string(part, "text")?.to_owned())
}

fn event_value<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a Value, ProtocolError> {
    object.get(key).ok_or(ProtocolError::InvalidSseField(key))
}

fn event_string<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, ProtocolError> {
    event_value(object, key)?
        .as_str()
        .ok_or(ProtocolError::InvalidSseField(key))
}

fn event_nonempty_string<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, ProtocolError> {
    let value = event_string(object, key)?;
    if value.trim().is_empty() {
        Err(ProtocolError::InvalidSseField(key))
    } else {
        Ok(value)
    }
}

fn event_u64(object: &Map<String, Value>, key: &'static str) -> Result<u64, ProtocolError> {
    event_value(object, key)?
        .as_u64()
        .ok_or(ProtocolError::InvalidSseField(key))
}

fn event_object<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a Map<String, Value>, ProtocolError> {
    event_value(object, key)?
        .as_object()
        .ok_or(ProtocolError::InvalidSseField(key))
}

fn event_array<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a [Value], ProtocolError> {
    event_value(object, key)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or(ProtocolError::InvalidSseField(key))
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
                {
                    "type": "message",
                    "id": "msg_user_1",
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "first\n"},
                        {"type": "input_text", "text": "  second"}
                    ]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {"type": "output_text", "text": "prior answer"}
                    ]
                }
            ],
            "tools": [],
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "reasoning": {"effort": "high", "summary": "auto"},
            "store": false,
            "stream": true,
            "stream_options": {"include_obfuscation": false},
            "include": ["reasoning.encrypted_content"],
            "service_tier": "default",
            "prompt_cache_key": "cache-key",
            "text": {"verbosity": "medium"},
            "client_metadata": {"originator": "codex", "kind": "agent"}
        })
    }

    #[test]
    fn text_request_round_trips_without_semantic_loss() {
        let original = request_fixture();
        let request = NormalizedResponsesRequest::parse(original.clone()).unwrap();
        assert_eq!(request.model(), "grok-4.6");
        let mut expected_upstream = original;
        expected_upstream
            .as_object_mut()
            .unwrap()
            .remove("client_metadata");
        expected_upstream["input"][0]
            .as_object_mut()
            .unwrap()
            .remove("id");
        expected_upstream["input"][1]["content"] = json!("prior answer");
        assert_eq!(request.to_xai_value(), expected_upstream);
        assert_eq!(request.into_xai_value(), expected_upstream);
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
                "encrypted_content": "opaque-encrypted-turn-state"
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
    fn tool_followup_strips_output_only_ids_and_preserves_reasoning_replay() {
        let original = two_turn_reasoning_request();
        let normalized = NormalizedResponsesRequest::parse(original.clone()).unwrap();
        let mut expected = original;
        expected.as_object_mut().unwrap().remove("client_metadata");
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
        assert_eq!(
            NormalizedResponsesRequest::parse(unknown).unwrap_err(),
            ProtocolError::InvalidRequestField("input")
        );

        let mut oversized = two_turn_reasoning_request();
        oversized["input"][1]["encrypted_content"] =
            json!("x".repeat(MAX_ENCRYPTED_REASONING_BYTES + 1));
        assert_eq!(
            NormalizedResponsesRequest::parse(oversized).unwrap_err(),
            ProtocolError::InvalidRequestField("encrypted_content")
        );

        let mut bad_order = two_turn_reasoning_request();
        bad_order["input"].as_array_mut().unwrap().swap(1, 3);
        assert_eq!(
            NormalizedResponsesRequest::parse(bad_order).unwrap_err(),
            ProtocolError::InvalidRequestField("input")
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
        assert_eq!(
            routing.conversation_id().to_string(),
            "11111111-1111-4111-8111-111111111111"
        );
        assert_eq!(
            routing.request_id().to_string(),
            "22222222-2222-4222-8222-222222222222"
        );
        assert_eq!(
            routing.agent_id().to_string(),
            "33333333-3333-4333-8333-333333333333"
        );
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
    fn grok_routing_ids_fail_closed_and_user_prompt_blocks_define_turn_index() {
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
        assert_eq!(normalized.grok_routing_metadata().unwrap().turn_index(), 2);

        current["prompt_cache_key"] = json!("not-a-uuid");
        assert_eq!(
            NormalizedResponsesRequest::parse(current.clone())
                .unwrap()
                .grok_routing_metadata()
                .unwrap_err(),
            ProtocolError::InvalidRequestField("prompt_cache_key")
        );

        current["prompt_cache_key"] = json!("11111111-1111-4111-8111-111111111111");
        current["client_metadata"]["turn_id"] = json!("not-a-uuid");
        assert_eq!(
            NormalizedResponsesRequest::parse(current)
                .unwrap()
                .grok_routing_metadata()
                .unwrap_err(),
            ProtocolError::InvalidRequestField("client_metadata")
        );
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
        assert_eq!(
            NormalizedResponsesRequest::parse(unknown).unwrap_err(),
            ProtocolError::UnsupportedTopLevelField
        );

        let mut custom_call = request_fixture();
        custom_call["input"] = json!([{
            "type": "custom_tool_call",
            "name": "patch",
            "call_id": "call_1",
            "input": "payload"
        }]);
        assert_eq!(
            NormalizedResponsesRequest::parse(custom_call).unwrap_err(),
            ProtocolError::UnsupportedInputItem
        );

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

        let mut specific = function_request_fixture();
        specific["tool_choice"] = json!({"type": "function", "name": "search"});
        let normalized = NormalizedResponsesRequest::parse(specific.clone()).unwrap();
        specific["input"][1].as_object_mut().unwrap().remove("id");
        specific["input"][2].as_object_mut().unwrap().remove("id");
        specific["input"][3].as_object_mut().unwrap().remove("id");
        assert_eq!(normalized.into_xai_value(), specific);
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
    fn function_request_rejects_broken_call_result_integrity() {
        let mut duplicate_call = function_request_fixture();
        duplicate_call["input"][2]["call_id"] = json!("call_read_1");
        assert_eq!(
            NormalizedResponsesRequest::parse(duplicate_call).unwrap_err(),
            ProtocolError::DuplicateCallId
        );

        let mut unmatched = function_request_fixture();
        unmatched["input"][3]["call_id"] = json!("call_missing");
        assert_eq!(
            NormalizedResponsesRequest::parse(unmatched).unwrap_err(),
            ProtocolError::UnmatchedFunctionOutput
        );

        let mut duplicate_output = function_request_fixture();
        duplicate_output["input"][4]["call_id"] = json!("call_read_1");
        assert_eq!(
            NormalizedResponsesRequest::parse(duplicate_output).unwrap_err(),
            ProtocolError::DuplicateFunctionOutput
        );

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

    fn feed_prefix(validator: &mut TextStreamValidator, count: usize) {
        for event in text_events().into_iter().take(count) {
            validator.accept_value(event).unwrap();
        }
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
    fn response_in_progress_is_admitted_once_between_created_and_output() {
        let mut events = text_events();
        for event in events.iter_mut().skip(1) {
            let sequence = event["sequence_number"].as_u64().unwrap();
            event["sequence_number"] = json!(sequence + 1);
        }
        events.insert(
            1,
            json!({
                "type": "response.in_progress",
                "sequence_number": 1,
                "response": {
                    "id": "resp_1",
                    "object": "response",
                    "created_at": 1234567890,
                    "model": "grok-4.6",
                    "status": "in_progress",
                    "output": []
                }
            }),
        );

        let mut validator = TextStreamValidator::new();
        for (index, original) in events.into_iter().enumerate() {
            let event = validator.accept_value(original.clone()).unwrap();
            assert_eq!(event.original(), &original);
            if index == 1 {
                assert_eq!(
                    event.kind(),
                    &TextStreamEventKind::ResponseInProgress {
                        response_id: "resp_1".to_owned()
                    }
                );
            }
        }
        assert!(validator.finish().is_ok());
    }

    #[test]
    fn stream_rejects_sequence_and_lifecycle_violations() {
        let mut duplicate = TextStreamValidator::new();
        duplicate.accept_value(text_events()[0].clone()).unwrap();
        let mut event = text_events()[1].clone();
        event["sequence_number"] = json!(0);
        assert_eq!(
            duplicate.accept_value(event).unwrap_err(),
            ProtocolError::NonIncreasingSequence
        );

        let mut illegal = TextStreamValidator::new();
        assert_eq!(
            illegal.accept_value(text_events()[3].clone()).unwrap_err(),
            ProtocolError::InvalidSseOrder
        );

        let mut incomplete = TextStreamValidator::new();
        incomplete.accept_value(text_events()[0].clone()).unwrap();
        assert_eq!(
            incomplete.finish().unwrap_err(),
            ProtocolError::StreamNotCompleted
        );

        let mut missing_content_done = TextStreamValidator::new();
        feed_prefix(&mut missing_content_done, 6);
        assert_eq!(
            missing_content_done
                .accept_value(text_events()[7].clone())
                .unwrap_err(),
            ProtocolError::InvalidSseOrder
        );

        let mut duplicate_content_done = TextStreamValidator::new();
        feed_prefix(&mut duplicate_content_done, 7);
        let mut event = text_events()[6].clone();
        event["sequence_number"] = json!(7);
        assert_eq!(
            duplicate_content_done.accept_value(event).unwrap_err(),
            ProtocolError::InvalidSseOrder
        );
    }

    #[test]
    fn stream_rejects_identifier_index_and_text_mismatches() {
        let mut wrong_item = TextStreamValidator::new();
        feed_prefix(&mut wrong_item, 3);
        let mut event = text_events()[3].clone();
        event["item_id"] = json!("msg_other");
        assert_eq!(
            wrong_item.accept_value(event).unwrap_err(),
            ProtocolError::ItemIdMismatch
        );

        let mut wrong_output = TextStreamValidator::new();
        feed_prefix(&mut wrong_output, 3);
        let mut event = text_events()[3].clone();
        event["output_index"] = json!(1);
        assert_eq!(
            wrong_output.accept_value(event).unwrap_err(),
            ProtocolError::OutputIndexMismatch
        );

        let mut wrong_content = TextStreamValidator::new();
        feed_prefix(&mut wrong_content, 3);
        let mut event = text_events()[3].clone();
        event["content_index"] = json!(1);
        assert_eq!(
            wrong_content.accept_value(event).unwrap_err(),
            ProtocolError::ContentIndexMismatch
        );

        let mut wrong_text = TextStreamValidator::new();
        feed_prefix(&mut wrong_text, 5);
        let mut event = text_events()[5].clone();
        event["text"] = json!("different");
        assert_eq!(
            wrong_text.accept_value(event).unwrap_err(),
            ProtocolError::TextMismatch
        );

        let mut wrong_response = TextStreamValidator::new();
        feed_prefix(&mut wrong_response, 8);
        let mut event = text_events()[8].clone();
        event["response"]["id"] = json!("resp_other");
        assert_eq!(
            wrong_response.accept_value(event).unwrap_err(),
            ProtocolError::ResponseIdMismatch
        );
    }

    fn function_stream_items() -> (Value, Value) {
        (
            json!({
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "read_file",
                "arguments": "{\"path\":\"README.md\"}",
                "status": "completed"
            }),
            json!({
                "type": "function_call",
                "id": "fc_2",
                "call_id": "call_2",
                "name": "search",
                "arguments": "{\"query\":\"Rust\"}",
                "status": "completed"
            }),
        )
    }

    fn function_events() -> Vec<Value> {
        let (done_0, done_1) = function_stream_items();
        vec![
            json!({
                "type": "response.created",
                "sequence_number": 0,
                "response": {"id": "resp_tools", "status": "in_progress", "output": []}
            }),
            json!({
                "type": "response.output_item.added",
                "sequence_number": 1,
                "output_index": 0,
                "item": {
                    "type": "function_call", "id": "fc_1", "call_id": "call_1",
                    "name": "read_file", "arguments": "", "status": "in_progress"
                }
            }),
            json!({
                "type": "response.output_item.added",
                "sequence_number": 2,
                "output_index": 1,
                "item": {
                    "type": "function_call", "id": "fc_2", "call_id": "call_2",
                    "name": "search", "arguments": "", "status": "in_progress"
                }
            }),
            json!({
                "type": "response.function_call_arguments.delta",
                "sequence_number": 3,
                "item_id": "fc_2",
                "output_index": 1,
                "delta": "{\"query\":"
            }),
            json!({
                "type": "response.function_call_arguments.delta",
                "sequence_number": 4,
                "item_id": "fc_1",
                "output_index": 0,
                "delta": "{\"path\":"
            }),
            json!({
                "type": "response.function_call_arguments.delta",
                "sequence_number": 5,
                "item_id": "fc_2",
                "output_index": 1,
                "delta": "\"Rust\"}"
            }),
            json!({
                "type": "response.function_call_arguments.delta",
                "sequence_number": 6,
                "item_id": "fc_1",
                "output_index": 0,
                "delta": "\"README.md\"}"
            }),
            json!({
                "type": "response.function_call_arguments.done",
                "sequence_number": 7,
                "item_id": "fc_1",
                "output_index": 0,
                "arguments": "{\"path\":\"README.md\"}"
            }),
            json!({
                "type": "response.function_call_arguments.done",
                "sequence_number": 8,
                "item_id": "fc_2",
                "output_index": 1,
                "arguments": "{\"query\":\"Rust\"}"
            }),
            json!({
                "type": "response.output_item.done",
                "sequence_number": 9,
                "output_index": 1,
                "item": done_1.clone()
            }),
            json!({
                "type": "response.output_item.done",
                "sequence_number": 10,
                "output_index": 0,
                "item": done_0.clone()
            }),
            json!({
                "type": "response.completed",
                "sequence_number": 11,
                "response": {
                    "id": "resp_tools",
                    "status": "completed",
                    "output": [done_0, done_1]
                }
            }),
        ]
    }

    fn feed_function_prefix(validator: &mut TextStreamValidator, count: usize) {
        for event in function_events().into_iter().take(count) {
            validator.accept_value(event).unwrap();
        }
    }

    #[test]
    fn complete_interleaved_two_function_stream_preserves_order_and_arguments() {
        let mut validator = TextStreamValidator::new();
        let mut deltas = vec![String::new(), String::new()];
        for original in function_events() {
            let event = validator.accept_value(original.clone()).unwrap();
            assert_eq!(event.original(), &original);
            if let TextStreamEventKind::FunctionCallArgumentsDelta {
                output_index,
                delta,
                ..
            } = event.kind()
            {
                deltas[*output_index as usize].push_str(delta);
            }
        }
        assert_eq!(deltas[0], "{\"path\":\"README.md\"}");
        assert_eq!(deltas[1], "{\"query\":\"Rust\"}");
        assert_eq!(validator.state(), TextStreamState::Completed);
        assert!(validator.finish().is_ok());
    }

    #[test]
    fn function_stream_rejects_duplicate_indices_and_mismatched_identity() {
        let mut duplicate_index = TextStreamValidator::new();
        feed_function_prefix(&mut duplicate_index, 2);
        let mut event = function_events()[2].clone();
        event["output_index"] = json!(0);
        assert_eq!(
            duplicate_index.accept_value(event).unwrap_err(),
            ProtocolError::InvalidOutputIndexOrder
        );

        let mut duplicate_call = TextStreamValidator::new();
        feed_function_prefix(&mut duplicate_call, 2);
        let mut event = function_events()[2].clone();
        event["item"]["call_id"] = json!("call_1");
        assert_eq!(
            duplicate_call.accept_value(event).unwrap_err(),
            ProtocolError::DuplicateCallId
        );

        let mut wrong_item = TextStreamValidator::new();
        feed_function_prefix(&mut wrong_item, 3);
        let mut event = function_events()[3].clone();
        event["item_id"] = json!("fc_other");
        assert_eq!(
            wrong_item.accept_value(event).unwrap_err(),
            ProtocolError::ItemIdMismatch
        );

        let mut wrong_name = TextStreamValidator::new();
        feed_function_prefix(&mut wrong_name, 9);
        let mut event = function_events()[9].clone();
        event["item"]["name"] = json!("other");
        assert_eq!(
            wrong_name.accept_value(event).unwrap_err(),
            ProtocolError::FunctionNameMismatch
        );

        let mut wrong_call_id = TextStreamValidator::new();
        feed_function_prefix(&mut wrong_call_id, 9);
        let mut event = function_events()[9].clone();
        event["item"]["call_id"] = json!("call_other");
        assert_eq!(
            wrong_call_id.accept_value(event).unwrap_err(),
            ProtocolError::CallIdMismatch
        );
    }

    #[test]
    fn function_stream_rejects_argument_and_completion_corruption() {
        let mut wrong_arguments = TextStreamValidator::new();
        feed_function_prefix(&mut wrong_arguments, 7);
        let mut event = function_events()[7].clone();
        event["arguments"] = json!("{\"path\":\"other\"}");
        assert_eq!(
            wrong_arguments.accept_value(event).unwrap_err(),
            ProtocolError::FunctionArgumentsMismatch
        );

        let mut malformed = TextStreamValidator::new();
        feed_function_prefix(&mut malformed, 7);
        let mut event = function_events()[7].clone();
        event["arguments"] = json!("{bad}");
        assert_eq!(
            malformed.accept_value(event).unwrap_err(),
            ProtocolError::InvalidFunctionArguments
        );

        let mut incomplete = TextStreamValidator::new();
        feed_function_prefix(&mut incomplete, 9);
        let mut event = function_events()[11].clone();
        event["sequence_number"] = json!(9);
        assert_eq!(
            incomplete.accept_value(event).unwrap_err(),
            ProtocolError::InvalidSseOrder
        );

        let mut wrong_order = TextStreamValidator::new();
        feed_function_prefix(&mut wrong_order, 11);
        let mut event = function_events()[11].clone();
        event["response"]["output"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        assert_eq!(
            wrong_order.accept_value(event).unwrap_err(),
            ProtocolError::ItemIdMismatch
        );
    }

    fn reasoning_and_text_events() -> Vec<Value> {
        let reasoning_done = json!({
            "type": "reasoning",
            "id": "reasoning_1",
            "summary": [{"type": "summary_text", "text": "Plan carefully."}],
            "content": [{"type": "reasoning_text", "text": "hidden"}],
            "encrypted_content": "opaque-ciphertext",
            "status": "completed"
        });
        let message_done = json!({
            "type": "message",
            "id": "msg_reasoned",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": "Answer", "annotations": []}]
        });
        vec![
            json!({
                "type": "response.created", "sequence_number": 0,
                "response": {"id": "resp_reasoned", "status": "in_progress", "output": []}
            }),
            json!({
                "type": "response.output_item.added", "sequence_number": 1,
                "output_index": 0,
                "item": {
                    "type": "reasoning", "id": "reasoning_1", "summary": [],
                    "status": "in_progress"
                }
            }),
            json!({
                "type": "response.reasoning_summary_part.added", "sequence_number": 2,
                "item_id": "reasoning_1", "output_index": 0, "summary_index": 0,
                "part": {"type": "summary_text", "text": ""}
            }),
            json!({
                "type": "response.reasoning_summary_text.delta", "sequence_number": 3,
                "item_id": "reasoning_1", "output_index": 0, "summary_index": 0,
                "delta": "Plan "
            }),
            json!({
                "type": "response.reasoning_summary_text.delta", "sequence_number": 4,
                "item_id": "reasoning_1", "output_index": 0, "summary_index": 0,
                "delta": "carefully."
            }),
            json!({
                "type": "response.reasoning_summary_text.done", "sequence_number": 5,
                "item_id": "reasoning_1", "output_index": 0, "summary_index": 0,
                "text": "Plan carefully."
            }),
            json!({
                "type": "response.reasoning_summary_part.done", "sequence_number": 6,
                "item_id": "reasoning_1", "output_index": 0, "summary_index": 0,
                "part": {"type": "summary_text", "text": "Plan carefully."}
            }),
            json!({
                "type": "response.reasoning_text.delta", "sequence_number": 7,
                "item_id": "reasoning_1", "output_index": 0, "content_index": 0,
                "delta": "hidden"
            }),
            json!({
                "type": "response.reasoning_text.done", "sequence_number": 8,
                "item_id": "reasoning_1", "output_index": 0, "content_index": 0,
                "text": "hidden"
            }),
            json!({
                "type": "response.output_item.done", "sequence_number": 9,
                "output_index": 0, "item": reasoning_done.clone()
            }),
            json!({
                "type": "response.output_item.added", "sequence_number": 10,
                "output_index": 1,
                "item": {
                    "type": "message", "id": "msg_reasoned", "role": "assistant",
                    "status": "in_progress", "content": []
                }
            }),
            json!({
                "type": "response.content_part.added", "sequence_number": 11,
                "item_id": "msg_reasoned", "output_index": 1, "content_index": 0,
                "part": {"type": "output_text", "text": "", "annotations": []}
            }),
            json!({
                "type": "response.output_text.delta", "sequence_number": 12,
                "item_id": "msg_reasoned", "output_index": 1, "content_index": 0,
                "delta": "Answer"
            }),
            json!({
                "type": "response.output_text.done", "sequence_number": 13,
                "item_id": "msg_reasoned", "output_index": 1, "content_index": 0,
                "text": "Answer"
            }),
            json!({
                "type": "response.content_part.done", "sequence_number": 14,
                "item_id": "msg_reasoned", "output_index": 1, "content_index": 0,
                "part": {"type": "output_text", "text": "Answer", "annotations": []}
            }),
            json!({
                "type": "response.output_item.done", "sequence_number": 15,
                "output_index": 1, "item": message_done.clone()
            }),
            json!({
                "type": "response.completed", "sequence_number": 16,
                "response": {
                    "id": "resp_reasoned", "status": "completed",
                    "output": [reasoning_done, message_done]
                }
            }),
        ]
    }

    fn mixed_reasoning_message_function_events() -> Vec<Value> {
        let base = reasoning_and_text_events();
        let function_done = json!({
            "type": "function_call",
            "id": "fc_mixed",
            "call_id": "call_mixed",
            "name": "lookup",
            "arguments": "{\"query\":\"mixed\"}",
            "status": "completed"
        });
        let mut events = Vec::new();
        events.extend(base.iter().take(13).cloned());
        events.push(json!({
            "type": "response.output_item.added", "sequence_number": 13,
            "output_index": 2,
            "item": {
                "type": "function_call", "id": "fc_mixed", "call_id": "call_mixed",
                "name": "lookup", "arguments": "", "status": "in_progress"
            }
        }));
        events.push(json!({
            "type": "response.function_call_arguments.delta", "sequence_number": 14,
            "item_id": "fc_mixed", "output_index": 2,
            "delta": "{\"query\":\"mixed\"}"
        }));
        for (sequence, mut event) in base.iter().skip(13).take(3).cloned().enumerate() {
            event["sequence_number"] = json!(sequence as u64 + 15);
            events.push(event);
        }
        events.push(json!({
            "type": "response.function_call_arguments.done", "sequence_number": 18,
            "item_id": "fc_mixed", "output_index": 2,
            "arguments": "{\"query\":\"mixed\"}"
        }));
        events.push(json!({
            "type": "response.output_item.done", "sequence_number": 19,
            "output_index": 2, "item": function_done.clone()
        }));
        let mut completed = base[16].clone();
        completed["sequence_number"] = json!(20);
        completed["response"]["output"]
            .as_array_mut()
            .unwrap()
            .push(function_done);
        events.push(completed);
        events
    }

    #[test]
    fn mixed_reasoning_message_and_function_stream_tracks_orthogonal_lifecycles() {
        let events = mixed_reasoning_message_function_events();
        let mut validator = TextStreamValidator::new();
        for original in &events {
            assert_eq!(
                validator.accept_value(original.clone()).unwrap().original(),
                original
            );
        }
        assert!(validator.finish().is_ok());

        let mut wrong_index = TextStreamValidator::new();
        for event in events.iter().take(13) {
            wrong_index.accept_value(event.clone()).unwrap();
        }
        let mut event = events[13].clone();
        event["output_index"] = json!(1);
        assert_eq!(
            wrong_index.accept_value(event).unwrap_err(),
            ProtocolError::InvalidOutputIndexOrder
        );

        let mut duplicate_item = TextStreamValidator::new();
        for event in events.iter().take(13) {
            duplicate_item.accept_value(event.clone()).unwrap();
        }
        let mut event = events[13].clone();
        event["item"]["id"] = json!("msg_reasoned");
        assert_eq!(
            duplicate_item.accept_value(event).unwrap_err(),
            ProtocolError::DuplicateItemId
        );

        let mut wrong_completion_order = TextStreamValidator::new();
        for event in events.iter().take(20) {
            wrong_completion_order.accept_value(event.clone()).unwrap();
        }
        let mut event = events[20].clone();
        event["response"]["output"]
            .as_array_mut()
            .unwrap()
            .swap(1, 2);
        assert!(wrong_completion_order.accept_value(event).is_err());
    }

    #[test]
    fn complete_reasoning_and_text_stream_is_lossless_and_indexed() {
        let mut validator = TextStreamValidator::new();
        let mut summary = String::new();
        let mut content = String::new();
        for original in reasoning_and_text_events() {
            let event = validator.accept_value(original.clone()).unwrap();
            assert_eq!(event.original(), &original);
            match event.kind() {
                TextStreamEventKind::ReasoningSummaryTextDelta { delta, .. } => {
                    summary.push_str(delta);
                }
                TextStreamEventKind::ReasoningTextDelta { delta, .. } => {
                    content.push_str(delta);
                }
                _ => {}
            }
        }
        assert_eq!(summary, "Plan carefully.");
        assert_eq!(content, "hidden");
        assert!(validator.finish().is_ok());
    }

    #[test]
    fn xai_reasoning_only_delta_sequence_is_validated_at_completion() {
        let events = vec![
            json!({
                "type": "response.created", "sequence_number": 0,
                "response": {"id": "resp_minimal", "status": "in_progress", "output": []}
            }),
            json!({
                "type": "response.reasoning_summary_text.delta", "sequence_number": 1,
                "item_id": "reasoning_minimal", "output_index": 0, "summary_index": 0,
                "delta": "concise reasoning"
            }),
            json!({
                "type": "response.completed", "sequence_number": 2,
                "response": {
                    "id": "resp_minimal", "status": "completed",
                    "output": [{
                        "type": "reasoning", "id": "reasoning_minimal",
                        "summary": [{"type": "summary_text", "text": "concise reasoning"}],
                        "encrypted_content": "opaque", "status": "completed"
                    }]
                }
            }),
        ];
        let mut validator = TextStreamValidator::new();
        for original in events {
            assert_eq!(
                validator.accept_value(original.clone()).unwrap().original(),
                &original
            );
        }
        assert!(validator.finish().is_ok());
    }

    #[test]
    fn reasoning_stream_rejects_identity_order_text_and_cipher_corruption() {
        let events = reasoning_and_text_events();

        let mut wrong_id = TextStreamValidator::new();
        for event in events.iter().take(3) {
            wrong_id.accept_value(event.clone()).unwrap();
        }
        let mut event = events[3].clone();
        event["item_id"] = json!("other_reasoning");
        assert_eq!(
            wrong_id.accept_value(event).unwrap_err(),
            ProtocolError::ItemIdMismatch
        );

        let mut wrong_text = TextStreamValidator::new();
        for event in events.iter().take(5) {
            wrong_text.accept_value(event.clone()).unwrap();
        }
        let mut event = events[5].clone();
        event["text"] = json!("different");
        assert_eq!(
            wrong_text.accept_value(event).unwrap_err(),
            ProtocolError::ReasoningMismatch
        );

        let mut missing_done = TextStreamValidator::new();
        for event in events.iter().take(9) {
            missing_done.accept_value(event.clone()).unwrap();
        }
        let mut event = events[16].clone();
        event["sequence_number"] = json!(9);
        assert_eq!(
            missing_done.accept_value(event).unwrap_err(),
            ProtocolError::InvalidSseOrder
        );

        let mut empty_cipher = TextStreamValidator::new();
        for event in events.iter().take(9) {
            empty_cipher.accept_value(event.clone()).unwrap();
        }
        let mut event = events[9].clone();
        event["item"]["encrypted_content"] = json!("");
        assert_eq!(
            empty_cipher.accept_value(event).unwrap_err(),
            ProtocolError::InvalidSseField("item.encrypted_content")
        );
    }

    #[test]
    fn stream_rejects_unknown_nontext_and_post_terminal_events() {
        for event_type in [
            "response.custom_tool_call_input.delta",
            "response.reasoning_summary_text.unknown",
            "response.image_generation_call.partial_image",
            "response.unverified",
        ] {
            let mut validator = TextStreamValidator::new();
            let event = json!({
                "type": event_type,
                "sequence_number": 0,
                "delta": "not inspected"
            });
            assert_eq!(
                validator.accept_value(event).unwrap_err(),
                ProtocolError::UnsupportedSseEvent
            );
        }

        let mut validator = TextStreamValidator::new();
        for event in text_events() {
            validator.accept_value(event).unwrap();
        }
        assert_eq!(
            validator
                .accept_value(text_events()[8].clone())
                .unwrap_err(),
            ProtocolError::EventAfterCompletion
        );
    }
}
