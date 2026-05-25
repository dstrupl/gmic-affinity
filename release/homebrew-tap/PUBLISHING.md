# Publishing the homebrew tap repo

This directory is staging for the `dstrupl/homebrew-gmic-affinity`
GitHub repo.

## Status (2026-05-25): tap repo live

The tap repo is live at `dstrupl/homebrew-gmic-affinity`. The first
collaborator-run signed release (`v0.2.0`) completed successfully:
the release pipeline published the notarised GitHub release artifact
and bumped the live tap cask to version `0.2.0`.

From here the release pipeline (`scripts/release-bump-cask.sh`, called
from `make release-bump-cask`) takes over: it clones the tap, bumps
`version` + `sha256`, runs `brew style`, commits, and pushes.

The v0.2-deferral comment block was stripped automatically during the
v0.2.0 stable bump. Same for `version` / `sha256`: those are filled
in from the release zip on each run.

## One-time access check (new signing collaborator)

Do this after the signing collaborator confirms they are ready to run
the signed-release pipeline (see `release/notarisation/SIGNING.md`
§setup) but before they run `make release`.

```bash
# 1. Confirm the tap repo exists and is reachable from your account.
gh repo view dstrupl/homebrew-gmic-affinity

# 2. Grant the signing collaborator push access to the tap repo.
#    They already need push to dstrupl/gmic-affinity; they need the
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

After they accept the invite, have them verify from a fresh shell that
the repo is reachable (this is what `release-preflight` checks):

```bash
gh repo view dstrupl/homebrew-gmic-affinity
```

You're done. The collaborator can now run
`make release RELEASE_VERSION=vX.Y.Z` and the pipeline will auto-bump
the cask in this repo on success.

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
