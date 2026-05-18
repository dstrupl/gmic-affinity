# gmic-affinity build / install
#
# Targets:
#   make              -> ARM-only release build assembled into GmicFilter.plugin
#   make bundle       -> same as `make`
#   make universal    -> build aarch64 + x86_64 and lipo into one universal binary
#   make pipl         -> compile GmicFilter.r -> GmicFilter.rsrc (needs PHOTOSHOP_SDK)
#   make install      -> copy GmicFilter.plugin into every detected Affinity Plugins folder
#   make uninstall    -> remove the installed plugin
#   make clean        -> cargo clean + remove bundle artefacts
#
# Configuration via environment:
#   PHOTOSHOP_SDK              path to the unpacked Adobe Photoshop SDK
#                              (default: $HOME/SDKs/photoshop-sdk)
#   PHOTOSHOP_SDK_RESOURCES    override resources include dir if SDK layout differs
#   PHOTOSHOP_SDK_HEADERS      override headers include dir if SDK layout differs
#   AFFINITY_PLUGINS_DIRS      comma-separated list of install destinations
#                              (default: Affinity Photo 2 + Affinity v3)
#   AFFINITY_PLUGINS_DIR       legacy singular override; if set, replaces
#                              AFFINITY_PLUGINS_DIRS (back-compat)

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
#   -lobjc                   Objective-C runtime (required for objc2 /
#                           AppKit UI code in the `live` feature).
#   -framework AppKit       resolves the few real C symbols we touch
#                           that aren't loaded through the ObjC runtime
#                           class machinery (NSApp* class lookups go via
#                           objc_getClass and stay lazy, but constants
#                           like NSEventTrackingRunLoopMode are real
#                           extern C data and must be bound at link time).
#                           Implicitly pulls Foundation in.
BUNDLE_LDFLAGS := \
  -bundle \
  -mmacosx-version-min=$(MACOSX_DEPLOYMENT_TARGET) \
  -Wl,-exported_symbol,_PluginMain \
  -Wl,-dead_strip \
  -lobjc \
  -framework AppKit

# Cargo features. The default build is a safe no-op PluginMain; set
# `make ... FEATURES=live` to enable the real pixel pipeline (M3 + M4).
# The FilterRecord layout is now SDK-verified (see tests/layout.rs), so
# `FEATURES=live` is safe to install whenever you want the gmic pipeline.
FEATURES ?=
# Deferred (=) on purpose: target-scoped overrides like
#   release: FEATURES := live
# need this to re-evaluate at recipe expansion time. With `:=` here the
# string is locked in at parse time and the override is silently
# ignored (which would ship a no-op release zip).
CARGO_FEATURE_ARGS = $(if $(FEATURES),--features $(FEATURES),)

# Comma-separated list of Affinity Plugins folders. The install /
# universal-install / uninstall targets iterate over the list and operate
# on every entry whose *parent* directory exists (proxy for "this version
# of Affinity is installed"). Entries whose parent is missing are
# skipped silently.
#
# v0.1 ships for both Affinity Photo 2 and Affinity by Canva v3 (see
# docs/design/2026-05-18-release-v0.1-distribution.md §4.1).
#
# The legacy singular AFFINITY_PLUGINS_DIR is honoured for backwards
# compatibility: if it is set explicitly (env or make-arg), it overrides
# AFFINITY_PLUGINS_DIRS so existing developer workflows keep working.
AFFINITY_PLUGINS_DIRS ?= $(HOME)/Library/Application Support/Affinity Photo 2/Plugins,$(HOME)/Library/Application Support/Affinity/Plugins
ifdef AFFINITY_PLUGINS_DIR
EFFECTIVE_PLUGINS_DIRS := $(AFFINITY_PLUGINS_DIR)
else
EFFECTIVE_PLUGINS_DIRS := $(AFFINITY_PLUGINS_DIRS)
endif

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

.PHONY: all bundle universal universal-install install uninstall clean test fmt clippy help pipl check-lfs refresh-catalogue picker-example audit-unsupported release

all: bundle

