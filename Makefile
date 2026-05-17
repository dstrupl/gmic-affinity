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

BUNDLE      := GmicFilter.plugin
BUNDLE_BIN  := $(BUNDLE)/Contents/MacOS/GmicFilter
BUNDLE_PIPL := $(BUNDLE)/Contents/Resources/PiPLs.json
PLIST       := Info.plist
PIPL_SRC    := PiPLs.json

LIB_NAME    := libGmicFilter.dylib
ARM_LIB     := target/aarch64-apple-darwin/release/$(LIB_NAME)
X86_LIB     := target/x86_64-apple-darwin/release/$(LIB_NAME)

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

# Path to the unpacked Adobe Photoshop SDK. Only needed if you want to
# re-verify FilterRecord layout against PIFilter.h (see SETUP.md and
# tests/layout.rs). The pipl resource is now plain JSON committed in this
# repo (PiPLs.json), so the SDK is no longer a build dependency.
PHOTOSHOP_SDK ?= $(HOME)/SDKs/photoshop-sdk

.PHONY: all bundle universal universal-install install uninstall clean test fmt clippy help

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
bundle: $(ARM_LIB)
	@mkdir -p "$(BUNDLE)/Contents/MacOS" "$(BUNDLE)/Contents/Resources"
	cp "$(ARM_LIB)" "$(BUNDLE_BIN)"
	chmod +x "$(BUNDLE_BIN)"
	cp "$(PLIST)" "$(BUNDLE)/Contents/Info.plist"
	cp "$(PIPL_SRC)" "$(BUNDLE_PIPL)"
	codesign --force --deep --sign - "$(BUNDLE)"

universal: $(ARM_LIB) $(X86_LIB)
	@mkdir -p "$(BUNDLE)/Contents/MacOS" "$(BUNDLE)/Contents/Resources"
	lipo -create "$(ARM_LIB)" "$(X86_LIB)" -output "$(BUNDLE_BIN)"
	chmod +x "$(BUNDLE_BIN)"
	cp "$(PLIST)" "$(BUNDLE)/Contents/Info.plist"
	cp "$(PIPL_SRC)" "$(BUNDLE_PIPL)"
	codesign --force --deep --sign - "$(BUNDLE)"

# Note: the architecture-specific outputs are intentionally PHONY because the
# enabled feature set changes their contents but not their path; we always
# want cargo to re-evaluate.
.PHONY: $(ARM_LIB) $(X86_LIB)

$(ARM_LIB):
	cargo build --release --target aarch64-apple-darwin $(CARGO_FEATURE_ARGS)

$(X86_LIB):
	cargo build --release --target x86_64-apple-darwin $(CARGO_FEATURE_ARGS)

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
	rm -rf "$(BUNDLE)"
