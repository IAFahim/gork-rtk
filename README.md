<div align="center">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/gork-build-symbol-white.png">
    <source media="(prefers-color-scheme: light)" srcset="docs/assets/gork-build-symbol-black.png">
    <img alt="Gork Build logo" src="docs/assets/gork-build-symbol-black.png" width="96">
  </picture>
  <br>
  Gork Build (<code>gork</code>)
</h1>

**Gork Build: the VSCodium-style community build of Grok Build — [research / product analytics hard-off](https://gist.github.com/cereblab/dc9a40bc26120f4540e4e09b75ffb547)**

An independent, community-maintained distribution of
[SpaceXAI Grok Build](https://github.com/xai-org/grok-build) with vendor
telemetry hard-off and a community rebrand (compatibility identifiers such as
`~/.grok`, `GROK_*`, and API hosts are retained).

**Privacy note:** agent-selected model context (prompts, tool results, file
contents the agent reads) still goes to the model API — that is how cloud
coding works. Research uploads and product analytics are hard-off separately;
they are not the same channel.

[Building from source](#build-from-source) ·
[Phone remote setup](#phone-remote-telegram--setup) ·
[Privacy](#privacy-guarantees-client) ·
[Documentation](#documentation) ·
[Contributing](#contributing) ·
[License](#license)

![Gork Build TUI](docs/assets/gork-build-tui-screenshot.jpg)

**Gork Build is to [Grok Build](https://github.com/xai-org/grok-build) what
[VSCodium](https://github.com/VSCodium/vscodium) is to VS Code.**

This repository contains the Rust source for the `gork` CLI/TUI and agent
runtime, forked from [`xai-org/grok-build`](https://github.com/xai-org/grok-build)
with research telemetry hard-off and a community rebrand (paths, env prefixes,
and API hosts kept for compatibility).

</div>

---

Comparison at a glance:

| | Grok Build (upstream) | **Gork Build** (this fork) |
|--|----------------------|---------------------------|
| License | Apache-2.0 | Apache-2.0 (same code) |
| Agent / tools / TUI | Full | Full |
| Model inference | Yes (Grok API) | Yes (your credentials) |
| Mixpanel / product events | On by default in releases | **Hard-off** |
| GCS research / session traces | Upload pipeline present | **Hard-off** |
| Whole-repo research packaging | Present upstream | **Disabled** |
| Vendor auto-update | Yes (`x.ai/cli`) | **Hard-disabled** (rebuild / community releases) |
| Coding-data retention | Opt-in available | **Opt-out only (locked)** |
| PreToolUse tool-input rewrite | Allow / deny only | **+ Claude/RTK `updatedInput`** (this branch) |

---

## RTK (this branch)

This branch adds Claude Code–compatible **PreToolUse `updatedInput`** so
[RTK](https://github.com/rtk-ai/rtk) can transparently rewrite shell commands
(e.g. `git status` → `rtk git status`) for 60–90% fewer tokens on common
dev tooling. Privacy hard-offs from mainline Gork Build are unchanged.

Wire the hook (once):

```sh
mkdir -p ~/.grok/hooks
cat > ~/.grok/hooks/rtk-rewrite.json <<'EOF'
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|run_terminal_command|Shell",
        "hooks": [
          { "type": "command", "command": "rtk hook claude", "timeout": 5 }
        ]
      }
    ]
  }
}
EOF
```

Requires `rtk` on `PATH`. See [`docs/RTK.md`](docs/RTK.md).

---

## Why this exists

Independent [wire analysis of Grok Build 0.2.93](https://gist.github.com/cereblab/dc9a40bc26120f4540e4e09b75ffb547)
showed that research upload paths (session traces, and historically whole-repo
snapshots) could leave the machine even when “Improve the model” was off —
including secrets in files the agent read. Upstream open-sourced the harness;
**Gork Build** re-ships that code with **privacy by construction**:

- No product analytics (Mixpanel / `events` telemetry)
- No client-side research / trace / session-state uploads to GCS
- Remote feature flags **cannot** re-enable those paths
- Coding-data retention is **opt-out only** (no opt-in path)
- Vendor auto-update is **hard-disabled**: Gork Build never installs from
  x.ai update channels (`x.ai/cli/install.*`); that path would replace this
  fork with official Grok Build. Update by rebuilding from source or installing
  community releases from **this** project.

**What still leaves the machine:** whatever the agent must send to the Grok
**model API** to work (prompts + tool results for files it actually reads).
That is required for a cloud coding agent and is separate from the research /
product-analytics hard-offs. Gork Build does not add extra research packaging
on top.

## Install (Linux x86_64 release)

Privacy + RTK prebuilt binary:

```sh
curl -fsSL https://raw.githubusercontent.com/IAFahim/gork-rtk/main/scripts/install.sh | bash
```

Releases: https://github.com/IAFahim/gork-rtk/releases

## Phone remote (Telegram) — setup

Control Gork Build from your phone: **sessions, notes, media, live agent, permissions/plans as buttons**.

Architecture notes: [`docs/TELEGRAM-NATIVE.md`](docs/TELEGRAM-NATIVE.md).

### Prerequisites

1. **This PC** with Rust toolchain (`rustup`) if building from source.
2. **Gork agent binary** on the machine (privacy build):
   ```sh
   curl -fsSL https://raw.githubusercontent.com/IAFahim/gork-rtk/main/scripts/install.sh | bash
   # → ~/.local/bin/gork  (and/or ~/.grok/bin/gork)
   gork --version
   gork login    # or first TUI launch — must be signed in for model API
   ```
3. **Telegram bot** (one bot **per PC** — do not share the token with another machine or with the old Python bridge at the same time):
   - Open [@BotFather](https://t.me/BotFather) → `/newbot` → copy the **token**
4. **Your Telegram user id** (numeric), e.g. from [@userinfobot](https://t.me/userinfobot)

### One-shot install (recommended)

```sh
git clone https://github.com/IAFahim/gork-rtk.git
cd gork-rtk

# Build gork-telegram, install to ~/.local/bin, write systemd unit + env scaffold
bash scripts/install-telegram.sh
```

Edit secrets (created on first run, mode `600`):

```sh
nano ~/.grok/telegram.env
```

```bash
TELEGRAM_BOT_TOKEN=123456:ABC-DEF…     # from BotFather
ALLOWED_USER_IDS=987654321             # your numeric id (comma-separated ok)
ORCHESTRATOR_HOST_ID=lab-pc            # short label on /start
GORK_TELEGRAM_CWD=/home/YOU/Github/my-project
GORK_BIN=/home/YOU/.local/bin/gork     # privacy agent from install.sh
# Optional:
# GORK_TELEGRAM_SESSION=               # load this session id on start
# GROK_SESSIONS_ROOT=/home/YOU/.grok/sessions
# GORK_TELEGRAM_DATA=/home/YOU/.grok/telegram-native
```

Start and enable always-on:

```sh
systemctl --user daemon-reload
systemctl --user enable --now gork-telegram.service
systemctl --user status gork-telegram.service
journalctl --user -u gork-telegram -f
# optional (keep running after logout):
# sudo loginctl enable-linger "$USER"
```

### Manual run (no systemd)

```sh
cd gork-rtk
cargo build -p xai-gork-telegram --release

export TELEGRAM_BOT_TOKEN=…
export ALLOWED_USER_IDS=…
export GORK_BIN=$HOME/.local/bin/gork
export GORK_TELEGRAM_CWD=$HOME/Github/my-project
export ORCHESTRATOR_HOST_ID=$(hostname -s)

./target/release/gork-telegram
# or after install-telegram.sh:  gork-telegram
```

### Env reference

| Variable | Required | Meaning |
|----------|----------|---------|
| `TELEGRAM_BOT_TOKEN` | **yes** | BotFather token (this PC only) |
| `ALLOWED_USER_IDS` | **yes** | Comma-separated Telegram user ids; **empty = deny all** |
| `GORK_BIN` | recommended | Path to `gork` agent (`agent stdio`) |
| `GORK_TELEGRAM_CWD` | recommended | Default project directory for new/live sessions |
| `ORCHESTRATOR_HOST_ID` | optional | Label shown on `/start` |
| `GORK_TELEGRAM_SESSION` | optional | Session id to load on startup |
| `GROK_SESSIONS_ROOT` | optional | Default `~/.grok/sessions` |
| `GORK_TELEGRAM_DATA` | optional | Notes + inbound media; default `~/.grok/telegram-native` |

### Phone daily loop

1. Message the bot → `/start` or `/home`
2. **Sessions** → tap **Use** on a chat (or **New**)
3. **Go live** (`/live`) when you want the real agent
4. Type freely → reply body returns in Telegram  
   (live **off** → text parks in **Notes**)
5. Photo/file → saved on PC; if live, path is prompted to the agent
6. **Drain→live** / `/drain` sends the next parked note into live
7. Tool / multi-option / plan holds appear as **buttons**
8. `/stop` when done

### Stop the old Python bridge

If you previously ran [`grok-telegram-bridge`](https://github.com/IAFahim/grok-telegram-bridge), **stop it** before starting native remote — only **one** long-poller may use a bot token:

```sh
systemctl --user stop grok-telegram-bridge.service 2>/dev/null || true
# or kill the python -m grok_telegram_bridge process
```

### Troubleshooting

| Symptom | Check |
|---------|--------|
| Bot ignores you | `ALLOWED_USER_IDS` must include **your** numeric id; bot restarted after edit |
| “spawn agent” / prompt fails | `GORK_BIN` points at a working `gork`; run `gork login` on the PC |
| No reply, only hang | `journalctl --user -u gork-telegram -f`; agent may need network/API auth |
| Two bots fighting | Don’t run Python bridge + `gork-telegram` on the same token |
| Unit inactive after reboot | `loginctl enable-linger $USER` for user systemd |

---

## Build from source

Requirements: Rust (see `rust-toolchain.toml`), `protoc` (see `bin/protoc`).

```sh
cargo run -p xai-grok-pager-bin              # build + launch TUI (binary: gork)
cargo build -p xai-grok-pager-bin --release  # target/release/gork
cargo check -p xai-grok-pager-bin
```

Install the release binary somewhere on your `PATH` as `gork` (and optionally
`grok` if you want the upstream command name).

On first launch, authenticate with your Grok / xAI account the same way
upstream does — model access still goes through the Grok API.

## Privacy guarantees (client)

| Channel | Gork Build behavior |
|---------|-------------------|
| `POST …/v1/responses` (model) | Used for inference only |
| `POST …/v1/storage` research traces | **Never enabled** (`resolve_trace_upload` → false) |
| Mixpanel / product events | **No-op / never constructed** |
| Sentry | Only if you set `SENTRY_DSN` yourself |
| Vendor auto-update (`x.ai/cli/install.*`) | **Hard-disabled** — rebuild from source / community releases |
| `is_data_collection_disabled` | Always **true** in this build |

See [`PRIVACY.md`](PRIVACY.md) for details and residual risks.

## Configuration tips

```toml
# ~/.grok/config.toml — all of these are already the Gork Build defaults
[features]
telemetry = false

[telemetry]
trace_upload = false
mixpanel_enabled = false
```

`[cli] auto_update` cannot re-enable vendor channels: this build never installs
from x.ai (enforced at the install chokepoint). Rebuild from source or use
community releases.

## Documentation

User guide (upstream docs tree, still accurate for features):

[`crates/codegen/xai-grok-pager/docs/user-guide/`](crates/codegen/xai-grok-pager/docs/user-guide/)

## Contributing

External contributions are welcome. See [`CONTRIBUTING.md`](CONTRIBUTING.md)
for setup, commit style, and PR expectations. Security reports: [`SECURITY.md`](SECURITY.md).

## Made with Grok 4.5

<div align="center">

![Gork Build session — Made with Grok 4.5](docs/assets/made-with-grok-4.5.png)

</div>

## Relationship to upstream

This repository is a fork of [`xai-org/grok-build`](https://github.com/xai-org/grok-build).
We intend to pull upstream fixes periodically while keeping the privacy
hard-offs.

**Credit:** original Grok Build is developed and published by SpaceXAI under
Apache-2.0. Gork Build is an independent community distribution and is **not**
affiliated with, endorsed by, or sponsored by SpaceXAI or xAI. Grok, Grok Build,
xAI, and SpaceXAI are trademarks of their respective owners.

## License

Apache License 2.0 — see [`LICENSE`](LICENSE) and attribution in [`NOTICE`](NOTICE).

Upstream copyright (SpaceXAI) is retained as required by Apache-2.0. Community
modifications are copyright the Gork Build contributors.

## Security

Please do **not** open public issues for security reports that include secrets.
See [`SECURITY.md`](SECURITY.md).