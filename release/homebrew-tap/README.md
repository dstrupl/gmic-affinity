# homebrew-gmic-affinity

Homebrew tap for [`gmic-affinity`](https://github.com/dstrupl/gmic-affinity) —
a Photoshop-compatible filter plugin that bridges
[G'MIC](https://gmic.eu/) into Affinity Photo on macOS.

## Status: live as of v0.2.0

This tap is live. The v0.2.0 release produced a Developer ID-signed,
notarised, and stapled `GmicFilter-v0.2.0.zip`; the release pipeline
bumped the cask in `dstrupl/homebrew-gmic-affinity` to version
`0.2.0` with SHA256
`2288000f1016562e8f10a19b5f38d5b86de48941546289e71415126277cfbc62`.
Local smoke testing confirmed `brew install --cask gmic-affinity`
installs into both Affinity plugin folders, and Affinity Photo 2 loads
and runs the plugin.

The reason is upstream-Homebrew policy, not anything specific to this
project: starting **2026-09-01**, Homebrew ends support for casks
that fail Apple Gatekeeper checks
([Homebrew/brew#20755](https://github.com/homebrew/brew/issues/20755)).
The `quarantine false` cask DSL stanza that previous versions of
brew offered as a workaround for unsigned bundles was removed in
late 2025. Together those changes make this cask infeasible for
v0.1, where the bundle is only ad-hoc-signed.

Stable releases now use the collaborator-run signed release pipeline:
Developer ID signing, notarisation, stapling, GitHub Release publish,
and tap bump are covered by the upstream runbook in
[`release/notarisation/SIGNING.md`](../notarisation/SIGNING.md).

## User commands

These are the public install / update / uninstall commands users run:

```bash
brew tap dstrupl/gmic-affinity
brew install --cask gmic-affinity
# updates …
brew upgrade --cask gmic-affinity
# removal …
brew uninstall --cask gmic-affinity
```

The cask installs `GmicFilter.plugin` into every Affinity Photo
plugins folder it finds on the user's machine (Affinity Photo 2
and/or Affinity Photo v3) and declares the runtime `gmic` formula
as a dependency. Restart Affinity afterwards.

If `Filters → Plugins → G'MIC` is missing, open
**Affinity → Settings → Photoshop Plugins** and tick
*"Allow unknown plugins to be used"*.

## Per-release update procedure (maintainers)

Stable releases are normally automated by the upstream
`make release RELEASE_VERSION=vX.Y.Z` pipeline. After publishing the
GitHub release, it runs `scripts/release-bump-cask.sh`, which:

1. Computes the asset SHA256:
   ```bash
   curl -sL https://github.com/dstrupl/gmic-affinity/releases/download/vX.Y.Z/GmicFilter-vX.Y.Z.zip \
     | shasum -a 256
   ```
2. Bumps `version` and `sha256` in
   [`Casks/gmic-affinity.rb`](./Casks/gmic-affinity.rb).
3. Runs `brew style`.
4. Commits and pushes to the tap.

Users get the update on their next `brew upgrade --cask`.

The full release design and the Homebrew-deprecation rationale that
held this tap back from v0.1 live in the upstream project's
[`docs/design/2026-05-18-release-v0.1-distribution.md`](https://github.com/dstrupl/gmic-affinity/blob/main/docs/design/2026-05-18-release-v0.1-distribution.md)
(§5 + §12).
