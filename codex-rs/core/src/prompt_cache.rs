//! Prompt cache key strategy aligned with Hermes `agent/transports/codex.py`.
//!
//! Hash the static request prefix (instructions + tool schemas) so recurring jobs with
//! fresh thread/session ids still hit the same provider cache bucket. Fall back to the
//! thread id when there is nothing static to hash.

use codex_model_provider_info::ModelProviderInfo;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;

/// Content-address the prompt cache key from instructions and tool JSON.
///
/// Returns `pck_<sha256[:24]>` or `None` when both inputs are empty.
pub fn content_addressed_prompt_cache_key(
    instructions: &str,
    tools: Option<&[Value]>,
) -> Option<String> {
    let has_instructions = !instructions.is_empty();
    let has_tools = tools.is_some_and(|t| !t.is_empty());
    if !has_instructions && !has_tools {
        return None;
    }

    let tools_part = tools
        .map(|tool_list| {
            let mut sorted_tools: Vec<&Value> = tool_list.iter().collect();
            sorted_tools.sort_by_cached_key(|t| {
                t.as_object()
                    .and_then(|o| o.get("name").or_else(|| o.get("type")))
                    .map(|v| v.to_string())
                    .unwrap_or_default()
            });
            serde_json::to_string(&sorted_tools).unwrap_or_default()
        })
        .unwrap_or_default();

    // `\0` separator so instructions ending in tool JSON cannot collide with a
    // request whose instructions contain that JSON and whose tools are empty.
    let content = format!("{}\x00{}", instructions, tools_part);
    let digest = Sha256::digest(content.as_bytes());
    let hex = format!("{digest:x}");
    Some(format!("pck_{}", &hex[..24.min(hex.len())]))
}

pub fn is_xai_provider(provider: &ModelProviderInfo) -> bool {
    provider.is_xai()
        || provider
            .base_url
            .as_deref()
            .is_some_and(|url| url.contains("api.x.ai"))
}

pub fn resolve_prompt_cache_key(
    provider: &ModelProviderInfo,
    instructions: &str,
    tools: Option<&[Value]>,
    thread_id: &str,
    override_key: Option<&str>,
) -> String {
    if let Some(key) = override_key {
        return key.to_string();
    }
    if is_xai_provider(provider) || !provider.is_openai() {
        if let Some(key) = content_addressed_prompt_cache_key(instructions, tools) {
            return key;
        }
    }
    thread_id.to_string()
}
