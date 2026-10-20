<div align="center">

# wallr

GPU-accelerated wallpaper engine for wayland, rendered natively with `wgpu` on `wlr-layer-shell`.

<a href="https://github.com/programmersd21/wallr">
  <img src="https://raw.githubusercontent.com/programmersd21/wallr/main/assets/demo.gif" alt="wallr demo" width="800">
</a>

<p align="center">
  <a href="https://github.com/programmersd21/wallr/stargazers"><img src="https://img.shields.io/github/stars/programmersd21/wallr?style=for-the-badge&logo=github&logoColor=f9e2af&labelColor=11111b&color=f9e2af" alt="GitHub stars"></a>
  <a href="https://github.com/programmersd21/wallr/releases/latest"><img src="https://img.shields.io/github/v/release/programmersd21/wallr?style=for-the-badge&logo=github&logoColor=a6e3a1&labelColor=11111b&color=a6e3a1" alt="Latest release"></a>
  <a href="https://aur.archlinux.org/packages/wallr-bin"><img src="https://img.shields.io/aur/version/wallr-bin?style=for-the-badge&logo=archlinux&logoColor=cba6f7&labelColor=11111b&color=cba6f7" alt="AUR version"></a>
  <a href="https://aur.archlinux.org/packages/wallr-bin"><img src="https://img.shields.io/aur/votes/wallr-bin?style=for-the-badge&logo=archlinux&logoColor=f5c2e7&labelColor=11111b&color=f5c2e7" alt="AUR votes"></a>
  <a href="https://crates.io/crates/wallr"><img src="https://img.shields.io/crates/v/wallr?style=for-the-badge&logo=rust&logoColor=fab387&labelColor=11111b&color=fab387" alt="Crates.io version"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/MSRV-1.88-94e2d5?style=for-the-badge&logo=rust&logoColor=94e2d5&labelColor=11111b&color=94e2d5" alt="Minimum supported Rust version"></a>
</p>

</div>


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
wallr set video.mp4 --fps 30
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
