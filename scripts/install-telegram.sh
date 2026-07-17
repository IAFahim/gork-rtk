#!/usr/bin/env bash
# Install / enable native gork-telegram phone remote (systemd user unit).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="${GORK_INSTALL_DIR:-$HOME/.local/bin}"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
ENV_FILE="${GORK_TELEGRAM_ENV:-$HOME/.grok/telegram.env}"
CWD="${GORK_TELEGRAM_CWD:-$HOME}"

mkdir -p "$BIN_DIR" "$UNIT_DIR" "$(dirname "$ENV_FILE")"

echo "Building gork-telegram (release)…"
(cd "$ROOT" && cargo build -p xai-gork-telegram --release)

install -m 755 "$ROOT/target/release/gork-telegram" "$BIN_DIR/gork-telegram"
# Convenience: gork-telegram also as gork-tg
ln -sfn "$BIN_DIR/gork-telegram" "$BIN_DIR/gork-tg"

# Prefer already-installed privacy gork as the agent
GORK_CANDIDATE=""
for c in "$BIN_DIR/gork" "$HOME/.grok/bin/gork" "$HOME/.local/bin/gork" "$(command -v gork 2>/dev/null || true)"; do
  if [ -n "$c" ] && [ -x "$c" ]; then
    GORK_CANDIDATE="$c"
    break
  fi
done

if [ ! -f "$ENV_FILE" ]; then
  cat > "$ENV_FILE" <<EOF
# Native Gork phone remote — keep this file chmod 600
# Docs: https://github.com/IAFahim/gork-rtk#phone-remote-telegram--setup

TELEGRAM_BOT_TOKEN=REPLACE_ME
ALLOWED_USER_IDS=REPLACE_ME
ORCHESTRATOR_HOST_ID=$(hostname -s 2>/dev/null || echo this-pc)
GORK_TELEGRAM_CWD=$CWD
GORK_BIN=${GORK_CANDIDATE:-$BIN_DIR/gork}
# Optional:
# GORK_TELEGRAM_SESSION=
# GROK_SESSIONS_ROOT=$HOME/.grok/sessions
# GORK_TELEGRAM_DATA=$HOME/.grok/telegram-native
EOF
  chmod 600 "$ENV_FILE"
  echo "Wrote $ENV_FILE — edit TELEGRAM_BOT_TOKEN + ALLOWED_USER_IDS"
else
  echo "Keeping existing $ENV_FILE"
fi

cat > "$UNIT_DIR/gork-telegram.service" <<EOF
[Unit]
Description=Gork Build native Telegram phone remote
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
EnvironmentFile=$ENV_FILE
WorkingDirectory=$CWD
ExecStart=$BIN_DIR/gork-telegram
Restart=on-failure
RestartSec=3

# Ensure PATH can find gork if GORK_BIN is a bare name
Environment=PATH=$BIN_DIR:/usr/local/bin:/usr/bin

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
echo ""
echo "Installed:"
echo "  binary  $BIN_DIR/gork-telegram"
echo "  env     $ENV_FILE"
echo "  unit    gork-telegram.service"
if [ -n "$GORK_CANDIDATE" ]; then
  echo "  agent   $GORK_CANDIDATE"
else
  echo "  agent   NOT FOUND — run scripts/install.sh or set GORK_BIN in $ENV_FILE"
fi
echo ""
echo "Next steps:"
echo "  1) nano $ENV_FILE   # set TELEGRAM_BOT_TOKEN and ALLOWED_USER_IDS"
echo "  2) Stop any old Python bridge using the same bot token:"
echo "       systemctl --user stop grok-telegram-bridge.service 2>/dev/null || true"
echo "  3) systemctl --user enable --now gork-telegram.service"
echo "  4) journalctl --user -u gork-telegram -f"
echo "  5) Open Telegram → your bot → /start"
echo ""
echo "Optional (survive logout): sudo loginctl enable-linger \"\$USER\""
