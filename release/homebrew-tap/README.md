# homebrew-gmic-affinity

Homebrew tap for [`gmic-affinity`](https://github.com/dstrupl/gmic-affinity) —
a Photoshop-compatible filter plugin that bridges
[G'MIC](https://gmic.eu/) into Affinity Photo on macOS.

## Install

```bash
brew tap dstrupl/gmic-affinity
brew install --cask gmic-affinity
```

This installs `GmicFilter.plugin` into every Affinity Photo plugins
folder it finds on your machine (Affinity Photo 2 and/or Affinity Photo
v3) and pulls in the runtime `gmic` dependency. Restart Affinity Photo
afterwards.

If `Filters → Plugins → G'MIC` is missing, open
**Affinity → Settings → Photoshop Plugins** and tick
*"Allow unknown plugins to be used"*.

## Update

```bash
brew upgrade --cask gmic-affinity
```

## Uninstall

```bash
brew uninstall --cask gmic-affinity
```

## Per-release update procedure (maintainers)

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

The full release design lives in the upstream project's
[`docs/design/2026-05-18-release-v0.1-distribution.md`](https://github.com/dstrupl/gmic-affinity/blob/main/docs/design/2026-05-18-release-v0.1-distribution.md).
