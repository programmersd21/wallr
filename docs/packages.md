# Animation packages

Local packages live below `~/.local/share/wallr/packages/`. A package directory contains `wallr.yaml` (or `animation.yaml`). `--animation <name>` resolves `name` to a package directory in the registry (or to a YAML file when a file path is given).

Remote references are `username/repo` (the `github:` prefix is also accepted, matching `extends` syntax). The repository root must contain `wallr.yaml` or `<repo>.yaml`. `wallr install <username/repo>` downloads the package, validates it, and stores it at `~/.local/share/wallr/packages/<repo>/wallr.yaml`.

Remote packages are fetched with `curl` on the `main` or `master` branch and cached under the configured cache directory; `install` always fetches fresh.

`extends` merges base → package → current file. Child values win, maps merge, and circular references fail validation. Local `extends` parents are resolved recursively from the registered packages; `github:owner/repo` parents are fetched remotely.
