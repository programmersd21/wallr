# wallr

GPU-accelerated wallpaper engine for wayland, rendered natively with `wgpu` on `wlr-layer-shell`.

## install

```bash
cargo install wallr
```

Arch: `yay -S wallr-bin` · Nix: `nix run github:programmersd21/wallr`

## usage

```bash
wallr set wallpaper.jpg
wallr set wallpaper.jpg -e grow -o center -d 850ms
wallr set video.mp4 -e wave -d 900ms
wallr set wallpaper.jpg --duration 0 --no-theme
```

`wallr set` starts the daemon automatically. Full reference: [docs/cli-reference.md](docs/cli-reference.md).

## config

`~/.config/wallr/config.yaml`

```yaml
wallpaper:
  default: "~/Pictures/Wallpapers/default.png"
  mode: "fill"

theme:
  provider: "matugen"

reload:
  - "waybar"
```

Full schema: [docs/config-reference.md](docs/config-reference.md).

## requirements

- compositor: hyprland, sway, niri, or kde plasma 6 (`wlr-layer-shell`)
- rust: stable, for building from source
- ffmpeg: required only when building from source; release binaries are statically linked

## build from source

```bash
git clone https://github.com/programmersd21/wallr.git
cd wallr
cargo install --path wallr
```

Dependencies: see [CONTRIBUTING.md](CONTRIBUTING.md).

## docs

[changelog](CHANGELOG.md) · [cli](docs/cli-reference.md) · [config](docs/config-reference.md) · [architecture](docs/architecture.md) · [benchmarks](docs/benchmarks.md) · [video wallpapers](docs/video-wallpaper.md)

## troubleshooting

- `libavutil.so` mismatch: reinstall the latest release or rebuild with `cargo install --path wallr --force`.
- input intercepted: confirm compositor support, restart daemon (`pkill wallr && wallr daemon`), and on niri add a background layer rule.

## contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## license

MIT. See [LICENSE](LICENSE).
```
filePath: /home/programmersd/Projects/github/wallr/README.md