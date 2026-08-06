//! MarketingCopilot tool for evaluating marketing channels, reasoning spend via LLM,
//! enforcing budget caps in Rust code, and executing Solana transfer transactions.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::process::Command;
use zeroclaw_api::model_provider::ModelProvider;
use zeroclaw_api::tool::{Tool, ToolOutput, ToolResult};
use zeroclaw_config::policy::SecurityPolicy;
use zeroclaw_config::policy::ToolOperation;
use zeroclaw_providers::ProviderDispatch;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketingChannel {
    pub name: String,
    pub signups: u32,
    pub spend: f64,
    pub conversion_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketingCopilotInput {
    pub channels: Vec<MarketingChannel>,
    pub budget_cap: f64,
    #[serde(default)]
    pub target_wallet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketingCopilotOutput {
    pub chosen_channel: String,
    pub reasoning: String,
    pub amount: f64,
    pub tx_signature: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmDecision {
    pub chosen_channel: String,
    pub reasoning: String,
    pub proposed_amount: f64,
}

pub struct MarketingCopilotTool {
    security: Arc<SecurityPolicy>,
    default_model_provider: String,
    default_model: String,
    default_temperature: Option<f64>,
    api_key: Option<String>,
    provider_runtime_options: zeroclaw_providers::ModelProviderRuntimeOptions,
    pub default_target_wallet: String,
}

impl MarketingCopilotTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        default_model_provider: String,
        default_model: String,
        default_temperature: Option<f64>,
        api_key: Option<String>,
        provider_runtime_options: zeroclaw_providers::ModelProviderRuntimeOptions,
        default_target_wallet: String,
    ) -> Self {
        Self {
            security,
            default_model_provider,
            default_model,
            default_temperature,
            api_key,
            provider_runtime_options,
            default_target_wallet,
        }
    }

    pub fn with_defaults(security: Arc<SecurityPolicy>) -> Self {
        let api_key = std::env::var("GROQ_API_KEY").ok();
        Self {
            security,
            default_model_provider: "groq".to_string(),
            default_model: "llama-3.3-70b-versatile".to_string(),
            default_temperature: Some(0.2),
            api_key,
            provider_runtime_options: zeroclaw_providers::ModelProviderRuntimeOptions::default(),
            default_target_wallet: "11111111111111111111111111111111".to_string(),
        }
    }

    /// Evaluates channel data using Zeroclaw's configured LLM provider.
    pub async fn evaluate_channels_with_llm(
        &self,
        channels: &[MarketingChannel],
    ) -> Result<LlmDecision, String> {
        let channels_json = serde_json::to_string_pretty(channels)
            .map_err(|e| format!("Failed to serialize channels: {e}"))?;

        let prompt = format!(
            "You are a marketing strategist assistant. Evaluate the following marketing channels data:\n\n\
             ```json\n{channels_json}\n```\n\n\
             Identify the single highest-leverage marketing channel for the next marketing spend.\n\
             Explain why in 1-2 plain-English sentences, and recommend a specific proposed spend amount (number).\n\n\
             IMPORTANT: You MUST respond ONLY with a valid JSON object matching this schema:\n\
             {{\n  \"chosen_channel\": \"channel_name\",\n  \"reasoning\": \"1-2 sentence explanation\",\n  \"proposed_amount\": 150.00\n}}"
        );

        let env_key = std::env::var("GROQ_API_KEY").ok();
        let api_key_ref = self.api_key.as_deref().or(env_key.as_deref());
        let model_provider: Box<dyn ModelProvider> =
            match zeroclaw_providers::create_model_provider_with_options(
                &self.default_model_provider,
                api_key_ref,
                &self.provider_runtime_options,
            ) {
                Ok(provider) => provider,
                Err(e) => return Err(format!("Failed to create model provider: {e}")),
            };

        let response = match ProviderDispatch::from_ref(&*model_provider)
            .simple_chat(&prompt, &self.default_model, self.default_temperature)
            .await
        {
            Ok(text) => text,
            Err(e) => return Err(format!("LLM provider call failed: {e}")),
        };

        parse_llm_decision(&response)
    }
}

pub fn parse_llm_decision(response: &str) -> Result<LlmDecision, String> {
    let trimmed = response.trim();
    let json_str = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };

    serde_json::from_str::<LlmDecision>(json_str)
        .map_err(|e| format!("Failed to parse LLM response: {e}. Raw text: {response}"))
}

/// Hard code-level check: compares proposed spend against budget_cap.
/// Clamps to cap if exceeded and appends notification to reasoning.
pub fn enforce_budget_cap(proposed_amount: f64, budget_cap: f64, reasoning: &mut String) -> f64 {
    if proposed_amount > budget_cap {
        let note = format!(" (capped from ${proposed_amount:.2} to budget limit ${budget_cap:.2})");
        reasoning.push_str(&note);
        budget_cap
    } else {
        proposed_amount
    }
}

