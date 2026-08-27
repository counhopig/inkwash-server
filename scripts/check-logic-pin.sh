#!/usr/bin/env bash
# Verifies the inkwash-logic git pin in Cargo.toml is not behind the
# firmware repo's HEAD for logic/. Run manually whenever the sync contract
# may have changed (e.g. after a firmware-repo sync/logic commit):
#
#   scripts/check-logic-pin.sh [remote]
#
# Exit 0: pinned rev is current for logic/, or the remote is unreachable
#         (offline - treated as unknown, not failure).
# Exit 1: logic/ has commits after the pinned rev - review them and bump
#         `rev` in Cargo.toml (`cargo update -p inkwash-logic && cargo test`
#         after), or confirm the new commits don't touch the wire contract.
#
# The wire-contract fixture tests in src/models.rs are the CI-enforced half
# of this guardrail; this script is the "is the pin stale" half, kept
# manual because the pin is deliberately bumped only when this repo wants
# an inkwash-logic update, not on every firmware commit.
set -euo pipefail

cd "$(dirname "$0")/.."

PIN="$(grep -oE 'rev = "[0-9a-f]{40}"' Cargo.toml | head -1 | cut -d'"' -f2)"
if [[ -z "$PIN" ]]; then
  echo "error: no 40-hex rev pin found in Cargo.toml" >&2
  exit 1
fi

REMOTE="${1:-https://github.com/counhopig/inkwash-firmware}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if ! git clone --quiet --filter=blob:none --no-checkout "$REMOTE" "$TMP/repo" 2>/dev/null; then
  echo "note: could not reach $REMOTE (offline?); pin check skipped"
  exit 0
fi

if ! git -C "$TMP/repo" rev-parse --verify --quiet "$PIN^{commit}" >/dev/null; then
  echo "error: pinned rev $PIN not found in $REMOTE history" >&2
  exit 1
fi

AHEAD="$(git -C "$TMP/repo" rev-list --count "$PIN"..HEAD -- logic/)"
if [[ "$AHEAD" -gt 0 ]]; then
  echo "warning: inkwash-logic is $AHEAD commit(s) ahead of the pinned rev $PIN (logic/)."
  echo "  Review the changes and bump \`rev\` in Cargo.toml if the wire contract moved:"
  echo "    git log --oneline $PIN..HEAD -- logic/   # in a firmware-repo checkout"
  echo "  After bumping: cargo update -p inkwash-logic && cargo test"
  exit 1
fi

echo "ok: pinned rev $PIN is current for logic/"
