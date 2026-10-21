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

## install

```sh
cargo install wallr
```

this installs the `wallr` client and `wallr-daemon`. release archives include
both binaries. arch users can install `wallr-bin` from the aur; nix users can
run `nix run github:programmersd21/wallr`.

## use

```sh
wallr set wallpaper.jpg
wallr set wallpaper.jpg -e grow -o center -d 850ms
wallr set video.mp4 --fps 30
wallr ipc status
```

`wallr set` starts the daemon when `daemon.auto_start` is enabled. to run it
manually, use `wallr daemon` or start `wallr-daemon` directly. see the
[cli reference](docs/cli-reference.md).

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

## requirements

- a wayland compositor with `wlr-layer-shell`
- rust 1.90 or newer and FFmpeg development libraries for dynamic source builds;
  static builds also need the Linux tools listed by
  `scripts/install-static-ffmpeg.sh`
- no ffmpeg runtime install for the official static release binaries

## build

```sh
git clone https://github.com/programmersd21/wallr.git
cd wallr
cargo build --workspace --release --locked
```

For a static FFmpeg build, run `./scripts/install-static-ffmpeg.sh`, then export
the variables and run the build command it prints.

more setup and project conventions are in [contributing](CONTRIBUTING.md).

## documentation

[cli](docs/cli-reference.md) · [configuration](docs/config-reference.md) ·
[effects](docs/effects-reference.md) · [video](docs/video-wallpaper.md) ·
[compositors](docs/compositor-support.md) · [architecture](docs/architecture.md) ·
[benchmarks](docs/benchmarks.md) · [changelog](CHANGELOG.md)

## license

MIT. See [LICENSE](LICENSE).
