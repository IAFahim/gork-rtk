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

if [ ! -f "$ENV_FILE" ]; then
  cat > "$ENV_FILE" <<EOF
# Native Gork phone remote — chmod 600 this file
TELEGRAM_BOT_TOKEN=REPLACE_ME
ALLOWED_USER_IDS=REPLACE_ME
ORCHESTRATOR_HOST_ID=$(hostname -s 2>/dev/null || echo this-pc)
GORK_TELEGRAM_CWD=$CWD
# Optional: GORK_BIN=$BIN_DIR/gork
# Optional: GORK_TELEGRAM_SESSION=
EOF
  chmod 600 "$ENV_FILE"
  echo "Wrote $ENV_FILE — edit token + user id, then re-run this script."
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

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
echo "Unit installed: gork-telegram.service"
echo ""
echo "Next:"
echo "  1) Edit $ENV_FILE"
echo "  2) systemctl --user enable --now gork-telegram.service"
echo "  3) journalctl --user -u gork-telegram -f"
echo "  4) Message your bot from an allowlisted account"
echo ""
echo "Binary: $BIN_DIR/gork-telegram"
echo "Agent: set GORK_BIN to your privacy gork binary (install.sh) if not on PATH"
