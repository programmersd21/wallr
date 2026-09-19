#!/usr/bin/env bash
# Wallr wallpaper rotation demo: cycles the six built-in transitions over
# the sample images in samples/.

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

# The six built-in transitions, one per effect index. Params are aligned
# with EFFECTS by index.
EFFECTS=("fade" "wipe" "slide" "wave" "grow" "outer")
PARAMS=(
    ""
    "--direction 1,0"
    "--direction 0,1"
    "--angle 45"
    "--origin bottom_right"
    "--origin top_left"
)

echo "=========================================="
echo "Starting Wallpaper Rotation Demo"
echo "=========================================="
COMPLETED=0
for ((i = 0; i < ${#IMAGES[@]}; i++)); do
    IMAGE="${IMAGES[$i]}"
    EFFECT="${EFFECTS[$((i % ${#EFFECTS[@]}))]}"
    EXTRA="${PARAMS[$((i % ${#PARAMS[@]}))]}"
    # shellcheck disable=SC2206
    EXTRA_ARGS=($EXTRA)

    echo "--- $((i + 1))/$(( ${#IMAGES[@]} )): $EFFECT ${EXTRA_ARGS[*]:-}"
    # shellcheck disable=SC2086 # intentional word splitting for effect flags
    "$WALLR" set "$IMAGE" \
        --effect "$EFFECT" \
        ${EXTRA_ARGS[@]} \
        --duration 2000ms \
        --no-theme

    COMPLETED=$((COMPLETED + 1))
    sleep 3
done

echo "=========================================="
echo "Demo completed: $COMPLETED wallpaper(s) shown."
echo "=========================================="