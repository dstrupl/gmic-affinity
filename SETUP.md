# Development setup

These steps prepare a Mac for building `gmic-affinity` from source. They only
need to be done once.

## 1. Toolchain

```bash
# Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Both Apple Silicon and Intel targets (universal binary)
rustup target add aarch64-apple-darwin x86_64-apple-darwin

# G'MIC binary
brew install gmic-qt
```

Verify:

```bash
which gmic            # expect /opt/homebrew/bin/gmic on Apple Silicon
gmic version          # expect 3.x or later
which Rez             # expect /usr/bin/Rez (from Xcode Command Line Tools)
xcode-select -p       # if missing, run: xcode-select --install
```

## 2. Adobe Photoshop SDK

The Photoshop plugin format ships its menu metadata in a `pipl` resource. We
build that resource with Apple's `Rez` tool from a small `.r` source, which
`#include`s three headers from the Adobe Photoshop SDK
(`PIDefines.h`, `PITypes.r`, `PIGeneral.r`).

The SDK is free but gated by an Adobe developer account.

### Steps

1. Create (or sign into) a free Adobe developer account at
   <https://console.adobe.io>.
2. Navigate to **Downloads -> Creative Cloud -> Photoshop** and download the
   latest **Photoshop C++ SDK** for macOS. The file is a `.zip` of roughly
   100-300 MB.
3. Unpack to a stable path. This project defaults to:

   ```
   ~/SDKs/photoshop-sdk/
   ```

   so that after unpacking you should see, for example,
   `~/SDKs/photoshop-sdk/photoshopapi/`.

4. Export `PHOTOSHOP_SDK` in your shell so the `Makefile` can find it:

   ```bash
   # ~/.zshrc (or ~/.bashrc)
   export PHOTOSHOP_SDK="$HOME/SDKs/photoshop-sdk"
   ```

   Then `source ~/.zshrc` (or open a new shell).

### What we use from the SDK

| File                                         | Used by                  |
|----------------------------------------------|--------------------------|
| `photoshopapi/photoshop/PIFilter.h`          | Struct-offset reference  |
| `photoshopapi/resources/PIDefines.h`         | `Rez` include for pipl   |
| `photoshopapi/resources/PITypes.r`           | `Rez` include for pipl   |
| `photoshopapi/resources/PIGeneral.r`         | `Rez` include for pipl   |

If the layout inside your downloaded SDK differs, set
`PHOTOSHOP_SDK_RESOURCES` and/or `PHOTOSHOP_SDK_HEADERS` to the correct
sub-paths and the `Makefile` will pick them up.

### If you cannot install the SDK

Milestones M1, M3, M4, M5, M6 do not strictly require the SDK. Only M2
(pipl resource for the proper menu name "G'MIC...") needs it. Without M2
the plugin still loads, but Affinity will show it under a default category.

## 3. Affinity Photo

1. Install Affinity Photo 2 from the App Store (or Affinity by Canva v3).
2. Open Affinity Photo and go to:
   **Affinity Photo 2 -> Settings -> Photoshop Plugins**.
3. Tick **"Allow unknown plugins to be used"**.
4. Restart Affinity Photo.

The `make install` target copies the built bundle into:

```
~/Library/Application Support/Affinity Photo 2/Plugins/
```
