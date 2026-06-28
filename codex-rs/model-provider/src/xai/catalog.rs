use codex_models_manager::model_info::model_info_from_slug;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;

/// xAI chat models callable via SuperGrok / X Premium+ OAuth.
///
/// Mirrors Hermes' curated `xai-oauth` catalog: `grok-build-0.1` is pinned first,
/// OAuth-only extras such as `grok-composer-2.5-fast` are included, and retired
/// May 2026 models are omitted.
const XAI_CHAT_MODELS: &[(&str, &str, &str, i64)] = &[
    (
        "grok-build-0.1",
        "Grok Build 0.1",
        "Default Grok coding model for SuperGrok OAuth.",
        256_000,
    ),
    (
        "grok-composer-2.5-fast",
        "Grok Composer 2.5 Fast",
        "Fast Grok Composer model available via OAuth.",
        200_000,
    ),
    (
        "grok-4.3",
        "Grok 4.3",
        "Previous default Grok model with a 1M-token context window.",
        1_000_000,
    ),
    (
        "grok-4.20-0309-reasoning",
        "Grok 4.20 Reasoning",
        "Reasoning-focused Grok 4.20 variant.",
        2_000_000,
    ),
    (
        "grok-4.20-0309-non-reasoning",
        "Grok 4.20",
        "Standard Grok 4.20 variant.",
        2_000_000,
    ),
    (
        "grok-4.20-multi-agent-0309",
        "Grok 4.20 Multi-Agent",
        "Multi-agent Grok 4.20 variant.",
        2_000_000,
    ),
];

pub(crate) fn static_model_catalog() -> ModelsResponse {
    ModelsResponse {
        models: XAI_CHAT_MODELS
            .iter()
            .enumerate()
            .map(
                |(priority, (slug, display_name, description, context_window))| {
                    xai_model(
                        slug,
                        display_name,
                        description,
                        i32::try_from(priority).unwrap_or(i32::MAX),
                        *context_window,
                    )
                },
            )
            .collect(),
    }
}

fn xai_model(
    slug: &str,
    display_name: &str,
    description: &str,
    priority: i32,
    context_window: i64,
) -> ModelInfo {
    let mut model = model_info_from_slug(slug);
    model.display_name = display_name.to_string();
    model.description = Some(description.to_string());
    model.visibility = ModelVisibility::List;
    model.supported_in_api = true;
    model.priority = priority;
    model.context_window = Some(context_window);
    model.max_context_window = Some(context_window);
    model.used_fallback_model_metadata = false;
    model.additional_speed_tiers.clear();
    model.service_tiers.clear();
    model.default_service_tier = None;
    model
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn catalog_matches_hermes_xai_oauth_curated_order() {
        let catalog = static_model_catalog();

        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.slug.as_str())
                .collect::<Vec<_>>(),
            vec![
                "grok-build-0.1",
                "grok-composer-2.5-fast",
                "grok-4.3",
                "grok-4.20-0309-reasoning",
                "grok-4.20-0309-non-reasoning",
                "grok-4.20-multi-agent-0309",
            ]
        );
    }

    #[test]
    fn catalog_models_are_picker_visible_with_display_names() {
        let catalog = static_model_catalog();

        assert_eq!(catalog.models[0].display_name, "Grok Build 0.1");
        assert_eq!(catalog.models[1].display_name, "Grok Composer 2.5 Fast");
        assert!(
            catalog
                .models
                .iter()
                .all(|model| model.visibility == ModelVisibility::List)
        );
    }
}
