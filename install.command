#!/usr/bin/env bash
# install.command — double-clickable installer for the manual (zip) install
# path. Lives at the repo root, gets copied verbatim into the release zip
# by `make release`, and is also the runbook for what `brew install --cask
# gmic-affinity` does underneath. Keep behaviour aligned with §4.3 of
# docs/design/2026-05-18-release-v0.1-distribution.md.
#
# Idempotent: running it again replaces the already-installed bundle.

set -euo pipefail

cd "$(dirname "$0")"

PLUGIN="GmicFilter.plugin"
if [[ ! -d "$PLUGIN" ]]; then
  echo "ERROR: $PLUGIN not found next to install.command." >&2
  echo "       Run this script from inside the unzipped release folder." >&2
  exit 1
fi

# Strip Gatekeeper quarantine if it's been set (it will be on a zip the
# user downloaded via Safari/Chrome). xattr -dr is idempotent: if the
# attribute isn't there it just returns non-zero, which we swallow.
xattr -dr com.apple.quarantine "$PLUGIN" 2>/dev/null || true

# Affinity plugin folders we know about. Add new ones here as Affinity
# ships new versions; the script auto-skips any whose parent dir is
# absent (proxy for "this Affinity version is not installed").
TARGETS=(
  "$HOME/Library/Application Support/Affinity Photo 2/Plugins"
  "$HOME/Library/Application Support/Affinity/Plugins"
)

INSTALLED=0
for dir in "${TARGETS[@]}"; do
  parent="$(dirname "$dir")"
  if [[ -d "$parent" ]]; then
    mkdir -p "$dir"
    rm -rf "$dir/$PLUGIN"
    cp -R "$PLUGIN" "$dir/"
    echo "Installed: $dir/$PLUGIN"
    INSTALLED=$((INSTALLED + 1))
  fi
done

if [[ "$INSTALLED" -eq 0 ]]; then
  echo "" >&2
  echo "WARNING: no Affinity Photo install detected on this machine." >&2
  echo "Tried:" >&2
  for dir in "${TARGETS[@]}"; do
    echo "  - $dir" >&2
  done
  echo "Install Affinity Photo 2 or Affinity Photo v3 first, then re-run this script." >&2
  exit 2
fi

echo ""
echo "Done. Restart Affinity Photo to pick up the new plugin."
echo "If Filters → Plugins → G'MIC does not appear, check:"
echo "  - Affinity → Settings → Photoshop Plugins → 'Allow unknown plugins to be used' (must be ticked)"
echo "  - The 'gmic' binary is installed:  brew install gmic-qt   (or just  brew install gmic)"
echo "  - Logs: ~/Library/Logs/gmic-affinity.log"
