# Cookbook

1. Soft crossfade: `wallr set ~/Pictures/wallpaper.png -e fade -d 800ms`.
2. Circular wipe from a corner: `wallr set ~/Pictures/wallpaper.png -e grow -o bottom_right -d 850ms`.
3. Angled wipe: `wallr set ~/Pictures/wallpaper.png -e wipe -a 45 -d 800ms`.
4. Bezier pacing on any effect: add `--easing bezier`.
5. Instant switch for scripts: `wallr set ~/Pictures/wallpaper.png --duration 0 --no-theme`.
6. Per-output wallpapers: add `-m DP-1` / `-m HDMI-A-1`.
7. Pause all motion: `wallr ipc pause`; resume with `wallr ipc resume`.