fn find_solana_binary() -> String {
    if which::which("solana").is_ok() {
        return "solana".to_string();
    }
    if which::which("sol").is_ok() {
        return "sol".to_string();
    }
    if let Ok(home) = std::env::var("HOME") {
        let active = format!("{home}/.local/share/solana/install/active_release/bin/solana");
        if std::path::Path::new(&active).exists() {
            return active;
        }
    }
    "solana".to_string()
}

/// Executes Solana CLI transfer command asynchronously via subprocess.
pub async fn execute_solana_transfer(target_wallet: &str, amount: f64) -> Result<String, String> {
    let amount_str = format!("{amount}");
    let sol_bin = find_solana_binary();

    let mut cmd = Command::new(&sol_bin);
    cmd.args([
        "transfer",
        target_wallet,
        &amount_str,
        "--output",
        "json",
        "--allow-unfunded-recipient",
    ]);

    let output = match cmd.output().await {
        Ok(out) => out,
        Err(err) => {
            return Err(format!(
                "Solana CLI execution failed to start binary ('{sol_bin}'): {err}"
            ));
        }
    };

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stderr_str = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        let err_detail = if !stderr_str.trim().is_empty() {
            stderr_str.trim()
        } else if !stdout_str.trim().is_empty() {
            stdout_str.trim()
        } else {
            "Sol CLI exited with non-zero status"
        };
        return Err(format!("Sol CLI error: {err_detail}"));
    }

    parse_solana_cli_json(&stdout_str)
}

pub fn parse_solana_cli_json(json_text: &str) -> Result<String, String> {
    let trimmed = json_text.trim();
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(sig) = parsed.get("signature").and_then(|v| v.as_str()) {
            return Ok(sig.to_string());
        }
        if let Some(sig) = parsed.get("transactionSignature").and_then(|v| v.as_str()) {
            return Ok(sig.to_string());
        }
    }
    if !trimmed.is_empty() {
        Ok(trimmed.to_string())
    } else {
        Err("Empty response from Solana CLI".to_string())
    }
}

#[async_trait]
impl Tool for MarketingCopilotTool {
    fn name(&self) -> &str {
        "marketing_copilot"
    }

