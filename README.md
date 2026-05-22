# Rusty Bots

A cross-platform desktop app for creating, managing, and running Discord bots with AI persona support powered by xAI's Grok models.

- **Backend:** Rust (Tauri 2.x, Serenity for Discord, Tokio)
- **Frontend:** Tauri webview (vanilla HTML/CSS/JS — no node build needed)
- **AI:** xAI Grok via chat-completions API with tool calling
- **Auto-build:** GitHub Actions for macOS, Windows, Linux

## Features

- Add/edit/remove multiple Discord bots, each with their own token
- Bots come online the moment you connect a token
- Per-bot dashboard: list of guilds it's in, members, recent activity
- Moderation actions from the dashboard: kick, ban, timeout, add/remove role
- AI persona: when the bot is @mentioned, the last 10 messages of the channel are sent to Grok along with the persona prompt. Grok can call tools to reply, react, moderate, and manage roles.
- Configurable model per bot (`grok-4.3`, `grok-4.20-reasoning`, `grok-4.20-non-reasoning`, or any model id you type).

## Dev

```
cargo tauri dev
```

## Build

Pushing a tag like `v0.1.0` triggers the GitHub Actions workflow to build installers for all three platforms.
