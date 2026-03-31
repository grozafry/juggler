use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub routing: RoutingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoutingConfig {
    pub aliases: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Route {
    pub provider_name: String,
    pub internal_model: String,
}

pub fn resolve_route(alias: &str, config: &GatewayConfig) -> Route {
    // 1. Check if the incoming string matches an explicit alias in config.yaml
    let target = if let Some(target_val) = config.routing.aliases.get(alias) {
        target_val.as_str()
    } else {
        // 2. If it's not an alias, assume the user passed `provider/raw-model-name`
        // or just default to gemini if it has no slash.
        alias
    };

    if let Some((provider, model)) = target.split_once('/') {
        Route {
            provider_name: provider.to_string(),
            internal_model: model.to_string(),
        }
    } else {
        // Fallback for an un-mapped model name, default to gemini assuming Gemini models are passed raw
        Route {
            provider_name: "gemini".to_string(),
            internal_model: target.to_string(),
        }
    }
}
