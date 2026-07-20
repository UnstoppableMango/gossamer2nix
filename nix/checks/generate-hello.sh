#!/usr/bin/env bash
# Regenerates the nix/checks/hello fixture from scratch via `gos new`.
# Run inside `nix develop` (or with `gos` on PATH).
set -euo pipefail

target="${1:-"$(git rev-parse --show-toplevel)/hello"}"
rm -rf "$target"
gos new example.com/hello --path "$target"
