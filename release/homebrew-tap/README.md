# homebrew-gmic-affinity

Homebrew tap for [`gmic-affinity`](https://github.com/dstrupl/gmic-affinity) —
a Photoshop-compatible filter plugin that bridges
[G'MIC](https://gmic.eu/) into Affinity Photo on macOS.

## Status: not yet published

This tap is **scaffolding for v0.2**. The cask file passes
`brew style` and is shape-locked, but it is not installable today
and the GitHub repo backing the tap (`dstrupl/homebrew-gmic-affinity`)
has not been created.

The reason is upstream-Homebrew policy, not anything specific to this
project: starting **2026-09-01**, Homebrew ends support for casks
that fail Apple Gatekeeper checks
([Homebrew/brew#20755](https://github.com/homebrew/brew/issues/20755)).
The `quarantine false` cask DSL stanza that previous versions of
brew offered as a workaround for unsigned bundles was removed in
late 2025. Together those changes make this cask infeasible for
v0.1, where the bundle is only ad-hoc-signed.

The cask becomes immediately viable once the bundle is signed with an
Apple Developer ID and notarised, at which point Gatekeeper accepts
the load and brew lays it down without any quarantine workaround.
That work is v0.2.

Until then, install gmic-affinity via the GitHub release zip:
<https://github.com/dstrupl/gmic-affinity/releases/latest>.

## What this directory becomes (v0.2 onwards)

When the tap repo is published — see `PUBLISHING.md` — these are the
public install / update / uninstall commands users will run:

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

## Per-release update procedure (maintainers, v0.2 onwards)

After the [project repo](https://github.com/dstrupl/gmic-affinity)
publishes a new release tag `vX.Y.Z`:

1. Compute the asset SHA256:
   ```bash
   curl -sL https://github.com/dstrupl/gmic-affinity/releases/download/vX.Y.Z/GmicFilter-vX.Y.Z.zip \
     | shasum -a 256
   ```
2. Open a PR bumping `version` and `sha256` in
   [`Casks/gmic-affinity.rb`](./Casks/gmic-affinity.rb).
3. Wait for the `cask audit` workflow to pass; merge.
4. Users get the update on their next `brew upgrade --cask`.

The full release design and the Homebrew-deprecation rationale that
held this tap back from v0.1 live in the upstream project's
[`docs/design/2026-05-18-release-v0.1-distribution.md`](https://github.com/dstrupl/gmic-affinity/blob/main/docs/design/2026-05-18-release-v0.1-distribution.md)
(§5 + §12).
