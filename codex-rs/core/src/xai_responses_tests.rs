use super::*;
use codex_protocol::models::ContentItem;
use pretty_assertions::assert_eq;

#[test]
fn developer_messages_are_rewritten_as_user_messages() {
    let input = vec![ResponseItem::Message {
        id: None,
        role: "developer".to_string(),
        content: vec![ContentItem::InputText {
            text: "context".to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }];

    let sanitized = sanitize_input_for_xai(input);

    assert_eq!(
        sanitized,
        vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "context".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }]
    );
}

#[test]
fn user_messages_use_role_only_wire_format() {
    let wire = response_item_to_xai_wire_value(&ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "hi".to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    })
    .expect("user message");

    assert_eq!(
        wire,
        serde_json::json!({
            "role": "user",
            "content": [{"type": "input_text", "text": "hi"}],
        })
    );
    assert!(wire.get("type").is_none());
}

#[test]
fn assistant_messages_with_phase_use_typed_message_wire_format() {
    let wire = response_item_to_xai_wire_value(&ResponseItem::Message {
        id: Some("msg_1".to_string()),
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "hello".to_string(),
        }],
        phase: Some(codex_protocol::models::MessagePhase::FinalAnswer),
        internal_chat_message_metadata_passthrough: None,
    })
    .expect("assistant message");

    assert_eq!(wire["type"], "message");
    assert_eq!(wire["role"], "assistant");
    assert_eq!(wire["status"], "completed");
    assert_eq!(wire["phase"], "final_answer");
}

