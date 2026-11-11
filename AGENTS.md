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
     (`none sub up avg paeth adaptive minentropy`), or `sweep` for all of them,
     verifies the decode round-trip, and exports per image `encoded.png`,
     `decoded.png`, `summary.json` (bits/pixel), `times.json` (whole-encode and
     filter-prediction times), and `debug/filter_map.png` (one colored row per
     scanline: black/red/green/blue/yellow = none/sub/up/avg/paeth) +
     `debug/filters.json` (per-row decisions). CLI:
     `[--out DIR] [--mode MODE] [--repeat N] INPUT [INPUT...]`; `--repeat N`
     repeats each encode and reports the minimum.
   - `harness/run_harness.sh`: wrapper around the example
     (`harness/run_harness.sh [--out DIR] [--mode MODE | --sweep] [--repeat N] INPUT...`,
     or `FILTER_MODE=...`). `INPUT` may be a file or a folder; folders are
     searched recursively for `*.png` and keep their relative layout under
     `<out>/<folder>/`. Unsupported images are reported and skipped. A sweep
     ends with one cumulative table over all images (bytes/times summed,
     bits/pixel averaged). Outputs land in `results/` (gitignored).
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

- `src/filter/mod.rs` — `Filter` enum (L19–27, default `Adaptive`), internal
  `RowFilter` 0–4 (L62–68), `Filter::from_simple` compression→filter mapping
  (L48–57), `filter_internal` (L359–496, 32-byte chunks for autovectorization),
  `adaptive_filter` (L498–526, tries `[Up, Sub, Avg, Paeth]`, `<=` tie-break
  biases Paeth, early exit at cost 0), entry point `filter` (L528–546),
  cost functions `sum_buffer` (L606–627, MSAD heuristic) and `entropy` +
  `ilog2i` (L548–603, MinEntropy heuristic), decoder-side `unfilter` (L100–357).
- `src/filter/paeth.rs` — three equivalent Paeth predictors; the encoder uses
  `filter_paeth_fpnge` (L45–84).
- `src/common.rs` — `BytesPerPixel` (L112–119), `Info::bpp_in_prediction`
  (L831–833).
- `src/encoder.rs` — call sites: `Writer::zlib_encode_image_data` (L944–1118;
  filter byte written per row at L1060/L1108; NoCompression writes raw rows
  with byte 0 at L1036), `StreamWriter::write` (~L1989–2040).

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
