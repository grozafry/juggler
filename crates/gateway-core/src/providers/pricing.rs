/// Cost pricing table — price per 1M tokens in USD (as of early 2025)
/// Source: official provider pricing pages
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub provider: &'static str,
}

pub fn get_pricing(model: &str) -> ModelPricing {
    let m = model.to_lowercase();

    // ── Gemini ──────────────────────────────────────────────────────────────
    if m.contains("gemini-2.5-flash") || m.contains("gemini-2.0-flash") {
        return ModelPricing { input_per_million: 0.075, output_per_million: 0.30, provider: "gemini" };
    }
    if m.contains("gemini-1.5-flash") {
        return ModelPricing { input_per_million: 0.075, output_per_million: 0.30, provider: "gemini" };
    }
    if m.contains("gemini-1.5-pro") || m.contains("gemini-pro") {
        return ModelPricing { input_per_million: 1.25, output_per_million: 5.00, provider: "gemini" };
    }
    // ── Anthropic ────────────────────────────────────────────────────────────
    if m.contains("claude-3-5-sonnet") || m.contains("claude-3.5-sonnet") {
        return ModelPricing { input_per_million: 3.00, output_per_million: 15.00, provider: "anthropic" };
    }
    if m.contains("claude-3-5-haiku") || m.contains("claude-3.5-haiku") {
        return ModelPricing { input_per_million: 0.80, output_per_million: 4.00, provider: "anthropic" };
    }
    if m.contains("claude-3-haiku") {
        return ModelPricing { input_per_million: 0.25, output_per_million: 1.25, provider: "anthropic" };
    }
    if m.contains("claude-3-sonnet") {
        return ModelPricing { input_per_million: 3.00, output_per_million: 15.00, provider: "anthropic" };
    }
    if m.contains("claude-3-opus") {
        return ModelPricing { input_per_million: 15.00, output_per_million: 75.00, provider: "anthropic" };
    }

    // Unknown model — zero cost (will show as $0.0000 in dashboard)
    ModelPricing { input_per_million: 0.0, output_per_million: 0.0, provider: "unknown" }
}

pub fn compute_cost_usd(model: &str, prompt_tokens: u32, completion_tokens: u32) -> f64 {
    let p = get_pricing(model);
    (prompt_tokens as f64 * p.input_per_million
        + completion_tokens as f64 * p.output_per_million)
        / 1_000_000.0
}