#[test]
fn unsupported_input_items_are_dropped() {
    let input = vec![
        ResponseItem::Compaction {
            id: None,
            encrypted_content: "secret".to_string(),
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::WebSearchCall {
            id: Some("ws_1".to_string()),
            status: Some("completed".to_string()),
            action: None,
            internal_chat_message_metadata_passthrough: None,
        },
    ];

    assert_eq!(sanitize_input_for_xai(input), Vec::<ResponseItem>::new());
}

#[test]
fn custom_tool_calls_are_converted_to_function_calls() {
    let input = vec![ResponseItem::CustomToolCall {
        id: None,
        status: None,
        call_id: "call_1".to_string(),
        name: "apply_patch".to_string(),
        namespace: Some("codex".to_string()),
        input: r#"{"path":"a.txt"}"#.to_string(),
        internal_chat_message_metadata_passthrough: None,
    }];

    assert_eq!(
        sanitize_input_for_xai(input),
        vec![ResponseItem::FunctionCall {
            id: None,
            name: "apply_patch".to_string(),
            namespace: None,
            arguments: r#"{"path":"a.txt"}"#.to_string(),
            call_id: "call_1".to_string(),
            internal_chat_message_metadata_passthrough: None,
        }]
    );
}

#[test]
fn namespace_tools_are_flattened_to_functions() {
    let tools = vec![serde_json::json!({
        "type": "namespace",
        "name": "codex",
        "description": "Codex tools",
        "tools": [{
            "type": "function",
            "name": "exec_command",
            "description": "Run a command",
            "strict": false,
            "parameters": {"type": "object", "properties": {}}
        }]
    })];

    assert_eq!(
        sanitize_tools_for_xai(&tools),
        vec![serde_json::json!({
            "type": "function",
            "name": "exec_command",
            "description": "Run a command",
            "strict": false,
            "parameters": {"type": "object", "properties": {}}
        })]
    );
}

#[test]
fn client_web_search_function_is_swapped_for_native_tool() {
    let tools = vec![serde_json::json!({
        "type": "function",
        "name": "web_search",
        "description": "Search the web",
        "strict": false,
        "parameters": {"type": "object", "properties": {}}
    })];

    assert_eq!(
        sanitize_tools_for_xai(&tools),
        vec![serde_json::json!({"type": "web_search"})]
    );
}

#[test]
fn slash_enums_and_pattern_keys_are_stripped_from_tool_schemas() {
    let tools = vec![serde_json::json!({
        "type": "function",
        "name": "pick_model",
        "description": "Pick a model",
        "strict": false,
        "parameters": {
            "type": "object",
            "properties": {
                "model": {
                    "type": "string",
                    "enum": ["Qwen/Qwen3.5-0.8B"],
                    "pattern": "^[a-z]+$"
                }
            }
        }
    })];

    let sanitized = sanitize_tools_for_xai(&tools);
    let model_schema = &sanitized[0]["parameters"]["properties"]["model"];
    assert!(model_schema.get("enum").is_none());
    assert!(model_schema.get("pattern").is_none());
}

#[test]
fn adapt_responses_request_applies_input_and_tool_sanitization() {
    let mut request = ResponsesApiRequest {
        model: "grok-4.3".to_string(),
        instructions: "base".to_string(),
        input: vec![ResponseItem::Message {
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: "context".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }],
        tools: Some(vec![serde_json::json!({
            "type": "custom",
            "name": "apply_patch",
            "description": "Patch files",
            "format": {"type": "text", "syntax": "lark", "definition": "..."}
        })]),
        tool_choice: "auto".to_string(),
        parallel_tool_calls: true,
        reasoning: None,
        store: true,
        stream: true,
        include: vec!["reasoning.encrypted_content".to_string()],
        service_tier: Some("auto".to_string()),
        prompt_cache_key: Some("pck_test".to_string()),
        text: None,
        client_metadata: None,
    };

    adapt_responses_request(&mut request);

    assert!(request.input[0].is_user_message());
    assert_eq!(request.tools.as_ref().unwrap()[0]["type"], "function");
    assert!(!request.store);
    assert!(request.service_tier.is_none());
    assert_eq!(request.tool_choice, "auto");
}

#[test]
fn adapt_responses_request_clears_tool_choice_when_tools_are_removed() {
    let mut request = ResponsesApiRequest {
        model: "grok-4.3".to_string(),
        instructions: String::new(),
        input: vec![],
        tools: Some(vec![serde_json::json!({
            "type": "namespace",
            "name": "codex",
            "tools": []
        })]),
        tool_choice: "auto".to_string(),
        parallel_tool_calls: true,
        reasoning: None,
        store: true,
        stream: true,
        include: vec![],
        service_tier: None,
        prompt_cache_key: None,
        text: None,
        client_metadata: None,
    };

    adapt_responses_request(&mut request);

    assert!(request.tools.is_none());
    assert!(request.tool_choice.is_empty());

    let body = encode_responses_request_for_xai(&request).expect("encode xAI request");
    assert!(body.get("tool_choice").is_none());
    assert!(body.get("parallel_tool_calls").is_none());
}

#[test]
fn encode_responses_request_for_xai_matches_hermes_user_input_shape() {
    let mut request = ResponsesApiRequest {
        model: "grok-4.3".to_string(),
        instructions: "base".to_string(),
        input: vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "hi".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }],
        tools: None,
        tool_choice: "auto".to_string(),
        parallel_tool_calls: true,
        reasoning: None,
        store: false,
        stream: true,
        include: xai_reasoning_include(),
        service_tier: None,
        prompt_cache_key: Some("pck_test".to_string()),
        text: None,
        client_metadata: None,
    };
    adapt_responses_request(&mut request);

    let body = encode_responses_request_for_xai(&request).expect("encode xAI request");
    assert_eq!(
        body["input"],
        serde_json::json!([{
            "role": "user",
            "content": [{"type": "input_text", "text": "hi"}],
        }])
    );
    assert_eq!(
        body["include"],
        serde_json::json!(["reasoning.encrypted_content"])
    );
    assert!(body.get("tool_choice").is_none());
}

#[test]
fn grok_reasoning_effort_allowlist_matches_hermes() {
    assert!(grok_supports_reasoning_effort("grok-4.3"));
    assert!(grok_supports_reasoning_effort("grok-4.20-multi-agent-0309"));
    assert!(!grok_supports_reasoning_effort("grok-composer-2.5-fast"));
}
