# CLI Reference

`wallr` is controlled via subcommands and ergonomic flags.

```text
wallr set <path> [-e <effect>] [-d <duration>] [-o <origin>] [-a <angle>] [-m <output>] [-t <theme>] [--mode <mode>]
wallr daemon [--max-fps <fps>]
wallr reload
wallr quit
wallr doctor
wallr config <get|set|path>
wallr monitor <list|current>
wallr ipc <pause|resume|reload|status|info|stop|seek|blank|restore>
```

---

## Wallpaper Commands

### `wallr set <path>` (alias: `wallr img`)
Sets the wallpaper for your desktop. Automatically launches the background daemon if not already running.

```bash
# Basic wallpaper set (2s linear crossfade, like awww `simple`)
wallr set ~/Pictures/wallpaper.png

# Transition effects with short flags
wallr set ~/Pictures/wallpaper.png -e grow -o center -d 850ms
wallr set ~/Pictures/wallpaper.png -e wipe -a 45 -d 800ms
wallr set ~/Pictures/wallpaper.png -e wave -d 900ms

# Instant switch, no transition or theming (fastest path)
wallr set ~/Pictures/wallpaper.png --duration 0 --no-theme

# Pipe image bytes from stdin (staged to a temp file automatically)
grim - | wallr set - --duration 0 --no-theme

# Target specific output and scaling mode
wallr set ~/Pictures/wallpaper.png -m DP-1 --mode fit

# Force dynamic theming (Matugen, Wallust, Pywal) or disable
wallr set ~/Pictures/wallpaper.png -t matugen
wallr set ~/Pictures/wallpaper.png --no-theme
```

#### Flags
| Flag | Long | Description |
|---|---|---|
| `-e` | `--effect <NAME>` | Transition: `simple`, `fade`, `wipe`, `slide`, `left`, `right`, `top`, `bottom`, `wave`, `grow`, `center`, `outer`, `any`, `random` (see [effects](effects-reference.md)) |
| `-d` | `--duration <TIME>` | Wall-clock duration (`700ms`, `1s`, `1.2s`) |
| `-o` | `--origin <PRESET\|X,Y>` | Origin: `top_left`, `top`, `top_right`, `left`, `center`, `right`, `bottom_left`, `bottom`, `bottom_right`, or normalized `x,y` |
| `-a` | `--angle <DEG>` | Wipe/wave travel angle in degrees (`0` = right, `90` = up) |
| `-m` | `--monitor <OUTPUT>` | Target output (e.g. `DP-1`, `HDMI-A-1`) |
| `-t` | `--theme <PROVIDER>` | One-shot theme generator: `matugen`, `wallust`, `pywal`, `none` |
| | `--mode <MODE>` | Scaling mode: `fill`, `fit`, `stretch`, `center`, `tile` (default: `fill`) |
| | `--easing <CURVE>` | Easing curve: `linear`, `ease_in`, `ease_out`, `ease_in_out`, `bezier` |
| | `--direction <X,Y>` | Direction vector for `wipe` and `slide` (e.g. `1,0`) |
| | `--from <VAL>`, `--to <VAL>` | Fade opacity range (default `0` to `1`) |
| | `--frequency <HZ>`, `--amplitude <VAL>` | Wave density and height |
| | `--softness <VAL>` | Wipe edge feather (`0.01` to `0.5`) |

---

## Daemon

### `wallr daemon`
Starts the persistent Wayland layer-shell daemon.
```bash
wallr daemon
wallr daemon --max-fps 120
```

### `wallr reload`
Re-reads configuration from disk and live-applies settings (`max_fps`, `hw_decode`, etc.) without restarting the daemon.

### `wallr quit`
Gracefully shuts down the running daemon and removes the Unix socket.

---

## Utilities

### `wallr doctor`
Runs environment checks (Wayland socket, layer-shell support, theme binary availability, and loop risks).

### `wallr monitor <list|current>`
Queries connected Wayland outputs and current display dimensions.

### `wallr config <get|set|path>`
Reads or edits the YAML config file.

---

## IPC controls

`wallr ipc <subcommand>` talks to the running daemon directly:

| Subcommand | Purpose |
|---|---|
| `pause` / `resume` | Pause or resume video/GIF playback (`-m` for one output) |
| `reload` | Same as `wallr reload` |
| `status` | Daemon state (`running` / `paused`) |
| `info` | GPU, decoder, and per-output details |
| `stop` | Same as `wallr quit` |
| `seek <ts>` | Seek video (`HH:MM:SS` or seconds) |
| `blank` / `restore` | Blank an output to black / restore its wallpaper |
