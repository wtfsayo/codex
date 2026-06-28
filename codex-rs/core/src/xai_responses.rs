//! Adapt Codex `/responses` requests for xAI's Grok API.
//!
//! xAI exposes an OpenAI-compatible `/v1/responses` surface but rejects several
//! Codex-specific input item types and tool shapes. This module strips or converts
//! those payloads before they are sent upstream.

use codex_api::Reasoning;
use codex_api::ResponsesApiRequest;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use serde_json::Map;
use serde_json::Value;

const XAI_NATIVE_TOOL_TYPES: &[&str] = &[
    "web_search",
    "x_search",
    "collections_search",
    "file_search",
    "code_execution",
    "code_interpreter",
    "mcp",
    "shell",
];

const XAI_STRIP_SCHEMA_KEYS: &[&str] = &["pattern", "format"];

/// Returns whether the Grok model accepts a `reasoning.effort` dial.
pub fn grok_supports_reasoning_effort(model: &str) -> bool {
    let mut name = model.trim().to_lowercase();
    if let Some((_, slug)) = name.rsplit_once('/') {
        name = slug.to_string();
    }
    name.starts_with("grok-3-mini")
        || name.starts_with("grok-4.20-multi-agent")
        || name.starts_with("grok-4.3")
}

/// Build xAI-compatible reasoning controls when effort is configured.
pub fn build_xai_reasoning(
    model: &str,
    effort: Option<ReasoningEffortConfig>,
) -> Option<Reasoning> {
    if !grok_supports_reasoning_effort(model) {
        return None;
    }
    effort.map(|effort| Reasoning {
        effort: Some(effort),
        summary: None,
        context: None,
    })
}

/// Hermes always requests encrypted reasoning on xAI when reasoning is enabled.
pub fn xai_reasoning_include() -> Vec<String> {
    vec!["reasoning.encrypted_content".to_string()]
}

/// Encode a Responses request using xAI's wire format for `input` items.
pub fn encode_responses_request_for_xai(
    request: &ResponsesApiRequest,
) -> Result<Value, serde_json::Error> {
    let mut body = serde_json::to_value(request)?;
    let wire_input = request
        .input
        .iter()
        .filter_map(response_item_to_xai_wire_value)
        .collect::<Vec<_>>();
    if let Some(input) = body.get_mut("input") {
        *input = Value::Array(wire_input);
    }
    Ok(body)
}

/// Sanitize a built [`ResponsesApiRequest`] for xAI's Grok `/responses` API.
pub fn adapt_responses_request(request: &mut ResponsesApiRequest) {
    request.input = sanitize_input_for_xai(std::mem::take(&mut request.input));
    request.tools = request
        .tools
        .take()
        .map(|tools| sanitize_tools_for_xai(&tools))
        .filter(|tools| !tools.is_empty());
    request.store = false;
    request.service_tier = None;
}

fn response_item_to_xai_wire_value(item: &ResponseItem) -> Option<Value> {
    match item {
        ResponseItem::Message {
            id,
            role,
            content,
            phase,
            ..
        } => Some(message_to_xai_wire(
            role,
            content,
            id.as_deref(),
            phase.as_ref(),
        )),
        ResponseItem::FunctionCall {
            call_id,
            name,
            arguments,
            ..
        } => Some(serde_json::json!({
            "type": "function_call",
            "call_id": call_id,
            "name": name,
            "arguments": arguments,
        })),
        ResponseItem::FunctionCallOutput {
            call_id, output, ..
        } => Some(serde_json::json!({
            "type": "function_call_output",
            "call_id": call_id,
            "output": output,
        })),
        ResponseItem::Reasoning {
            summary,
            encrypted_content,
            ..
        } => encrypted_content
            .as_ref()
            .filter(|content| !content.is_empty())
            .map(|encrypted_content| {
                serde_json::json!({
                    "type": "reasoning",
                    "encrypted_content": encrypted_content,
                    "summary": summary,
                })
            }),
        _ => None,
    }
}

fn message_to_xai_wire(
    role: &str,
    content: &[ContentItem],
    id: Option<&str>,
    phase: Option<&codex_protocol::models::MessagePhase>,
) -> Value {
    let wire_content = content_items_to_xai_wire(role, content);
    if role == "assistant" && (id.is_some() || phase.is_some()) {
        let mut message = Map::new();
        message.insert("type".to_string(), Value::String("message".to_string()));
        message.insert("role".to_string(), Value::String(role.to_string()));
        message.insert("status".to_string(), Value::String("completed".to_string()));
        message.insert("content".to_string(), wire_content);
        if let Some(id) = id.filter(|id| !id.is_empty()) {
            message.insert("id".to_string(), Value::String(id.to_string()));
        }
        if let Some(phase) = phase {
            message.insert(
                "phase".to_string(),
                Value::String(
                    serde_json::to_value(phase)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_string))
                        .unwrap_or_else(|| "final_answer".to_string()),
                ),
            );
        }
        Value::Object(message)
    } else {
        serde_json::json!({
            "role": role,
            "content": wire_content,
        })
    }
}

