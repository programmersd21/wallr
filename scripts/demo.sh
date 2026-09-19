#!/usr/bin/env bash
# Wallr transition demo: applies every built-in transition over the sample
# images, records a frame sequence of each change with grim, and assembles
# the result into a looping GIF with ffmpeg.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

if [[ -x "./target/release/wallr" ]]; then
    WALLR="./target/release/wallr"
elif [[ -x "./target/debug/wallr" ]]; then
    WALLR="./target/debug/wallr"
else
    echo "Building wallr in release mode first..."
    cargo build --release
    WALLR="./target/release/wallr"
fi

echo "Using wallr binary: $WALLR"
$WALLR doctor || true
sleep 3

IMAGES=()
for image in samples/*.png; do
    [[ -f "$image" ]] && IMAGES+=("$image")
done
[[ ${#IMAGES[@]} -gt 0 ]] || { echo "no samples/*.png found" >&2; exit 1; }

EFFECTS=("fade" "wipe" "slide" "wave" "grow" "outer")
PARAMS=(
    ""
    "--direction 1,0"
    "--direction 0,1"
    "--angle 45"
    "--origin bottom_right"
    "--origin top_left"
)

DURATION_MS="${DEMO_DURATION_MS:-2000}"
FRAMES="${DEMO_FRAMES:-10}"
OUT_GIF="${1:-demo.gif}"
GAP_MS=$((DURATION_MS / FRAMES))
GAP_SECS="$(awk -v ms="$GAP_MS" 'BEGIN { printf "%.3f", ms / 1000 }')"

RECORD_DIR="$(mktemp -d)"
trap 'rm -rf "$RECORD_DIR"' EXIT

echo "=========================================="
echo "Starting Wallpaper Rotation Demo"
echo "=========================================="

record=()
APPLIED=0
SEQUENCE=0
for ((e = 0; e < ${#EFFECTS[@]}; e++)); do
    EFFECT="${EFFECTS[$e]}"
    EXTRA="${PARAMS[$e]}"
    IMAGE="${IMAGES[$((e % ${#IMAGES[@]}))]}"
    # shellcheck disable=SC2206
    EXTRA_ARGS=($EXTRA)

    echo "[record] $((e + 1))/${#EFFECTS[@]} $EFFECT ${EXTRA_ARGS[*]:-} <- $(basename "$IMAGE")"
    # shellcheck disable=SC2086 # intentional word splitting for effect flags
    "$WALLR" set "$IMAGE" \
        --effect "$EFFECT" \
        ${EXTRA_ARGS[@]} \
        --duration "$DURATION_MS"ms \
        --no-theme

    # Capture this transition mid-flight: wallr set returns immediately and
    # the transition runs async for DURATION_MS, so grab frames on a timer.
    if command -v grim >/dev/null && command -v ffmpeg >/dev/null; then
        for ((f = 0; f < FRAMES; f++)); do
            grim "$RECORD_DIR/$(printf '%04d' "$SEQUENCE").png" || true
            SEQUENCE=$((SEQUENCE + 1))
            sleep "$GAP_SECS"
        done
    else
        sleep 2
    fi

    record+=("$EFFECT")
    APPLIED=$((APPLIED + 1))
done

echo "=========================================="
echo "Applied: $APPLIED transition(s)."
echo "------------------------------------------"
for EFFECT in "${EFFECTS[@]}"; do
    count=0
    for applied in "${record[@]}"; do
        [[ "$applied" == "$EFFECT" ]] && count=$((count + 1))
    done
    printf '  %-6s %sx\n' "$EFFECT" "$count"
done

if [[ -z "$(find "$RECORD_DIR" -name '*.png' 2>/dev/null)" ]]; then
    echo "------------------------------------------"
    echo "No frames captured (grim/ffmpeg unavailable?); skipping GIF."
    exit 0
fi

echo "------------------------------------------"
echo "Encoding $OUT_GIF from $SEQUENCE frame(s)..."
ffmpeg -y -loglevel error -framerate "${FRAMES}" \
    -i "$RECORD_DIR/%04d.png" \
    -vf "split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=5" \
    -loop 0 "$OUT_GIF"
echo "------------------------------------------"
echo "Demo complete. GIF written to: $OUT_GIF"
echo "=========================================="