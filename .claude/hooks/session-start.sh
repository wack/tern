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

# Install cargo-make if not already installed
if ! command -v cargo-make &> /dev/null; then
  echo "Installing cargo-make..."
  cargo install cargo-make
else
  echo "cargo-make already installed"
fi

# Install cargo-nextest if not already installed
if ! command -v cargo-nextest &> /dev/null; then
  echo "Installing cargo-nextest..."
  cargo install cargo-nextest
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
