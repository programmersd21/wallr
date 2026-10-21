# wallr-common

Shared vocabulary for `wallr`: effects, IPC commands, configuration types, and CLI shapes.

This crate has no GPU, wayland, or decoding dependencies. It is used by both the `wallr` client and `wallr-core`.

## features

- effect definitions and parameter overrides
- IPC protocol types and validation
- configuration schema and duration parsing
- clap CLI argument structures

## usage

```toml
[dependencies]
wallr-common = "0.8.1"
```

## license

MIT. See [LICENSE](../LICENSE).
