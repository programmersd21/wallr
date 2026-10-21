# cli reference

`wallr` is the client. `wallr-daemon` is the wayland service. Both binaries
are installed by `cargo install wallr` and included in release archives.

```sh
wallr --help
wallr set --help
wallr-daemon --help
```

Help is colored in interactive terminals. Command descriptions and status
labels use lowercase text; paths and configuration values keep their spelling.

## client commands

| command | purpose |
|:--|:--|
| `wallr set <path>` | set an image, gif, or video wallpaper |
| `wallr doctor` | check the session and available providers |
| `wallr config get <key>` | read a configuration value |
| `wallr config set <key> <value>` | update a configuration value |
| `wallr config path` | print the active configuration path |
| `wallr reload` | reload configuration and run reload commands |
| `wallr monitor list` | list connected outputs |
| `wallr monitor current` | show the focused output |
| `wallr ipc <command>` | control playback or query daemon state |
| `wallr quit` | stop the daemon |
| `wallr daemon` | start the daemon in the foreground |

Client options:

| option | purpose |
|:--|:--|
| `-c, --config <path>` | use a configuration file |
| `-v`, `-vv` | show debug or trace logging |
| `-q, --quiet` | suppress non-error output |
| `-h, --help` | show help |
| `-V, --version` | show the version |

## setting a wallpaper

```sh
wallr set <path> [options]
```

| option | purpose |
|:--|:--|
| `-m, --monitor <output>` | target one output |
| `--mode <mode>` | choose `fill`, `fit`, `stretch`, `center`, or `tile` |
| `--fps <fps>` | cap video presentation at 1 to 240 fps |
| `-t, --theme <provider>` | choose `matugen`, `wallust`, `pywal`, or `none` |
| `--no-theme` | skip theme generation |
| `-e, --effect <name>` | choose a transition |
| `-d, --duration <time>` | set its duration, such as `700ms` or `1.2s` |
| `-o, --origin <point>` | choose a preset or normalized `x,y` point |
| `-a, --angle <degrees>` | set the wipe angle |
| `--direction <x,y>` | set a wipe or slide direction |
| `--easing <curve>` | choose the transition curve |
| `--from`, `--to` | set fade endpoints |
| `--frequency`, `--amplitude` | set wave parameters |
| `--softness <value>` | set wipe-edge softness |

Use `-` as the path to read an image from standard input. Input is limited to
256 MiB. See the [transition reference](effects-reference.md) for effect names
and behavior.

## daemon

`wallr daemon` starts `wallr-daemon` and waits for it to exit. For direct use:

```sh
wallr-daemon [-c <path>] [--max-fps <fps>] [-v | -vv] [-q]
```

`--max-fps` accepts 1 to 240. `-v` enables debug logs; `-vv` enables trace
logs. Without verbosity, the daemon is quiet. `--quiet` limits output to
errors. `-c, --config` selects a configuration file; `-h, --help` and
`-V, --version` show daemon help and version. `wallr set` starts it when
`daemon.auto_start` is enabled.

## ipc

| command | purpose |
|:--|:--|
| `pause` / `resume` | pause or resume gif and video playback |
| `seek <time>` | seek to seconds, `mm:ss`, or `hh:mm:ss` |
| `status` | show daemon state |
| `info` | show gpu, decoder, and output details |
| `reload` | reload the running daemon |
| `blank` / `restore` | blank an output or restore its wallpaper |
| `stop` | stop the daemon |

Use `--monitor <output>` on playback commands to target one output. Without
it, supported playback commands apply to all outputs.
