#!/usr/bin/env bash
# scripts/release-preflight.sh
#
# Pre-release checks for `make release` (the v0.2+ signed + notarised
# pipeline). Bails with a clear error before any side-effecting step
# runs; we'd rather refuse to start than half-publish a release.
#
# Called by Makefile target `release-preflight` with four positional
# arguments. Run standalone for debugging:
#
#   ./scripts/release-preflight.sh \
#     v0.2.0 \
#     "Developer ID Application: Your Name (TEAMID)" \
#     gmic-affinity-notary \
#     git@github.com:dstrupl/homebrew-gmic-affinity.git
#
# Every check is intentionally narrow: each one fails with a single
# concrete error message pointing at the fix, not a generic "preflight
# failed" wall of text. See release/notarisation/SIGNING.md for the
# friend-facing setup instructions and IMPLEMENTATION_NOTES.md §11 for
# the operator-side runbook.

set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "usage: $0 RELEASE_VERSION DEVELOPER_ID_APP_SIGNATURE NOTARYTOOL_KEYCHAIN_PROFILE TAP_REPO_URL" >&2
  exit 64
fi

RELEASE_VERSION=$1
DEVELOPER_ID_APP_SIGNATURE=$2
NOTARYTOOL_KEYCHAIN_PROFILE=$3
TAP_REPO_URL=$4

# Cosmetic helper: fail with a labelled error and exit 1.
die() {
  echo ""
  echo "[release-preflight] ERROR: $*" >&2
  echo ""
  exit 1
}

ok() { echo "[release-preflight] ok: $*"; }

echo "[release-preflight] Running pre-release checks..."
echo ""

