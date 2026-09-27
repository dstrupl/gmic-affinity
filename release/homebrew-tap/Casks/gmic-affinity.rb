cask "gmic-affinity" do
  version "0.3.1"
  # Set automatically by the per-release tap bump. To compute locally:
  #   curl -sL https://github.com/dstrupl/gmic-affinity/releases/download/v#{version}/GmicFilter-v#{version}.zip | shasum -a 256
  sha256 "66ac6b48a76ea1f2a2c7a80fede89a62a6847e2cb543c407d4bc5d5a73b12350"

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
  # Current Homebrew rejects redundant minimum macOS versions under
  # Homebrew/OSDependsOn. The bundle's LSMinimumSystemVersion remains the
  # authoritative macOS 11 runtime floor.
  depends_on :macos

  # Homebrew manages the Photo 2 copy as the bundle artifact. A structured
  # postflight step makes the independent v3 copy: declaring the same source
  # twice works on a fresh install but collides while Homebrew backs up the
  # predecessor cask during an upgrade.
  artifact "GmicFilter-v#{version}/GmicFilter.plugin",
           target: "~/Library/Application Support/Affinity Photo 2/Plugins/GmicFilter.plugin"

  postflight_steps do
    copy "Library/Application Support/Affinity Photo 2/Plugins/GmicFilter.plugin",
         "Library/Application Support/Affinity/Plugins/GmicFilter.plugin",
         source_base: :home,
         target_base: :home,
         recursive:   true
  end

  uninstall_postflight_steps do
    remove "Library/Application Support/Affinity/Plugins/GmicFilter.plugin",
           base:      :home,
           recursive: true
  end

  caveats <<~EOS
    G'MIC for Affinity is installed for Affinity Photo 2 and Affinity Photo v3.
    Restart Affinity Photo to pick up the plugin.

    If Filters → Plugins → G'MIC is missing, enable:
      Affinity → Settings → Photoshop Plugins → "Allow unknown plugins to be used"

    Logs:   ~/Library/Logs/gmic-affinity.log
    Issues: https://github.com/dstrupl/gmic-affinity/issues
  EOS
end
