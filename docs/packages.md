# Animation packages

Local packages live below `~/.local/share/wallr/packages/`. A package directory contains `wallr.yaml` (or `animation.yaml`). Remote references are exactly `username/repo`; the repository root must contain `wallr.yaml` or `<repo>.yaml`.

`extends` merges base → package → current file. Child values win, maps merge, and circular references fail validation. Remote packages are cached under the configured cache directory and validated before use.
