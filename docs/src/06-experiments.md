# Lecture experiments and discussion

Everything below runs from the repo root and takes seconds.

## Live demo script

```bash
# 1. A photo, default adaptive mode — look at the map and the JSON
harness/run_harness.sh tests/benches/kodim23.png
#    → results/kodim23/summary.json, results/kodim23/debug/filter_map.png, filters.json

# 2. Same photo with a single fixed filter, for contrast
harness/run_harness.sh --mode paeth  tests/benches/kodim23.png
harness/run_harness.sh --mode none   tests/benches/kodim23.png

# 3. A text/UI screenshot — watch Up (green) dominate
harness/run_harness.sh tests/benches/lorem_ipsum_screenshot.png

# 4. Compare all modes on one image in a loop, collect bits/pixel
for m in none sub up avg paeth adaptive minentropy; do
  harness/run_harness.sh --out /tmp/fdemo --mode "$m" tests/benches/kodim23.png
  cat "/tmp/fdemo/kodim23/summary.json"
done

# 5. The two heuristics side by side
FILTER_MODE=adaptive   harness/run_harness.sh --out /tmp/fdemo tests/benches/kodim23.png
FILTER_MODE=minentropy harness/run_harness.sh --out /tmp/fdemo tests/benches/kodim23.png
```

Open `results/<image>/debug/filter_map.png` in an image viewer next to
`summary.json`; diff the maps of two modes to see which rows changed their mind.

## Questions to ask the class

1. **Why does a fixed filter sometimes beat adaptive on a specific image?**
   (The heuristic is not the true compressed size — it's a proxy. E.g. on the
   tango icon, `avg` is terrible: why? Residuals around ±128 map to large
   unsigned bytes, and the sum-of-absolute heuristic punishes that.)
2. **Which filter would you expect on a horizontal gradient? On a vertical
   striped pattern? On a chessboard?** Check with `filters.json`.
3. **Why is `none` sometimes the best single choice?** (Noise / already
   compressed data: filtering can *inflate* residuals — every residual is a
   byte, and prediction errors add up mod 256.)
4. **Why does the adaptive loop skip `NoFilter`?** (If all residuals are bad,
   a heuristic can't see that raw bytes would have been better.)
5. **Why ties going to Paeth?** (A deliberate bias — flip it and measure.)
6. **What does the filter byte cost?** 1 byte per scanline: for a 512-row image
   that's 512 bytes, ~0.1% of a 500 KB file — negligible. For a 10 000-row
   1-pixel-wide image it's 10 KB — relevant! (Width-1 images: every row is 1
   byte + 1 filter byte.)
7. **Where does the filter byte live in the file?** Inflate the IDAT stream of
   `results/kodim23/encoded.png` (e.g. `python3 -c "import zlib,sys; ..."`) and
   look at the first byte of each row — it should match `filters.json`.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

- The adaptive candidate list `[Up, Sub, Avg, Paeth]` (`NoFilter` is not tried)
  and the `cost <= min_cost` tie-break:
  [`adaptive_filter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L498-L526).

</details>

## Going further

- Make the adaptive loop also try `NoFilter`, or flip the tie-break to `<` —
  rebuild the harness and re-run demo 4. The debug log makes the effect visible
  immediately.
- Try `MinEntropy` on text vs. photos: the histogram cost models Huffman better,
  so it usually wins where byte *distribution* matters more than byte *magnitude*.
- Interlaced encoding resets prediction per Adam7 pass — the filter log then has
  one entry per pass scanline, in pass order.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

- Patch target: [`adaptive_filter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L498-L526)
  (`src/filter/mod.rs`).
- Adam7 prediction reset: `src/encoder.rs` L973, L1013, L1056 in this checkout;
  per-pass filter log documented on `Writer::filter_log`, L610–616 — **local
  commits `7bc5cec` / `3fa225c`, not upstream**.

</details>

## Build & serve these notes

```bash
cd docs
mdbook serve        # http://localhost:3000 — good for projecting
mdbook build        # static site in docs/book/
```
