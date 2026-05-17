# gmic-affinity build / install
#
# Targets:
#   make              -> ARM-only release build assembled into GmicFilter.plugin
#   make bundle       -> same as `make`
#   make universal    -> build aarch64 + x86_64 and lipo into one universal binary
#   make pipl         -> compile GmicFilter.r -> GmicFilter.rsrc (needs PHOTOSHOP_SDK)
#   make install      -> copy GmicFilter.plugin into Affinity Photo 2 Plugins folder
#   make uninstall    -> remove the installed plugin
#   make clean        -> cargo clean + remove bundle artefacts
#
# Configuration via environment:
#   PHOTOSHOP_SDK              path to the unpacked Adobe Photoshop SDK
#                              (default: $HOME/SDKs/photoshop-sdk)
#   PHOTOSHOP_SDK_RESOURCES    override resources include dir if SDK layout differs
#   PHOTOSHOP_SDK_HEADERS      override headers include dir if SDK layout differs
#   AFFINITY_PLUGINS_DIR       override install destination

SHELL := /bin/bash

BUNDLE        := GmicFilter.plugin
BUNDLE_BIN    := $(BUNDLE)/Contents/MacOS/GmicFilter
BUNDLE_PIPL   := $(BUNDLE)/Contents/Resources/PiPLs.json
BUNDLE_RSRC   := $(BUNDLE)/Contents/Resources/GmicFilter.rsrc
BUNDLE_PKGINFO:= $(BUNDLE)/Contents/PkgInfo
BUNDLE_LPROJ  := $(BUNDLE)/Contents/Resources/en.lproj
PLIST         := Info.plist
PIPL_SRC      := PiPLs.json
PIPL_RSRC_SRC := GmicFilter.r
PIPL_RSRC_OUT := GmicFilter.rsrc
LPROJ_SRC     := en.lproj
# Legacy Carbon-style 8-byte PkgInfo: 4-char package type + 4-char creator.
# Modern macOS ignores it, but several Photoshop-plugin hosts probe for it
# as a cheap pre-filter before reading the Mach-O / pipl. It mirrors the
# CFBundlePackageType=8BFM / CFBundleSignature=8BIM keys in Info.plist.
PKGINFO_TEXT  := 8BFM8BIM

# Cargo produces a static archive per target; we then relink it with
# `clang -bundle` to get an MH_BUNDLE Mach-O (filetype 8) that the Adobe /
# Affinity Photoshop-plugin loader will accept. A plain cdylib produces
# MH_DYLIB (filetype 6) and is silently rejected.
LIB_NAME    := libGmicFilter.a
ARM_LIB     := target/aarch64-apple-darwin/release/$(LIB_NAME)
X86_LIB     := target/x86_64-apple-darwin/release/$(LIB_NAME)

# Per-arch bundle Mach-Os, produced by `clang -bundle` from the static libs.
ARM_BUNDLE_BIN := target/aarch64-apple-darwin/release/GmicFilter
X86_BUNDLE_BIN := target/x86_64-apple-darwin/release/GmicFilter

# Lowest macOS we support (matches Info.plist LSMinimumSystemVersion and the
# minimum Affinity Photo 2 system requirement).
MACOSX_DEPLOYMENT_TARGET ?= 11.0

# Bundle link flags. Notes:
#   -bundle                 produce MH_BUNDLE (the only filetype Photoshop /
#                           Affinity hosts will load from a .plugin bundle).
#   -exported_symbol _PluginMain
#                           keep only our SDK entry point in the dynamic
#                           symbol table; everything else is internal.
#   -dead_strip             drop unreachable code/data; harmless for a
#                           single-entry bundle and shrinks the binary.
BUNDLE_LDFLAGS := \
  -bundle \
  -mmacosx-version-min=$(MACOSX_DEPLOYMENT_TARGET) \
  -Wl,-exported_symbol,_PluginMain \
  -Wl,-dead_strip

