<div align="center">

### *Wallr*

**Native Wayland wallpapers. GPU-rendered. Built with Rust.**

<a href="https://github.com/programmersd21/wallr">
  <img src="https://raw.githubusercontent.com/programmersd21/wallr/main/assets/demo.gif" alt="Wallr Demo" width="860">
</a>

<br>

<!-- Project stats -->
<a href="https://github.com/programmersd21/wallr/stargazers"><img alt="Stars" src="https://shieldcn.dev/github/stars/programmersd21/wallr.svg?variant=secondary&theme=zinc&font=geist"></a>
<a href="https://github.com/programmersd21/wallr/releases/latest"><img alt="Release" src="https://shieldcn.dev/github/release/programmersd21/wallr.svg?variant=secondary&theme=zinc&font=geist"></a>
<a href="https://github.com/programmersd21/wallr/releases"><img alt="Downloads" src="https://shieldcn.dev/github/downloads/programmersd21/wallr.svg?variant=secondary&theme=zinc&font=geist"></a>
<a href="https://crates.io/crates/wallr"><img alt="Crates.io" src="https://shieldcn.dev/crates/wallr.svg?variant=secondary&theme=zinc&font=geist"></a>
<a href="https://github.com/programmersd21/wallr/actions"><img alt="CI" src="https://shieldcn.dev/github/ci/programmersd21/wallr.svg?variant=secondary&theme=zinc&font=geist"></a>
<a href="https://github.com/programmersd21/wallr/blob/main/LICENSE"><img alt="License" src="https://shieldcn.dev/github/license/programmersd21/wallr.svg?variant=secondary&theme=zinc&font=geist"></a>

<!-- Tech stack -->
<a href="https://wayland.freedesktop.org/"><img alt="Wayland" src="https://shieldcn.dev/badge/Wayland-Native-violet.svg?variant=secondary&theme=zinc&font=geist"></a>
<a href="https://wgpu.rs/"><img alt="wgpu" src="https://shieldcn.dev/badge/wgpu-Renderer-blue.svg?variant=secondary&theme=zinc&font=geist"></a>
<a href="https://www.rust-lang.org/"><img alt="Rust" src="https://shieldcn.dev/badge/Rust-2024-orange.svg?variant=secondary&theme=zinc&font=geist"></a>

GPU-accelerated wallpaper engine for Wayland with native `wl-layer-shell` rendering powered by **wgpu**.

</div>

## Introduction

Wallr sets and animates wallpapers on Wayland compositors that support `wlr-layer-shell`. It renders its own background surface with `wgpu` — it does not shell out to `hyprpaper`, `awww`, or `swaybg`.

Theme generation (Matugen, Wallust, Pywal) is supported as an optional step that runs after a wallpaper is applied — including for video and GIF wallpapers via automatic first-frame extraction to `~/.cache/wallr/theme/` (cached by source path + mtime). It is not required and not part of the core rendering path.

## Features

- 11 built-in transitions: fade, blur, wipe, slide, zoom, pixelate, ripple, dissolve, wave, grow, outer.
- GIF wallpapers, decoded once and cached to avoid re-decoding on loop.
- Video wallpapers (MP4, WebM, MKV) with hardware-accelerated decoding via FFmpeg.
- Five scaling modes: fill (cover), fit (contain), stretch, center (1:1), tile (repeat).
- Transition duration is wall-clock based, independent of monitor refresh rate.
- Background daemon (`wallr daemon`) that owns the surface over a Unix socket.
- Directory watching (`wallr watch`) to apply new files automatically.
- Per-monitor wallpapers and scaling modes.
- Preview mode to test an effect before applying it.
- YAML animation packages with an install/search registry.
- Automatic GPU selection on hybrid graphics systems.

## Requirements

- Rust (stable)
- A compositor with `wlr-layer-shell`: Hyprland, Sway, niri (with a layer rule), or KDE Plasma 6
- GNOME/Mutter is not supported — it does not implement the protocol
- FFmpeg development libraries, for video wallpapers (detected at build time when building from source)

Prebuilt release binaries are self-contained: FFmpeg is statically linked at build time, so they keep working regardless of the FFmpeg version installed on your system. Binaries built from source (`cargo install`, distro packages) link the system FFmpeg dynamically and must be rebuilt when your system FFmpeg ABI changes.

