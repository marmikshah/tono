# tono — a deterministic sound engine (library + CLI).
#
# `make help` lists every target. `make verify` is exactly what CI runs.

BIN     := target/release/tono
RELEASE_BRANCH ?= master

.DEFAULT_GOAL := help
.PHONY: help run build build-release install desktop play test fmt lint check pre-commit-checks verify release hooks clean

help: ## List available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

run: build-release ## Build release and print the CLI usage
	$(BIN) --help

build: ## Debug build
	cargo build

build-release: ## Optimized release build → target/release/tono
	cargo build --release

install: ## Install the `tono` CLI into ~/.cargo/bin
	cargo install --path .

desktop: ## Build the native desktop studio (Tauri + cpal + MIDI) — NOT in the default build/CI
	cargo build -p tono-desktop --release
	@echo "→ run it:  target/release/tono-desktop"

play: ## Run the programmatic playground (cpal speaker output) — NOT in the default build/CI
	cargo run -p tono-play --example playground

test: ## Run the test suite
	cargo test

fmt: ## Format all sources
	cargo fmt --all

lint: ## Clippy with warnings denied
	cargo clippy --all-targets -- -D warnings

check: fmt lint test ## Pre-commit gate (mutating): format + clippy + tests

pre-commit-checks: ## CI lint gate (non-mutating): fmt --check + clippy. Pair with 'make test'.
	cargo fmt --all -- --check
	cargo clippy --all-targets -- -D warnings

verify: pre-commit-checks test ## Exactly what CI runs (fmt --check + clippy + test) - non-mutating

release: ## Cut a release: guard clean master, tag vX.Y.Z from Cargo.toml, push (CI publishes to crates.io)
	@[ "$$(git branch --show-current)" = "$(RELEASE_BRANCH)" ] || { echo "Release only from $(RELEASE_BRANCH)."; exit 1; }
	@git diff --quiet && git diff --cached --quiet || { echo "Working tree dirty — commit before releasing."; exit 1; }
	@V=$$(sed -n '/^\[workspace\.package\]/,/^\[/ s/^version = "\([^"]*\)".*/\1/p' Cargo.toml); \
		echo "→ Releasing v$$V"; \
		if git rev-parse "v$$V" >/dev/null 2>&1; then echo "tag v$$V exists — bump version in Cargo.toml first."; exit 1; fi; \
		git tag -a "v$$V" -m "v$$V" && git push origin "v$$V"; \
		echo "✓ Tagged v$$V. The release workflow publishes to crates.io — watch GitHub Actions."

hooks: ## Enable the pre-push gate (runs 'make verify' before every push)
	git config core.hooksPath .githooks
	@echo "pre-push hook enabled"

clean: ## Remove build artifacts
	cargo clean