# Cargo features. The default build is a safe no-op PluginMain; set
# `make ... FEATURES=live` to enable the real pixel pipeline (M3 + M4).
# The FilterRecord layout is now SDK-verified (see tests/layout.rs), so
# `FEATURES=live` is safe to install whenever you want the gmic pipeline.
FEATURES ?=
CARGO_FEATURE_ARGS := $(if $(FEATURES),--features $(FEATURES),)

# Default install path is Affinity Photo 2. The Affinity v3 path is
#   $(HOME)/Library/Application Support/Affinity/Plugins
# but v3 is not the M1 target.
AFFINITY_PLUGINS_DIR ?= $(HOME)/Library/Application Support/Affinity Photo 2/Plugins

# Path to the unpacked Adobe Photoshop SDK. Required for two things:
#   1. Compiling the legacy pipl resource (GmicFilter.r -> .rsrc) — without
#      the binary pipl Affinity Photo 2 silently rejects the bundle during
#      enumeration, even though we also ship the modern PiPLs.json (which
#      only newer Adobe Photoshop / GIMP know about). AKVIS, Topaz, Nik etc.
#      all ship this legacy .rsrc; this is why they work in Affinity.
#   2. Re-verifying FilterRecord layout against PIFilter.h (see SETUP.md
#      and tests/layout.rs).
PHOTOSHOP_SDK ?= $(HOME)/SDKs/photoshop-sdk

# Rez source / header search paths inside the SDK. Override if your SDK
# layout differs from the 2026 release (which nests everything under
# pluginsdk/).
REZ_RESOURCES ?= $(PHOTOSHOP_SDK)/pluginsdk/photoshopapi/resources
REZ_HEADERS   ?= $(PHOTOSHOP_SDK)/pluginsdk/photoshopapi/photoshop
REZ_COMMON    ?= $(PHOTOSHOP_SDK)/pluginsdk/samplecode/common/includes

# Rez invocation:
#   -i …           include search dirs (Rez doesn't honour CPATH).
#   -d __PIMac__=1 picks the mac-host branch of the SDK's #ifdefs.
#   -d PRAGMA_ONCE=0 silences `#if PRAGMA_ONCE` in PIResDefines.h — the
#                  macro is set by Carbon's MacTypes.r, which Apple
#                  stopped shipping in the Command Line Tools.
#   -useDF         emit the data fork (modern macOS doesn't keep resource
#                  forks on APFS, and CFBundle reads the data fork).
REZ_FLAGS := \
  -i "$(REZ_RESOURCES)" \
  -i "$(REZ_HEADERS)" \
  -i "$(REZ_COMMON)" \
  -d "__PIMac__=1" \
  -d "PRAGMA_ONCE=0" \
  -useDF

.PHONY: all bundle universal universal-install install uninstall clean test fmt clippy help pipl

all: bundle

help:
	@echo "Targets:"
	@echo "  make bundle             Build ARM-only GmicFilter.plugin (no-op PluginMain)."
	@echo "  make universal          Build universal (arm64 + x86_64) GmicFilter.plugin."
	@echo "  make install            Install GmicFilter.plugin into Affinity Photo 2."
	@echo "  make universal-install  Convenience: 'make universal' then 'make install'."
	@echo "  make uninstall          Remove the installed plugin."
	@echo "  make test               Run cargo test under both default and --features live."
	@echo "  make clippy             Run cargo clippy --all-targets --all-features -D warnings."
	@echo "  make fmt                Run cargo fmt."
	@echo "  make clean              Remove build artefacts."
	@echo ""
	@echo "Feature flag:"
	@echo "  FEATURES=live           Real M3/M4 PluginMain (TIFF round-trip via gmic)."
	@echo "                          Example: make universal-install FEATURES=live"
	@echo ""
	@echo "Environment:"
	@echo "  AFFINITY_PLUGINS_DIR    install dest (default: Affinity Photo 2 Plugins)"
	@echo "  PHOTOSHOP_SDK           SDK root, optional (default: \$$HOME/SDKs/photoshop-sdk)"

