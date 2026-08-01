# Configuration reference

Configuration is YAML. The merge order is package defaults, then `~/.config/wallr/config.yaml`, then command-line overrides.

```yaml
wallpaper: {default: ~/Pictures/Wallpapers, mode: fill, monitors: []}
animation: {use: minimal/minimal, duration: 2000ms}
theme: {provider: matugen}
matugen: {enabled: true, mode: dark, scheme: scheme-tonal-spot, contrast: 0, wait: true, args: []}
hooks: {before: [], after: [], error: []}
reload: []
daemon: {auto_start: true, socket: "$XDG_RUNTIME_DIR/wallr.sock"}
watch: {enabled: false, dir: ~/Pictures/Wallpapers, debounce: 500ms}
cache: {dir: ~/.cache/wallr, max_size: 512MB}
plugins: {matugen: {enabled: false}, wallust: {enabled: false}, pywal: {enabled: false}}
```
