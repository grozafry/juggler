use crate::server::models::GatewayConfig;
use crate::server::handlers::AppState;
use rand::Rng;

#[derive(Debug, Clone)]
pub struct Route {
    pub provider_name: String,
    pub internal_model: String,
}

pub fn resolve_route_with_state(alias: &str, state: &AppState) -> Result<Route, u64> {
    let target = if let Some(targets) = state.config.routing.aliases.get(alias) {
        if targets.is_empty() {
            alias.to_string()
        } else {
            // Filter targets that are not OPEN circuit breakers
            let mut available_targets = Vec::new();
            let mut all_retry_afters = Vec::new();

            for t in targets {
                // Determine circuit breaker check
                let cb_check = if t.provider.starts_with("gemini") {
                    state.gemini_cb.acquire()
                } else if t.provider.starts_with("anthropic") {
                    state.anthropic_cb.acquire()
                } else {
                    Ok(()) // unknown provider, assume closed
                };

                match cb_check {
                    Ok(_) => available_targets.push(t),
                    Err(retry_after) => all_retry_afters.push(retry_after),
                }
            }

            if available_targets.is_empty() {
                // All failed! Return the max retry_after
                let max_retry = all_retry_afters.into_iter().max().unwrap_or(60);
                return Err(max_retry);
            }

            let total_weight: u32 = available_targets.iter().map(|t| t.weight).sum();
            if total_weight == 0 {
                available_targets[0].provider.clone()
            } else {
                let mut rng = rand::thread_rng();
                let roll = rng.gen_range(0..total_weight);
                let mut cumulative = 0;
                let mut selected = available_targets[0].provider.clone();
                for target in available_targets {
                    cumulative += target.weight;
                    if roll < cumulative {
                        selected = target.provider.clone();
                        break;
                    }
                }
                selected
            }
        }
    } else {
        alias.to_string()
    };

    if let Some((provider, model)) = target.split_once('/') {
        Ok(Route {
            provider_name: provider.to_string(),
            internal_model: model.to_string(),
        })
    } else {
        Ok(Route {
            provider_name: "gemini".to_string(),
            internal_model: target.to_string(),
        })
    }
}
