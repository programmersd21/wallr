# CLI reference

The CLI is implemented with Clap. The current commands are:

```text
wallr img <path> [--no-theme] [--theme <matugen|wallust|pywal|none>] [--monitor <name>] [--animation <package>] [--duration <800ms|1.2s>]
wallr set <path> [same options as img]
wallr new <name> [--shader]
wallr daemon
wallr watch <directory>
wallr preview <path> [--watch] [--animation <package|yaml>] [effect flags]
wallr validate <animation.yaml>
wallr doctor
wallr install <username/repo>
wallr publish
wallr search <query>
wallr cache clear|info
wallr reload
wallr config get|set|path
wallr monitor list|current
wallr ipc pause|resume|reload|preview|stop|status
```

`--theme <PROVIDER>` forces the theme pipeline for one invocation only (overrides `theme.provider` from the config without modifying it). `--no-theme` still disables it entirely; passing both makes `--no-theme` win. Example: `wallr set ~/Pictures/a.jpg --theme matugen` regenerates the Material You scheme from the image even if the config provider is unset or different.

Run `wallr <command> --help` for the exact flags emitted by the installed binary.
