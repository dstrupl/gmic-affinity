cask "gmic-affinity" do
  version "0.2.0"
  # Set automatically by the per-release tap bump. To compute locally:
  #   curl -sL https://github.com/dstrupl/gmic-affinity/releases/download/v#{version}/GmicFilter-v#{version}.zip | shasum -a 256
  sha256 "2288000f1016562e8f10a19b5f38d5b86de48941546289e71415126277cfbc62"

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
  # `:big_sur` (bare symbol) is the modern lower-bound form. brew
  # style rejects the older `">= :big_sur"` string-comparator shape
  # under Homebrew/OSDependsOn — the bare symbol means "this macOS or
  # later", which is what we want.
  depends_on macos: :big_sur

  # Install one source bundle into both Affinity plugin folders. Homebrew
  # accepts two `artifact` stanzas pointing at the same source path; this
  # was verified by the v0.2.0 cask install.
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
