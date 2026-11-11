# AGENTS.md — what this repo is and what I'm doing with it

This is my working copy of [`image-rs/image-png`](https://github.com/image-rs/image-png)
(the `png` crate, v0.18.1) with local commits on top of upstream `master`.

## Current work: predictive filters + a lecture

I am working on **PNG predictive (scanline) filters** in the encoder and
preparing a **lecture about them**. Two things are in progress:

1. **Debug outputs that show the filter choice per scanline** (local commits
   `7bc5cec` "first harness", `3fa225c` "refined" — *not upstream*):
   - `src/encoder.rs`: `Encoder::set_filter_log()` (L352) and
     `set_filter_timing()` (L364); `Writer::filter_log()` / `filter_time()` and
     `StreamWriter::filter_log()` / `filter_time()`; the internal
     `record_filter()` (L653) and `record_filter_time()` (L659). Filter bytes
     are recorded at L1038 / L1061 / L1107 and in `StreamWriter::write`
     (~L2025); timing brackets each `filter(...)` call at L1055–1059 /
     L1102–1106 / L2011–2022.
   - `examples/filter_harness.rs`: encodes one or more inputs (PNG files or
     folders, searched recursively) with a chosen `Filter` mode
     (`none sub up avg paeth adaptive minentropy greedy`), or `sweep` for all of
     them, verifies the decode round-trip, and exports per image `encoded.png`,
     `decoded.png`, `summary.json` (bits/pixel), `times.json` (whole-encode and
     filter-prediction times), and `debug/filter_map.png` (one colored row per
     scanline: black/red/green/blue/yellow = none/sub/up/avg/paeth) +
     `debug/filters.json` (per-row decisions). CLI:
     `[--out DIR] [--mode MODE] [--threshold T[,T...]] [--repeat N] INPUT [INPUT...]`;
     `--repeat N` repeats each encode and reports the minimum.
   - `harness/run_harness.sh`: wrapper around the example
     (`harness/run_harness.sh [--out DIR] [--mode MODE | --sweep] [--threshold T[,T...]] [--repeat N] INPUT...`,
     or `FILTER_MODE=...` / `FILTER_THRESHOLD=...`). `INPUT` may be a file or a
     folder; folders are searched recursively for `*.png` and keep their
     relative layout under `<out>/<folder>/`. Unsupported images are reported
     and skipped. A sweep ends with one cumulative table over all images
     (bytes/times summed, bits/pixel averaged). Outputs land in `results/`
     (gitignored).
   - `greedy` mode: `Filter::GreedyAdaptive` (`src/filter/mod.rs`) starts like
     `Adaptive` on the first row, then evaluates the previous row's filter
     first. If its mean absolute residual per byte is at or below the threshold
     (default `DEFAULT_GREEDY_THRESHOLD = 2.0`, settable with
     `Encoder::set_filter_threshold`), that filter is kept; otherwise it falls
     back to the full adaptive search, reusing the already computed cost.
     `--threshold` accepts a comma-separated list, so a sweep can compare
     several thresholds (rows/folders are named `greedy@<threshold>`).
   - Known gap: the `FdeflateUltraFast` stored-fallback path
     (`src/encoder.rs` L1080–1083) writes filter byte 0 without logging it.

2. **Lecture slides as an mdbook in `docs/`** (commit `d113a6d` "first version
   of mdbook"). The pages in `docs/src/` are meant to be **projected during the
   lecture** — keep them didactic, short, with diagrams in ASCII code blocks
   (pure `+---|` ASCII only: box-drawing glyphs misalign in browser fonts) and
   real measured numbers from the harness. Build/serve with:

   ```bash
   cd docs && mdbook serve     # http://localhost:3000, for projecting
   mdbook build                # static output in docs/book/ (gitignored)
   ```

   Do not re-add a custom theme/CSS — default mdbook styling was chosen
   deliberately. The single exception is `docs/impl-ref.css` (registered via
   `additional-css` in `docs/book.toml`, path relative to the book root),
   which styles the "Implementation reference" fragments (see below). Real
   harness outputs used as figures live in `docs/src/assets/`.

## Where the filter code lives (upstream, line numbers = this checkout)

- `src/filter/mod.rs` — `Filter` enum (L24–36, default `Adaptive`, includes
  `GreedyAdaptive`), internal `RowFilter` 0–4 (L71–77), `Filter::from_simple`
  compression→filter mapping (L57–66), `filter_internal` (L405–542, 32-byte
  chunks for autovectorization), `adaptive_filter` (L544–553, tries
  `[Up, Sub, Avg, Paeth]`, `<=` tie-break biases Paeth, early exit at cost 0),
  `adaptive_filter_seeded` (L559–599, same search but reuses a precomputed
  candidate cost), `greedy_adaptive_filter` (L607–639, previous-row reuse +
  threshold), entry point `filter` (L641–673), cost functions `sum_buffer`
  (L733–754, MSAD heuristic) and `entropy` + `ilog2i` (L677–730, MinEntropy
  heuristic), decoder-side `unfilter` (L146–403).
- `src/filter/paeth.rs` — three equivalent Paeth predictors; the encoder uses
  `filter_paeth_fpnge` (L45–84).
- `src/common.rs` — `BytesPerPixel` (L112–119), `Info::bpp_in_prediction`
  (L831–833).
- `src/encoder.rs` — call sites: `Writer::zlib_encode_image_data` (L976–1167;
  filter byte written per row at L1102/L1156; NoCompression writes raw rows
  with byte 0 at L1069), `StreamWriter::write` (L2049–2107).

## Conventions when editing the docs

- Cite code with links to
  `https://github.com/image-rs/image-png/blob/master/<path>#L<n>` for upstream
  files; for the local-only debug additions (commits `7bc5cec`/`3fa225c`) cite
  repo-relative paths + line numbers and say "local commit".
- Wrap code references (source links, quoted snippets, line-number citations)
  in "Implementation reference" fragments so they are visually split from the
  conceptual discussion:
  `<details class="impl-ref" open><summary>Implementation reference</summary>`
  … `</details>`. Leave a blank line after `<summary>` and before
  `</details>` — mdbook only parses the markdown inside the fragment when
  those blank lines are present.
- `> [!NOTE]`-style alerts are NOT supported by the installed mdbook (0.4.48) —
  use `> **Note:** ...` blockquotes.
- mdbook tables = markdown pipe tables; anything that must align goes in a
  fenced code block with ASCII only.
- Don't commit `docs/book/` or `results/` (both gitignored).
