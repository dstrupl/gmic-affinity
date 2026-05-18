G'MIC for Affinity Photo — installation
=======================================

This zip contains GmicFilter.plugin, a Photoshop-compatible filter
plugin that adds a "G'MIC..." entry under Filters → Plugins → G'MIC
in Affinity Photo. It works for Affinity Photo 2 and Affinity Photo
v3 (Affinity by Canva).

1. Make sure gmic is installed:

       brew install gmic-qt
       (or just  brew install gmic   if you only need the CLI)

2. Double-click install.command in this folder. macOS may prompt:

       "install.command cannot be opened because it is from an
        unidentified developer."

   If so, do one of:
       a. Right-click install.command  →  Open  →  Open
       b. Or in Terminal, from this folder:
              xattr -dr com.apple.quarantine .
              ./install.command

   The script installs GmicFilter.plugin into every Affinity Photo
   plugins folder it detects on your machine:
       ~/Library/Application Support/Affinity Photo 2/Plugins/
       ~/Library/Application Support/Affinity/Plugins/

3. Restart Affinity Photo. Open an 8-bit RGB document, then look for:

       Filters → Plugins → G'MIC → G'MIC…

If the plugin is not detected, check:

   Affinity → Settings → Photoshop Plugins →
       "Allow unknown plugins to be used"   (must be ticked)

Logs:    ~/Library/Logs/gmic-affinity.log
Issues:  https://github.com/dstrupl/gmic-affinity/issues
