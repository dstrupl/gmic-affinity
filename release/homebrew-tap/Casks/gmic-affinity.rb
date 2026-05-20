cask "gmic-affinity" do
  version "0.1.0"
  # Set automatically by the per-release tap PR. To compute locally:
  #   curl -sL https://github.com/dstrupl/gmic-affinity/releases/download/v#{version}/GmicFilter-v#{version}.zip | shasum -a 256
  sha256 "REPLACE_WITH_RELEASE_ZIP_SHA256"

  url "https://github.com/dstrupl/gmic-affinity/releases/download/v#{version}/GmicFilter-v#{version}.zip"
  name "G'MIC for Affinity Photo"
  desc "Photoshop-compatible filter plugin bridging G'MIC into Affinity Photo"
  homepage "https://github.com/dstrupl/gmic-affinity"

  # The plugin shells out to the gmic CLI. The `gmic` formula provides
  # exactly that binary plus the runtime libs (cimg, fftw, libtiff,
  # libpng, openexr, libomp). It does not ship a Qt GUI, but we don't
  # need one — the picker dialog is our own native Cocoa code.
  #
  # G'MIC-Qt (the standalone GUI / GIMP plugin from gmic.eu) is NOT a
  # Homebrew formula or cask and is intentionally not depended on. It
  # is unrelated to this plugin; users who want it can grab it from
  # https://gmic.eu/download.html separately.
  depends_on formula: "gmic"
  depends_on macos:   ">= :big_sur"

  # ============================================================
  # NOT YET PUBLISHED — held back pending notarisation (v0.2).
  # ============================================================
  # Phase 0 step 3 (2026-05-19) confirmed empirically that both
  # Affinity Photo 2 and Affinity Photo v3 (3.2.1) reject this bundle
  # at filter invocation time when com.apple.quarantine is set:
  # macOS Gatekeeper blocks the dlopen of an ad-hoc-signed Mach-O
  # carrying the quarantine bit inside a hardened-runtime host
  # process and shows the "GmicFilter.plugin Not Opened — Apple
  # could not verify..." alert.
  #
  # The original v0.1 plan was to declare `quarantine false` here so
  # brew would strip the bit at install time. That stanza no longer
  # exists in the modern Cask DSL — it was removed alongside the
  # `--no-quarantine` install-flag deprecation in Sept-Oct 2025
  # (Homebrew/brew#20755, Homebrew/brew#20929). On 2026-09-01
  # Homebrew will end support for casks that fail Gatekeeper checks
  # entirely, regardless of any user-side workaround. Therefore an
  # unsigned/un-notarised bundle cannot be distributed via brew.
  #
  # v0.1 ships only via the manual zip + install.command path
  # (see docs/design/2026-05-18-release-v0.1-distribution.md §4.4),
  # which strips the quarantine bit user-side via `xattr -dr` and is
  # unaffected by the Homebrew deprecation. v0.2 work covers Apple
  # Developer enrolment + signing + notarisation; once the produced
  # bundle is notarised, this cask becomes immediately viable
  # (Gatekeeper accepts the load, no `quarantine false` knob needed)
  # and we publish the tap repo from `release/homebrew-tap/`.
  #
  # Until then this file is staged-but-unpublished scaffolding. It
  # passes `brew style` so the future tap-repo bring-up is one bump
  # of `version` + `sha256` away. It is NOT a working cask today.

  # Install one source bundle into both Affinity plugin folders. If
  # `brew audit --cask` rejects two `artifact` stanzas pointing at the
  # same source path, fall back to a single `artifact` plus a
  # `preflight` block doing the second copy with FileUtils.cp_r — see
  # release design doc §5.2 risk #3.
  artifact "GmicFilter-v#{version}/GmicFilter.plugin",
           target: "~/Library/Application Support/Affinity Photo 2/Plugins/GmicFilter.plugin"
  artifact "GmicFilter-v#{version}/GmicFilter.plugin",
           target: "~/Library/Application Support/Affinity/Plugins/GmicFilter.plugin"

  caveats <<~EOS
    G'MIC for Affinity is installed for Affinity Photo 2 and Affinity Photo v3.
    Restart Affinity Photo to pick up the plugin.

    If Filters → Plugins → G'MIC is missing, enable:
      Affinity → Settings → Photoshop Plugins → "Allow unknown plugins to be used"

    Logs:   ~/Library/Logs/gmic-affinity.log
    Issues: https://github.com/dstrupl/gmic-affinity/issues
  EOS
end
