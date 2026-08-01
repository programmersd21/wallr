#!/usr/bin/env bash

# Exit on errors, unset variables, and failed pipelines.
set -euo pipefail

# Find the project root directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$PROJECT_ROOT"

# Resolve binary path
if [ -f "./target/release/wallr" ]; then
    WALLR="./target/release/wallr"
elif [ -f "./target/debug/wallr" ]; then
    WALLR="./target/debug/wallr"
else
    echo "Building wallr in release mode first..."
    cargo build --release
    WALLR="./target/release/wallr"
fi

echo "Using wallr binary: $WALLR"

# Diagnostic check. A compositor/GPU warning should not prevent the demo from
# explaining which transitions are available.
$WALLR doctor || true

sleep 3
echo "=========================================="
echo "Starting Wallpaper Rotation Demo"
echo "=========================================="

# Add samples/image_01.png through samples/image_11.png to run the complete
# eleven-image sequence. Missing files are reported and skipped safely.
IMAGES=(
    "samples/image_01.png"
    "samples/image_02.png"
    "samples/image_03.png"
    "samples/image_04.png"
    "samples/image_05.png"
    "samples/image_06.png"
    "samples/image_07.png"
    "samples/image_08.png"
    "samples/image_09.png"
    "samples/image_10.png"
    "samples/image_11.png"
)

# All 11 built-in transitions, one per image, in the order shown by
# `wallr set --help`. Each effect keeps the wallpaper pinned to its
# screen crop; only the blend or reveal mask moves.
EFFECTS=(
    "fade"
    "blur"
    "wipe"
    "slide"
    "zoom"
    "pixelate"
    "ripple"
    "dissolve"
    "wave"
    "grow"
    "outer"
)

# Optional per-effect parameters, aligned with EFFECTS by index.
PARAMS=(
    ""
    ""
    "--direction 1,0"
    "--direction 0,1"
    "--origin center"
    ""
    "--origin center"
    "--scale 12"
    "--angle 45"
    "--origin bottom_right"
    "--origin top_left"
)

# `--theme matugen` regenerates the Material You color scheme from each
# image so the desktop theme follows the wallpaper. Drop the flag (or
# pass `--no-theme`) to skip theme generation.
COMPLETED=0
for i in "${!IMAGES[@]}"; do
    IMAGE="${IMAGES[$i]}"
    EFFECT="${EFFECTS[$i]}"
    EXTRA="${PARAMS[$i]}"
    NUMBER=$((i + 1))

    if [[ ! -f "$IMAGE" ]]; then
        echo "--- Image $NUMBER / 11 skipped: $IMAGE is missing ---"
        continue
    fi

    # shellcheck disable=SC2086 # intentional word splitting for effect flags
    read -r -a EXTRA_ARGS <<< "$EXTRA"

    echo "--- Image $NUMBER / 11: $EFFECT ---"
    echo "Setting wallpaper: $IMAGE"
    echo "Using transition: $EFFECT ${EXTRA_ARGS[*]:-} (2000ms)"
    echo "Updating matugen theme from: $IMAGE"

    "$WALLR" set "$IMAGE" \
        --effect "$EFFECT" \
        "${EXTRA_ARGS[@]}" \
        --duration 2000ms \
        --mode fill \
        --theme matugen

    COMPLETED=$((COMPLETED + 1))
    echo "Transition complete. Resting for 3 seconds..."
    sleep 3
done

echo "=========================================="
echo "Demo completed: $COMPLETED / 11 images displayed."
echo "=========================================="
