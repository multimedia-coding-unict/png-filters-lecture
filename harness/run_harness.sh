#!/usr/bin/env bash
# Execution harness for PNG filter-decision experiments.
#
# Wrapper around the `filter_harness` example: encodes one or more images with
# the selected filter mode (default: adaptive), verifies the decode round-trip,
# and exports the result plus per-scanline filter decisions and timings.
#
# Inputs may be PNG files or directories. Directories are searched recursively
# for *.png and keep their relative layout under the folder name:
#
#   <out>/<image-stem>/             for a file input
#   <out>/<folder>/<relative-path>/ for a directory input
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
#   <out>/<image-stem>/<mode>/...
#   <out>/<folder>/<relative-path>/<mode>/...
#
# In folder mode an unsupported image (16-bit, indexed, interlaced) is reported
# and skipped; the remaining images are still processed and the script exits
# non-zero at the end if any failed.
#
# Usage:
#   harness/run_harness.sh [--out DIR] [--mode MODE | --sweep] \
#       [--threshold T[,T...]] [--repeat N] INPUT [INPUT...]
#
# Mode: none, sub, up, avg, paeth, adaptive (default), minentropy, greedy, or
# sweep. Can also be set through the FILTER_MODE environment variable.
# --threshold T[,T...] sets the greedy threshold (mean residual per byte) and
# accepts a comma-separated list to sweep several values; in a sweep the greedy
# rows are named greedy@<threshold>. Can also be set through the
# FILTER_THRESHOLD environment variable.
# --repeat N encodes each mode N times and reports the minimum (default: 1);
# can also be set through the FILTER_REPEAT environment variable.

set -euo pipefail

cd "$(dirname "$0")/.." # repository root, so `cargo run` finds the crate

OUT="results"
MODE="${FILTER_MODE:-adaptive}"
THRESHOLD="${FILTER_THRESHOLD:-}"
REPEAT="${FILTER_REPEAT:-1}"
INPUTS=()

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
    --threshold)
        THRESHOLD="$2"
        shift 2
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
        INPUTS+=("$1")
        shift
        ;;
    esac
done

if [ ${#INPUTS[@]} -eq 0 ]; then
    echo "error: no input given (see --help)" >&2
    exit 1
fi

ARGS=(--out "$OUT" --mode "$MODE" --repeat "$REPEAT")
if [ -n "$THRESHOLD" ]; then
    ARGS+=(--threshold "$THRESHOLD")
fi

cargo run --release --quiet --example filter_harness -- "${ARGS[@]}" "${INPUTS[@]}"
