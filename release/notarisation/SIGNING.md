# Signing & releasing `gmic-affinity` — maintainer guide

This document is for the project maintainer who owns the Apple Developer
Program membership and publishes signed, notarised macOS releases. It
explains the one-time setup on the maintainer's Mac and the command used
for each stable release.

If you're not preparing a stable release, use one of these instead:

- [`README.md`](../../README.md) — what the project is and how end
  users install it.
- [`IMPLEMENTATION_NOTES.md`](../../IMPLEMENTATION_NOTES.md) §11 —
  the operator runbook and verification checklist.
- [`docs/design/2026-05-18-release-v0.1-distribution.md`](../../docs/design/2026-05-18-release-v0.1-distribution.md)
  §12 — why this signing dance is necessary at all (Homebrew dropped
  the `quarantine false` cask escape hatch in late 2025; from
  2026-09-01 every cask must produce a Gatekeeper-passing artefact,
  which means a Developer ID-signed and notarised bundle).

## TL;DR

Every stable release is `make release RELEASE_VERSION=vX.Y.Z` from a clean
working tree on `main`. The Makefile does the build, signs with the
maintainer's Developer ID, submits to Apple's notary service, staples, verifies,
publishes a GitHub release, and bumps the Homebrew tap cask. You're
done in ~5–10 minutes per release, most of which is Apple's notary
queue.

The first time you run it there's a one-time setup (~30 minutes) to
install certs, save credentials in your keychain, and write a small
config file. That setup is the rest of this document.

## Why signing stays local

The maintainer now has an Apple Developer Program membership and signs
stable releases directly. The membership provides the two capabilities
required for distribution outside the Mac App Store:

1. Get a **Developer ID Application** certificate (the only signing
   identity Apple's notary service accepts for distributable Mac
   software outside the Mac App Store).
2. Submit notarisation requests via `notarytool`, which authenticates
   against an Apple Developer team.

Both happen on the maintainer's Mac, with secret material stored in the
local keychain. The signed and notarised binary is uploaded to GitHub by
`gh`, and the Homebrew tap cask is bumped with the new version and SHA256.

Releases through v0.3.0 used a trusted signing collaborator. Starting
with v0.3.1, the maintainer's own Developer ID Application certificate
and Apple Developer team are the release identity. Existing releases
remain valid under the identity that originally signed them.

Why we don't push the signing into CI: the Apple Developer ID
certificate and notarisation credentials are sensitive enough that they
remain on the maintainer's Mac, encrypted in a keychain, rather than being
uploaded as GitHub Actions secrets. CI also can't access Touch ID-protected
keychain items, which is what makes the local flow ergonomic. This is a
deliberate trade-off — see §12 of the design doc.

## What you need (prerequisites)

You need all of the below before you can run `make release` for the
first time. The preflight (§one-time setup, step 7) verifies most of
this and bails with a clear message for anything missing, so don't
worry about getting it perfect on the first try.

### Apple side

- An **active Apple Developer Program membership** attached to the
  maintainer's Apple Account.
- An **Apple ID** with the membership attached. The Apple ID's email
  is what you'll authenticate `notarytool` with.
