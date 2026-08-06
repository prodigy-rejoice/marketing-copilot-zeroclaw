# Marketing Copilot — Zeroclaw Solana Plugin

An AI agent, built on [Zeroclaw](https://github.com/zeroclaw-labs/zeroclaw), that reasons about where your marketing budget should go — and, once you approve it, actually spends it, in a real Solana transaction.

Built for the **Superteam Brasil × Zeroclaw "Build Solana-native plugins for Zeroclaw"** bounty.

## What it does

Tell it your channel performance (signups, spend, conversion rate per channel) over Discord, and it:

1. Reasons about which channel is the best next spend, using an LLM (Groq)
2. Enforces a hard budget cap in code — not just a prompt instruction
3. Asks a human for approval before anything is spent
4. On approval, executes a real Solana transfer via the Solana CLI
5. Reports back the reasoning, the amount, and the transaction signature

## Example real run

```
Input: google_ads (120 signups, $400 spend, 4% conversion), twitter_ads (350 signups, $300 spend, 8% conversion)
Budget cap: 0.01 SOL

Output:
{
  "chosen_channel": "twitter_ads",
  "reasoning": "Twitter ads have a higher conversion rate and more signups compared to Google ads...(capped from $200.00 to budget limit $0.01)",
  "amount": 0.01,
  "status": "success",
  "tx_signature": "5f2QR6HTkKpW7ZVGD98cabwwu5EvFU8Ayn2mhAV1E7aebbK1WUr28MmzQvdSqotifpzKRSE9ywmvhkzvXve9h3ac"
}
```

Verify it yourself: [explorer.solana.com/tx/5f2QR6...devnet](https://explorer.solana.com/tx/5f2QR6HTkKpW7ZVGD98cabwwu5EvFU8Ayn2mhAV1E7aebbK1WUr28MmzQvdSqotifpzKRSE9ywmvhkzvXve9h3ac?cluster=devnet)

## Custody tier: T1 (build, human approves)

The agent never signs or submits a transaction on its own. It proposes; a human approves or declines. No secrets beyond the LLM and wallet keys are held by the model itself.

**Tested:** we tried prompt-injecting the agent to ignore its budget cap and to redirect funds to a different wallet.
- The hard-coded budget cap held — a request to send far more than the cap was rejected/clamped every time, regardless of the message claiming "owner approval."
- The wallet-redirect attempt was **not** blocked in code, but the substituted destination was visible in the human approval prompt before anything sent — a reviewing human would catch it. This is a known limitation, documented here rather than hidden: destination is currently protected by human review, not a code-level allowlist. A destination allowlist enforced the same way as the budget cap is the natural next step.

## Architecture

- **Runtime:** Zeroclaw (Rust), plugin implemented via the `Tool` trait in `crates/zeroclaw-tools/src/marketing_copilot.rs`
- **LLM provider:** Groq (`llama-3.1-8b-instant` / `llama-3.3-70b-versatile`), swappable via Zeroclaw's provider config
- **On-chain execution:** shells out to the Solana CLI (`solana transfer ... --output json`) rather than embedding a raw Solana SDK integration
- **Channel:** Discord, via Zeroclaw's built-in Discord channel support
- **Approval flow:** Zeroclaw's native approval-gate mechanism — the agent's tool call pauses and waits for an explicit human reply before executing

## Reproducing this

1. Clone and build [Zeroclaw](https://github.com/zeroclaw-labs/zeroclaw) from source: `cargo build --release`
2. Copy this plugin's code into `crates/zeroclaw-tools/src/marketing_copilot.rs` and register it per `crates/zeroclaw-tools/src/lib.rs`
3. Set up a Discord bot (Developer Portal → enable Message Content Intent), add its token to config
4. Get a free Groq API key from [console.groq.com](https://console.groq.com/keys)
5. Create a Solana devnet wallet and fund it via a devnet faucet: `solana-keygen new`, `solana config set --url devnet`, request devnet SOL
6. Configure `~/.zeroclaw/config.toml` — set your Groq API key, Discord bot token, and add a risk-profile tool allowlist restricting the agent to `marketing_copilot` (this keeps requests within free-tier LLM rate limits)
7. Run `cargo run -- daemon`, message the bot in Discord, and approve when prompted

## What's next

- Add a destination-wallet allowlist enforced in code, closing the gap found in prompt-injection testing
- Durable-nonce handling for approval flows that take longer than a blockhash's validity window
- Support for real (non-mock) traction data sources

## License

MIT