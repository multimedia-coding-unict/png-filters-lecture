# png-filters-lecture

> **This is a teaching fork of [`image-rs/image-png`](https://github.com/image-rs/image-png)**
> (the `png` crate, v0.18.1). It exists for a **lecture on PNG predictive
> (scanline) filters** and is not intended as an upstream contribution.

## Lecture notes

The lecture slides are an [mdbook](https://rust-lang.github.io/mdBook/) in
[`docs/`](docs/), published on GitHub Pages at:

**https://multimedia-coding-unict.github.io/png-filters-lecture/**

The [`pages.yml`](.github/workflows/pages.yml) workflow rebuilds and
redeploys them on every push that touches `docs/`; locally you can preview
them with `cd docs && mdbook serve`.

## Scope of this fork

Local commits on top of upstream `master` add:

- debug instrumentation in the encoder that logs the per-scanline filter
  choice and the filter-prediction time (`Encoder::set_filter_log()` /
  `set_filter_timing()` in `src/encoder.rs`);
- a filter-decision harness (`examples/filter_harness.rs`,
  `harness/run_harness.sh`, see "Filter-decision harness (local)" below) that
  encodes images in every filter mode and exports the per-scanline filter
  decisions and timings;
- a custom `greedy` adaptive filter mode (`Filter::GreedyAdaptive` in
  `src/filter/mod.rs`) that reuses the previous row's filter when its residual
  on the current row is below a threshold, and otherwise falls back to a
  seeded search;
- the lecture notes in `docs/` that get published to GitHub Pages (link above).

The decoder, the encoder's default behavior, and the test suite are unchanged
from upstream.

---

# PNG Decoder/Encoder
[![Build Status](https://github.com/image-rs/image-png/workflows/Rust%20CI/badge.svg)](https://github.com/image-rs/image-png/actions)
[![Documentation](https://docs.rs/png/badge.svg)](https://docs.rs/png)
[![Crates.io](https://img.shields.io/crates/v/png.svg)](https://crates.io/crates/png)
[![License](https://img.shields.io/crates/l/png.svg)](https://github.com/image-rs/image-png)

Robust and performant PNG decoder/encoder in pure Rust. Also supports [APNG](https://en.wikipedia.org/wiki/APNG).

No `unsafe` code, battle-tested, and fuzzed on [OSS-fuzz](https://github.com/google/oss-fuzz).

## Performance

Performance is typically on par with or better than libpng.

Includes a fast encoding mode powered by [fdeflate](https://crates.io/crates/fdeflate) that is dramatically faster than the fastest mode of libpng while *simultaneously* providing better compression ratio.

On nightly Rust compiler you can slightly speed up decoding of some images by enabling the `unstable` feature of this crate.

## Filter-decision harness (local)

This working copy adds a small harness for experimenting with the encoder's
scanline filter modes (`none`, `sub`, `up`, `avg`, `paeth`, `adaptive`,
`minentropy`). It encodes one or more images, verifies the decode round-trip,
and exports the result plus the per-scanline filter decisions.

### The lecture benchmark

**This is the benchmark used during the lecture** — the four kodim photos, every
filter mode, five repetitions each:

```bash
harness/run_harness.sh --sweep --repeat 5 \
    tests/benches/kodim02.png tests/benches/kodim07.png \
    tests/benches/kodim17.png tests/benches/kodim23.png
```

After the per-image tables, the sweep ends with a single cumulative table for
the whole input set (bytes and times summed over the images, bits/pixel averaged
over the images). Measured on this machine:

```text
=== cumulative over 4 image(s) ===
mode            total bytes  avg bits/px   total enc ms  total filt ms  filter %
none                2734325        13.91        128.379          0.229      0.2%
sub                 2424280        12.33        273.588          0.379      0.1%
up                  2436507        12.39        281.376          0.392      0.1%
avg                 2461545        12.52        319.826          0.444      0.1%
paeth               2454288        12.48        304.445          0.685      0.2%
adaptive            2420105        12.31        316.212          1.764      0.6%
minentropy          2417885        12.30        325.388          8.431      2.6%
```

The punchline for the lecture: on the four photos `sub`/`up`/`adaptive`/
`minentropy` land within ~1% of each other in size, while `none` costs ~13%
more; the adaptive heuristics buy that last fraction of a percent with a
noticeably larger filter-prediction time (`adaptive` roughly 4x a fixed filter,
`minentropy` another ~5x on top).

### Usage

```bash
harness/run_harness.sh [--out DIR] [--mode MODE | --sweep] [--repeat N] INPUT [INPUT...]

# a single image with the default adaptive mode
harness/run_harness.sh tests/benches/kodim23.png
```

The example itself accepts the same inputs:

```bash
cargo run --release --example filter_harness -- \
    --out results --mode adaptive --repeat 3 \
    tests/benches/kodim02.png tests/benches/kodim07.png \
    tests/benches/kodim17.png tests/benches/kodim23.png
```

- `INPUT` may be a PNG file or a directory. Directories are searched recursively
  for `*.png` and keep their relative layout under the folder name
  (`results/<folder>/<relative-path>/`).
- `--sweep` runs every mode and compares them in one `times.json`; after all
  images it prints the cumulative table shown above.
- `--repeat N` repeats each encode and reports the minimum, which smooths out
  timing noise.
- `FILTER_MODE` and `FILTER_REPEAT` can be used instead of the flags.

Results land in `results/` (gitignored). For a file input:

```text
results/<image>/
  times.json            encoding times, one entry per mode
  encoded.png           image encoded with the selected mode
  decoded.png           decoder output, re-encoded losslessly
  summary.json          encoded size and bits/pixel
  debug/
    filter_map.png      one colored row per scanline = its chosen filter
    filters.json        per-scanline filter decisions
```

For a directory input the same layout is nested under the folder name, e.g.
`results/pngsuite/basn0g08/`. Folder mode reports unsupported images (16-bit,
indexed, interlaced) and continues with the rest; it exits non-zero at the end
if any failed.

`times.json` separates the whole encode (`encode_total_ns`) from only the
filter prediction path (`filter_prediction_ns`), measured inside the encoder via
`Encoder::set_filter_timing`. Each entry also carries the individual runs and
min/median/mean aggregates. With `--sweep`, the per-mode files live in
`results/<image>/<mode>/` and the aggregate `times.json` stays at the top.

The input must be an 8-bit, non-indexed, non-interlaced PNG (gray, gray+alpha,
RGB or RGBA). See `docs/src/05-debug-harness.md` for the full walkthrough.

## License

Licensed under either of

 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
