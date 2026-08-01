# Matugen integration

Wallr can call Matugen after an animation:

```yaml
theme: {provider: matugen}
matugen: {enabled: true, mode: dark, wait: true, args: []}
```

`matugen.mode` is passed to matugen's `--mode` flag, so it must be `light` or `dark` (the default is `dark`). `scheme` maps to `--type` and `contrast` to `--contrast`. Wallr runs `matugen image <wallpaper> --mode <mode> --type <scheme> --contrast <n> --source-color-index 0`, followed by any extra `args`. Matugen then runs its own template `post_hook` commands; Wallr additionally runs its `reload` list afterwards.

## Forcing matugen per invocation

`wallr set` / `wallr img` accept `--theme <PROVIDER>` to override the configured provider for one call, without config edits:

```bash
wallr set ~/Pictures/wallpaper.jpg --theme matugen
```

This is what `scripts/demo.sh` uses so every rotated wallpaper also regenerates the Material You scheme, even when the user's config has a different provider or none at all. `--theme none` and `--no-theme` both skip theme generation for that call.

## Loop prevention

Matugen can also call Wallr through its `[config.wallpaper]` command. Always pass `--no-theme`:

```toml
[config.wallpaper]
command = "wallr"
arguments = ["img", "--no-theme"]
set = true
```

Without `--no-theme`, Matugen → Wallr → Matugen loops indefinitely. Wallr does not duplicate Matugen template `post_hook` behavior; use Wallr's `reload` list for independent reload commands.