help:
	@echo "Targets:"
	@echo "  make bundle             Build ARM-only GmicFilter.plugin (no-op PluginMain)."
	@echo "  make universal          Build universal (arm64 + x86_64) GmicFilter.plugin."
	@echo "  make install            Install GmicFilter.plugin into every detected Affinity Plugins folder."
	@echo "  make universal-install  Convenience: 'make universal' then 'make install'."
	@echo "  make uninstall          Remove the installed plugin."
	@echo "  make picker-example     Open the picker standalone (no Affinity install needed)."
	@echo "  make refresh-catalogue  Regenerate assets/gmic-catalogue.* from local gmic."
	@echo "  make audit-unsupported  Histogram of catalogue params still missing a parser."
	@echo "  make test               Run cargo test under both default and --features live."
	@echo "  make clippy             Run cargo clippy --all-targets --all-features -D warnings."
	@echo "  make fmt                Run cargo fmt."
	@echo "  make release            Build the universal bundle and assemble dist/GmicFilter-<ver>.zip."
	@echo "  make clean              Remove build artefacts."
	@echo ""
	@echo "Feature flag:"
	@echo "  FEATURES=live           Real M3/M4 PluginMain (TIFF round-trip via gmic)."
	@echo "                          Example: make universal-install FEATURES=live"
	@echo ""
	@echo "Environment:"
	@echo "  AFFINITY_PLUGINS_DIRS   comma-separated install dests (default: Affinity Photo 2 + v3)"
	@echo "  AFFINITY_PLUGINS_DIR    legacy singular override; replaces the list if set"
	@echo "  PHOTOSHOP_SDK           SDK root, optional (default: \$$HOME/SDKs/photoshop-sdk)"

# Fail fast if the LFS-tracked catalogue snapshot is still a pointer
# file. Without this guard `include_bytes!("../assets/gmic-catalogue.gmic.gz")`
# would compile against the LFS pointer text ("version
# https://git-lfs.github.com/spec/v1\noid sha256:…") instead of a real
# gzip stream, the catalogue would parse to zero filters at runtime,
# and the picker would open empty with no obvious explanation. A real
# gzip file always starts with the two magic bytes 0x1f 0x8b.
check-lfs:
	@head -c 2 assets/gmic-catalogue.gmic.gz 2>/dev/null | od -An -tx1 | tr -d ' \n' | \
	  grep -q '^1f8b' || { \
	    echo ""; \
	    echo "ERROR: assets/gmic-catalogue.gmic.gz is not a real gzip file."; \
	    echo "       Looks like Git LFS hasn't pulled it. Run:"; \
	    echo ""; \
	    echo "           git lfs install   # one-time per machine"; \
	    echo "           git lfs pull"; \
	    echo ""; \
	    exit 1; }

# Fast iteration: ARM-only build assembled into the plugin bundle.
bundle: check-lfs $(ARM_BUNDLE_BIN) $(PIPL_RSRC_OUT)
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

universal: check-lfs $(ARM_BUNDLE_BIN) $(X86_BUNDLE_BIN) $(PIPL_RSRC_OUT)
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

