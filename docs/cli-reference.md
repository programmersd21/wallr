# CLI reference

The CLI is implemented with Clap. The current commands are:

```text
wallr img <path> [--mode <fill|fit|stretch|center|tile>] [--no-theme] [--theme <matugen|wallust|pywal|none>] [--monitor <name>] [--animation <package>] [--duration <800ms|1.2s>]
wallr set <path> [same options as img]
wallr new <name> [--shader]
wallr daemon
wallr watch <directory>
wallr preview <path> [--watch] [--animation <package|yaml>] [--mode <fill|fit|stretch|center|tile>] [effect flags]
wallr validate <animation.yaml>
wallr doctor
wallr install <username/repo>
wallr publish
wallr search <query>
wallr cache clear|info
wallr reload
wallr config get|set|path
wallr monitor list|current
wallr ipc pause|resume|reload|preview|stop|status|info|seek <timestamp>
wallr quit
```

`--theme <PROVIDER>` forces the theme pipeline for one invocation only (overrides `theme.provider` from the config without modifying it). `--no-theme` still disables it entirely; passing both makes `--no-theme` win. Example: `wallr set ~/Pictures/a.jpg --theme matugen` regenerates the Material You scheme from the image even if the config provider is unset or different.

`--mode <SCALING>` sets the image scaling mode: `fill` (cover, crops to fill), `fit` (contain, letterbox/pillarbox), `stretch` (ignores aspect ratio), `center` (1:1 centered), `tile` (repeat). Default is `fill`. The mode is applied at the shader level, so transitions and live playback both use it.

`wallr ipc info` reports version, GPU, decoder state, and playback position; `wallr ipc seek` accepts `HH:MM:SS`, `M:SS`, or plain seconds. `wallr quit` is an alias for `wallr ipc stop` and removes the daemon socket before exiting.

`wallr monitor list` queries the daemon for all connected outputs and prints each output's name and resolution. `wallr monitor current` returns the primary output. Output names are resolved from the compositor (e.g. `DP-1`, `HDMI-A-1`, `eDP-1`); if the compositor doesn't provide names, a fallback based on make/model or a generated ID is used.

Run `wallr <command> --help` for the exact flags emitted by the installed binary.
