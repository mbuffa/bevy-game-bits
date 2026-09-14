# bevy-game-bits — command catalog.
#
# Run `make help` (or just `make`) to list every target. This file is the one
# place command-line rules for this repo live; see CLAUDE.md's "Web Builds"
# section for the reasoning behind the wasm targets.

.DEFAULT_GOAL := help
SHELL := /bin/bash

# --- Example discovery -------------------------------------------------
# Single-file examples/NNN-name.rs, plus directory examples/NNN-name/main.rs.
# New examples need no edit here — they're picked up automatically.
# (Not `$(wildcard examples/*/)`: on macOS's glob(3) a trailing-slash pattern
# doesn't reliably filter to directories only, so this uses `find` instead.)
EXAMPLES     := $(sort $(notdir $(basename $(wildcard examples/*.rs))) \
                $(shell find examples -mindepth 1 -maxdepth 1 -type d -exec basename {} \;))

# Examples that can't (or shouldn't) run on web, one per line with a reason.
# 005-wireframe-2d requests WgpuFeatures::POLYGON_MODE_LINE, which its own
# top-of-file comment says is native-only (DX12/Vulkan/Metal, not web).
WEB_SKIP     := 005-wireframe-2d
WEB_EXAMPLES := $(filter-out $(WEB_SKIP),$(EXAMPLES))

DIST         := dist

.PHONY: help list \
	run-% build build-release test fmt fmt-check check lint \
	wasm-prereqs web-run-% web-% web-all check-assets serve \
	clean clean-web

help: ## Show this list of targets.
	@echo "bevy-game-bits — available targets:"
	@echo
	@awk 'BEGIN {FS = ":.*##"} /^[a-zA-Z0-9_.%-]+:.*##/ { printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@echo
	@echo "Examples (native): $(EXAMPLES)"
	@echo "Examples (web):    $(WEB_EXAMPLES)"
	@echo "Skipped from web:  $(WEB_SKIP)"

list: ## Print the discovered example lists (native / web / skipped).
	@echo "EXAMPLES:     $(EXAMPLES)"
	@echo "WEB_EXAMPLES: $(WEB_EXAMPLES)"
	@echo "WEB_SKIP:     $(WEB_SKIP)"

# --- Native --------------------------------------------------------------

run-%: ## run-<name>: Run one example natively, e.g. `make run-009-world-map`.
	cargo run --example $*

build: ## Debug build of the library and every example.
	cargo build

build-release: ## Release build of the library and every example.
	cargo build --release

test: ## Run the test suite (cargo test).
	cargo test

fmt: ## Apply rustfmt to the whole crate.
	cargo fmt

fmt-check: ## Check formatting without writing (what CI would run).
	cargo fmt -- --check

check: ## Fast type-check without codegen.
	cargo check --all-targets

lint: ## Run Bevy-specific lints via `bevy lint`.
	bevy lint

# --- Web -------------------------------------------------------------------

wasm-prereqs: ## Install the wasm32-unknown-unknown target (bevy-cli installs wasm-bindgen/wasm-opt itself on first use).
	rustup target add wasm32-unknown-unknown
	@echo "bevy-cli will offer to install wasm-bindgen-cli/wasm-opt on first web build/run (--yes auto-confirms)."

# `--wasm-opt true` is passed explicitly below rather than relied on as a release
# default: bevy-cli's "defaults to true for release" only fires from
# [package.metadata.bevy_cli] config resolution, which early-returns when that
# table is absent from Cargo.toml (as it is here) — so without the flag, release
# web builds silently ship an unstripped, un-Os'd wasm binary (~70 MB vs ~28 MB
# for 000-jump, verified). --yes auto-installs wasm-opt itself on first use.

web-run-%: ## web-run-<name>: Dev loop — serve one example in the browser from target/, e.g. `make web-run-000-jump`.
	bevy run --yes --release --example $* web --open --wasm-opt true

web-%: $(DIST)/.assets ## web-<name>: Build one example for web and copy it into dist/, e.g. `make web-009-world-map`.
	bevy build --yes --release --example $* web --bundle --wasm-opt true
	mkdir -p $(DIST)/build
	cp -R target/bevy_web/web-release/$*/build/. $(DIST)/build/
	cp target/bevy_web/web-release/$*/index.html $(DIST)/$*.html

web-all: check-assets $(addprefix web-,$(WEB_EXAMPLES)) $(DIST)/index.html ## Build every web-buildable example into dist/ with a gallery index.
	@echo "dist/ is ready — run 'make serve' to view it."

# Asset mirror, shared by every example (see CLAUDE.md's "Web Builds" section
# for why one dist/assets/ works for all bundles). Rebuilds only when an
# asset file actually changed.
$(DIST)/.assets: $(shell find assets -type f)
	mkdir -p $(DIST)
	rsync -a --delete --exclude '.DS_Store' assets/ $(DIST)/assets/
	@touch $@

# Gallery page: tools/gallery.html with <!-- EXAMPLES --> replaced by one
# <li> per web example plus a greyed-out <li> per skipped one.
$(DIST)/index.html: tools/gallery.html
	@mkdir -p $(DIST)
	@awk -v examples="$(WEB_EXAMPLES)" -v skipped="$(WEB_SKIP)" '\
		/<!-- EXAMPLES -->/ { \
			n = split(examples, ex, " "); \
			for (i = 1; i <= n; i++) \
				print "      <li><a href=\"" ex[i] ".html\">" ex[i] "</a></li>"; \
			m = split(skipped, sk, " "); \
			for (i = 1; i <= m; i++) \
				print "      <li class=\"skipped\">" sk[i] " (native only)</li>"; \
			next; \
		} \
		{ print } \
	' $< > $@

check-assets: ## Fail if any LFS-tracked asset is still an unfetched pointer file.
	@pointers=$$(git lfs ls-files -n 2>/dev/null | xargs -I{} sh -c 'head -c 40 "{}" | grep -q "^version https://git-lfs" && echo {}'); \
	if [ -n "$$pointers" ]; then \
		echo "Unfetched LFS pointer files found (run 'git lfs pull'):"; \
		echo "$$pointers"; \
		exit 1; \
	fi

serve: ## Serve dist/ locally at http://localhost:8000 (application/wasm is a builtin mimetype since Python 3.9).
	python3 -m http.server 8000 --directory $(DIST)

# --- Cleanup -----------------------------------------------------------

clean: ## Remove all cargo build output.
	cargo clean

clean-web: ## Remove dist/ and target/bevy_web.
	rm -rf $(DIST) target/bevy_web
