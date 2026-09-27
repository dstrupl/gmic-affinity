#!/usr/bin/env bash
# Move a Developer ID identity from the macOS data-protection keychain into
# the legacy login keychain used by `security` and `codesign`.
#
# Keychain Access must perform the export because private keys in the newer
# data-protection store are intentionally unavailable to command-line tools.
# Keychain Access can export only to a user-approved location such as Downloads.
# This helper gives that encrypted export a unique name, immediately moves it
# into a private temporary directory, imports it with a hidden terminal password
# prompt, verifies a real signing operation, and removes both possible copies on
# every exit path.

set -euo pipefail

usage() {
  cat <<'EOF'
usage: scripts/migrate-signing-identity.sh "Developer ID Application: Name (TEAMID)"

Use this only when Keychain Access and Xcode show the Developer ID Application
certificate and its private key, but this command prints no matching identity:

  security find-identity -v -p codesigning

The script pauses while you export the private key from Keychain Access. Type
the export password at security(1)'s hidden terminal prompt; the script never
reads or stores it.
EOF
}

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  usage >&2
  exit 64
fi

EXPECTED_IDENTITY=$1
LOGIN_KEYCHAIN=$(security default-keychain -d user | tr -d '"[:space:]')

if [ -z "$LOGIN_KEYCHAIN" ] || [ ! -f "$LOGIN_KEYCHAIN" ]; then
  echo "ERROR: could not resolve the user's default login keychain." >&2
  exit 1
fi

if security find-identity -v -p codesigning "$LOGIN_KEYCHAIN" 2>/dev/null \
   | grep -F "$EXPECTED_IDENTITY" >/dev/null; then
  echo "Signing identity is already available to codesign:"
  echo "  $EXPECTED_IDENTITY"
  exit 0
fi

umask 077
WORK_DIR=$(mktemp -d /private/tmp/gmic-affinity-signing.XXXXXX)
P12_PATH="$WORK_DIR/developer-id-identity.p12"
TEST_BINARY="$WORK_DIR/codesign-test"
WORK_SUFFIX=${WORK_DIR##*.}
EXPORT_PATH="$HOME/Downloads/gmic-affinity-developer-id-$WORK_SUFFIX.p12"

if [ -e "$EXPORT_PATH" ]; then
  echo "ERROR: refusing to overwrite existing export path: $EXPORT_PATH" >&2
  exit 1
fi

cleanup() {
  # The PKCS#12 is encrypted, but remove it promptly anyway. APFS and SSD wear
  # levelling do not provide reliable per-file overwrite guarantees, so the
  # protection comes from encryption plus deletion, not a fake "secure erase".
  if [[ "${WORK_DIR:-}" == /private/tmp/gmic-affinity-signing.* ]]; then
    rm -f -- "${P12_PATH:-}" "${TEST_BINARY:-}"
    rmdir -- "$WORK_DIR" 2>/dev/null || true
  fi
  if [[ "${EXPORT_PATH:-}" == "$HOME"/Downloads/gmic-affinity-developer-id-*.p12 ]]; then
    rm -f -- "$EXPORT_PATH"
  fi
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

cat <<EOF
Keychain Access export required

1. In Keychain Access, select login > My Certificates.
2. Expand this certificate:
     $EXPECTED_IDENTITY
3. Select its private-key child.
4. Choose File > Export Items.
5. Save the encrypted export exactly here:
     $EXPORT_PATH
6. Set a temporary export password. Do not paste it into this terminal.

The encrypted export is moved into a private temporary directory immediately
and deleted automatically after import or on failure.
EOF

read -r -p "Press Return after the .p12 export finishes (or Ctrl-C to cancel): "

if [ ! -f "$EXPORT_PATH" ]; then
  echo "ERROR: expected export not found: $EXPORT_PATH" >&2
  exit 1
fi
mv -- "$EXPORT_PATH" "$P12_PATH"
chmod 600 "$P12_PATH"

echo ""
echo "Importing into: $LOGIN_KEYCHAIN"
echo "Type the temporary .p12 password at the hidden terminal prompt."

# Omitting -P is deliberate: `security import` reads the password without
# echoing it, keeping the PKCS#12 password out of this script, process arguments,
# and logs.
security import "$P12_PATH" \
  -k "$LOGIN_KEYCHAIN" \
  -f pkcs12 \
  -t agg \
  -T /usr/bin/codesign \
  -T /usr/bin/security

# The keychain owns the identity now; delete the portable private-key copy
# before waiting for the trust-policy cache or running any further checks.
rm -f -- "$P12_PATH"

identity_found=false
for _ in {1..10}; do
  if security find-identity -v -p codesigning "$LOGIN_KEYCHAIN" 2>/dev/null \
     | grep -F "$EXPECTED_IDENTITY" >/dev/null; then
    identity_found=true
    break
  fi
  sleep 1
done

if [ "$identity_found" != true ]; then
  echo "ERROR: import completed, but codesign still cannot see the identity:" >&2
  echo "  $EXPECTED_IDENTITY" >&2
  exit 1
fi

cp /usr/bin/true "$TEST_BINARY"
codesign --force --sign "$EXPECTED_IDENTITY" --timestamp=none "$TEST_BINARY"
codesign --verify --strict "$TEST_BINARY"

echo ""
echo "Developer ID identity migrated and verified:"
echo "  $EXPECTED_IDENTITY"
echo "Temporary export removed:"
echo "  $P12_PATH"