# -------- 1. RELEASE_VERSION shape --------
#
# Stable releases through `make release` must be exact semver tags
# (vX.Y.Z). Pre-release tags (vX.Y.Z-rc.N etc.) go through release.yml
# + `make release-unsigned` instead and never reach this script.
# `git describe` defaults like `v0.1.0-2-gabcdef0` or `v0.1.0-dirty`
# are also rejected: a release must be a deliberate, named tag.
if [[ ! "$RELEASE_VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  die "RELEASE_VERSION='$RELEASE_VERSION' is not a stable semver tag (vX.Y.Z).
       Pass it explicitly, e.g.:  make release RELEASE_VERSION=v0.2.0
       Pre-releases (v*-rc*, v*-beta* ...) go through release-unsigned."
fi
ok "RELEASE_VERSION=$RELEASE_VERSION"

# -------- 1b. version metadata matches --------
#
# The zip/cask version comes from RELEASE_VERSION, but the bundle also
# carries Info.plist metadata and the Rust crate has its own package
# version. For real releases, keep those in sync so users and crash
# reports don't see a stale version inside a newer signed artifact.
# v0.0.0 is reserved for the setup dry run documented in SIGNING.md.
if [ "$RELEASE_VERSION" != "v0.0.0" ]; then
  BARE_VERSION=${RELEASE_VERSION#v}
  CARGO_VERSION=$(sed -nE 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' Cargo.toml | head -1)
  PLIST_VERSION=$(plutil -extract CFBundleShortVersionString raw Info.plist 2>/dev/null || true)
  if [ "$CARGO_VERSION" != "$BARE_VERSION" ]; then
    die "Cargo.toml version is '$CARGO_VERSION', expected '$BARE_VERSION' for $RELEASE_VERSION.
       Bump Cargo.toml before releasing."
  fi
  if [ "$PLIST_VERSION" != "$BARE_VERSION" ]; then
    die "Info.plist CFBundleShortVersionString is '$PLIST_VERSION', expected '$BARE_VERSION' for $RELEASE_VERSION.
       Bump Info.plist before releasing."
  fi
  ok "Cargo.toml and Info.plist version metadata match $BARE_VERSION"
else
  ok "version metadata check skipped for setup dry run"
fi

# -------- 2. clean working tree --------
#
# A dirty tree means the binary we're about to sign and ship doesn't
# match any reachable commit, which makes post-mortems much harder.
if [ -n "$(git status --porcelain)" ]; then
  die "working tree is dirty. Commit or stash changes before releasing.
       Run 'git status' to see what's pending."
fi
ok "working tree clean"

# -------- 3. on a real branch tracking origin --------
#
# Detached HEAD or a branch with unpushed commits would mean the GitHub
# release points at a commit nobody else can fetch.
HEAD_BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || echo "")
if [ -z "$HEAD_BRANCH" ]; then
  die "HEAD is detached. Check out main (or your release branch) first."
fi

UPSTREAM=$(git rev-parse --abbrev-ref --symbolic-full-name "@{u}" 2>/dev/null || echo "")
if [ -z "$UPSTREAM" ]; then
  die "branch '$HEAD_BRANCH' has no upstream. 'git push -u origin $HEAD_BRANCH' first."
fi

# Are we ahead of upstream? We need a fetch first to know.
git fetch --quiet origin "$HEAD_BRANCH" || die "git fetch origin $HEAD_BRANCH failed."
LOCAL_HEAD=$(git rev-parse HEAD)
REMOTE_HEAD=$(git rev-parse "$UPSTREAM")
if [ "$LOCAL_HEAD" != "$REMOTE_HEAD" ]; then
  die "local HEAD ($LOCAL_HEAD) != $UPSTREAM ($REMOTE_HEAD).
       Push or rebase before releasing — gh release create will publish at HEAD."
fi
ok "on $HEAD_BRANCH, in sync with $UPSTREAM"

# -------- 4. signing identity available --------
#
# Two checks: the variable is set, AND the certificate actually exists
# in the login keychain. Without the cert, codesign will fail with a
# cryptic "no identity found" message later.
if [ -z "$DEVELOPER_ID_APP_SIGNATURE" ]; then
  die "DEVELOPER_ID_APP_SIGNATURE is empty. Add it to .env.local — see release/notarisation/SIGNING.md."
fi
if ! security find-identity -v -p codesigning 2>/dev/null \
     | grep -F "$DEVELOPER_ID_APP_SIGNATURE" >/dev/null; then
  die "Developer ID identity not found in keychain:
         $DEVELOPER_ID_APP_SIGNATURE
       Run 'security find-identity -v -p codesigning' to see what's installed.
       The certificate must be installed in your login keychain (see SIGNING.md §setup)."
fi
ok "signing identity present in keychain"

# -------- 5. notarytool keychain profile --------
#
# `xcrun notarytool history` is the cheapest call that exercises the
# stored credentials end-to-end (Apple ID + app-specific password +
# Team ID). Anything other than success here means the profile is
# missing or stale.
if [ -z "$NOTARYTOOL_KEYCHAIN_PROFILE" ]; then
  die "NOTARYTOOL_KEYCHAIN_PROFILE is empty."
fi
if ! xcrun notarytool history --keychain-profile "$NOTARYTOOL_KEYCHAIN_PROFILE" >/dev/null 2>&1; then
  die "notarytool keychain profile '$NOTARYTOOL_KEYCHAIN_PROFILE' is missing or invalid.
       Re-run 'xcrun notarytool store-credentials' (see SIGNING.md §setup)."
fi
ok "notarytool keychain profile usable"

# -------- 6. gh authenticated with write scope --------
if ! gh auth status >/dev/null 2>&1; then
  die "gh is not authenticated. Run 'gh auth login' first."
fi
ok "gh authenticated"

# -------- 7. tap repo reachable --------
#
# Extract owner/name from a git URL of either ssh or https form. We
# only need this to call `gh repo view`; the actual clone happens in
# release-bump-cask.sh and uses the URL verbatim.
TAP_OWNER_REPO=$(echo "$TAP_REPO_URL" | sed -E -e 's#.*[:/]([^/]+/[^/]+)\.git$#\1#' -e 's#\.git$##')
if [ -z "$TAP_OWNER_REPO" ] || [ "$TAP_OWNER_REPO" = "$TAP_REPO_URL" ]; then
  die "could not parse owner/repo from TAP_REPO_URL='$TAP_REPO_URL'."
fi
if ! gh repo view "$TAP_OWNER_REPO" >/dev/null 2>&1; then
  die "tap repo '$TAP_OWNER_REPO' is unreachable. Either it doesn't exist yet
       or your gh auth has no read access to it. If the repo exists, check
       that the collaboration invite was accepted."
fi
if ! git ls-remote --exit-code "$TAP_REPO_URL" HEAD >/dev/null 2>&1; then
  die "tap repo URL '$TAP_REPO_URL' is not clone-readable from this machine.
       Check SSH keys, HTTPS credentials, or override TAP_REPO_URL in .env.local."
fi
ok "tap repo $TAP_OWNER_REPO reachable via gh and git"

# -------- 8. release/tag state is safe --------
#
# Re-releasing the same GitHub release would either fail or silently
# replace a published artifact. Both outcomes are bad; require an
# explicit version bump or manual cleanup.
if gh release view "$RELEASE_VERSION" --repo "$(gh repo view --json nameWithOwner -q .nameWithOwner)" >/dev/null 2>&1; then
  die "GitHub release $RELEASE_VERSION already exists.
       Pick a new RELEASE_VERSION (semver bump) or, if you really mean to
       replace it, delete it first via 'gh release delete $RELEASE_VERSION'."
fi

# If the project lead pre-created a signed annotated tag, it must point
# at exactly the commit we just checked against upstream. Otherwise
# `gh release create` would publish release metadata for one commit
# while the local zip was built from another.
LOCAL_TAG_SHA=$(git rev-parse -q --verify "refs/tags/$RELEASE_VERSION^{}" 2>/dev/null || true)
REMOTE_TAG_LINES=$(git ls-remote --tags origin "refs/tags/$RELEASE_VERSION" "refs/tags/$RELEASE_VERSION^{}" 2>/dev/null || true)
REMOTE_TAG_SHA=$(echo "$REMOTE_TAG_LINES" | awk '/\^\{\}$/ {print $1; found=1} END {if (!found) exit 1}' 2>/dev/null || true)
if [ -z "$REMOTE_TAG_SHA" ]; then
  REMOTE_TAG_SHA=$(echo "$REMOTE_TAG_LINES" | awk '{print $1; exit}' 2>/dev/null || true)
fi

if [ -n "$LOCAL_TAG_SHA" ] && [ -z "$REMOTE_TAG_SHA" ]; then
  die "local tag $RELEASE_VERSION exists but is not on origin.
       Push the tag first if it is intentional, or delete the local tag before releasing."
fi
if [ -n "$REMOTE_TAG_SHA" ] && [ "$REMOTE_TAG_SHA" != "$LOCAL_HEAD" ]; then
  die "origin tag $RELEASE_VERSION points at $REMOTE_TAG_SHA, but HEAD is $LOCAL_HEAD.
       Move to the tagged commit or choose a new RELEASE_VERSION."
fi
if [ -n "$LOCAL_TAG_SHA" ] && [ "$LOCAL_TAG_SHA" != "$LOCAL_HEAD" ]; then
  die "local tag $RELEASE_VERSION points at $LOCAL_TAG_SHA, but HEAD is $LOCAL_HEAD.
       Move to the tagged commit or choose a new RELEASE_VERSION."
fi
if [ -n "$REMOTE_TAG_SHA" ]; then
  ok "existing tag $RELEASE_VERSION points at HEAD"
else
  ok "release tag $RELEASE_VERSION is free"
fi

# -------- 9. install.command + release/README.txt + cask present --------
[ -f install.command ] || die "install.command missing at repo root."
[ -f release/README.txt ] || die "release/README.txt missing."
[ -f release/homebrew-tap/Casks/gmic-affinity.rb ] || die "release/homebrew-tap/Casks/gmic-affinity.rb missing."
ok "release inputs present"

echo ""
echo "[release-preflight] all checks passed; safe to proceed."
