# contributing

wallr is a rust wayland wallpaper engine. Builds require rust 1.90 or newer,
wayland development headers, and FFmpeg development libraries. Static FFmpeg
builds are optional; prepare their toolchain with
`scripts/install-static-ffmpeg.sh`.

## build and test

Install the matching packages for your distribution, then run:

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --release --locked
```

For a static-FFmpeg release build, run the helper above and then use the
export and cargo commands it prints.

The workspace contains three crates:

- `wallr-common`: shared cli, configuration, effect, and ipc types.
- `wallr-core`: daemon, wayland integration, rendering, media, and themes.
- `wallr`: the `wallr` client and `wallr-daemon` binaries.

For performance changes, record the wallr revision, compositor, gpu, driver,
display layout, input files, and tool versions. Run benchmarks from a wayland
session and do not generalize results beyond the measured setup.

## code changes

Add tests for behavior at public boundaries and regressions. Keep errors
useful, avoid unused dependencies, and preserve configuration and ipc
compatibility unless a change is necessary.

New transitions usually require coordinated changes to the shader, effect
type, uniform mapping, and tests. New theme providers need a runner and a
`wallr doctor` availability check.

## pull requests

Use a short commit subject that describes the change. Summarize user-visible
behavior and link related issues in the pull request. Run formatting, clippy,
tests, and relevant runtime checks before requesting review.

contributions are licensed under the [mit license](LICENSE).
