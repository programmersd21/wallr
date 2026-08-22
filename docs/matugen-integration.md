# Matugen integration

Wallr can call Matugen after an animation:

```yaml
theme: {provider: matugen}
matugen: {enabled: true, mode: dark, wait: true, args: []}
```

`matugen.mode` is passed to matugen's `--mode` flag, so it must be `light` or `dark` (the default is `dark`). `scheme` maps to `--type` and `contrast` to `--contrast`. Wallr runs `matugen image <wallpaper> --mode <mode> --type <scheme> --contrast <n> --source-color-index 0`, followed by any extra `args`. Matugen then runs its own template `post_hook` commands; Wallr additionally runs its `reload` list afterwards.

Video and GIF wallpapers now work for theming too: when the wallpaper is a video (`mp4`/`webm`/`mkv`/`mov`/`avi`/`m4v`) or GIF, Wallr extracts the first frame to `~/.cache/wallr/theme/<hash>.png` (FFmpeg for video, `image` crate for GIF) and passes that still to the theme provider. The extracted frame is cached by source path + mtime and reused until the source changes. All providers (`matugen`, `wallust`, `pywal`) benefit — no manual extraction is needed.

## Forcing matugen per invocation

`wallr set` / `wallr img` accept `--theme <PROVIDER>` to override the configured provider for one call, without config edits:

```bash
wallr set ~/Pictures/wallpaper.jpg --theme matugen
wallr set ~/Pictures/video.mp4 --theme matugen   # first frame → matugen
wallr set ~/Pictures/anim.gif --theme wallust     # first frame → wallust
```

This is what `scripts/demo.sh` uses so every rotated wallpaper also regenerates the scheme, even when the user's config has a different provider or none at all. `--theme none` and `--no-theme` both skip theme generation for that call.

## Loop prevention

Matugen can also call Wallr through its `[config.wallpaper]` command. Always pass `--no-theme`:

```toml
[config.wallpaper]
command = "wallr img --no-theme {{ image }}"
set = true
```

Without `--no-theme`, Matugen → Wallr → Matugen loops indefinitely. Wallr does not duplicate Matugen template `post_hook` behavior; use Wallr's `reload` list for independent reload commands.

> **Migrating from deprecated Matugen config:** older Matugen configs split the wallpaper command into separate keys:
> ```toml
> [config.wallpaper]
> command = "wallr"
> arguments = ["img", "--no-theme"]
> set = true
> ```
> Current Matugen emits `⚠ You should not define arguments inside of [config.wallpaper] anymore. Use the command instead and use the {{ image }} keyword`. Migrate to the single `command` string with `{{ image }}` as shown above — e.g. `command = "wallr img --no-theme {{ image }}"`.
