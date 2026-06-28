# Sonarium — sound-engineering MCP server.
#
# Bare `make` builds release and runs the HTTP MCP server (the default).
# `make help` lists everything.

BIN     := target/release/sonarium
BIND    ?= 127.0.0.1:8787
WORKDIR ?= ./sounds

.DEFAULT_GOAL := run
.PHONY: help run serve stdio build release test fmt lint check verify hooks clean install daemon daemon-status daemon-uninstall

help: ## List available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}'

run: serve ## Default: release build, then run the HTTP MCP server

serve: release ## Run the streamable HTTP MCP server (BIND, WORKDIR overridable)
	@echo "sonarium HTTP MCP → http://$(BIND)/mcp   (workdir: $(WORKDIR))"
	SONARIUM_WORKDIR=$(WORKDIR) $(BIN) --http $(BIND)

stdio: release ## Run the stdio MCP server (client spawns the binary)
	SONARIUM_WORKDIR=$(WORKDIR) $(BIN)

build: ## Debug build
	cargo build

release: ## Optimized release build → target/release/sonarium
	cargo build --release

WASM_BINDGEN_VERSION := 0.2.126

wasm: ## Build the browser playground (sonarium-core → WASM into docs/playground/pkg)
	rustup target add wasm32-unknown-unknown
	command -v wasm-bindgen >/dev/null || cargo install wasm-bindgen-cli --version $(WASM_BINDGEN_VERSION) --locked
	cargo build -p sonarium-wasm --target wasm32-unknown-unknown --release
	wasm-bindgen target/wasm32-unknown-unknown/release/sonarium_wasm.wasm \
		--out-dir docs/playground/pkg --target web --no-typescript
	@echo "→ serve it:  python3 -m http.server -d docs/playground 8080"

desktop: ## Build the optional native studio (cpal real-time audio) — NOT in the default build/CI
	cargo build -p sonarium-desktop --release
	@echo "→ native preview:  target/release/sonarium-desktop play docs/examples/retro-coin.json"

test: ## Run the test suite
	cargo test

fmt: ## Format all sources
	cargo fmt --all

lint: ## Clippy with warnings denied
	cargo clippy --all-targets -- -D warnings

check: fmt lint test ## Pre-commit gate: format + clippy + tests

verify: ## Exactly what CI runs (fmt --check + clippy + test) - non-mutating
	cargo fmt --all -- --check
	cargo clippy --all-targets -- -D warnings
	cargo test

hooks: ## Enable the pre-push gate (runs 'make verify' before every push)
	git config core.hooksPath .githooks
	@echo "pre-push hook enabled"

clean: ## Remove build artifacts
	cargo clean

daemon: release ## Install + start the background daemon (launchd / systemd --user)
	$(BIN) service install --bind $(BIND) --workdir $(abspath $(WORKDIR))

daemon-status: ## Show daemon state
	$(BIN) service status

daemon-uninstall: ## Stop + remove the daemon
	$(BIN) service uninstall

install: release ## Print the command to register with Claude Code over HTTP
	@echo "1) start the server:  make serve"
	@echo "2) register client:   claude mcp add --transport http sonarium http://$(BIND)/mcp"
