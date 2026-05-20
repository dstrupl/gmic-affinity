# Publishing the homebrew tap repo

This directory is staging for the `dstrupl/homebrew-gmic-affinity`
GitHub repo.

## Status (2026-05-20): one-time bootstrap pending, then automated

The tap repo does not yet exist on GitHub. The project lead needs to
bootstrap it **once**, _before_ the signing collaborator runs
`make release` for the first stable v0.2 release. After that the
release pipeline (`scripts/release-bump-cask.sh`, called from
`make release-bump-cask`) takes over: every subsequent release just
clones, bumps `version` + `sha256`, runs `brew style`, commits, and
pushes.

In other words: the tap repo is a one-time `gh repo create` + initial
push from the project lead, and from then on it's effectively a
write-only target of the release pipeline.

The bootstrap is gated on Apple Developer enrolment of the signing
collaborator landing first (see
[`release/notarisation/SIGNING.md`](../notarisation/SIGNING.md)) — the
first push to the tap repo should be a working notarised cask, not
unpublishable scaffolding. Bootstrapping early would just leave a
broken cask sitting on GitHub.

The cask file's v0.2-deferral comment block is stripped automatically
on the first stable bump by `scripts/release-bump-cask.sh`'s Python
mutator — you don't need to remove it by hand. Same for `version` /
`sha256`: those get filled in from the release zip on each run.

## One-time bootstrap (project lead, before the friend's first `make release`)

Do this once, after the signing collaborator confirms their Apple
Developer setup is done (see `release/notarisation/SIGNING.md` §setup)
but before they run `make release` for `v0.2.0`.

```bash
# 1. Create the repo on GitHub. The `homebrew-` prefix is mandatory
#    for `brew tap dstrupl/gmic-affinity` to discover it.
gh repo create dstrupl/homebrew-gmic-affinity --public \
  --description "Homebrew tap for gmic-affinity"

# 2. Stage the tap contents from this project repo and push them
#    as the initial commit. The contents come straight out of
#    release/homebrew-tap/ — anything in there gets pushed verbatim,
#    so make sure that directory is the version you want shipped.
TAP_DIR=$(mktemp -d)
cp -R release/homebrew-tap/. "$TAP_DIR/"
cd "$TAP_DIR"
git init -b main
git add .
git commit -m "Initial commit: gmic-affinity tap (cask waits for first release bump)"
git remote add origin git@github.com:dstrupl/homebrew-gmic-affinity.git
git push -u origin main

# 3. Grant the signing collaborator push access to the tap repo.
#    They already have push to dstrupl/gmic-affinity; they need the
#    same on dstrupl/homebrew-gmic-affinity so release-bump-cask can
#    `git push origin HEAD` from their machine. The recommended
#    grant is a direct collaborator role with write permission:
gh api \
  --method PUT \
  -H "Accept: application/vnd.github+json" \
  /repos/dstrupl/homebrew-gmic-affinity/collaborators/<their-github-username> \
  -f permission=push
# Then have them accept the collaboration invite from their email or
# at https://github.com/dstrupl/homebrew-gmic-affinity/invitations.
```

Verify from a fresh shell that the friend would also see it as
reachable (this is what `release-preflight` checks):

```bash
gh repo view dstrupl/homebrew-gmic-affinity
```

You're done. The friend can now run `make release RELEASE_VERSION=v0.2.0`
and the pipeline will auto-bump the cask in this repo on success.

## After each release (automated)

The release pipeline does this for you. Concretely,
`scripts/release-bump-cask.sh`:

1. Computes SHA256 of `dist/GmicFilter-vX.Y.Z.zip`.
2. Clones `dstrupl/homebrew-gmic-affinity` into a tempdir (depth 1).
3. Strips the v0.2-deferral comment block from
   `Casks/gmic-affinity.rb` if present (one-time-only, idempotent).
4. Updates `version "X.Y.Z"` and `sha256 "<computed>"`.
5. Runs `brew style Casks/gmic-affinity.rb` to verify clean.
6. Commits with the message
   `Bump gmic-affinity to X.Y.Z` + SHA + URL.
7. `git push origin HEAD`.

If anything fails, the script bails before pushing — safe to re-run.
The published GitHub release of the project repo at that point is
already up; only the cask bump is missing, and re-running just that
script (with the same arguments `make release-bump-cask` would have
passed) finishes the job.

## Why this directory still lives in the project repo

The cask source-of-truth lives in the tap repo once it's bootstrapped.
This directory remains in the project repo because:

- It documents what the tap _initially_ contained, for future repo
  archaeology.
- It carries the v0.2-deferral comment block that
  `release-bump-cask.sh` strips on the first stable release — the
  block is the only place the deferral rationale is captured in
  Cask DSL form, useful as a paper trail.
- The tap-repo CI (`.github/workflows/audit.yml` in this directory)
  is staged here for the bootstrap step to copy in.

After the bootstrap push, edits to the cask should happen in the tap
repo directly (or via `release-bump-cask.sh`), not by editing files
in this directory and re-bootstrapping. If you ever need to change
cask DSL substantively (e.g. add a new artifact stanza, change
`depends_on`), edit `dstrupl/homebrew-gmic-affinity` directly and let
the next release bump pick up the new structure with refreshed
`version` / `sha256`.