# Fast iteration: ARM-only build assembled into the plugin bundle.
bundle: $(ARM_BUNDLE_BIN) $(PIPL_RSRC_OUT)
	@mkdir -p "$(BUNDLE)/Contents/MacOS" "$(BUNDLE)/Contents/Resources"
	cp "$(ARM_BUNDLE_BIN)" "$(BUNDLE_BIN)"
	chmod +x "$(BUNDLE_BIN)"
	cp "$(PLIST)" "$(BUNDLE)/Contents/Info.plist"
	cp "$(PIPL_SRC)" "$(BUNDLE_PIPL)"
	cp "$(PIPL_RSRC_OUT)" "$(BUNDLE_RSRC)"
	printf '%s' '$(PKGINFO_TEXT)' > "$(BUNDLE_PKGINFO)"
	rm -rf "$(BUNDLE_LPROJ)" && cp -R "$(LPROJ_SRC)" "$(BUNDLE_LPROJ)"
	codesign --force --deep --sign - "$(BUNDLE)"
	@$(MAKE) --no-print-directory verify-bundle

universal: $(ARM_BUNDLE_BIN) $(X86_BUNDLE_BIN) $(PIPL_RSRC_OUT)
	@mkdir -p "$(BUNDLE)/Contents/MacOS" "$(BUNDLE)/Contents/Resources"
	lipo -create "$(ARM_BUNDLE_BIN)" "$(X86_BUNDLE_BIN)" -output "$(BUNDLE_BIN)"
	chmod +x "$(BUNDLE_BIN)"
	cp "$(PLIST)" "$(BUNDLE)/Contents/Info.plist"
	cp "$(PIPL_SRC)" "$(BUNDLE_PIPL)"
	cp "$(PIPL_RSRC_OUT)" "$(BUNDLE_RSRC)"
	printf '%s' '$(PKGINFO_TEXT)' > "$(BUNDLE_PKGINFO)"
	rm -rf "$(BUNDLE_LPROJ)" && cp -R "$(LPROJ_SRC)" "$(BUNDLE_LPROJ)"
	codesign --force --deep --sign - "$(BUNDLE)"
	@$(MAKE) --no-print-directory verify-bundle

# Compile the legacy pipl resource from source. The SDK include paths
# come from REZ_FLAGS above and so respect PHOTOSHOP_SDK / REZ_HEADERS
# overrides. We deliberately depend on the source so editing it triggers
# a rebuild of every bundle target.
#
# CI (and contributors without the Adobe SDK installed) need to build the
# bundle too. To keep them unblocked we also commit the produced
# `GmicFilter.rsrc` to the repo and fall back to the committed copy
# whenever the SDK is missing — but only if the committed `.rsrc` is at
# least as fresh as the `.r` source (so editing the source without an
# SDK still fails loudly instead of shipping stale metadata).
pipl $(PIPL_RSRC_OUT): $(PIPL_RSRC_SRC)
	@if [ -d "$(REZ_RESOURCES)" ]; then \
	  echo "/usr/bin/Rez $(REZ_FLAGS) -o $(PIPL_RSRC_OUT) $(PIPL_RSRC_SRC)"; \
	  /usr/bin/Rez $(REZ_FLAGS) -o "$(PIPL_RSRC_OUT)" "$(PIPL_RSRC_SRC)"; \
	  echo "Compiled $(PIPL_RSRC_SRC) -> $(PIPL_RSRC_OUT) ($$(stat -f %z "$(PIPL_RSRC_OUT)") bytes)"; \
	elif [ -f "$(PIPL_RSRC_OUT)" ] && [ "$(PIPL_RSRC_OUT)" -nt "$(PIPL_RSRC_SRC)" -o ! "$(PIPL_RSRC_OUT)" -ot "$(PIPL_RSRC_SRC)" ]; then \
	  echo "Adobe Photoshop SDK not found at $(PHOTOSHOP_SDK); reusing committed $(PIPL_RSRC_OUT)."; \
	  touch "$(PIPL_RSRC_OUT)"; \
	else \
	  echo "ERROR: Adobe Photoshop SDK not found at $(PHOTOSHOP_SDK) and no usable $(PIPL_RSRC_OUT)."; \
	  echo "       Either install the SDK (see SETUP.md) or revert your $(PIPL_RSRC_SRC) edits."; \
	  exit 1; \
	fi

