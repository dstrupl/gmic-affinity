# Publishing the homebrew tap repo

This directory is staging for the `dstrupl/homebrew-gmic-affinity`
GitHub repo. The repo does not yet exist; create it and push the
contents of this folder when you're ready to ship the brew install
path. Until then the cask file lives here so it can be reviewed
alongside the rest of the v0.1 release work.

## One-time setup

```bash
# 1. Create the repo on GitHub. The `homebrew-` prefix is mandatory
#    for `brew tap dstrupl/gmic-affinity` to discover it.
gh repo create dstrupl/homebrew-gmic-affinity --public \
  --description "Homebrew tap for gmic-affinity"

# 2. Push the contents of this directory to the new repo.
TAP_DIR=$(mktemp -d)
cp -R release/homebrew-tap/. "$TAP_DIR/"
cd "$TAP_DIR"
git init -b main
git add .
git commit -m "Initial commit: gmic-affinity v0.1.0 cask"
git remote add origin git@github.com:dstrupl/homebrew-gmic-affinity.git
git push -u origin main
```

## After each upstream release

See `release/homebrew-tap/README.md` "Per-release update procedure".

## Why this lives in the project repo for now

Until the tap repo is created, version-controlling the cask alongside
the project itself keeps the cask reviewable in the same PR/commit
history as the release pipeline that produces its zip artifact. Once
the tap repo exists and is bootstrapped, the canonical copy of
`Casks/gmic-affinity.rb` lives there and this directory becomes
read-only / can be removed from the project repo.