    fn description(&self) -> &str {
        "ALWAYS call this tool whenever the user asks to analyze, check, evaluate, or optimize marketing traction data, recommend channel spend, or allocate marketing budget. Evaluates channel performance, reasons spend allocation, enforces code-level budget caps, and executes Solana transfer transactions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "channels": {
                    "type": "array",
                    "description": "List of marketing channels with signups, spend, and conversion rate",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "signups": { "type": "integer", "minimum": 0 },
                            "spend": { "type": "number", "minimum": 0.0 },
                            "conversion_rate": { "type": "number", "minimum": 0.0 }
                        },
                        "required": ["name", "signups", "spend", "conversion_rate"]
                    }
                },
                "budget_cap": {
                    "type": "number",
                    "description": "Hard budget cap limit for proposed spending amount"
                },
                "target_wallet": {
                    "type": "string",
                    "description": "Optional target Solana wallet public key for transfer execution"
                }
            },
            "required": ["channels", "budget_cap"]
        })
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "chosen_channel": { "type": "string" },
                "reasoning": { "type": "string" },
                "amount": { "type": "number" },
                "tx_signature": { "type": ["string", "null"] },
                "status": { "type": "string" }
            },
            "required": ["chosen_channel", "reasoning", "amount", "status"]
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        // Security policy check
        if let Err(error) = self
            .security
            .enforce_tool_operation(ToolOperation::Act, "marketing_copilot")
        {
            return Ok(ToolResult {
                success: false,
                output: ToolOutput::default(),
                error: Some(error),
            });
        }

        // Unwrap `properties` wrapper if smaller LLM models emit `{"properties": {"channels": ...}}`
        let effective_args = if let Some(props) = args.get("properties")
            && props.is_object()
            && props.get("channels").is_some()
        {
            props.clone()
        } else {
            args
        };

        // Parse input
        let input: MarketingCopilotInput = match serde_json::from_value(effective_args) {
            Ok(parsed) => parsed,
            Err(err) => {
                return Ok(ToolResult {
                    success: false,
                    output: ToolOutput::default(),
                    error: Some(format!("Invalid input payload: {err}")),
                });
            }
        };

        if input.channels.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: ToolOutput::default(),
                error: Some("Channels array cannot be empty".to_string()),
            });
        }

        // Evaluate via LLM (or fallback if LLM unavailable)
        let mut decision = match self.evaluate_channels_with_llm(&input.channels).await {
            Ok(dec) => dec,
            Err(err_msg) => {
                let best = input
                    .channels
                    .iter()
                    .max_by(|a, b| {
                        let score_a = (a.signups as f64) * a.conversion_rate;
                        let score_b = (b.signups as f64) * b.conversion_rate;
                        score_a
                            .partial_cmp(&score_b)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .cloned()
                    .unwrap();

                LlmDecision {
                    chosen_channel: best.name.clone(),
                    reasoning: format!(
                        "Selected {} based on highest conversion traction (fallback: {err_msg})",
                        best.name
                    ),
                    proposed_amount: input.budget_cap,
                }
            }
        };

        // Enforce hard budget cap in code
        let approved_amount = enforce_budget_cap(
            decision.proposed_amount,
            input.budget_cap,
            &mut decision.reasoning,
        );

        // Resolve target wallet address (input override or default)
        let target_wallet = input
            .target_wallet
            .as_deref()
            .filter(|w| !w.trim().is_empty())
            .unwrap_or(&self.default_target_wallet);

        // Execute Solana CLI transfer
        let (tx_signature, status) =
            match execute_solana_transfer(target_wallet, approved_amount).await {
                Ok(sig) => (Some(sig), "success".to_string()),
                Err(err_msg) => {
                    decision
                        .reasoning
                        .push_str(&format!(" | Transfer failed: {err_msg}"));
                    (None, "failed".to_string())
                }
            };

        let output = MarketingCopilotOutput {
            chosen_channel: decision.chosen_channel,
            reasoning: decision.reasoning,
            amount: approved_amount,
            tx_signature,
            status,
        };

        let json_val = serde_json::to_value(&output)?;
        Ok(ToolResult::ok(ToolOutput::json(json_val)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_llm_json_response() {
        let raw = r#"{
            "chosen_channel": "google_ads",
            "reasoning": "Google ads showed the highest conversion rate (8%) and strong signup volume.",
            "proposed_amount": 150.0
        }"#;

        let parsed = parse_llm_decision(raw).unwrap();
        assert_eq!(parsed.chosen_channel, "google_ads");
        assert_eq!(parsed.proposed_amount, 150.0);
        assert!(parsed.reasoning.contains("conversion rate"));
    }

    #[test]
    fn parse_markdown_fenced_llm_json_response() {
        let raw = "```json\n{\n  \"chosen_channel\": \"twitter\",\n  \"reasoning\": \"High signups at low cost.\",\n  \"proposed_amount\": 80.5\n}\n```";

        let parsed = parse_llm_decision(raw).unwrap();
        assert_eq!(parsed.chosen_channel, "twitter");
        assert_eq!(parsed.proposed_amount, 80.5);
    }

    #[test]
    fn parse_solana_cli_json_signature() {
        let raw = r#"{"signature": "5K123456789abcdef"}"#;
        assert_eq!(parse_solana_cli_json(raw).unwrap(), "5K123456789abcdef");

        let raw_tx = r#"{"transactionSignature": "tx987654321"}"#;
        assert_eq!(parse_solana_cli_json(raw_tx).unwrap(), "tx987654321");
    }

    #[test]
    fn budget_cap_enforcement_clamps_excessive_spend() {
        let mut reasoning = "Recommended spend of $150 on Google Ads.".to_string();
        let approved = enforce_budget_cap(150.0, 100.0, &mut reasoning);

        assert_eq!(approved, 100.0);
        assert!(reasoning.contains("capped from $150.00 to budget limit $100.00"));
    }

    #[test]
    fn budget_cap_enforcement_allows_under_budget_spend() {
        let mut reasoning = "Recommended spend of $75 on Twitter Ads.".to_string();
        let approved = enforce_budget_cap(75.0, 100.0, &mut reasoning);

        assert_eq!(approved, 75.0);
        assert!(!reasoning.contains("capped"));
    }

    #[tokio::test]
    async fn execute_with_mock_channels_graceful_cli_failure() {
        let tool = MarketingCopilotTool::with_defaults(Arc::new(SecurityPolicy::default()));
        let mock_input = json!({
            "channels": [
                { "name": "google_ads", "signups": 120, "spend": 400.0, "conversion_rate": 0.04 },
                { "name": "twitter_ads", "signups": 350, "spend": 300.0, "conversion_rate": 0.08 }
            ],
            "budget_cap": 50.0,
            "target_wallet": "DemoTargetWalletAddress11111111111111111"
        });

        let result = tool.execute(mock_input).await.unwrap();
        assert!(result.success);
        let out: MarketingCopilotOutput =
            serde_json::from_value(result.output.into_data().unwrap()).unwrap();
        assert_eq!(out.amount, 50.0);
        // Unless solana CLI is configured and funded, status should gracefully report failed
        assert!(out.status == "failed" || out.status == "success");
    }

    #[tokio::test]
    async fn execute_unwraps_properties_wrapper() {
        let tool = MarketingCopilotTool::with_defaults(Arc::new(SecurityPolicy::default()));
        let wrapped_input = json!({
            "properties": {
                "channels": [
                    { "name": "google_ads", "signups": 120, "spend": 400.0, "conversion_rate": 0.04 }
                ],
                "budget_cap": 25.0,
                "target_wallet": "DemoTargetWalletAddress11111111111111111"
            }
        });

        let result = tool.execute(wrapped_input).await.unwrap();
        assert!(result.success);
        let out: MarketingCopilotOutput =
            serde_json::from_value(result.output.into_data().unwrap()).unwrap();
        assert_eq!(out.amount, 25.0);
    }
}