# Note: the architecture-specific outputs are intentionally PHONY because the
# enabled feature set changes their contents but not their path; we always
# want cargo to re-evaluate.
.PHONY: $(ARM_LIB) $(X86_LIB) $(ARM_BUNDLE_BIN) $(X86_BUNDLE_BIN) verify-bundle

$(ARM_LIB):
	cargo build --release --target aarch64-apple-darwin $(CARGO_FEATURE_ARGS)

$(X86_LIB):
	cargo build --release --target x86_64-apple-darwin $(CARGO_FEATURE_ARGS)

# Relink the per-arch static archive into a Mach-O bundle. We use clang as
# the link driver so it brings in the right SDK sysroot, libSystem, etc.
# `-Wl,-force_load,...` ensures every object in the archive is pulled in,
# even if the linker doesn't see a direct reference (Rust's std contains
# init/dtor objects we don't want silently dropped).
$(ARM_BUNDLE_BIN): $(ARM_LIB)
	clang $(BUNDLE_LDFLAGS) -arch arm64 \
	  -Wl,-force_load,"$(ARM_LIB)" \
	  -o "$@"

$(X86_BUNDLE_BIN): $(X86_LIB)
	clang $(BUNDLE_LDFLAGS) -arch x86_64 \
	  -Wl,-force_load,"$(X86_LIB)" \
	  -o "$@"

# Belt-and-braces check: every Photoshop-plugin host expects MH_BUNDLE
# (filetype 8). If our binary is anything else (MH_DYLIB = 6 was the M1
# regression) the bundle will be silently rejected during enumeration.
verify-bundle:
	@for slice in $$(lipo -archs "$(BUNDLE_BIN)" 2>/dev/null); do \
	  ftype=$$(otool -h -arch $$slice "$(BUNDLE_BIN)" | awk '/^ 0x/ {print $$5; exit}'); \
	  if [ "$$ftype" != "8" ]; then \
	    echo "ERROR: $(BUNDLE_BIN) slice $$slice has Mach-O filetype $$ftype (expected 8 = MH_BUNDLE)"; \
	    exit 1; \
	  fi; \
	done; \
	echo "verify-bundle: $(BUNDLE_BIN) is MH_BUNDLE on all slices."

install: bundle
	@mkdir -p "$(AFFINITY_PLUGINS_DIR)"
	rm -rf "$(AFFINITY_PLUGINS_DIR)/$(BUNDLE)"
	cp -R "$(BUNDLE)" "$(AFFINITY_PLUGINS_DIR)/"
	@echo "Installed to $(AFFINITY_PLUGINS_DIR)/$(BUNDLE)"
	@echo "Restart Affinity Photo 2 to pick up the change."

universal-install: universal
	@mkdir -p "$(AFFINITY_PLUGINS_DIR)"
	rm -rf "$(AFFINITY_PLUGINS_DIR)/$(BUNDLE)"
	cp -R "$(BUNDLE)" "$(AFFINITY_PLUGINS_DIR)/"
	@echo "Installed (universal) to $(AFFINITY_PLUGINS_DIR)/$(BUNDLE)"
	@echo "Restart Affinity Photo 2 to pick up the change."

uninstall:
	rm -rf "$(AFFINITY_PLUGINS_DIR)/$(BUNDLE)"
	@echo "Removed $(AFFINITY_PLUGINS_DIR)/$(BUNDLE) (if it existed)."

test:
	cargo test $(CARGO_FEATURE_ARGS)
	cargo test --features live

fmt:
	cargo fmt

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

clean:
	cargo clean
	rm -rf "$(BUNDLE)" "$(PIPL_RSRC_OUT)"
