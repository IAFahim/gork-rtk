# Phone remote for Gork: what went wrong & the native path

## What you asked for

Claude Code–style **mobile control**:

- Drive the agent from Telegram (or any chat surface)
- See chat / tool progress / reply text on the phone
- Notifications when input is needed (permission, question, plan) or when a turn finishes
- Full power of the real agent (tools, RTK, hooks, sessions) — not a dumbed-down wrapper

## What `IAFahim/grok-telegram-bridge` actually is

It is **not** a totally wrong idea. The good part:

| Good | Why |
|------|-----|
| Uses **`grok agent stdio` + ACP** | Official integration path (same as Zed/IDEs) |
| Permissions / questions / plan as buttons | Maps ACP reverse-requests to Telegram keyboards |
| Notes queue offline | Phone can park work without agent running |
| Fail-closed allowlist | Required for a bot with host FS access |

The half-assed / painful parts (why it feels “stupid”):

| Problem | Effect |
|---------|--------|
| **Python subprocess wrapper** around the binary | Second process, threading, deadlocks (stderr was `DEVNULL` to avoid pipe fill), incomplete ACP surface |
| **Separate agent process ≠ open TUI** | README admits it: live is *not* the desktop Grok window you already have open |
| **Dual mode (notes vs live)** | Product complexity; easy to get “message parked, agent not running” confusion |
| **Session create via `grok -p`** | Odd path; catalog/skeleton fallbacks; not “attach to my current chat” |
| **Incomplete protocol** | ACP has rich `x.ai/*` methods; bridge only implements a subset → missing power |
| **Not in-tree with Gork** | Drift on every Gork upgrade; no shared types, no compile-time ACP contract |
| **~9k LOC Python** | Fighting the agent from outside instead of being a first-class client |

So: **ACP was the right seam**. **Python wrapping the CLI forever was the wrong product shape.**

## Native architecture (what we want)

```
┌─────────────────┐     Bot API      ┌──────────────────────────────┐
│  Telegram app   │ ◄──────────────► │  gork telegram  (Rust)       │
│  (phone)        │   long-poll /    │  - allowlist auth            │
└─────────────────┘   webhooks       │  - message ↔ ACP session     │
                                     │  - buttons ↔ permissions     │
                                     └──────────────┬───────────────┘
                                                    │ in-process or
                                                    │ same-host ACP
                                     ┌──────────────▼───────────────┐
                                     │  Agent runtime (existing)    │
                                     │  tools · RTK hooks · privacy │
                                     │  sessions under ~/.grok      │
                                     └──────────────────────────────┘
```

### Preferred designs (best → ok)

1. **`gork telegram` subcommand (in-process ACP client)**  
   - Same binary as agent; shares session store, auth, config  
   - Long-poll Telegram; drive agent via internal ACP / shell session APIs  
   - Full power: every tool, hook, RTK rewrite already live in the agent process  

2. **`gork agent serve` + thin Telegram client**  
   - Agent already has WebSocket ACP server mode  
   - Telegram process only translates Bot API ↔ JSON-RPC  
   - Can be Rust crate next to gork-rtk; still “native”, not PTY scraping  

3. **Hooks only (notifications, not full control)**  
   - `Stop` / `Notification` / `PermissionDenied` → HTTP to a bot  
   - Great for “ping me when done / needs input”  
   - **Not** enough for full remote control by itself  

**Do not**: scrape TUI, inject keystrokes, or wrap `script(1)` / tmux as the control plane.

## Mapping features → native primitives

| Phone UX | Gork native primitive |
|----------|------------------------|
| Send message | ACP `session/prompt` |
| Stream reply / tools | `session/update` (`agent_message_chunk`, `tool_call`, …) |
| Approve tools | `session/request_permission` reverse-request → Telegram buttons |
| Multi-choice / plan | `x.ai/ask_user_question`, `x.ai/exit_plan_mode` |
| Turn finished | `Stop` hook or ACP turn complete |
| Browse sessions | `~/.grok/sessions` + `session/load` |
| Attach to work | ACP load/resume session id (not a second headless spawn if avoidable) |
| Always-on PC | systemd user unit running `gork telegram` |

## Immediate practical path

1. **Ship installable gork-rtk** (this repo’s releases) on every PC that should be remote-controlled.  
2. **Keep the Python bridge temporarily** but point `binary = "gork"` at the privacy+RTK build; fix only critical bugs.  
3. **Implement `gork telegram`** (or sibling `gork-tg` crate in this monorepo) as the real product; port UX ideas from the Python bridge, not its process model.  
4. Optional: Telegram webhook + `gork agent serve` for multi-device without multiple bots if we later design host routing carefully.

## Security (non-negotiable)

- Allowlist Telegram user ids (fail closed)  
- One bot token per host unless we design multi-host auth  
- Never auto-approve tools from Telegram without explicit opt-in  
- Path open only under known roots  
- Privacy build already blocks research upload; model API still sees task context  

## Status

- Bridge repo: https://github.com/IAFahim/grok-telegram-bridge (Python, ACP-over-stdio, transitional)  
- Binary: https://github.com/IAFahim/gork-rtk (privacy + RTK)  
- **Native phone remote (shipped in-tree):** crate `xai-gork-telegram` → binary **`gork-telegram`**

### Run native remote (complete phone control)

```bash
# Build
cargo build -p xai-gork-telegram --release

# Env (fail-closed without allowlist)
export TELEGRAM_BOT_TOKEN=…
export ALLOWED_USER_IDS=123456789
export GORK_TELEGRAM_CWD=/path/to/project
export GORK_BIN=$HOME/.local/bin/gork   # privacy+RTK agent from install.sh
export ORCHESTRATOR_HOST_ID=$(hostname -s)

./target/release/gork-telegram
# or always-on:
bash scripts/install-telegram.sh
systemctl --user enable --now gork-telegram.service
```

**Phone loop (full):**

| Action | Command / UI |
|--------|----------------|
| Browse chats | `/sessions` or **Sessions** button |
| Use a chat | Tap **Use** under a session |
| New chat | `/new` or **New** |
| Offline notes | Type while live **off** → parked |
| Go live | `/live` — real `gork agent stdio` |
| Free-text live | Type → agent reply body in Telegram |
| Tool / plan holds | Inline buttons |
| Photo / file | Saved under data dir; live → path prompt |
| Drain notes | `/drain` or **Drain→live** |
| History peek | `/history` |
| Stop live | `/stop` |

**Agent process:** `gork agent stdio` (official ACP path). Tools, hooks, RTK apply.  
**Not:** sharing an already-open TUI window (separate process by design).