# Bash helper used by install / universal-install / uninstall. We expand
# the comma-separated list inline rather than storing it in $(foreach) so
# directory entries that contain spaces (they all do — "Affinity Photo 2"
# / "Application Support") survive correctly.
# Note on shell-loop quoting: we assign the make-expanded list to a
# shell variable first, then iterate over the *unquoted parameter
# expansion* `$$dirs`. This is deliberate. `for dir in $(VAR)` lets
# Make interpolate the value into the script source, and the shell
# parser then tokenises on whitespace at parse time, breaking
# directories that contain spaces (every Affinity path does:
# "Application Support", "Affinity Photo 2", ...). Word-splitting on
# parameter expansions, by contrast, uses IFS at runtime, so
# `IFS=','; for dir in $$dirs;` splits cleanly on commas only.
define INSTALL_BUNDLE_TO_TARGETS
	@set -e; \
	dirs='$(EFFECTIVE_PLUGINS_DIRS)'; \
	IFS=','; \
	installed=0; \
	skipped=0; \
	for dir in $$dirs; do \
	  parent="$$(dirname "$$dir")"; \
	  if [ -d "$$parent" ]; then \
	    mkdir -p "$$dir"; \
	    rm -rf "$$dir/$(BUNDLE)"; \
	    cp -R "$(BUNDLE)" "$$dir/"; \
	    echo "Installed: $$dir/$(BUNDLE)"; \
	    installed=$$((installed + 1)); \
	  else \
	    echo "Skipped (no $$parent): $$dir"; \
	    skipped=$$((skipped + 1)); \
	  fi; \
	done; \
	if [ "$$installed" -eq 0 ]; then \
	  echo ""; \
	  echo "WARNING: no Affinity install detected under any of:"; \
	  for dir in $$dirs; do echo "  - $$dir"; done; \
	  echo "Install Affinity Photo 2 or v3, or set AFFINITY_PLUGINS_DIR explicitly."; \
	  exit 2; \
	fi; \
	echo "Restart Affinity to pick up the change."
endef

define UNINSTALL_BUNDLE_FROM_TARGETS
	@set -e; \
	dirs='$(EFFECTIVE_PLUGINS_DIRS)'; \
	IFS=','; \
	for dir in $$dirs; do \
	  if [ -e "$$dir/$(BUNDLE)" ]; then \
	    rm -rf "$$dir/$(BUNDLE)"; \
	    echo "Removed: $$dir/$(BUNDLE)"; \
	  else \
	    echo "Not present: $$dir/$(BUNDLE)"; \
	  fi; \
	done
endef

install: bundle
	$(INSTALL_BUNDLE_TO_TARGETS)

universal-install: universal
	$(INSTALL_BUNDLE_TO_TARGETS)

uninstall:
	$(UNINSTALL_BUNDLE_FROM_TARGETS)

test:
	cargo test $(CARGO_FEATURE_ARGS)
	cargo test --features live

fmt:
	cargo fmt

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

clean:
	cargo clean
	rm -rf "$(BUNDLE)" "$(PIPL_RSRC_OUT)" "$(DIST_DIR)"

# -------- release packaging --------
#
# `make release` builds the universal bundle (FEATURES=live by default —
# this is what end users want; override on the command line if needed)
# and assembles the distribution zip used by both the GitHub release
# and the Homebrew tap. Layout matches §2 of
# docs/design/2026-05-18-release-v0.1-distribution.md:
#
#   dist/GmicFilter-<version>.zip
#   └── GmicFilter-<version>/
#       ├── GmicFilter.plugin/
#       ├── install.command
#       └── README.txt
#
# RELEASE_VERSION defaults to `git describe`. CI overrides it with the
# tag name (e.g. v0.1.0); for local dry runs it picks up something like
# `v0.1.0-2-gabcdef0` which is fine for testing the layout.
RELEASE_VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
DIST_DIR        := dist
RELEASE_NAME    := GmicFilter-$(RELEASE_VERSION)
RELEASE_STAGE   := $(DIST_DIR)/$(RELEASE_NAME)
RELEASE_ZIP     := $(DIST_DIR)/$(RELEASE_NAME).zip

