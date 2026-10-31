<div align="center">

# wallr

wallpaper engine for wayland, with gpu transitions, gif and video playback,
and optional theme updates.

<p align="center">
  <a href="https://github.com/programmersd21/wallr"><img src="https://raw.githubusercontent.com/programmersd21/wallr/main/assets/demo.gif" alt="wallr demo" width="800"></a>
</p>

<p align="center">
  <a href="https://github.com/programmersd21/wallr/stargazers"><img src="https://img.shields.io/github/stars/programmersd21/wallr?style=for-the-badge&logo=github&logoColor=f9e2af&labelColor=11111b&color=f9e2af" alt="github stars"></a>
  <a href="https://github.com/programmersd21/wallr/releases/latest"><img src="https://img.shields.io/github/v/release/programmersd21/wallr?style=for-the-badge&logo=github&logoColor=a6e3a1&labelColor=11111b&color=a6e3a1" alt="latest release"></a>
  <a href="https://aur.archlinux.org/packages/wallr-bin"><img src="https://img.shields.io/aur/version/wallr-bin?style=for-the-badge&logo=archlinux&logoColor=cba6f7&labelColor=11111b&color=cba6f7" alt="aur version"></a>
  <a href="https://aur.archlinux.org/packages/wallr-bin"><img src="https://img.shields.io/aur/votes/wallr-bin?style=for-the-badge&logo=archlinux&logoColor=f5c2e7&labelColor=11111b&color=f5c2e7" alt="aur votes"></a>
  <a href="https://crates.io/crates/wallr"><img src="https://img.shields.io/crates/v/wallr?style=for-the-badge&logo=rust&logoColor=fab387&labelColor=11111b&color=fab387" alt="crates.io version"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/msrv-1.90-94e2d5?style=for-the-badge&logo=rust&logoColor=94e2d5&labelColor=11111b&color=94e2d5" alt="minimum supported rust version"></a>
</p>

</div>

## table of contents

- [background](#background)
- [features](#features)
- [why wallr](#why-wallr)
- [pros and cons](#pros-and-cons)
- [requirements](#requirements)
- [installation](#installation)
- [quick start](#quick-start)
- [configuration](#configuration)
- [documentation](#documentation)
- [troubleshooting](#troubleshooting)
- [contributing](#contributing)
- [license](#license)

## background

wallr is a native wayland wallpaper engine written in rust. it runs a small
daemon that owns layer-shell surfaces, renders transitions with wgpu, and
plays gif and video wallpapers without replacing the compositor's own
wallpaper handling.

it is built for users who want animated, video, or transitional wallpapers on
wayland compositors that support `wlr-layer-shell`, while keeping memory use
and idle cpu cost predictable.

## features

- static image wallpapers through a shared-memory fast path
- gpu transitions for images, gifs, and videos
- animated gif playback with bounded frame caching
- video playback through FFmpeg, with software fallback
- multi-monitor and output hotplug handling
- per-output scaling modes: fill, fit, stretch, center, and tile
- optional theme generation with matugen, wallust, or pywal
- user hooks and reload commands
- ipc commands for pause, resume, seek, blank, restore, and status
- shared-memory presentation for static images to avoid unnecessary rendering
- bounded decoder queues and seek error propagation

## why wallr

wallr is useful when you want more than a static image but do not want a
heavy desktop shell or a compositor-specific wallpaper plugin. it keeps the
wallpaper as a background layer surface, handles multiple outputs, and can
switch media without restarting the daemon.

it is especially useful for:

- animated wallpapers on wlroots-based compositors
- short video loops as desktop backgrounds
- themed desktops where wallpaper changes trigger color generation
- multi-monitor setups
- users who want a command-line driven workflow

## pros and cons

### pros

- native wayland integration instead of X11 compatibility layers
- gpu-accelerated transitions and animation
- bounded memory budgets for decoded video frames
- shared-memory path for static images
- multi-output and hotplug aware
- optional theme integration
- command-line and ipc driven workflow
- open source and packaged for arch, nix, and crates.io

### cons

- requires a wayland compositor with `wlr-layer-shell`
- hardware decoding depends on drivers and supported backends
- video playback can use significant cpu or memory on low-end systems
- some features depend on external tools such as FFmpeg or theme providers
- compositor and gpu behaviour must be verified on each target system
- animated and video wallpapers use more resources than static images

## requirements

- a wayland compositor with `wlr-layer-shell`
- rust 1.90 or newer for source builds
- FFmpeg development libraries for dynamic source builds
- official static release binaries do not require a separate FFmpeg install
- for static builds, the Linux tools listed by
  `scripts/install-static-ffmpeg.sh`

## installation

### cargo

```sh
cargo install wallr
```

this installs the `wallr` client and `wallr-daemon`.

### arch linux

```sh
yay -S wallr-bin
```

### nix

```sh
nix run github:programmersd21/wallr
```

### release archive

download the latest archive from the
[releases page](https://github.com/programmersd21/wallr/releases), extract it,
and put both `wallr` and `wallr-daemon` on your `PATH`.

## build from source

```sh
git clone https://github.com/programmersd21/wallr.git
cd wallr
cargo build --workspace --release --locked
```

for a static FFmpeg build, run `./scripts/install-static-ffmpeg.sh`, then export
the variables it prints and run the same build command.

## quick start

```sh
wallr set wallpaper.jpg
wallr set wallpaper.jpg -e grow -o center -d 850ms
wallr set video.mp4 --fps 30
wallr ipc status
```

`wallr set` starts the daemon automatically when `daemon.auto_start` is enabled.
to run it manually, use `wallr daemon` or start `wallr-daemon` directly. see
the [cli reference](docs/cli-reference.md).

## configuration

the default file is `~/.config/wallr/config.yaml`:

```yaml
wallpaper:
  mode: fill
  loop_video: true

theme:
  provider: matugen

daemon:
  auto_start: true
  max_fps: 60
```

see the [configuration reference](docs/config-reference.md) for all options.

## documentation

[cli](docs/cli-reference.md) · [configuration](docs/config-reference.md) ·
[effects](docs/effects-reference.md) · [video](docs/video-wallpaper.md) ·
[compositors](docs/compositor-support.md) · [architecture](docs/architecture.md) ·
[benchmarks](docs/benchmarks.md) · [changelog](CHANGELOG.md)

## troubleshooting

- if `wallr doctor` reports a missing wayland session, check `WAYLAND_DISPLAY`
- if hardware decoding fails, set `video.hw_decode: software`
- if a video does not play, test a still image first, then run `wallr ipc info`
- if theme generation fails, the wallpaper change still succeeds and the error
  is reported separately
- static release binaries do not need a separate FFmpeg runtime install

## contributing

contributions are welcome. see [CONTRIBUTING.md](CONTRIBUTING.md) for setup,
style, testing, and pull request expectations.

## license

MIT. See [LICENSE](LICENSE).
