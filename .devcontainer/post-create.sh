#!/usr/bin/env bash
# Devcontainer bootstrap (#14): install the migration CLIs that aren't dev-container
# features, then warm the build so clone-to-build works out of the box. Versions
# are pinned in .tool-versions.
set -euo pipefail

# Pull pinned versions from .tool-versions (KEY VALUE per line).
ver() { awk -v k="$1" '$1==k {print $2}' .tool-versions; }

GH_AI_VERSION="$(ver gh-actions-importer)"
ADO2GH_VERSION="$(ver ado2gh)"

echo "==> Installing gh actions-importer extension (${GH_AI_VERSION})"
gh extension install github/gh-actions-importer --pin "v${GH_AI_VERSION}" || \
  gh extension install github/gh-actions-importer || true

echo "==> Pre-pulling the actions-importer Docker image (${GH_AI_VERSION})"
docker pull "ghcr.io/actions/gh-actions-importer:${GH_AI_VERSION}" || true

echo "==> ado2gh (${ADO2GH_VERSION}) — install on demand with:"
echo "    gh extension install github/gh-gei"

echo "==> Warming the workspace build"
cargo fetch
( cd portal && npm ci )

echo "Devcontainer ready. 'cargo build' and 'cd portal && npm run dev' should work."
