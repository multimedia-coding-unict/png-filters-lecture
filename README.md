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
`minentropy`). It encodes an image, verifies the decode round-trip, and exports
the result plus the per-scanline filter decisions:

```bash
harness/run_harness.sh [--out DIR] [--mode MODE | --sweep] [--repeat N] IMAGE [IMAGE...]
```

- `--sweep` runs every mode and compares them in one `times.json`.
- `--repeat N` repeats each encode and reports the minimum, which smooths out
  timing noise.
- `FILTER_MODE` and `FILTER_REPEAT` can be used instead of the flags.

Results land in `results/` (gitignored):

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

`times.json` separates the whole encode (`encode_total_ns`) from only the
filter prediction path (`filter_prediction_ns`), measured inside the encoder via
`Encoder::set_filter_timing`. Each entry also carries the individual runs and
min/median/mean aggregates. With `--sweep`, the per-mode files live in
`results/<image>/<mode>/` and the aggregate `times.json` stays at the top.

The input must be an 8-bit, non-indexed PNG (gray, gray+alpha, RGB or RGBA).
See `docs/src/05-debug-harness.md` for the full walkthrough.

## License

Licensed under either of

 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
