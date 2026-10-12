#!/bin/bash
# Prepares a Claude Code cloud session (Linux) so lint, tests and the Windows cross-check work.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

# fontconfig for Slint's font lookup on Linux, plus llvm-rc, which the Windows resource step
# needs when cross-checking.
APT_PACKAGES=(libfontconfig-dev pkg-config llvm build-essential)
missing=()
for p in "${APT_PACKAGES[@]}"; do
  dpkg -s "$p" >/dev/null 2>&1 || missing+=("$p")
done
if [ ${#missing[@]} -gt 0 ]; then
  SUDO=""
  [ "$(id -u)" -ne 0 ] && SUDO="sudo"
  $SUDO apt-get update -qq
  DEBIAN_FRONTEND=noninteractive $SUDO apt-get install -y -qq "${missing[@]}"
fi

# Rust: components for lint, and the MSVC target for `cargo check-win`.
rustup component add clippy rustfmt >/dev/null
rustup target add x86_64-pc-windows-msvc >/dev/null

# Download crates for both targets so the first build does not wait on the network.
cargo fetch
