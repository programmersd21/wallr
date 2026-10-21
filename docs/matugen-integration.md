# theme providers

The configured theme provider runs after a wallpaper change:

```yaml
theme:
  provider: matugen

matugen:
  enabled: true
  mode: dark
  scheme: scheme-tonal-spot
  contrast: 0
  wait: true
  args: []

reload:
  - waybar
```

wallr supports `matugen`, `wallust`, `pywal`, or `none`. Install the selected
program separately. Use `wallr doctor` to check which providers are available.

Override the configured provider for one wallpaper:

```sh
wallr set wallpaper.jpg --theme matugen
wallr set video.mp4 --theme matugen
wallr set wallpaper.jpg --no-theme
```

For gif and video, wallr extracts a still frame for theme generation while
playback continues. The frame is cached and refreshed when the source file
changes.

## prevent loops

If Matugen calls wallr to set its input image, disable theme generation on
that call:

```toml
[config.wallpaper]
command = "wallr set --no-theme {{ image }}"
set = true
```

Otherwise Matugen can invoke wallr, which invokes Matugen again. Matugen's
template hooks are separate from wallr's `reload` commands.
