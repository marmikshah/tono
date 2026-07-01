# Tono — sound-engineering MCP server.
#
# Bare `make` builds release and runs the HTTP MCP server (the default).
# `make help` lists everything.

BIN     := target/release/tono
BIND    ?= 127.0.0.1:8787
WORKDIR ?= ./sounds
RELEASE_BRANCH ?= master

.DEFAULT_GOAL := run
.PHONY: help run serve stdio build build-release release desktop play test fmt lint check pre-commit-checks verify hooks clean install daemon daemon-status daemon-uninstall

help: ## List available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

run: serve ## Default: release build, then run the HTTP MCP server

serve: build-release ## Run the streamable HTTP MCP server (BIND, WORKDIR overridable)
	@echo "tono HTTP MCP → http://$(BIND)/mcp   (workdir: $(WORKDIR))"
	TONO_WORKDIR=$(WORKDIR) $(BIN) --http $(BIND)

stdio: build-release ## Run the stdio MCP server (client spawns the binary)
	TONO_WORKDIR=$(WORKDIR) $(BIN)

build: ## Debug build
	cargo build

build-release: ## Optimized release build → target/release/tono
	cargo build --release

desktop: ## Build the native desktop studio (Tauri window + cpal audio + MIDI) — NOT in the default build/CI
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
	@V=$$(grep -m1 '^version = ' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/'); \
		echo "→ Releasing v$$V"; \
		if git rev-parse "v$$V" >/dev/null 2>&1; then echo "tag v$$V exists — bump version in Cargo.toml first."; exit 1; fi; \
		git tag -a "v$$V" -m "v$$V" && git push origin "v$$V"; \
		echo "✓ Tagged v$$V. The release workflow publishes to crates.io — watch GitHub Actions."

hooks: ## Enable the pre-push gate (runs 'make verify' before every push)
	git config core.hooksPath .githooks
	@echo "pre-push hook enabled"

clean: ## Remove build artifacts
	cargo clean

daemon: build-release ## Install + start the background daemon (launchd / systemd --user)
	$(BIN) service install --bind $(BIND) --workdir $(abspath $(WORKDIR))

daemon-status: ## Show daemon state
	$(BIN) service status

daemon-uninstall: ## Stop + remove the daemon
	$(BIN) service uninstall

install: build-release ## Print the command to register with Claude Code over HTTP
	@echo "1) start the server:  make serve"
	@echo "2) register client:   claude mcp add --transport http tono http://$(BIND)/mcp"
