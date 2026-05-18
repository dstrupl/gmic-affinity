#!/usr/bin/env bash
# scripts/phase0-quarantine-test.sh
#
# Automates the shell-only portion of Phase 0 step 3 from
# docs/design/2026-05-18-release-v0.1-distribution.md §3:
#
#   1. Take the currently-installed GmicFilter.plugin from the
#      Affinity Photo 2 folder (or wherever the caller points us).
#   2. Stamp it with a com.apple.quarantine xattr that mimics what
#      `brew install --cask` lays down on a downloaded artifact.
#   3. Reinstall it into every Affinity plugin folder that exists on
#      this machine.
#
# WHAT THIS SCRIPT CAN DO:        stamp + reinstall the bundle.
# WHAT THIS SCRIPT CANNOT DO:     verify the bundle still loads inside
#                                  Affinity Photo 2 / v3. That last step
#                                  requires a human in front of the
#                                  Affinity GUI, opening an 8-bit RGB
#                                  document and exercising
#                                  Filters → Plugins → G'MIC → G'MIC….
#                                  Record the result in the design
#                                  doc's Phase 0 tracker (§11).
#
# Usage:
#   scripts/phase0-quarantine-test.sh [path/to/source/GmicFilter.plugin]
#
# If no argument is given, the script picks the bundle from the first
# Affinity-2 install dir that has one.

set -euo pipefail

PLUGIN="GmicFilter.plugin"

DEFAULT_TARGETS=(
  "$HOME/Library/Application Support/Affinity Photo 2/Plugins"
  "$HOME/Library/Application Support/Affinity/Plugins"
)

SRC="${1:-}"
if [[ -z "$SRC" ]]; then
  for dir in "${DEFAULT_TARGETS[@]}"; do
    if [[ -d "$dir/$PLUGIN" ]]; then
      SRC="$dir/$PLUGIN"
      break
    fi
  done
fi

if [[ -z "$SRC" || ! -d "$SRC" ]]; then
  echo "ERROR: no source $PLUGIN found." >&2
  echo "Pass an explicit path or run 'make universal-install' first." >&2
  exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
echo "Staging: $TMP"

cp -R "$SRC" "$TMP/"

# Mimic the quarantine bit Safari / brew lay down on a downloaded zip.
# Format: flags;hex-timestamp;agent-name;|agent-bundle-id
QUARANTINE_HEX_TS=$(printf '%x' "$(date +%s)")
QUARANTINE_VALUE="0081;${QUARANTINE_HEX_TS};gmic-affinity-phase0;|com.test.gmic-affinity"

xattr -w com.apple.quarantine "$QUARANTINE_VALUE" "$TMP/$PLUGIN"
echo "Set com.apple.quarantine on staged copy:"
echo "  $(xattr -p com.apple.quarantine "$TMP/$PLUGIN")"

INSTALLED=0
for dir in "${DEFAULT_TARGETS[@]}"; do
  parent="$(dirname "$dir")"
  if [[ -d "$parent" ]]; then
    mkdir -p "$dir"
    rm -rf "$dir/$PLUGIN"
    cp -R "$TMP/$PLUGIN" "$dir/"
    echo "Reinstalled (quarantined) into: $dir/$PLUGIN"
    INSTALLED=$((INSTALLED + 1))
  fi
done

if [[ "$INSTALLED" -eq 0 ]]; then
  echo "WARNING: no Affinity install detected; nothing to overwrite." >&2
  exit 2
fi

echo ""
echo "Done. Now (HUMAN STEP):"
echo "  1. Restart Affinity Photo 2.       Verify Filters → Plugins → G'MIC loads."
echo "  2. Restart Affinity Photo v3.      Verify the same."
echo "  3. Record the result in"
echo "     docs/design/2026-05-18-release-v0.1-distribution.md §11"
echo "     under 'Step 3 — Quarantine tolerance'."
echo ""
echo "After testing, restore the un-quarantined bundle with:"
echo "    make universal-install FEATURES=live"