## Installation

```bash
cargo install wallr
```

```bash
yay -S wallr-bin
```

### Nix

With Nix installed:

```bash
# Run directly
nix run github:programmersd21/wallr

# Install to your profile
nix profile install github:programmersd21/wallr

# Build without installing
nix build github:programmersd21/wallr
```

For NixOS users, you can also add Wallr as an overlay or input to your flake:

```nix
{
  inputs.wallr.url = "github:programmersd21/wallr";

  outputs = { self, nixpkgs, wallr, ... }: {
    # Use wallr.packages.${system}.wallr in your configuration
  };
}
```

Development shell:

```bash
nix develop
```

### Build from source:

```bash
# Arch
sudo pacman -S rust wayland wayland-protocols pkg-config ffmpeg

# Fedora
sudo dnf install rust cargo wayland-devel wayland-protocols-devel pkg-config ffmpeg-devel

# Ubuntu/Debian
sudo apt install rustc cargo libwayland-dev wayland-protocols pkg-config libavcodec-dev libavformat-dev libavutil-dev libswscale-dev
```

```bash
git clone https://github.com/programmersd21/wallr.git
cd wallr
cargo install --path wallr
```

## Quick start

```bash
wallr set wallpaper.jpg
wallr set wallpaper.jpg --effect grow --origin bottom_right --duration 1.2s
wallr set animated.gif --effect fade --duration 500ms
wallr set video.mp4 --effect wave --duration 1s
wallr preview wallpaper.jpg --effect wave --angle 45
```

`wallr set` starts the daemon automatically if it isn't running.

```bash
wallr daemon
wallr watch ~/Pictures
wallr ipc pause
wallr ipc resume
wallr ipc seek 1:30
wallr ipc info
wallr quit
wallr doctor
wallr validate anim.yaml
```

Full flag reference: [docs/cli-reference.md](docs/cli-reference.md)

## Configuration

`~/.config/wallr/config.yaml`:

```yaml
wallpaper:
  default: "~/Pictures/Wallpapers/default.png"
  mode: "fill"

animation:
  use: "smooth/crossfade"
  duration: "2000ms"

theme:
  provider: "matugen"

reload:
  - "waybar"
  - "dunst"
```

If Matugen calls `wallr set` as its own wallpaper command, pass `--no-theme` on that call to avoid a feedback loop.

Full schema: [docs/config-reference.md](docs/config-reference.md)

## Architecture

`wallr` is a CLI that talks to `wallr daemon` over a Unix socket. The daemon owns the layer-shell surface, the `wgpu` renderer, and the animation engine. Wallpaper changes are rendered as GPU transitions from the previous image; GIFs continue playing frame-by-frame once the transition ends, and videos are decoded by FFmpeg with hardware acceleration where available.

Details: [docs/architecture.md](docs/architecture.md)

## Documentation

- [Changelog](CHANGELOG.md)
- [CLI reference](docs/cli-reference.md)
- [Configuration reference](docs/config-reference.md)
- [Animation authoring](docs/animation-authoring.md)
- [Architecture](docs/architecture.md)
- [Matugen integration](docs/matugen-integration.md)
- [Video wallpaper support](docs/video-wallpaper.md)

## Troubleshooting

**`error while loading shared libraries: libavutil.so.58: cannot open shared object file`.** Your installed binary was built against an older FFmpeg ABI. Reinstall using the latest release (which now ships a statically linked FFmpeg), or if you built from source, rebuild against the FFmpeg currently installed on your system:

```bash
cargo install --path wallr --force
```

**Wallpaper blocks clicks or keyboard input.** It shouldn't — the surface is rendered on `Layer::Background`, with `KeyboardInteractivity::None` and an empty input region. If this happens:

1. Confirm your compositor is one of the supported ones.
2. Check compositor logs for layer-shell errors.
3. Restart the daemon: `pkill wallr && wallr daemon`.
4. On niri, add a layer-shell rule permitting Wallr on the background layer.

## Star History

[![RepoStars](https://repostars.dev/api/embed?repo=programmersd21%2Fwallr&theme=sunset)](https://repostars.dev/?repos=programmersd21%2Fwallr&theme=sunset)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. See [LICENSE](LICENSE).
