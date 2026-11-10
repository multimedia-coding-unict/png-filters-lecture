# Implementation tour of `image-png`

This page maps the concepts from the previous pages onto the actual code.
Links point at upstream
[`image-rs/image-png` master](https://github.com/image-rs/image-png); line
numbers refer to this checkout (identical for the files below, which the local
debug commits didn't touch — except `src/encoder.rs`, noted separately).

## Dispatch: one entry point per scanline

```rust
pub(crate) fn filter(method: Filter, bpp: BytesPerPixel,
                     previous: &[u8], current: &[u8],
                     output: &mut [u8]) -> RowFilter
```

[`src/filter/mod.rs` L528–546](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L528-L546):
fixed modes go straight to `filter_internal`; `Adaptive`/`MinEntropy` go to
`adaptive_filter` with their cost function. It returns the chosen `RowFilter`,
which the caller stores as the row's filter byte.

## Applying one filter: `filter_internal`

[`src/filter/mod.rs` L359–496](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L359-L496).
Each arm writes residuals with `wrapping_sub` into `output`:

- the row is processed in **32-byte chunks** (`CHUNK_SIZE = 32`, L369–372) so
  the compiler auto-vectorizes the inner loop;
- the **first `bpp` bytes** are special-cased (no left neighbour): Sub copies
  them raw (L399), Avg uses `previous[i] / 2` (L456–458), Paeth calls the
  predictor with `a = c = 0` (L490–492);
- Avg uses the branchless average `(a & b) + ((a ^ b) >> 1)` (L440–442);
- Paeth calls [`filter_paeth_fpnge`](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L45-L84)
  (L475) — the all-unsigned FPNGE formulation, chosen because it vectorizes well.

## Three Paeth predictors in one file

[`src/filter/paeth.rs`](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs):

| function | used for | style |
| --- | --- | --- |
| [`filter_paeth`](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L6-L26) | decode, non-x86_64 | direct spec formulation with `i16` distances |
| [`filter_paeth_stbi`](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L28-L43) | decode, x86_64 | branch-free stb formulation (L28–43) |
| [`filter_paeth_fpnge`](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L45-L84) | **encode** (all platforms) | unsigned-only FPNGE variant |

All three are equivalent — verified by an exhaustive 256³ test
([L360–375](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L360-L375), `#[ignore]`d
because it takes ~20 s in debug builds).

## The decoder side: `unfilter`

Filtering is undone by
[`unfilter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L100-L357)
(same file — the crate keeps both directions together), which adds the
prediction back with `wrapping_add`. Remember the special case for the first
row (L108–115: `Paeth → Sub`, `Up → NoFilter` when `previous` is empty).
There is also an optional `portable_simd` module,
[`src/filter/simd.rs`](https://github.com/image-rs/image-png/blob/master/src/filter/simd.rs)
(behind the `unstable` feature), for fast *decode-time* Paeth on AArch64/x86_64.
Note the encoder's filtering itself is scalar code that relies on
auto-vectorization via the 32-byte chunks — no hand-written SIMD.

## Where the encoder calls `filter`

### `Writer` (whole-image writes): `zlib_encode_image_data`

`src/encoder.rs` L906–1072. One loop handles both non-interlaced and Adam7
interlaced data via a `LineCursor` (L911–960); the previous-row buffer is
**reset to zeros at the start of each Adam7 pass** (`filter_pass`, L973, and
`prev = &zeroed_prev[..]` at L1013/L1056 — matches the spec, where each pass
restarts prediction). Then, depending on the DEFLATE backend:

| backend | lines | what happens per row |
| --- | --- | --- |
| `NoCompression` (stored) | L992–1004 | filter byte `0` written, raw row — no filtering at all |
| `FdeflateUltraFast` (fdeflate crate) | L1005–1046 | `filter(...)` → filter byte + residuals (L1017–1020); if the compressed result would be *larger* than stored, it falls back to stored rows (L1028–1045) |
| `Level(n)` (flate2/zlib) | L1047–1068 | `filter(...)` → filter byte (L1062) + residuals (L1063) into a `ZlibEncoder` |

In all filtered branches the pattern is identical:

```rust
let filter_type = filter(filter_method, bpp, prev, line, &mut current[1..]);
current[0] = filter_type as u8;        // the filter byte prefixes the row
// ... write `current` to the compressor ...
```

### `StreamWriter` (incremental writes)

`src/encoder.rs` L1929–1975 (`impl Write for StreamWriter`): incoming bytes fill
`curr_buf`; when a full scanline is buffered, `filter()` runs (L1951–1957), the
filter byte + `filtered_buf` are written to the active compressor
(Flate2 or fdeflate, L1962–1967), and `prev_buf`/`curr_buf` are swapped (L1969) —
that's how "the row above" is remembered between `write()` calls.

## Bytes per pixel in prediction

`bpp` comes from
[`Info::bpp_in_prediction`](https://github.com/image-rs/image-png/blob/master/src/common.rs#L831-L833)
→ [`BytesPerPixel`](https://github.com/image-rs/image-png/blob/master/src/common.rs#L112-L119)
(`One, Two, Three, Four, Six, Eight`). Sub-8-bit images use `One`: one byte
covers several pixels, per the spec.

## Code map

| What | Where |
| --- | --- |
| `Filter` enum (public API), default `Adaptive` | [`src/filter/mod.rs` L8–33](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L8-L33) |
| `Filter::from_simple` (compression → filter) | [`src/filter/mod.rs` L48–57](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L48-L57) |
| `RowFilter` (filter byte values 0–4) | [`src/filter/mod.rs` L59–98](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L59-L98) |
| `filter_internal` (apply one filter, 32-byte chunks) | [`src/filter/mod.rs` L359–496](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L359-L496) |
| `adaptive_filter` (try 4, keep cheapest) | [`src/filter/mod.rs` L498–526](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L498-L526) |
| `filter` (entry point) | [`src/filter/mod.rs` L528–546](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L528-L546) |
| `sum_buffer` / `entropy` / `ilog2i` (cost functions) | [`src/filter/mod.rs` L548–627](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L548-L627) |
| `unfilter` (decoder) | [`src/filter/mod.rs` L100–357](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L100-L357) |
| Paeth variants | [`src/filter/paeth.rs`](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs) |
| SIMD unfilter (decode, `unstable` feature) | [`src/filter/simd.rs`](https://github.com/image-rs/image-png/blob/master/src/filter/simd.rs) |
| `BytesPerPixel`, `bpp_in_prediction` | [`src/common.rs` L112–119, L831–833](https://github.com/image-rs/image-png/blob/master/src/common.rs#L112-L119) |
| encoder call sites (filter byte + row) | `src/encoder.rs` L906–1072 and L1929–1975 in this checkout; upstream: [encoder.rs](https://github.com/image-rs/image-png/blob/master/src/encoder.rs) |
| debug filter log | `src/encoder.rs` L340–348, L597–625, L1000/1019/1061, L1644–1660 — **local commits `7bc5cec` / `3fa225c`, not upstream** |

Next page: those debug outputs — how to make the encoder *show* its per-row
choices.