# `make release` always uses the live feature: this is the artifact end
# users install. Override on the command line only if you know why.
release: FEATURES := live
release: universal
	@if [ ! -f install.command ]; then \
	  echo "ERROR: install.command missing at repo root."; exit 1; \
	fi
	@if [ ! -f release/README.txt ]; then \
	  echo "ERROR: release/README.txt missing."; exit 1; \
	fi
	@rm -rf "$(RELEASE_STAGE)" "$(RELEASE_ZIP)"
	@mkdir -p "$(RELEASE_STAGE)"
	cp -R "$(BUNDLE)" "$(RELEASE_STAGE)/"
	cp install.command "$(RELEASE_STAGE)/install.command"
	cp release/README.txt "$(RELEASE_STAGE)/README.txt"
	chmod +x "$(RELEASE_STAGE)/install.command"
	# Strip Finder / quarantine / pre-existing xattrs from the staged
	# tree before zipping. Code-signing data for .plugin bundles lives in
	# Contents/_CodeSignature/ (real files), not in xattrs, so this is
	# safe and keeps the resulting zip free of `._*` AppleDouble
	# sidecars that confuse Linux/Windows users who unzip out of band.
	xattr -rc "$(RELEASE_STAGE)"
	# ditto, not zip(1), so symlinks inside the .plugin bundle survive
	# round-tripping cleanly. -c -k = create, PKZip-compatible.
	# --keepParent preserves the GmicFilter-<ver>/ top-level directory
	# inside the zip. --norsrc / --noextattr / --noqtn / --noacl
	# guarantee no AppleDouble files even if a fresh xattr lands on the
	# tree between `xattr -rc` and `ditto`.
	cd "$(DIST_DIR)" && ditto -c -k --keepParent \
	  --norsrc --noextattr --noqtn --noacl \
	  "$(RELEASE_NAME)" "$(RELEASE_NAME).zip"
	@echo ""
	@echo "Built $(RELEASE_ZIP)"
	@echo "Size:    $$(stat -f %z "$(RELEASE_ZIP)") bytes"
	@echo "SHA256:  $$(shasum -a 256 "$(RELEASE_ZIP)" | awk '{print $$1}')"
	@echo ""
	@echo "Cask url:  https://github.com/dstrupl/gmic-affinity/releases/download/$(RELEASE_VERSION)/$(RELEASE_NAME).zip"

# Regenerate the bundled catalogue snapshot from the locally installed
# gmic. Pipeline:
#   1. `gmic update` refreshes `~/.config/gmic/update<ver>.gmic`, which
#      is the canonical source for #@gui annotations driving the picker.
#   2. gzip -9 it into `assets/gmic-catalogue.gmic.gz` (the file we
#      `include_bytes!` and the file LFS tracks).
#   3. Re-dump the catalogue TOC so reviewers can diff it textually.
#   4. Record the source gmic version + timestamp so we can detect
#      when the snapshot has drifted out of sync.
#   5. Run the catalogue snapshot smoke test to make sure parsing the
#      regenerated snapshot still yields the expected anchor filters.
# Always commit changes from this target as a separate, machine-only
# patch — never bundle them with feature work.
refresh-catalogue:
	git lfs install
	gmic update >/dev/null 2>&1
	@UPDATE_FILE=$$(ls -t $$HOME/.config/gmic/update*.gmic 2>/dev/null | head -n1); \
	 if [ -z "$$UPDATE_FILE" ]; then \
	   echo "ERROR: no update*.gmic in ~/.config/gmic — is gmic installed?"; exit 1; \
	 fi; \
	 gzip -9 -c "$$UPDATE_FILE" > assets/gmic-catalogue.gmic.gz
	cargo run --bin dump-toc > assets/gmic-catalogue.toc.txt
	printf '%s\n%s\n' \
	  "$$(gmic --version 2>&1 | head -n1)" \
	  "$$(date -u +%FT%TZ)" \
	  > assets/gmic-catalogue.version.txt
	cargo test --test catalogue_snapshot
	@echo ""; echo "refresh-catalogue: changes in assets/:"; \
	 git status -- assets/

# Run the standalone Cocoa picker without installing the plugin. Forces
# `--release` because the picker's manual modal-session pump trips an
# objc2 debug-mode type-encoding panic on `beginModalSessionForWindow:`
# (see examples/picker.rs header).
picker-example: check-lfs
	cargo run --release --example picker --features live

# Diagnose which gmic parameter syntaxes our parser still maps to
# `ParamKind::Unknown`. Source-of-truth for prioritising new
# `parse_*` arms in src/catalogue/parser.rs. The bundled v3.7.6
# snapshot resolves to 0 unsupported params; a non-zero count after
# `make refresh-catalogue` is the signal to look at this output and
# either add a new arm or widen the
# `bundled_catalogue_has_no_unsupported_params` test deliberately.
audit-unsupported: check-lfs
	@cargo run --quiet --bin audit-unsupported
