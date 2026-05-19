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

  # Phase 0 step 3 (see docs/design/2026-05-18-release-v0.1-distribution.md
  # in the upstream project repo) decides whether this line is needed.
  # If Affinity loads the bundle with com.apple.quarantine set, leave
  # this line commented out (default brew behaviour: quarantine on).
  # If Affinity refuses the quarantined bundle, uncomment so brew
  # strips the bit on install.
  # quarantine false

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
