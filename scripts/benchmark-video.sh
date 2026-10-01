#!/usr/bin/env bash
set -euo pipefail

# Reproducible decoder benchmark. This measures decode/transfer behavior, not
# compositor presentation or GPU render time.

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
input=${1:-"$root_dir/assets/demo.mkv"}
report=${BENCHMARK_REPORT:-"$root_dir/benchmarks/video-$(date -u +%Y%m%d-%H%M%S).md"}

[[ -s "$input" ]] || { echo "error: missing video: $input" >&2; exit 2; }
mkdir -p "$(dirname "$report")"
probe="$root_dir/target/release/examples/video_probe"
cargo build -q -p wallr-core --example video_probe --release

run_probe() {
    local backend=$1
    TIMEFORMAT='wall_seconds=%R user_seconds=%U system_seconds=%S'
    { time "$probe" "$input" "$backend"; } 2>&1
}

# The probe exits non-zero on failure; tolerate that here so the `result: PASS`
# check below owns the failure path and prints the actionable message.
software=$(run_probe software || true)
if ! grep -qx 'result: PASS' <<<"$software"; then
    printf 'error: software probe failed; no benchmark report generated\n%s\n' "$software" >&2
    exit 1
fi
vaapi=$(run_probe vaapi || true)
nvdec=$(run_probe nvdec || true)

frames=$(awk '/^decoded / {print $2; exit}' <<<"$software")
vaapi_frames=$(awk '/^decoded / {print $2; exit}' <<<"$vaapi")
nvdec_frames=$(awk '/^decoded / {print $2; exit}' <<<"$nvdec")
software_rate=$(sed -n 's/^decoded .* (\([0-9.]*\) fps,.*/\1/p' <<<"$software" | head -n1)
software_state=$(sed -n 's/^active backend: .* (state: \([^,]*\),.*/\1/p' <<<"$software" | head -n1)
vaapi_rate=$(sed -n 's/^decoded .* (\([0-9.]*\) fps,.*/\1/p' <<<"$vaapi" | head -n1)
vaapi_backend=$(sed -n 's/^active backend: \([^ ]*\) (state:.*/\1/p' <<<"$vaapi" | head -n1)
vaapi_state=$(sed -n 's/^active backend: .* (state: \([^,]*\),.*/\1/p' <<<"$vaapi" | head -n1)
nvdec_rate=$(sed -n 's/^decoded .* (\([0-9.]*\) fps,.*/\1/p' <<<"$nvdec" | head -n1)
nvdec_backend=$(sed -n 's/^active backend: \([^ ]*\) (state:.*/\1/p' <<<"$nvdec" | head -n1)
nvdec_state=$(sed -n 's/^active backend: .* (state: \([^,]*\),.*/\1/p' <<<"$nvdec" | head -n1)
software_drops=$(awk -F'dropped frames: ' '/^active backend:/ {gsub(/\).*/, "", $2); print $2; exit}' <<<"$software")
vaapi_drops=$(awk -F'dropped frames: ' '/^active backend:/ {gsub(/\).*/, "", $2); print $2; exit}' <<<"$vaapi")
nvdec_drops=$(awk -F'dropped frames: ' '/^active backend:/ {gsub(/\).*/, "", $2); print $2; exit}' <<<"$nvdec")
software_cpu=$(awk -F'[ =]' '/^wall_seconds=/ {printf "%.3f", $4 + $6; exit}' <<<"$software")
vaapi_cpu=$(awk -F'[ =]' '/^wall_seconds=/ {printf "%.3f", $4 + $6; exit}' <<<"$vaapi")
nvdec_cpu=$(awk -F'[ =]' '/^wall_seconds=/ {printf "%.3f", $4 + $6; exit}' <<<"$nvdec")

vaapi_reason=""
if [[ $vaapi_backend != VAAPI || $vaapi_state != 'hardware active' ]] || ! grep -qx 'result: PASS' <<<"$vaapi" || ! grep -qx 'fallback occurred: false' <<<"$vaapi"; then
    vaapi_reason="probe failed, fell back, or did not remain hardware active (backend: ${vaapi_backend:-unavailable}, state: ${vaapi_state:-unavailable})"
    vaapi_frames= vaapi_rate= vaapi_cpu= vaapi_drops=
fi

nvdec_reason=""
if [[ $nvdec_backend != NVDEC || $nvdec_state != 'hardware active' ]] || ! grep -qx 'result: PASS' <<<"$nvdec" || ! grep -qx 'fallback occurred: false' <<<"$nvdec"; then
    nvdec_reason="probe failed, fell back, or did not remain hardware active (backend: ${nvdec_backend:-unavailable}, state: ${nvdec_state:-unavailable})"
    nvdec_frames= nvdec_rate= nvdec_cpu= nvdec_drops=
fi

{
    echo "# Wallr video decoder benchmark"
    echo
    echo "Machine-specific decoder measurements. This report does not measure compositor presentation or GPU render time."
    echo
    printf -- '- Date (UTC): %s\n' "$(date -u '+%Y-%m-%d %H:%M:%S')"
    printf -- '- Host: %s\n' "$(uname -n)"
    printf -- '- Input: `%s`\n' "$input"
    printf -- '- Wallr version: %s\n' "$(cargo run -q -p wallr -- --version 2>/dev/null)"
    echo
    echo "| Backend | Frames | Throughput (fps) | CPU time (s) | Decoder state | Dropped frames |"
    echo "|:--|--:|--:|--:|:--|--:|"
    printf '| Software | %s | %s | %s | %s | %s |\n' "${frames:-unavailable}" "${software_rate:-unavailable}" "${software_cpu:-unavailable}" "${software_state:-unavailable}" "${software_drops:-unavailable}"
    printf '| VAAPI | %s | %s | %s | %s | %s |\n' "${vaapi_frames:-unavailable}" "${vaapi_rate:-unavailable}" "${vaapi_cpu:-unavailable}" "${vaapi_state:-unavailable}" "${vaapi_drops:-unavailable}"
    printf '| NVDEC | %s | %s | %s | %s | %s |\n' "${nvdec_frames:-unavailable}" "${nvdec_rate:-unavailable}" "${nvdec_cpu:-unavailable}" "${nvdec_state:-unavailable}" "${nvdec_drops:-unavailable}"
    [[ -z $vaapi_reason ]] || printf '\n- VAAPI unavailable: %s.\n' "$vaapi_reason"
    [[ -z $nvdec_reason ]] || printf '\n- NVDEC unavailable: %s.\n' "$nvdec_reason"
} >"$report"

echo "Report saved to: $report"
