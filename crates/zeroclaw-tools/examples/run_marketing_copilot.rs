//! Standalone binary example for MarketingCopilotTool.
//!
//! Bypasses the full agent chat/tool-selection system entirely.
//! Calls `MarketingCopilotTool::execute` directly with hardcoded real input,
//! makes a single-purpose Groq LLM call for reasoning, and executes a real
//! Solana CLI transfer on devnet.
//!
//! Run with:
//!   GROQ_API_KEY="gsk_..." cargo run --release --example run_marketing_copilot -p zeroclaw-tools

use std::sync::Arc;
use zeroclaw_api::tool::Tool;
use zeroclaw_config::policy::SecurityPolicy;
use zeroclaw_tools::marketing_copilot::MarketingCopilotTool;

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

/// Query `solana balance <address>` and return the trimmed output.
async fn get_balance(address: &str) -> String {
    let sol_bin = find_solana_binary();
    let output = tokio::process::Command::new(&sol_bin)
        .args(["balance", address])
        .output()
        .await;
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            format!("(error: {})", err.trim())
        }
        Err(e) => format!("(unavailable: {e})"),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let api_key = std::env::var("GROQ_API_KEY")
        .or_else(|_| std::env::var("ZEROCLAW_providers__models__groq__default__api_key"))
        .ok();

    if api_key.is_none() {
        eprintln!(
            "NOTE: Neither GROQ_API_KEY nor ZEROCLAW_providers__models__groq__default__api_key is set.\n\
             The tool will evaluate channels using deterministic traction scoring fallback if the API call fails.\n"
        );
    }

    let funder_wallet = "4cmtMJmzC9BjmAWAEHFqmzGPnuvmMTx9xRn3er15A6uG";
    let vendor_wallet = "46G4njoWB6Uz8kyHJN4PEHWBzP63RrMtzTLGCae7hyzR";
    let security = Arc::new(SecurityPolicy::default());

    let tool = MarketingCopilotTool::new(
        security,
        "groq".to_string(),
        "llama-3.3-70b-versatile".to_string(),
        Some(0.2),
        api_key,
        zeroclaw_providers::ModelProviderRuntimeOptions::default(),
        vendor_wallet.to_string(),
    );

    let input = serde_json::json!({
        "channels": [
            { "name": "google_ads",  "signups": 120, "spend": 400.0, "conversion_rate": 0.04 },
            { "name": "twitter_ads", "signups": 350, "spend": 300.0, "conversion_rate": 0.08 }
        ],
        "budget_cap": 0.01,
        "target_wallet": vendor_wallet
    });

    println!("============================================================");
    println!("  MarketingCopilotTool — Direct Execution (Groq + Devnet)");
    println!("============================================================");
    println!("  Funder wallet : {funder_wallet}");
    println!("  Vendor wallet : {vendor_wallet}");
    println!("  Budget cap    : 0.01 SOL");
    println!("  Model         : groq / llama-3.3-70b-versatile");
    println!("============================================================\n");

    // --- Pre-transfer balances ---
    println!("── Pre-transfer balances ──");
    let pre_funder = get_balance(funder_wallet).await;
    let pre_vendor = get_balance(vendor_wallet).await;
    println!("  Funder : {pre_funder}");
    println!("  Vendor : {pre_vendor}");
    println!();

    // --- Execute tool ---
    println!("── Executing MarketingCopilotTool… ──\n");
    let result = tool.execute(input).await?;

    if result.success {
        println!("── Tool Output (success) ──\n{}\n", result.output.as_str());
    } else {
        eprintln!("── Tool Output (error) ──");
        if let Some(ref err) = result.error {
            eprintln!("  Error: {err}");
        }
        let text = result.output.as_str();
        if !text.is_empty() {
            println!("  Partial output:\n{text}");
        }
    }

    // --- Post-transfer balances ---
    println!("── Post-transfer balances ──");
    let post_funder = get_balance(funder_wallet).await;
    let post_vendor = get_balance(vendor_wallet).await;
    println!("  Funder : {post_funder}");
    println!("  Vendor : {post_vendor}");
    println!();

    // --- Explorer link ---
    if let Some(sig) = result
        .output
        .data()
        .and_then(|d| d.get("tx_signature"))
        .and_then(|v| v.as_str())
    {
        println!("── Solana Explorer ──");
        println!("  https://explorer.solana.com/tx/{sig}?cluster=devnet");
    }

    println!("\nDone.");
    Ok(())
}