fn content_items_to_xai_wire(role: &str, content: &[ContentItem]) -> Value {
    let text_type = if role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    Value::Array(
        content
            .iter()
            .filter_map(|item| match item {
                ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                    Some(serde_json::json!({
                        "type": text_type,
                        "text": text,
                    }))
                }
                ContentItem::InputImage { image_url, detail } => {
                    let mut image = serde_json::json!({
                        "type": "input_image",
                        "image_url": image_url,
                    });
                    if let Some(detail) = detail {
                        image["detail"] = serde_json::to_value(detail).unwrap_or(Value::Null);
                    }
                    Some(image)
                }
            })
            .collect(),
    )
}

fn sanitize_input_for_xai(input: Vec<ResponseItem>) -> Vec<ResponseItem> {
    input
        .into_iter()
        .filter_map(normalize_input_item_for_xai)
        .collect()
}

fn normalize_input_item_for_xai(item: ResponseItem) -> Option<ResponseItem> {
    match item {
        ResponseItem::Message {
            id,
            role,
            content,
            phase,
            internal_chat_message_metadata_passthrough,
        } => {
            if role == "developer" {
                Some(ResponseItem::Message {
                    id,
                    role: "user".to_string(),
                    content,
                    phase: None,
                    internal_chat_message_metadata_passthrough,
                })
            } else if role == "user" || role == "assistant" {
                Some(ResponseItem::Message {
                    id,
                    role,
                    content,
                    phase,
                    internal_chat_message_metadata_passthrough,
                })
            } else {
                None
            }
        }
        ResponseItem::FunctionCall {
            id,
            name,
            namespace: _,
            arguments,
            call_id,
            internal_chat_message_metadata_passthrough,
        } => Some(ResponseItem::FunctionCall {
            id,
            name,
            namespace: None,
            arguments,
            call_id,
            internal_chat_message_metadata_passthrough,
        }),
        ResponseItem::FunctionCallOutput {
            id,
            call_id,
            output,
            internal_chat_message_metadata_passthrough,
        } => Some(ResponseItem::FunctionCallOutput {
            id,
            call_id,
            output,
            internal_chat_message_metadata_passthrough,
        }),
        ResponseItem::CustomToolCall {
            id,
            status: _,
            call_id,
            name,
            namespace: _,
            input,
            internal_chat_message_metadata_passthrough,
        } => Some(ResponseItem::FunctionCall {
            id,
            name,
            namespace: None,
            arguments: input,
            call_id,
            internal_chat_message_metadata_passthrough,
        }),
        ResponseItem::CustomToolCallOutput {
            id,
            call_id,
            name: _,
            output,
            internal_chat_message_metadata_passthrough,
        } => Some(ResponseItem::FunctionCallOutput {
            id,
            call_id,
            output,
            internal_chat_message_metadata_passthrough,
        }),
        ResponseItem::Reasoning {
            id: _,
            summary,
            content: _,
            encrypted_content,
            internal_chat_message_metadata_passthrough: _,
        } => encrypted_content
            .filter(|content| !content.is_empty())
            .map(|encrypted_content| ResponseItem::Reasoning {
                id: None,
                summary,
                content: None,
                encrypted_content: Some(encrypted_content),
                internal_chat_message_metadata_passthrough: None,
            }),
        ResponseItem::AdditionalTools { .. }
        | ResponseItem::AgentMessage { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::ContextCompaction { .. }
        | ResponseItem::CompactionTrigger { .. }
        | ResponseItem::Other => None,
    }
}

fn sanitize_tools_for_xai(tools: &[Value]) -> Vec<Value> {
    let mut sanitized = Vec::new();
    let mut saw_client_web_search = false;

    for tool in tools {
        match convert_tool_for_xai(tool) {
            ToolConversion::Converted(converted) => {
                if converted.get("type").and_then(Value::as_str) == Some("function")
                    && converted.get("name").and_then(Value::as_str) == Some("web_search")
                {
                    saw_client_web_search = true;
                    continue;
                }
                sanitized.push(converted);
            }
            ToolConversion::Skip => {}
            ToolConversion::Flatten(nested) => sanitized.extend(nested),
        }
    }

    if saw_client_web_search
        && !sanitized
            .iter()
            .any(|tool| tool.get("type").and_then(Value::as_str) == Some("web_search"))
    {
        sanitized.push(Value::Object(
            [("type".to_string(), Value::String("web_search".to_string()))]
                .into_iter()
                .collect(),
        ));
    }

    sanitized
        .into_iter()
        .map(|mut tool| {
            sanitize_tool_schema_for_xai(&mut tool);
            tool
        })
        .collect()
}

fn sanitize_tool_schema_for_xai(tool: &mut Value) {
    if let Some(parameters) = tool.get_mut("parameters") {
        strip_xai_incompatible_schema_keys(parameters);
        return;
    }
    if let Some(function) = tool.get_mut("function")
        && let Some(parameters) = function.get_mut("parameters")
    {
        strip_xai_incompatible_schema_keys(parameters);
    }
}

fn strip_xai_incompatible_schema_keys(node: &mut Value) {
    match node {
        Value::Object(map) => {
            let is_schema_node = map.contains_key("type")
                || map.contains_key("anyOf")
                || map.contains_key("oneOf")
                || map.contains_key("allOf");
            if is_schema_node {
                for key in XAI_STRIP_SCHEMA_KEYS {
                    map.remove(*key);
                }
                if let Some(Value::Array(enum_values)) = map.get("enum")
                    && enum_values
                        .iter()
                        .any(|value| matches!(value, Value::String(s) if s.contains('/')))
                {
                    map.remove("enum");
                }
            }
            for value in map.values_mut() {
                strip_xai_incompatible_schema_keys(value);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_xai_incompatible_schema_keys(item);
            }
        }
        _ => {}
    }
}

enum ToolConversion {
    Converted(Value),
    Flatten(Vec<Value>),
    Skip,
}

fn convert_tool_for_xai(tool: &Value) -> ToolConversion {
    let Some(tool_type) = tool.get("type").and_then(Value::as_str) else {
        return ToolConversion::Skip;
    };

    if XAI_NATIVE_TOOL_TYPES.contains(&tool_type) {
        return ToolConversion::Converted(Value::Object(
            [("type".to_string(), Value::String(tool_type.to_string()))]
                .into_iter()
                .collect(),
        ));
    }

    if tool_type == "function" {
        return ToolConversion::Converted(convert_function_tool_for_xai(tool));
    }

    if tool_type == "custom" {
        return convert_custom_tool_for_xai(tool)
            .map_or(ToolConversion::Skip, ToolConversion::Converted);
    }

    if tool_type == "namespace" {
        let nested = tool
            .get("tools")
            .or_else(|| tool.get("functions"))
            .and_then(Value::as_array);
        if let Some(nested_tools) = nested {
            let converted = nested_tools
                .iter()
                .flat_map(|nested_tool| match convert_tool_for_xai(nested_tool) {
                    ToolConversion::Converted(converted) => vec![converted],
                    ToolConversion::Flatten(flattened) => flattened,
                    ToolConversion::Skip => Vec::new(),
                })
                .collect();
            return ToolConversion::Flatten(converted);
        }
        return ToolConversion::Skip;
    }

    ToolConversion::Skip
}

fn convert_function_tool_for_xai(tool: &Value) -> Value {
    let name = tool
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            tool.get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();

    let description = tool
        .get("description")
        .or_else(|| {
            tool.get("function")
                .and_then(|function| function.get("description"))
        })
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let parameters = tool
        .get("parameters")
        .or_else(|| {
            tool.get("function")
                .and_then(|function| function.get("parameters"))
        })
        .cloned()
        .unwrap_or_else(|| {
            serde_json::json!({
                "type": "object",
                "properties": {}
            })
        });

    serde_json::json!({
        "type": "function",
        "name": name,
        "description": description,
        "strict": tool.get("strict").and_then(Value::as_bool).unwrap_or(false),
        "parameters": parameters,
    })
}

fn convert_custom_tool_for_xai(tool: &Value) -> Option<Value> {
    let name = tool.get("name").and_then(Value::as_str)?.trim();
    if name.is_empty() {
        return None;
    }

    Some(serde_json::json!({
        "type": "function",
        "name": name,
        "description": tool.get("description").and_then(Value::as_str).unwrap_or(""),
        "strict": false,
        "parameters": {
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Freeform input for the original Codex custom tool."
                }
            },
            "required": ["input"],
            "additionalProperties": false
        }
    }))
}

#[cfg(test)]
#[path = "xai_responses_tests.rs"]
mod tests;
