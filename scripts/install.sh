#!/usr/bin/env bash
# Install Gork Build (privacy hard-off) + RTK rewrite support from GitHub releases.
set -euo pipefail

REPO="${GORK_RTK_REPO:-IAFahim/gork-rtk}"
INSTALL_DIR="${GORK_INSTALL_DIR:-$HOME/.local/bin}"
GROK_HOME="${GROK_HOME:-$HOME/.grok}"
TAG="${GORK_RTK_TAG:-latest}"
ARCH="$(uname -m)"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"

case "${OS}-${ARCH}" in
  linux-x86_64|linux-amd64) ASSET="gork-linux-x86_64.tar.gz"; BIN_NAME="gork-linux-x86_64" ;;
  *)
    echo "Unsupported platform: ${OS}-${ARCH}" >&2
    echo "Build from source: https://github.com/${REPO}" >&2
    exit 1
    ;;
esac

if ! command -v curl >/dev/null; then
  echo "curl is required" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if [ "$TAG" = "latest" ]; then
  URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
else
  URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET}"
fi

echo "Downloading ${URL}"
curl -fsSL "$URL" -o "$TMP/$ASSET"
tar -xzf "$TMP/$ASSET" -C "$TMP"
mkdir -p "$INSTALL_DIR" "$GROK_HOME/bin" "$GROK_HOME/downloads" "$GROK_HOME/hooks"

install -m 755 "$TMP/$BIN_NAME" "$GROK_HOME/downloads/gork-rtk-local"
ln -sfn "$GROK_HOME/downloads/gork-rtk-local" "$GROK_HOME/bin/gork"
ln -sfn "$GROK_HOME/downloads/gork-rtk-local" "$GROK_HOME/bin/grok"
ln -sfn "$GROK_HOME/bin/gork" "$INSTALL_DIR/gork"
ln -sfn "$GROK_HOME/bin/grok" "$INSTALL_DIR/grok"

# RTK hook (no-op rewrite if rtk not installed)
if [ ! -f "$GROK_HOME/hooks/rtk-rewrite.json" ]; then
  cat > "$GROK_HOME/hooks/rtk-rewrite.json" <<'EOF'
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
  echo "Installed RTK PreToolUse hook → $GROK_HOME/hooks/rtk-rewrite.json"
  echo "Install RTK separately: https://github.com/rtk-ai/rtk"
fi

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo "Add to PATH: export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac

echo "Installed:"
"$INSTALL_DIR/gork" --version || true
echo "Commands: gork | grok  (same privacy+RTK binary)"
