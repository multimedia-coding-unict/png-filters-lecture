# AGENTS.md — what this repo is and what I'm doing with it

This is my working copy of [`image-rs/image-png`](https://github.com/image-rs/image-png)
(the `png` crate, v0.18.1) with local commits on top of upstream `master`.

## Current work: predictive filters + a lecture

I am working on **PNG predictive (scanline) filters** in the encoder and
preparing a **lecture about them**. Two things are in progress:

1. **Debug outputs that show the filter choice per scanline** (local commits
   `7bc5cec` "first harness", `3fa225c` "refined" — *not upstream*):
   - `src/encoder.rs`: `Encoder::set_filter_log()` (L346),
     `Writer::filter_log()` / `StreamWriter::filter_log()`, and the internal
     `record_filter()` (L621) that pushes each row's filter-type byte (0–4)
     into a log, recorded at L1000 / L1019 / L1061 and in `StreamWriter::write`
     (~L1958).
   - `examples/filter_harness.rs`: encodes an image with a chosen
     `Filter` mode (`none sub up avg paeth adaptive minentropy`), verifies the
     decode round-trip, and exports `encoded.png`, `decoded.png`,
     `summary.json` (bits/pixel), and `debug/filter_map.png` (one colored row
     per scanline: black/red/green/blue/yellow = none/sub/up/avg/paeth) +
     `debug/filters.json` (per-row decisions).
   - `harness/run_harness.sh`: wrapper around the example
     (`harness/run_harness.sh [--out DIR] [--mode MODE] IMAGE...`,
     or `FILTER_MODE=...`); outputs land in `results/` (gitignored).
   - Known gap: the `FdeflateUltraFast` stored-fallback path
     (`src/encoder.rs` L1038–1041) writes filter byte 0 without logging it.

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
- `src/encoder.rs` — call sites: `Writer::zlib_encode_image_data` (L906–1072;
  filter byte written per row at L1018/L1062; NoCompression writes raw rows
  with byte 0 at L998), `StreamWriter::write` (~L1929–1975).

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
