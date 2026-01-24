#!/bin/bash
set -euo pipefail

# SessionStart hook for Tern
# Installs CLI tools needed for development: cargo-make, cargo-nextest, shellcheck

# Only run in remote context (Claude Code for the Web)
# Skip installation on local machines where tools are typically already installed
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

echo "Installing development tools for Tern..."

# Add wasm32-wasip2 target for WebAssembly builds
if ! rustup target list --installed | grep -q wasm32-wasip2; then
  echo "Adding wasm32-wasip2 target..."
  rustup target add wasm32-wasip2
else
  echo "wasm32-wasip2 target already installed"
fi

# Install cargo-binstall if not already installed (for fast binary downloads)
if ! command -v cargo-binstall &> /dev/null; then
  echo "Installing cargo-binstall..."
  curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash
fi

# Install cargo-make if not already installed
if ! command -v cargo-make &> /dev/null; then
  echo "Installing cargo-make..."
  cargo binstall -y cargo-make
else
  echo "cargo-make already installed"
fi

# Install cargo-nextest if not already installed
if ! command -v cargo-nextest &> /dev/null; then
  echo "Installing cargo-nextest..."
  cargo binstall -y cargo-nextest
else
  echo "cargo-nextest already installed"
fi

# Install shellcheck if not already installed
if ! command -v shellcheck &> /dev/null; then
  echo "Installing shellcheck..."
  apt-get update && apt-get install -y shellcheck
else
  echo "shellcheck already installed"
fi

# Ensure cargo bin directory is in PATH for this session
if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> "$CLAUDE_ENV_FILE"
fi

echo "Development tools installation complete!"
