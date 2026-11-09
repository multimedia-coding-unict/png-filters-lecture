#!/usr/bin/env bash
# Execution harness for PNG filter-decision experiments.
#
# Runs the `filter_harness` example: encodes an input image with each requested
# filter mode, decodes each result back (verifying the round-trip), and exports
# the results into a result folder:
#
#   <out>/<image-stem>/
#     <mode>.png                 image encoded with that filter mode
#     decoded_<mode>.png         decoder output re-encoded losslessly
#     summary.csv                per-mode encoded size and bits/pixel
#     debug/
#       filter_map_<mode>.png    2D map: color of scanline y = filter chosen for it
#       filters_<mode>.csv       row,filter_id,filter_name
#
# Usage:
#   harness/run_harness.sh [--out DIR] [--modes LIST] IMAGE [IMAGE...]
#
# Modes: none,sub,up,avg,paeth,adaptive,minentropy or "all" (default).
# Can also be set through the FILTER_MODES environment variable.

set -euo pipefail

cd "$(dirname "$0")/.." # repository root, so `cargo run` finds the crate

OUT="results"
MODES="${FILTER_MODES:-all}"
IMAGES=()

while [ $# -gt 0 ]; do
    case "$1" in
    --out)
        OUT="$2"
        shift 2
        ;;
    --modes)
        MODES="$2"
        shift 2
        ;;
    -h | --help)
        grep '^#' "$0" | sed 's/^# \{0,1\}//'
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
    cargo run --release --example filter_harness -- "$img" "$OUT/$stem" "$MODES"
done

echo "Results written under $OUT/"
