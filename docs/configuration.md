# Configuration

See [config-reference.md](config-reference.md). Configuration merges package defaults, the user YAML file, and invocation overrides. Matugen's default integration waits for `matugen image` before reload commands; use `--no-theme` when Matugen calls Wallr.

Invocation overrides include `--theme <matugen|wallust|pywal|none>`, which replaces `theme.provider` for a single `wallr set` / `wallr img` call (see [matugen-integration.md](matugen-integration.md#forcing-matugen-per-invocation)).
