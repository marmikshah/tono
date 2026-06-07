# Sonarium

An MCP server that turns an AI agent into a sound engineer.

Sonarium is an **orchestrator**: it provides instruments (oscillators, FM,
noise, a polyphonic sequencer), effects (filters, EQ, drive, delay, reverb,
dynamics), and mixing tools — and an agent composes with them through MCP tool
calls. Rendering is deterministic: the same synthesis graph always produces
identical audio, and a whole session can be saved as a replayable sequence of
tool calls.

Pure Rust, local, offline. No GPU, no API keys.

> **Status:** under construction — the crate is being built up module by
> module. Watch the commit history to follow along.

## Build

```bash
make release   # → target/release/sonarium
make check     # fmt + clippy -D warnings + tests (pre-commit gate)
```

## License

MIT
