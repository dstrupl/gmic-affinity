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
BUNDLE_RSRC := $(BUNDLE)/Contents/Resources/GmicFilter.rsrc
PLIST       := Info.plist
RSRC_SRC    := GmicFilter.r
RSRC_OUT    := GmicFilter.rsrc

LIB_NAME    := libGmicFilter.dylib
ARM_LIB     := target/aarch64-apple-darwin/release/$(LIB_NAME)
X86_LIB     := target/x86_64-apple-darwin/release/$(LIB_NAME)

# Cargo features. The default build is a safe no-op PluginMain; set
# `make ... FEATURES=live` once the FilterRecord layout is reconciled
# against PIFilter.h to enable the real pixel pipeline.
FEATURES ?=
CARGO_FEATURE_ARGS := $(if $(FEATURES),--features $(FEATURES),)

# Default install path is Affinity Photo 2. The Affinity v3 path is
#   $(HOME)/Library/Application Support/Affinity/Plugins
# but v3 is not the M1 target.
AFFINITY_PLUGINS_DIR ?= $(HOME)/Library/Application Support/Affinity Photo 2/Plugins

PHOTOSHOP_SDK            ?= $(HOME)/SDKs/photoshop-sdk
PHOTOSHOP_SDK_RESOURCES  ?= $(PHOTOSHOP_SDK)/photoshopapi/resources
PHOTOSHOP_SDK_HEADERS    ?= $(PHOTOSHOP_SDK)/photoshopapi/photoshop

.PHONY: all bundle universal universal-install pipl install uninstall clean test fmt clippy help

all: bundle

help:
	@echo "Targets:"
	@echo "  make bundle             Build ARM-only GmicFilter.plugin (no-op PluginMain)."
	@echo "  make universal          Build universal (arm64 + x86_64) GmicFilter.plugin."
	@echo "  make install            Install GmicFilter.plugin into Affinity Photo 2."
	@echo "  make universal-install  Convenience: 'make universal' then 'make install'."
	@echo "  make uninstall          Remove the installed plugin."
	@echo "  make pipl               Build GmicFilter.rsrc from GmicFilter.r (needs SDK)."
	@echo "  make test               Run cargo test under both default and --features live."
	@echo "  make clippy             Run cargo clippy --all-targets --all-features -D warnings."
	@echo "  make fmt                Run cargo fmt."
	@echo "  make clean              Remove build artefacts."
	@echo ""
	@echo "Feature flag:"
	@echo "  FEATURES=live           Real M3/M4 PluginMain (only after layout reconciled)."
	@echo "                          Example: make universal FEATURES=live"
	@echo ""
	@echo "Environment:"
	@echo "  PHOTOSHOP_SDK             SDK root (default: \$$HOME/SDKs/photoshop-sdk)"
	@echo "  AFFINITY_PLUGINS_DIR      install dest (default: Affinity Photo 2 Plugins)"

# Fast iteration: ARM-only build assembled into the plugin bundle.
bundle: $(ARM_LIB)
	@mkdir -p "$(BUNDLE)/Contents/MacOS" "$(BUNDLE)/Contents/Resources"
	cp "$(ARM_LIB)" "$(BUNDLE_BIN)"
	chmod +x "$(BUNDLE_BIN)"
	cp "$(PLIST)" "$(BUNDLE)/Contents/Info.plist"
	@if [ -f "$(RSRC_OUT)" ]; then \
	  cp "$(RSRC_OUT)" "$(BUNDLE_RSRC)"; \
	else \
	  echo "[bundle] note: no $(RSRC_OUT) found; run 'make pipl' for proper menu metadata"; \
	fi
	codesign --force --deep --sign - "$(BUNDLE)"

universal: $(ARM_LIB) $(X86_LIB)
	@mkdir -p "$(BUNDLE)/Contents/MacOS" "$(BUNDLE)/Contents/Resources"
	lipo -create "$(ARM_LIB)" "$(X86_LIB)" -output "$(BUNDLE_BIN)"
	chmod +x "$(BUNDLE_BIN)"
	cp "$(PLIST)" "$(BUNDLE)/Contents/Info.plist"
	@if [ -f "$(RSRC_OUT)" ]; then \
	  cp "$(RSRC_OUT)" "$(BUNDLE_RSRC)"; \
	else \
	  echo "[universal] note: no $(RSRC_OUT) found; run 'make pipl' for proper menu metadata"; \
	fi
	codesign --force --deep --sign - "$(BUNDLE)"

# Note: the architecture-specific outputs are intentionally PHONY because the
# enabled feature set changes their contents but not their path; we always
# want cargo to re-evaluate.
.PHONY: $(ARM_LIB) $(X86_LIB)

$(ARM_LIB):
	cargo build --release --target aarch64-apple-darwin $(CARGO_FEATURE_ARGS)

$(X86_LIB):
	cargo build --release --target x86_64-apple-darwin $(CARGO_FEATURE_ARGS)

pipl:
	@if [ ! -d "$(PHOTOSHOP_SDK_RESOURCES)" ]; then \
	  echo "ERROR: Photoshop SDK resources dir not found at $(PHOTOSHOP_SDK_RESOURCES)"; \
	  echo "       See SETUP.md to install the Adobe Photoshop SDK."; \
	  exit 1; \
	fi
	Rez -o "$(RSRC_OUT)" "$(RSRC_SRC)" -i "$(PHOTOSHOP_SDK_RESOURCES)"

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
	rm -rf "$(BUNDLE)" "$(RSRC_OUT)"
