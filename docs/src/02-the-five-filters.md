# The five PNG scanline filters

## The neighbourhood

Every byte of the current scanline is predicted from already-*reconstructed*
bytes (the decoder un-filters left-to-right, top-to-bottom, so these are always
available):

```text
previous row:      c   b
current row:       a   x   ←  x is the byte being filtered
```

- `a` = byte immediately to the **left** of `x` (same channel of the previous pixel)
- `b` = byte **above** `x` (previous scanline, same column)
- `c` = byte **above-left**

All arithmetic is **modulo 256** (`wrapping_sub` / `wrapping_add` in Rust —
that's why you see `wrapping_*` everywhere in the code).

**Important:** the offsets are in *bytes*, not pixels. For RGB (3 bytes/pixel),
`a` is 3 bytes to the left; this distance is called **bpp** ("bytes per pixel
in prediction"). See [`BytesPerPixel`](https://github.com/image-rs/image-png/blob/master/src/common.rs#L112-L119)
and [`Info::bpp_in_prediction`](https://github.com/image-rs/image-png/blob/master/src/common.rs#L831-L833).
For bit depths < 8, `bpp` is 1 byte — one byte holds several sub-byte pixels,
which is why the code comments in `src/common.rs` L86–90 and L818–822 warn that
`bytes_per_pixel` must not be "fixed" for prediction: the PNG spec defines it
this way.

## The five filters (spec §9, filter-type bytes 0–4)

| Byte | Name | Residual stored = `x −` ... | Code |
| ---: | --- | --- | --- |
| 0 | None | `0` (raw byte) | [`NoFilter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L375-L378) |
| 1 | Sub | `a` | [`Sub`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L379-L401) |
| 2 | Up | `b` | [`Up`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L402-L422) |
| 3 | Avg | `⌊(a + b) / 2⌋` | [`Avg`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L423-L460) |
| 4 | Paeth | `PaethPredictor(a, b, c)` | [`Paeth`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L461-L494) |

These five are the internal
[`RowFilter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L62-L68)
enum — exactly the values allowed in the per-scanline filter byte
([`RowFilter::from_u8`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L77-L86)).

### 1. Sub — predict from the left

`residual = x − a`. The first `bpp` bytes of a row have no left neighbour, so
`a = 0` and they are stored raw (see `output[..bpp].copy_from_slice(...)` at
[`filter_internal` L399](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L399)).

**Wins on:** horizontal gradients and smooth colour transitions.
Example, `bpp = 1`, row `[10, 20, 30, 40]` → Sub residuals `[10, 10, 10, 10]`.

### 2. Up — predict from above

`residual = x − b`. **Wins on:** vertical repetition, flat regions, anything
where the row looks like the previous row. Row `[10, 20, 30, 40]` filtered
with the identical previous row → `[0, 0, 0, 0]`. Perfect for DEFLATE.

### 3. Avg — predict from the mean of left and above

`residual = x − ⌊(a + b) / 2⌋`. A compromise between Sub and Up; good when the
image changes smoothly in *both* directions.

Nice implementation detail: the encoder doesn't compute `(a + b) / 2` with a
wider integer. It uses the branchless, overflow-free identity
`(a & b) + ((a ^ b) >> 1)` — see
[`filter_internal`, Avg arm L440–442](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L440-L442)
with the comment linking to [aggregate.org/MAGIC](http://aggregate.org/MAGIC/#Average%20of%20Integers).

### 4. Paeth — predict from the "closest" neighbour

The Paeth predictor picks whichever of `a`, `b`, `c` is closest to the linear
estimate `p = a + b − c`:

```text
pa = |p − a| = |b − c|
pb = |p − b| = |a − c|
pc = |p − c| = |a + b − 2c|

if pa ≤ pb and pa ≤ pc → a
else if pb ≤ pc        → b
else                   → c        (ties: a, then b — spec-mandated order)
```

**Wins on:** most natural photos — edges and textures in arbitrary directions.
The actual byte-level computation used by the encoder is the optimised
**FPNGE** variant [`filter_paeth_fpnge`](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L45-L84)
(all-unsigned, vectorization-friendly, proposed by Luca Versari). The file
contains three provably equivalent implementations — there's even an
(exhaustive, `#[ignore]`d) test that checks all 256³ inputs
([`paeth.rs` L360–375](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L360-L375)).

### 0. None — no prediction

Store `x` raw. Sometimes the residual is *larger* than the original byte (e.g.
noisy data), and None is the honest choice.

## Worked example

`bpp = 1` (8-bit grayscale). Previous row `b-row = [10, 20, 30, 40]`,
current row `x = [12, 19, 33, 41]`:

| filter | residuals (mod 256) | comment |
| --- | --- | --- |
| None | `12, 19, 33, 41` | raw |
| Sub | `12, 7, 14, 8` | `x − left` |
| Up | `2, 255, 3, 1` | `19 − 20 = −1 → 255` (wraps!) |
| Avg | `7, 3, 9, 5` | `x − ⌊(a+b)/2⌋` |
| Paeth | `2, 255, 3, 1` | here Paeth picks `b`, same as Up |

## Edge case: the first row

There is no previous row. The encoder simply starts with a **zeroed previous
row** (`zeroed_prev` in `src/encoder.rs` L963), and the decoder special-cases
an empty previous row in
[`unfilter` L108–115](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L108-L115):

```rust
if previous.is_empty() {
    if filter == Paeth { filter = Sub; }
    else if filter == Up { filter = NoFilter; }
}
```

which matches the spec's "treat out-of-bounds bytes as 0" rule: with `b = c = 0`,
Paeth degenerates to Sub and Up degenerates to None.

> **Tip for the lecture:** rule of thumb: **Sub loves horizontal structure, Up loves
> vertical structure, Avg loves 2-D smoothness, Paeth loves "whatever is
> closest" — i.e. most photos — and None loves noise.** The next page shows
> how the encoder decides, row by row, which one to use.
