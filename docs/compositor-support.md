# Compositor support

Wallr requires `wlr-layer-shell`.

| Compositor | Support |
|---|---|
| Hyprland | Yes |
| Sway | Yes |
| i3 on Wayland/Sway-compatible compositor | Yes |
| MangoWM | Yes |
| niri | Yes, with the rule below |
| KDE Plasma Wayland | Yes where layer-shell is enabled |
| GNOME/Mutter | No |

The wallpaper layer namespace is `wallr`:

```kdl
layer-rule { match namespace="^wallr$" place-within-backdrop true }
```
