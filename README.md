<div align="center">

### Wallr

**The wallpaper engine for Wayland.**

![demo](https://raw.githubusercontent.com/programmersd21/wallr/main/assets/demo.gif)

Wallr draws its own background surface with `wlr-layer-shell` and `wgpu`. It does not wrap `hyprpaper`, `swww`, or `swaybg`; it renders transitions itself and treats theme generation (Matugen, Wallust, Pywal) as an optional step after the fact, not the core of what it does.

[![License](https://img.shields.io/badge/License-MIT-000000?style=for-the-badge&labelColor=000000&color=8b5cf6)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-stable-000000?style=for-the-badge&labelColor=000000&color=dea584&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Wayland](https://img.shields.io/badge/Wayland-native-000000?style=for-the-badge&labelColor=000000&color=22d3ee&logo=wayland&logoColor=white)](https://wayland.freedesktop.org)
[![GPU](https://img.shields.io/badge/GPU-wgpu-000000?style=for-the-badge&labelColor=000000&color=4ade80)](https://wgpu.rs)
[![Crates.io](https://img.shields.io/crates/v/wallr?style=for-the-badge&labelColor=000000&color=f97316&label=Crates.io)](https://crates.io/crates/wallr)
[![CI](https://img.shields.io/github/actions/workflow/status/programmersd21/wallr/ci.yml?style=for-the-badge&labelColor=000000&color=22c55e&label=CI)](https://github.com/programmersd21/wallr/actions/workflows/ci.yml)
[![Downloads](https://img.shields.io/github/downloads/programmersd21/wallr/total?style=for-the-badge&labelColor=000000&color=38bdf8&label=Downloads)](https://github.com/programmersd21/wallr/releases)
[![Stars](https://img.shields.io/github/stars/programmersd21/wallr?style=for-the-badge&labelColor=000000&color=facc15&label=Stars)](https://github.com/programmersd21/wallr/stargazers)
[![PRs Welcome](https://img.shields.io/badge/PRs%20Welcome-welcome-000000?style=for-the-badge&labelColor=000000&color=ec4899)](CONTRIBUTING.md)
[![Made with love](https://img.shields.io/badge/Made%20with-%E2%9D%A4-000000?style=for-the-badge&labelColor=000000&color=f43f5e)](https://github.com/programmersd21)

</div>

## Features

- **11 transition effects**: fade, blur, wipe, slide, zoom, pixelate, ripple, dissolve, wave, grow, outer. Each tunable from the CLI or a YAML package (origin, direction, angle, easing, duration).
- **Live wallpapers**: animated GIFs play frame-by-frame once the transition completes, starting from the transition's incoming image so playback eases in cleanly.
- **Circular reveals**: `grow`, `outer`, and `ripple` expand as true, aspect-corrected circles.
- **Wall-clock timing**: transition duration holds exactly, regardless of refresh rate.
- **Background daemon**: `wallr daemon` owns the surface over a Unix socket; `wallr set` starts it automatically.
- **Directory watching**: `wallr watch <dir>` applies new files as they land.
- **Animation packages**: YAML timelines with inheritance and a registry (`install`, `search`, `publish`, `validate`).
- **Preview window**: judge an effect before it touches your desktop.
- **Per-monitor control**: independent wallpapers and scaling modes (`fill`, `fit`, `stretch`, `center`, `tile`).

## Requirements

Rust toolchain, Wayland client headers, and a compositor with `wlr-layer-shell` support: Hyprland, Sway, niri (with a layer rule), or KDE Plasma 6. GNOME/Mutter doesn't implement the protocol.

## Install

```bash
cargo install wallr
```

or manually, from source:

```bash
# Arch
sudo pacman -S rust wayland wayland-protocols pkg-config

# Fedora
sudo dnf install rust cargo wayland-devel wayland-protocols-devel pkg-config

# Ubuntu/Debian
sudo apt install rustc cargo libwayland-dev wayland-protocols pkg-config
```

```bash
git clone https://github.com/programmersd21/wallr.git
cd wallr
cargo install --path wallr
```

## Usage

```bash
wallr set wallpaper.jpg                                     # starts the daemon if needed
wallr set wallpaper.jpg --effect grow --origin bottom_right --duration 1.2s
wallr set animated.gif --effect fade --duration 500ms        # live wallpaper
wallr preview wallpaper.jpg --effect wave --angle 45         # test before applying
```

```bash
wallr daemon               # run the background daemon
wallr watch ~/Pictures     # auto-apply new files dropped into a folder
wallr doctor                # check compositor, GPU, and theme providers
wallr validate anim.yaml    # lint an animation package
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

Full schema: [docs/config-reference.md](docs/config-reference.md)

If Matugen is configured to call `wallr set` as its own wallpaper command, pass `--no-theme` on that call to avoid a feedback loop.

## Animation packages

Effects and timelines are plain YAML:

```yaml
name: liquid
duration: 2000ms
timeline:
  - at: 0ms
    fade: { from: 0.0, to: 1.0 }
  - at: 150ms
    ripple: { origin: center, frequency: 15.0, amplitude: 0.02, speed: 6.0 }
```

`wallr validate <file>` checks a package before you use it. Full authoring guide: [docs/animation-authoring.md](docs/animation-authoring.md)

## Architecture

The `wallr` CLI talks to `wallr daemon` over a Unix socket. The daemon owns the layer-shell surface, a `wgpu` renderer, and the animation engine. Every change is a GPU-rendered transition from the previous wallpaper, timed to wall-clock duration regardless of refresh rate; GIFs keep playing frame-by-frame once the transition ends.

Details: [docs/architecture.md](docs/architecture.md)

## Docs

- [CLI reference](docs/cli-reference.md)
- [Configuration reference](docs/config-reference.md)
- [Animation authoring](docs/animation-authoring.md)
- [Architecture](docs/architecture.md)
- [Matugen integration](docs/matugen-integration.md)

## Support

If Wallr has improved your workflow or desktop experience, consider supporting its continued development.

Your support helps fund new features, performance improvements, bug fixes, documentation, testing, and long term maintenance while keeping Wallr open source.

### Star on GitHub

Starring the repository helps more people discover the project and shows that the work is valuable.

### Sponsor

Financial support allows more time to be invested in building, maintaining, and improving Wallr.

Every contribution, regardless of size, directly supports the future of the project.

Thank you for supporting open source software.

## Star History

[![RepoStars](https://repostars.dev/api/embed?repo=programmersd21%2Fwallr&theme=sunset)](https://repostars.dev/?repos=torvalds%2Flinux&theme=sunset)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. See [LICENSE](LICENSE).
