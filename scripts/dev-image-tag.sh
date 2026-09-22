#!/usr/bin/env bash
# Prints the GHCR tag for a branch's dev image: "dev-" plus the branch name
# with every character Docker tags reject replaced by "-", capped at the
# 128-character tag limit.
set -euo pipefail

if [[ $# -ne 1 || -z "$1" ]]; then
  echo "usage: $0 <branch>" >&2
  exit 2
fi

sanitized="$(printf '%s' "$1" | tr -c 'A-Za-z0-9._-' '-')"
printf 'dev-%s\n' "${sanitized:0:124}"