- An **app-specific password** for that Apple ID, generated at
  [appleid.apple.com](https://appleid.apple.com) under Sign-In and
  Security → App-Specific Passwords. This is _not_ your iCloud
  password; Apple requires a separate per-app password for
  command-line tools that submit to the notary service.
- A **Developer ID Application** certificate installed in your login
  keychain. If you've ever shipped a notarised app from this machine
  you already have it; otherwise generate one in
  [Apple Developer → Certificates](https://developer.apple.com/account/resources/certificates/list)
  using Xcode (Settings → Accounts → Manage Certificates → `+` →
  Developer ID Application) — it's the cleanest path because Xcode
  also installs the matching private key.
- Your **Team ID**, a 10-character string visible on the Apple
  Developer membership page. You'll need it once when you save the
  notarytool credential profile (below).

### Local toolchain

- macOS (Apple silicon or Intel — `make release` builds a universal
  zip from a single host).
- **Xcode Command Line Tools** (`xcode-select --install`). Provides
  `codesign`, `xcrun notarytool`, `xcrun stapler`, `spctl`, and the
  Rust target SDKs.
- **Rust** (`rustup` from [rustup.rs](https://rustup.rs)) with the
  two macOS targets:

  ```bash
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  ```

- **Homebrew** with these formulae installed:
  - `gmic` (runtime dep — used by the plugin at runtime; the cask
    declares it as a `depends_on formula:`)
  - `git-lfs` (the bundled G'MIC catalogue snapshot lives in LFS)
  - `gh` (GitHub CLI, used to publish the release and manage the
    tap repo)

  ```bash
  brew install gmic git-lfs gh
  git lfs install   # one-time per machine, registers the LFS smudge filter
  ```

- The **Adobe Photoshop SDK** is _not_ required: the project commits
  the compiled `GmicFilter.rsrc` to the repo for exactly this reason
  (CI and contributors without the SDK can still build the bundle).
  See `Makefile:pipl` and `SETUP.md` for context if you're curious.

### Repo access

The active `gh` account needs write access to both repositories:

1. **`dstrupl/gmic-affinity`** — this repo. The release tag is
   created and pushed here, and the GitHub release with the zip
   asset is created here.
2. **`dstrupl/homebrew-gmic-affinity`** — the Homebrew tap. The
   release pipeline clones this, bumps the cask file, and pushes
   back. See
   [`release/homebrew-tap/PUBLISHING.md`](../homebrew-tap/PUBLISHING.md).

Authenticate `gh` as `dstrupl`, which owns both repositories:

```bash
gh auth login          # follow the prompts
gh auth status          # confirm
gh repo view dstrupl/gmic-affinity              # should print metadata
gh repo view dstrupl/homebrew-gmic-affinity     # should also print metadata
```

If either `gh repo view` command fails, repair the active GitHub login or
repository permissions before continuing.

## One-time setup

These steps you do once. Skip ahead to the per-release runbook (next
section) afterwards.

### 1. Clone the project repo

```bash
gh repo clone dstrupl/gmic-affinity
cd gmic-affinity
git lfs pull   # pulls assets/gmic-catalogue.gmic.gz if checkout did not hydrate it
```

### 2. Verify your Developer ID Application certificate is installed

```bash
security find-identity -v -p codesigning
```

You should see an entry like:

```
1) ABCD1234EF... "Developer ID Application: Your Name (TEAMID)"
```

Copy the full quoted string — that's your `DEVELOPER_ID_APP_SIGNATURE`
in the next step. If nothing matches, generate the certificate via
Xcode (see prerequisites above) and re-run.

On newer macOS releases, Certificate Assistant can place the private key in
the data-protection keychain. Keychain Access and Xcode then show the
certificate and private key, but `security find-identity` and `codesign` report
that no identity exists. Migrate only that identity into the legacy login
keychain with the repository helper:

```bash
scripts/migrate-signing-identity.sh \
  "Developer ID Application: Your Name (TEAMID)"
```

The helper creates a private directory under `/private/tmp`, pauses while you
export the selected private key from Keychain Access to the unique `.p12` path
in `Downloads` that it prints, moves the encrypted file into the private
directory immediately, imports it through `security(1)`'s hidden terminal
password prompt, performs a real disposable signing test, and deletes both
possible export paths on every exit path. Type the export password when
prompted; terminal input is not echoed. The password is never passed to the
script or placed in process arguments. Because APFS snapshots and SSD wear
levelling make per-file secure overwrites unreliable, the temporary copy is
protected by PKCS#12 encryption and prompt deletion rather than a misleading
"secure erase" claim.

### 3. Save your notary credentials in the keychain

This is the only local place the Apple ID and app-specific password are
stored. They go into the macOS keychain encrypted and are not written to
project files or shell history; `notarytool` uses them only to authenticate
to Apple's notary service.

```bash
xcrun notarytool store-credentials gmic-affinity-notary \
    --apple-id "you@example.com" \
    --team-id  "TEAMID"
```

- `gmic-affinity-notary` is the **profile name** — it's the handle
  the Makefile uses to look the credentials up. If you change it, set
  `NOTARYTOOL_KEYCHAIN_PROFILE` in `.env.local` (next step) to match.
- When prompted, enter the app-specific password from appleid.apple.com,
  _not_ the normal Apple Account password. Omitting `--password` keeps
  the secret out of shell history and process listings.
- `--team-id` is the 10-character Team ID from your Apple Developer
  membership page.

Verify it works:

```bash
xcrun notarytool history --keychain-profile gmic-affinity-notary
```

Empty history is fine and expected on a fresh setup; what matters is
that the command succeeds without "no profile found" or "auth
failed". If the credentials are wrong, this is where you'll find out —
better here than mid-release.

### 4. Create `.env.local` in the repo root

The Makefile's `make release` target sources this file (see
`Makefile` near the `-include .env.local` line) to pick up the
signing identity name. The file is gitignored (`.env.*` matches in
`.gitignore`) and should _never_ be committed.

```bash
cat > .env.local <<'EOF'
# Maintainer signing config. Gitignored.
# See release/notarisation/SIGNING.md.

# The exact common name of your Developer ID Application cert,
# as printed by `security find-identity -v -p codesigning`.
DEVELOPER_ID_APP_SIGNATURE = Developer ID Application: Your Name (TEAMID)

# The notarytool keychain profile name from step 3.
# Optional: defaults to gmic-affinity-notary.
# NOTARYTOOL_KEYCHAIN_PROFILE = gmic-affinity-notary

# Optional: tap repo URL. Defaults to the HTTPS URL authenticated by gh.
# Override only if you've forked the tap or deliberately use an SSH key that
# has write access to this repository.
# TAP_REPO_URL = https://github.com/dstrupl/homebrew-gmic-affinity.git
# TAP_REPO_URL = git@github.com:dstrupl/homebrew-gmic-affinity.git
EOF
```

Confirm it's gitignored:

```bash
git check-ignore -v .env.local
# .gitignore:23:.env.*	.env.local
```

### 5. Bootstrap your local cargo cache

This isn't strictly required (cargo will fetch on first build), but
warming the cache here means the first real `make release` doesn't
spend 10 minutes fetching dependencies. Pick whichever:

```bash
make universal FEATURES=live      # full unsigned build of the release-flavour bundle
# or, faster, just resolve and download deps:
cargo fetch
```

### 6. Configure git identity (if you haven't)

The cask-bump step commits to the tap repo using your local git
config. Make sure it's set:

```bash
git config --global user.name  "Your Name"
git config --global user.email "you@example.com"
```

### 7. Run the preflight against a dry-run version string

The preflight script bails before any side-effecting step, so it's
safe to run standalone:

```bash
./scripts/release-preflight.sh \
    v0.0.0 \
    "Developer ID Application: Your Name (TEAMID)" \
    gmic-affinity-notary \
    https://github.com/dstrupl/homebrew-gmic-affinity.git
```

Use the exact `DEVELOPER_ID_APP_SIGNATURE` value you put in
`.env.local` for the second argument. Use the exact tap URL that
`make release` will use for the fourth argument; the default shown
above is HTTPS. Keep `v0.0.0` as-is during setup; do not replace it
with the real release version here.

`v0.0.0` is a valid semver-shaped tag that should never exist as a
real project release, so the script can exercise the setup-sensitive
preflight without requiring the repo's package metadata to be bumped:
working-tree state, signing identity, notary creds, gh auth, tap-repo
API access and clone reachability, tag/release availability, and
release inputs. Everything should pass cleanly.

If anything fails here, fix it now. The actual release is not the
time to debug keychain auth.

You're done with setup. From now on, every release is one command.

## Per-release runbook

This is what the maintainer does for each stable release. Total wall-clock
time is typically 5–10 minutes, mostly Apple's notary queue.

### 1. Sync the working tree

```bash
cd gmic-affinity
git checkout main
git pull --ff-only
```

The release pipeline refuses to run on a dirty tree, on a detached
HEAD, or on a branch with unpushed commits. This is deliberate — we
want the published binary to correspond to a commit that's already
on origin and reviewable.

### 2. Run the release

```bash
make release RELEASE_VERSION=vX.Y.Z
```

Substitute `vX.Y.Z` with the semver selected for the release, for example
`v0.3.1`. Do not infer a version from the current tag: choose it explicitly
and update `Cargo.toml`, `Cargo.lock`, `CFBundleShortVersionString`,
`CFBundleGetInfoString` in both `Info.plist` and
`en.lproj/InfoPlist.strings`, and the integer `CFBundleVersion` before
running the pipeline.

### 3. Approve Touch ID prompts

You may see two Touch ID prompts during the run:

1. Once when `codesign --sign` accesses your Developer ID Application
   private key (early, during the build phase).
2. Possibly a second time if `notarytool submit` needs to fetch the
   keychain credentials (depends on your keychain settings).

Approve both. If you decline either, the run aborts with a clear
error and no GitHub release / cask bump happens — safe to retry.

### 4. Watch for "Notarized Developer ID"

Toward the end of the run, the verification phase prints:

```
=== spctl --assess (must say 'Notarized Developer ID') ===
.../GmicFilter.plugin: accepted
source=Notarized Developer ID
```

That `source=Notarized Developer ID` line is the key one: it means
Gatekeeper would accept this bundle on a fresh user's machine,
which is the whole point of the exercise. If the line says anything
else (`Unnotarized Developer ID`, `No Mac App Store`, etc.), do
**not** publish or continue the release. Diagnose the signing or
notarisation failure first.

### 5. Confirm the final summary

The last screenful is:

```
================================================================
  Released vX.Y.Z
================================================================

Smoke-test on a fresh user account:
  brew tap dstrupl/gmic-affinity
  brew trust --cask dstrupl/gmic-affinity/gmic-affinity
  brew install --cask dstrupl/gmic-affinity/gmic-affinity
```

That means:

- The GitHub release `vX.Y.Z` is live at
  `https://github.com/dstrupl/gmic-affinity/releases/tag/vX.Y.Z`,
  with the notarised zip attached.
- The Homebrew tap cask was bumped to that version + sha256 and
  pushed to `dstrupl/homebrew-gmic-affinity`.
- A user running the three `brew` commands now installs your build.

Record the GitHub release URL, cask version/SHA, and remaining functional
smoke-test status.

## When something goes wrong

The pipeline is split into independent phony targets so you can
retry individual phases without redoing the slow ones. Order, in
case you need to resume manually:

```
release-preflight     # all the safety checks
release-build-signed  # cargo build + Developer ID codesign
release-notarize      # ditto into a notary zip + xcrun notarytool submit --wait
release-staple        # xcrun stapler staple
release-verify        # codesign --verify + spctl --assess + stapler validate
release-publish       # final zip + gh release create
release-bump-cask     # clone tap, bump version + sha256, brew style, push
```

If notarisation rejects the bundle (rare on Macs that have shipped a
notarised app before, more common on first runs):

```bash
# Find the most recent submission ID from the failure output, then:
xcrun notarytool log <submission-id> \
    --keychain-profile gmic-affinity-notary
```

Apple's notarisation log is very specific about what's wrong (usually
a missing hardened runtime flag, an unsigned executable inside the
bundle, or an entitlement issue). Fix the build/codesign step rather
than repeatedly resubmitting the same rejected artifact.

If `gh release create` fails because the tag already exists on
GitHub, somebody (probably you, on a previous attempt) already
created the release. Either delete the partial release with
`gh release delete vX.Y.Z` and re-run, or finish manually with
`gh release upload`.

If `release-bump-cask` fails after `release-publish` succeeded, the
GitHub release is up but the cask wasn't bumped. Re-run just the
last step:

```bash
./scripts/release-bump-cask.sh \
    vX.Y.Z \
    dist/GmicFilter-vX.Y.Z.zip \
    https://github.com/dstrupl/homebrew-gmic-affinity.git
```

It's idempotent — safe to re-run.

## Security notes

- `.env.local` contains only the _name_ of your signing certificate,
  not the certificate itself. The actual cert and private key live
  in your login keychain and are accessed by `codesign` via macOS's
  Keychain Services API. They never appear on disk in plaintext and
  never leave the machine.
- Your Apple ID app-specific password is stored in the keychain by
  `xcrun notarytool store-credentials`, encrypted at rest, accessible
  only with your login password / Touch ID. It's never read by the
  Makefile.
- `.gitignore` matches `.env.*`, so accidentally typing `git add .`
  won't stage `.env.local`. Verify with `git check-ignore -v
  .env.local` if you ever want to be sure.
- The release pipeline doesn't store the signing identity, password,
  or any keychain item anywhere — it just exec's `codesign` and
  `xcrun notarytool` and lets macOS handle the secret material.
- If this Mac is retired, remove the local certificate/private key and
  `gmic-affinity-notary` keychain profile only after transferring or
  recreating the release identity on the replacement Mac. Do not revoke
  the Developer ID certificate merely to clean up one machine; revocation
  has consequences for software signed with that certificate.

## Frequently asked

**Q: Do I need to keep the repo and `.env.local` between releases?**
Yes. The repo is small. `.env.local` is the bridge between the
Makefile and your keychain, and it's gitignored anyway.

**Q: Can I run this on a different Mac later?**
Yes — repeat §one-time setup on the new machine. The Developer ID
certificate has to be exported from one keychain and imported into
the other; Apple's docs cover this under "Transfer your developer
identity".

**Q: What if my Apple ID changes its app-specific password?**
Re-run `xcrun notarytool store-credentials gmic-affinity-notary`
with the new password. The profile name stays the same; nothing
else needs to change.

**Q: What if I want to sign a development build for myself, without
notarising or publishing?**
Use `make universal && make install` — that produces an ad-hoc-signed
bundle and copies it into the local Affinity plugin folders. No GitHub
release, notarisation, or tap bump occurs.

**Q: How long does notarisation take?**
Apple's queue is usually <1 minute for small Mach-O bundles like
this. Outliers go to ~5 minutes; longer than that is unusual and
worth checking [Apple's system status](https://developer.apple.com/system-status/).

## When in doubt

Stop before `release-publish`. The cost of checking the certificate,
notary result, metadata, or target commit is much lower than the cost of
unpublishing and re-versioning a bad release.
