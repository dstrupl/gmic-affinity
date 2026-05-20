#!/usr/bin/env bash
# scripts/release-bump-cask.sh
#
# Bump the Homebrew cask in the tap repo to a freshly-released
# version. Called as the last step of `make release`.
#
# Steps:
#   1. Compute SHA256 of the release zip.
#   2. Clone TAP_REPO_URL into a tempdir.
#   3. Mutate Casks/gmic-affinity.rb in place (Python, since the
#      v0.2-deferral block strip is multi-line).
#   4. `brew style` to confirm clean.
#   5. Commit + push.
#
# Idempotent across the v0.2-deferral strip: re-running on an
# already-stripped cask is a no-op for that part. The version + sha256
# lines are always overwritten with the current values, which is the
# desired behaviour for bumps.
#
# Usage:
#   ./scripts/release-bump-cask.sh \
#     v0.2.0 \
#     dist/GmicFilter-v0.2.0.zip \
#     git@github.com:dstrupl/homebrew-gmic-affinity.git

set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: $0 RELEASE_VERSION RELEASE_ZIP TAP_REPO_URL" >&2
  exit 64
fi

RELEASE_VERSION=$1
RELEASE_ZIP=$2
TAP_REPO_URL=$3

# Cask `version "..."` is the bare semver without the leading v.
# Cask URL builds the leading v back in via "v#{version}".
CASK_VERSION="${RELEASE_VERSION#v}"

if [ ! -f "$RELEASE_ZIP" ]; then
  echo "ERROR: release zip '$RELEASE_ZIP' not found." >&2
  exit 1
fi

SHA256=$(shasum -a 256 "$RELEASE_ZIP" | awk '{print $1}')
echo "[release-bump-cask] $RELEASE_VERSION  sha256=$SHA256"

WORKDIR=$(mktemp -d -t gmic-affinity-tap.XXXXXX)
# Clean up the tempdir on any exit, including Ctrl-C.
trap 'rm -rf "$WORKDIR"' EXIT

echo "[release-bump-cask] cloning $TAP_REPO_URL into $WORKDIR..."
# --depth 1: we don't care about tap history here, only the latest cask
# state. If we ever do want history (e.g. for `git blame` on the cask),
# bump this to a real clone.
git clone --depth 1 "$TAP_REPO_URL" "$WORKDIR/tap"

CASK="$WORKDIR/tap/Casks/gmic-affinity.rb"
if [ ! -f "$CASK" ]; then
  echo "ERROR: $CASK missing — has the tap repo been initialised from release/homebrew-tap/?" >&2
  echo "       See release/homebrew-tap/PUBLISHING.md for the one-time bootstrap." >&2
  exit 1
fi

# Python is the cleanest way to do a multi-line regex strip. macOS
# ships /usr/bin/python3 by default, no extra deps.
echo "[release-bump-cask] bumping cask to $CASK_VERSION..."
/usr/bin/env python3 - "$CASK" "$CASK_VERSION" "$SHA256" <<'PY'
import re
import sys
from pathlib import Path

cask_path, cask_version, sha256 = sys.argv[1], sys.argv[2], sys.argv[3]
text = Path(cask_path).read_text()

# 1. Strip the v0.2-deferral comment block on the first run after
#    bootstrap. Idempotent: matches once when present, no-op afterwards.
#    We anchor on the unmistakable "NOT YET PUBLISHED" header line and
#    on the trailing sentence to avoid over-eager matches if comment
#    text ever drifts. The trailing blank line is consumed too so the
#    block disappears cleanly.
DEFERRAL_RE = re.compile(
    r"  # ={3,}\n"
    r"  # NOT YET PUBLISHED.*?It is NOT a working cask today\.\n"
    r"\n",
    re.DOTALL,
)
text, n_deferral = DEFERRAL_RE.subn("", text, count=1)
if n_deferral:
    print(f"  - stripped v0.2-deferral comment block (one-time, on first stable bump)")

# 2. Bump version. Quoted form: `version "X.Y.Z"`.
def replace_version(match):
    return f'{match.group(1)}"{cask_version}"'

text, n_version = re.subn(r'(version\s+)"[^"]+"', replace_version, text, count=1)
if n_version != 1:
    sys.exit(f"ERROR: failed to find/replace `version \"...\"` in {cask_path}")
print(f"  - version \"{cask_version}\"")

# 3. Bump sha256.
def replace_sha(match):
    return f'{match.group(1)}"{sha256}"'

text, n_sha = re.subn(r'(sha256\s+)"[^"]+"', replace_sha, text, count=1)
if n_sha != 1:
    sys.exit(f"ERROR: failed to find/replace `sha256 \"...\"` in {cask_path}")
print(f"  - sha256 \"{sha256}\"")

Path(cask_path).write_text(text)
PY

echo "[release-bump-cask] running brew style..."
# `brew style` is what the homebrew-cask reviewers will run. Failing
# here usually means cask DSL drift; fix the cask source in
# release/homebrew-tap/Casks/ before re-running the release.
(cd "$WORKDIR/tap" && brew style "Casks/gmic-affinity.rb")

echo "[release-bump-cask] committing and pushing..."
(
  cd "$WORKDIR/tap"
  git add Casks/gmic-affinity.rb
  if git diff --cached --quiet; then
    echo "[release-bump-cask] cask already at $CASK_VERSION — nothing to commit."
    exit 0
  fi
  git -c "user.email=$(git -C "${OLDPWD:-$(pwd)}" config user.email)" \
      -c "user.name=$(git -C "${OLDPWD:-$(pwd)}" config user.name)" \
      commit -m "Bump gmic-affinity to $CASK_VERSION

sha256: $SHA256
url:    https://github.com/dstrupl/gmic-affinity/releases/download/v$CASK_VERSION/GmicFilter-v$CASK_VERSION.zip
"
  git push origin HEAD
)

echo ""
echo "[release-bump-cask] tap bumped to $CASK_VERSION."
