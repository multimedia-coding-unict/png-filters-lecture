# The five PNG scanline filters

> **Convention for the rest of these notes:** the PNG spec describes filtering
> on the 2-D pixel grid, but most of the logic is per scanline. From this page
> on we therefore talk about **one scanline holding the values of a single
> color channel** — a one-dimensional sequence of bytes `x = [x0, x1, …, xn-1]`.
> When a filter needs the row above, "the previous scanline" just means the
> byte sequence at the same indices; the subdivision into scanlines is
> implicit in all examples below.
>
> The authoritative description is the spec — [§7.2 "Scanlines"](https://www.w3.org/TR/png/#7Scanline),
> [§9 "Filtering"](https://www.w3.org/TR/png/#9Filters): the 2-D neighbourhood
> (bytes `a`, `b`, `c`), the exact filter-type definitions and the edge
> behaviour are defined there and not repeated here.

Each byte `x[i]` of the scanline is stored as a residual
`x[i] − predictor` (all arithmetic mod 256 — `wrapping_sub` / `wrapping_add`
in Rust), where the predictor uses up to three already-reconstructed bytes:
`a` to the left, `b` above, `c` above-left. Offsets are in **bytes**, not
pixels: `a` is `bpp` bytes to the left, where `bpp` is the spec's "bytes per
pixel in prediction" (1 byte for bit depths < 8).

<details class="impl-ref" open>
<summary>Implementation reference</summary>

[`BytesPerPixel`](https://github.com/image-rs/image-png/blob/master/src/common.rs#L112-L119)
and
[`Info::bpp_in_prediction`](https://github.com/image-rs/image-png/blob/master/src/common.rs#L831-L833).
The code comments in `src/common.rs` L86–90 and L818–822 warn that
`bytes_per_pixel` must not be "fixed" for prediction.

</details>

## The five filters (spec §9.2, filter-type bytes 0–4)

| Byte | Name | Residual stored |
| ---: | --- | --- |
| 0 | None | `x` (raw byte) |
| 1 | Sub | `x − a` |
| 2 | Up | `x − b` |
| 3 | Avg | `x − ⌊(a + b) / 2⌋` |
| 4 | Paeth | `x − PaethPredictor(a, b, c)` |

Full definitions: [spec §9.2 "Filter types for filter method 0"](https://www.w3.org/TR/png/#9Filter-types).

<details class="impl-ref" open>
<summary>Implementation reference</summary>

The five arms of
[`filter_internal`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L359-L496):

| Byte | Name | Arm |
| ---: | --- | --- |
| 0 | None | [`NoFilter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L375-L378) |
| 1 | Sub | [`Sub`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L379-L401) |
| 2 | Up | [`Up`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L402-L422) |
| 3 | Avg | [`Avg`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L423-L460) |
| 4 | Paeth | [`Paeth`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L461-L494) |

These five are the internal
[`RowFilter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L62-L68)
enum — exactly the values allowed in the per-scanline filter byte
([`RowFilter::from_u8`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L77-L86)).

</details>

All per-filter examples below use one channel (`bpp = 1`) and the same
schematic: `previous` = the scanline above, `x` = scanline being filtered,
`prediction` = per-byte predictor, `res` = stored residuals (in bold).

### 1. Sub — predict from the left

`residual = x − a`. The first `bpp` bytes of the scanline have no left
neighbour, so `a = 0` and they are stored raw.

**Wins on:** horizontal gradients and smooth colour transitions (`previous` is
never read):

<pre>
previous   = [ 7,  7,  7,  7]
x          = [10, 20, 30, 40]
prediction = [ 0, 10, 20, 30]
res        = <b>[10, 10, 10, 10]</b>
</pre>

<details class="impl-ref" open>
<summary>Implementation reference</summary>

`output[..bpp].copy_from_slice(...)` at
[`filter_internal` L399](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L399).

</details>

### 2. Up — predict from above

`residual = x − b`, with `b = previous[i]` the byte at the same index of the
previous scanline. **Wins on:** vertical repetition and flat regions:

<pre>
previous   = [ 8, 20, 28, 36]
x          = [10, 20, 30, 40]
prediction = [ 8, 20, 28, 36]
res        = <b>[ 2,  0,  2,  4]</b>
</pre>

With `previous = x` the residuals would be all `0`, which DEFLATE compresses
perfectly.

### 3. Avg — predict from the mean of left and above

`residual = x − ⌊(a + b) / 2⌋`. A compromise between Sub and Up; good when the
image changes smoothly in *both* directions:

<pre>
previous   = [ 8, 20, 28, 36]
x          = [10, 20, 30, 40]
prediction = [ 4, 15, 24, 33]
res        = <b>[ 6,  5,  6,  7]</b>
</pre>

(e.g. at `i = 1`: `a = 10`, `b = 20`, so `20 − ⌊(10 + 20) / 2⌋ = 5`.)

Nice implementation detail: the encoder doesn't compute `(a + b) / 2` with a
wider integer. It uses the branchless, overflow-free identity
`(a & b) + ((a ^ b) >> 1)` — see
[aggregate.org/MAGIC](http://aggregate.org/MAGIC/#Average%20of%20Integers).

<details class="impl-ref" open>
<summary>Implementation reference</summary>

[`filter_internal`, Avg arm L440–442](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L440-L442),
with the comment linking to aggregate.org/MAGIC.

</details>

### 4. Paeth — predict from the "closest" neighbour

Paeth picks whichever of `a`, `b`, `c` is closest to the linear estimate
`p = a + b − c`, breaking ties in the order `a`, then `b`
([spec §9.4](https://www.w3.org/TR/png/#9Filter-type-4-Paeth)):

<pre>
previous   = [10, 10, 10, 10]
x          = [10, 20, 30, 40]
prediction = [10, 10, 20, 30]
res        = <b>[ 0, 10, 10, 10]</b>
</pre>

Byte by byte (`p = a + b − c`; ties resolve in the order `a`, `b`, `c`):

<pre>
i=0: a= 0, b=10, c= 0  p=10  ->  picks <b>b=10</b>  res = 10-10 = <b> 0</b>
i=1: a=10, b=10, c=10  p=10  ->  picks <b>a=10</b>  res = 20-10 = <b>10</b>
i=2: a=20, b=10, c=10  p=20  ->  picks <b>a=20</b>  res = 30-20 = <b>10</b>
i=3: a=30, b=10, c=10  p=30  ->  picks <b>a=30</b>  res = 40-30 = <b>10</b>
</pre>

At `i = 0` the edges make `a = c = 0`, so Paeth picks `b = 10`. From `i = 1` on
`b = c = 10`, hence `p = a` and Paeth picks `a` — the same residuals as Sub
here (Up on the same input would give `[0, 10, 20, 30]`).

**Wins on:** most natural photos — edges and textures in arbitrary directions.
The encoder computes it with an optimised **all-unsigned, vectorization-friendly
variant**.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

[`filter_paeth_fpnge`](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L45-L84).
The file contains three provably equivalent implementations — there's even an
(exhaustive, `#[ignore]`d) test that checks all 256³ inputs
([`paeth.rs` L360–375](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L360-L375)).

</details>

### 0. None — no prediction

Store `x` raw (`previous` is ignored):

<pre>
previous   = [ 8, 20, 28, 36]
x          = [10, 20, 30, 40]
prediction = [ 0,  0,  0,  0]
res        = <b>[10, 20, 30, 40]</b>
</pre>

Sometimes a *predictive* filter produces residuals larger than the original
bytes (e.g. noisy data), and None is the honest choice.

## Edge behaviour

Out-of-bounds bytes are treated as 0 ([spec §9.2](https://www.w3.org/TR/png/#9Filter-types)),
so on the first scanline `b = c = 0`: Up degenerates to None and Paeth to Sub.
The encoder just starts from a zeroed previous scanline; the decoder
special-cases an empty previous row.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

`zeroed_prev` in `src/encoder.rs` L963 (this checkout), and
[`unfilter` L108–115](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L108-L115):

```rust
if previous.is_empty() {
    if filter == Paeth { filter = Sub; }
    else if filter == Up { filter = NoFilter; }
}
```

</details>
