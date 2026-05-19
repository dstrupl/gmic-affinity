# Development setup

These steps prepare a Mac for building `gmic-affinity` from source. They only
need to be done once.

## 1. Toolchain

```bash
# Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Both Apple Silicon and Intel targets (universal binary)
rustup target add aarch64-apple-darwin x86_64-apple-darwin

# G'MIC CLI binary (the plugin shells out to this; G'MIC-Qt the GUI is
# NOT on Homebrew and is not needed for development of this plugin).
brew install gmic
```

Verify:

```bash
which gmic            # expect /opt/homebrew/bin/gmic on Apple Silicon
gmic version          # expect 3.x or later
which Rez             # expect /usr/bin/Rez (from Xcode Command Line Tools)
xcode-select -p       # if missing, run: xcode-select --install
```

## 2. Adobe Photoshop SDK (optional but recommended)

The repository ships the pipl resource as plain JSON
([PiPLs.json](./PiPLs.json)) that the `Makefile` copies into the bundle as
`Contents/Resources/PiPLs.json`. **You do not need the SDK to build, sign,
install or run the plugin.**

You only need the SDK if you want to:

- Re-verify `FilterRecord` offsets in [src/ps_types.rs](./src/ps_types.rs)
  against `PIFilter.h` (e.g. after a future SDK release changes the layout).
  The current offsets are pinned by `tests/layout.rs`.
- Browse Adobe's sample plug-ins for reference.

### If you do want the SDK

1. Create (or sign into) a free Adobe developer account at
   <https://developer.adobe.com/console>.
2. Find the Photoshop developer area and download **"Adobe Photoshop SDK"**
   for macOS (the 2026 v2 release is the one this repo was developed
   against). The file is a `.zip` of roughly 60-100 MB.
3. Unpack to a stable path. This project defaults to:

   ```
   ~/SDKs/photoshop-sdk/
   ```

   so that you have, for example,
   `~/SDKs/photoshop-sdk/pluginsdk/photoshopapi/photoshop/PIFilter.h`.

### What the SDK is used for here

| File                                                                  | Used by                                |
|-----------------------------------------------------------------------|----------------------------------------|
| `pluginsdk/photoshopapi/photoshop/PIFilter.h`                         | Source of truth for FilterRecord       |
| `pluginsdk/photoshopapi/photoshop/PIGeneral.h`                        | Source of truth for selectors / errors |
| `pluginsdk/pipl-schema.json`                                          | Schema for our PiPLs.json              |
| `pluginsdk/samplecode/filter/colormunger/common/PiPLs.json`           | Reference pipl in modern format        |

## 3. Affinity Photo

1. Install Affinity Photo 2 from the App Store (or Affinity by Canva v3).
2. Open Affinity Photo and go to:
   **Affinity Photo 2 -> Settings -> Photoshop Plugins**.
3. Tick **"Allow unknown plugins to be used"**.
4. Restart Affinity Photo.

The `make install` target copies the built bundle into every detected
Affinity Plugins folder:

```
~/Library/Application Support/Affinity Photo 2/Plugins/
~/Library/Application Support/Affinity/Plugins/        (Affinity Photo v3)
```

Folders whose parent doesn't exist are skipped, so machines with only
one Affinity version installed still work.
