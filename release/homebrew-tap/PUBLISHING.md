# Publishing the homebrew tap repo

This directory is staging for the `dstrupl/homebrew-gmic-affinity`
GitHub repo.

## Status (2026-05-20): hold publication until v0.2

The repo does not yet exist on GitHub. **Do not create it as part of
the v0.1 release.** The cask file in `Casks/gmic-affinity.rb` cannot
install successfully on a current Homebrew because the cask path
hard-depends on Apple-Gatekeeper-passing bundles that we don't
produce in v0.1 (we ship ad-hoc-signed, un-notarised). This was
discovered when the v0.1 cask smoke-test on `v0.1.0-rc.1` failed with
`undefined method 'quarantine'`. Full rationale and decision trail:
the upstream project's
[`docs/design/2026-05-18-release-v0.1-distribution.md`](https://github.com/dstrupl/gmic-affinity/blob/main/docs/design/2026-05-18-release-v0.1-distribution.md)
§12.

A local mirror of this directory lives at
`~/projects/homebrew-gmic-affinity/` with a single amended initial
commit and **no remote**. That mirror is ready to push as-is the
moment v0.2 lands; the v0.2 work is what makes the cask viable
(Apple Developer ID enrolment + signing + notarisation + stapler).

## One-time setup (run when v0.2 is ready)

```bash
# 1. Create the repo on GitHub. The `homebrew-` prefix is mandatory
#    for `brew tap dstrupl/gmic-affinity` to discover it.
gh repo create dstrupl/homebrew-gmic-affinity --public \
  --description "Homebrew tap for gmic-affinity"

# 2a. If using the local mirror at ~/projects/homebrew-gmic-affinity/
#     (preferred — preserves the existing single commit):
cd ~/projects/homebrew-gmic-affinity
git remote add origin git@github.com:dstrupl/homebrew-gmic-affinity.git
git push -u origin main

# 2b. Or, equivalently, push the contents of *this* directory fresh:
TAP_DIR=$(mktemp -d)
cp -R release/homebrew-tap/. "$TAP_DIR/"
cd "$TAP_DIR"
git init -b main
git add .
git commit -m "Initial commit: gmic-affinity v0.2.0 cask"
git remote add origin git@github.com:dstrupl/homebrew-gmic-affinity.git
git push -u origin main
```

Before pushing, drop the v0.2-deferral comment block from
`Casks/gmic-affinity.rb` (the file currently states inline that it is
not yet publishable) and bump `version` + `sha256` to point at the
v0.2.0 release artifact.

## After each upstream release

See `release/homebrew-tap/README.md` "Per-release update procedure".

## Why this lives in the project repo for now

Until the tap repo is created, version-controlling the cask alongside
the project itself keeps the cask reviewable in the same PR/commit
history as the release pipeline that produces its zip artifact. Once
the tap repo exists and is bootstrapped, the canonical copy of
`Casks/gmic-affinity.rb` lives there and this directory becomes
read-only / can be removed from the project repo.
