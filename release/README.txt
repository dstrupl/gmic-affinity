G'MIC for Affinity Photo — installation
=======================================

This zip contains GmicFilter.plugin, a Photoshop-compatible filter
plugin that adds a "G'MIC..." entry under Filters → Plugins → G'MIC
in Affinity Photo. It works for Affinity Photo 2 and Affinity Photo
v3 (Affinity by Canva).

1. Make sure the gmic CLI is installed:

       brew install gmic

   This plugin shells out to /opt/homebrew/bin/gmic; it has its own
   built-in picker dialog, so you do NOT need G'MIC-Qt (the
   standalone GUI / GIMP plugin) for this plugin to work. G'MIC-Qt
   is not on Homebrew anyway; if you want it for other reasons,
   download it from https://gmic.eu/download.html.

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

   If G'MIC appears but is greyed out, it is detected correctly but the
   document format is unsupported. In Affinity v3 use:

       Document → Setup → Convert Format / ICC Profile… → RGB/8

   Converting reduces a 16/32-bit document to 8-bit; duplicate it first
   if you need to preserve the higher-bit source.

If the plugin is not detected, check:

   Affinity → Settings → Photoshop Plugins →
       "Allow unknown plugins to be used"   (must be ticked)

If the picker opens but processing fails and the log reports "Library
not loaded", repair a stale Homebrew gmic installation:

       brew linkage --test gmic
       brew reinstall gmic
       gmic -version

Logs:    ~/Library/Logs/gmic-affinity.log
Issues:  https://github.com/dstrupl/gmic-affinity/issues
