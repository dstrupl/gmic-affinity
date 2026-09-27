# Publishing the homebrew tap repo

This directory is staging for the `dstrupl/homebrew-gmic-affinity`
GitHub repo.

## Status: tap repo live

The tap repo is live at `dstrupl/homebrew-gmic-affinity`. The first
signed release (`v0.2.0`) completed successfully:
the release pipeline published the notarised GitHub release artifact
and bumped the live tap cask. It has subsequently carried v0.3.0.

From here the release pipeline (`scripts/release-bump-cask.sh`, called
from `make release-bump-cask`) takes over: it clones the tap, bumps
`version` + `sha256`, runs `brew style`, commits, and pushes.

The v0.2-deferral comment block was stripped automatically during the
v0.2.0 stable bump. Same for `version` / `sha256`: those are filled
in from the release zip on each run.

## Maintainer access check

Run this before `make release` to verify the active `gh` account can write
both the project and tap repositories:

```bash
gh auth status
gh repo view dstrupl/gmic-affinity --json viewerPermission
gh repo view dstrupl/homebrew-gmic-affinity --json viewerPermission
git ls-remote git@github.com:dstrupl/homebrew-gmic-affinity.git HEAD
```

The maintainer should have `ADMIN` or `WRITE` permission on both repos.
The release preflight repeats the reachability checks before building.

## After each release (automated)

The release pipeline does this for you. Concretely,
`scripts/release-bump-cask.sh`:

1. Computes SHA256 of `dist/GmicFilter-vX.Y.Z.zip`.
2. Clones `dstrupl/homebrew-gmic-affinity` into a tempdir (depth 1).
3. Strips the old v0.2-deferral comment block from
   `Casks/gmic-affinity.rb` if present (historical one-time cleanup,
   idempotent).
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

- It documents what the tap initially contained, for future repo
  archaeology.
- It keeps a project-local mirror of the cask shape used by the live
  tap, including the historical deferral rationale in git history.
- The tap-repo CI and initial cask history were staged from here.

Now that the tap repo exists, edits to the live cask should happen in
the tap repo directly (or via `release-bump-cask.sh`), not by editing
files in this directory and trying to re-bootstrap. If you ever need to change
cask DSL substantively (e.g. add a new artifact stanza, change
`depends_on`), edit `dstrupl/homebrew-gmic-affinity` directly and let
the next release bump pick up the new structure with refreshed
`version` / `sha256`.
