#!/usr/bin/env bash
# Execution harness for PNG filter-decision experiments.
#
# Runs the `filter_harness` example: encodes an input image through the normal
# encoding process with the selected filter mode (default: adaptive), decodes the
# result back (verifying the round-trip), and exports it into a result folder:
#
#   <out>/<image-stem>/
#     times.json             encoding times, whole encode and filter path only
#     encoded.png            image encoded with the selected filter mode
#     decoded.png            decoder output re-encoded losslessly
#     summary.json           encoded size and bits/pixel
#     debug/
#       filter_map.png       2D map: color of scanline y = filter chosen for it
#       filters.json         per-scanline filter decisions
#
# With `--sweep` every mode is run and each gets its own subfolder:
#
#   <out>/<image-stem>/
#     times.json             one entry per mode, for comparison
#     none/adaptive/...      the per-mode outputs shown above
#
# Usage:
#   harness/run_harness.sh [--out DIR] [--mode MODE | --sweep] [--repeat N] IMAGE [IMAGE...]
#
# Mode: none, sub, up, avg, paeth, adaptive (default), minentropy, or sweep.
# Can also be set through the FILTER_MODE environment variable.
# --repeat N encodes each mode N times and reports the minimum (default: 1);
# can also be set through the FILTER_REPEAT environment variable.

set -euo pipefail

cd "$(dirname "$0")/.." # repository root, so `cargo run` finds the crate

OUT="results"
MODE="${FILTER_MODE:-adaptive}"
REPEAT="${FILTER_REPEAT:-1}"
IMAGES=()

while [ $# -gt 0 ]; do
    case "$1" in
    --out)
        OUT="$2"
        shift 2
        ;;
    --mode)
        MODE="$2"
        shift 2
        ;;
    --sweep)
        MODE="sweep"
        shift
        ;;
    --repeat)
        REPEAT="$2"
        shift 2
        ;;
    -h | --help)
        grep '^#' "$0" | sed '1d; s/^# \{0,1\}//'
        exit 0
        ;;
    *)
        IMAGES+=("$1")
        shift
        ;;
    esac
done

if [ ${#IMAGES[@]} -eq 0 ]; then
    echo "error: no input image given (see --help)" >&2
    exit 1
fi

mkdir -p "$OUT"

for img in "${IMAGES[@]}"; do
    if [ ! -f "$img" ]; then
        echo "error: no such file: $img" >&2
        exit 1
    fi
    stem="$(basename "$img")"
    stem="${stem%.*}"
    echo "=== $img -> $OUT/$stem ==="
    cargo run --release --example filter_harness -- \
        --repeat "$REPEAT" "$img" "$OUT/$stem" "$MODE"
done

echo "Results written under $OUT/"
