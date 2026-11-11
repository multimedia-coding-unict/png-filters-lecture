# The five PNG scanline filters

> **Convention for the rest of these notes:** the PNG spec describes filtering
> on the 2-D pixel grid, but most of the logic is per scanline. From this page
> on we therefore talk about **one scanline holding the values of a single
> color channel** - a one-dimensional sequence of bytes `x = [x0, x1, …, xn-1]`.
> In a multi-channel image the bytes of all channels are interleaved in the
> same scanline, and **one filter choice is shared by the whole scanline** -
> the examples below just abstract the channels away.
> When a filter needs the row above, "the previous scanline" just means the
> byte sequence at the same indices; the subdivision into scanlines is
> implicit in all examples below.
>
> The authoritative description is the spec - [§7.2 "Scanlines"](https://www.w3.org/TR/png/#7Scanline),
> [§9 "Filtering"](https://www.w3.org/TR/png/#9Filters): the 2-D neighbourhood
> (bytes `a`, `b`, `c`), the exact filter-type definitions and the edge
> behaviour are defined there and not repeated here.

Each byte `x[i]` of the scanline is stored as a residual `x[i] − predictor`,
where only the residual subtraction (and the decoder's addition of the
prediction) wraps mod 256 (`wrapping_sub` / `wrapping_add` in Rust); the
predictor itself is computed exactly, without overflow. The predictor uses up
to three already-reconstructed bytes:
`a` to the left, `b` above, `c` above-left. Offsets are in **bytes**, not
pixels: `a` is `bpp` bytes to the left, where `bpp` is the spec's "bytes per
pixel in prediction" (1 byte for bit depths < 8).

All of this happens on bytes (`u8`), and only the residual subtraction wraps
**mod 256**: the exact integer difference `x[i] − predictor` (which lies in
−255..=255) is reduced into 0..=255, so when `x[i]` is smaller than the
predictor the negative result wraps to the top of the byte range, e.g.
`3 − 5 = −2` is stored as the byte `254`. 

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
enum - exactly the values allowed in the per-scanline filter byte
([`RowFilter::from_u8`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L77-L86)).

</details>

In every schematic below, `prediction` is the per-byte predictor and `res`
the stored residuals (in bold).

### 1. Sub - predict from the left

`residual = x − a`. The first `bpp` bytes of the scanline have no left
neighbour, so `a = 0` and they are stored raw.

**Wins on:** horizontal gradients and smooth colour transitions (`previous` is
never read) - the example's `x` is close to such a gradient, so Sub is already
good:

<pre>
previous   = [16, 18, 11, 20]
x          = [21, 22, 23, 25]
prediction = [ 0, 21, 22, 23]
res        = <b>[21,  1,  1,  2]</b>
</pre>

### 2. Up - predict from above

`residual = x − b`, with `b = previous[i]` the byte at the same index of the
previous scanline. **Wins on:** vertical repetition and flat regions:

<pre>
previous   = [16, 18, 11, 20]
x          = [21, 22, 23, 25]
prediction = [16, 18, 11, 20]
res        = <b>[ 5,  4, 12,  5]</b>
</pre>

With `previous = x` the residuals would be all `0`, which DEFLATE compresses
perfectly.

### 3. Avg - predict from the mean of left and above

`residual = x − ⌊(a + b) / 2⌋`. A compromise between Sub and Up; good when the
image changes smoothly in *both* directions:

<pre>
previous   = [16, 18, 11, 20]
x          = [21, 22, 23, 25]
prediction = [ 8, 19, 16, 21]
res        = <b>[13,  3,  7,  4]</b>
</pre>

(e.g. at `i = 1`: `a = 21`, `b = 18`, so `22 − ⌊(21 + 18) / 2⌋ = 3`.)

Nice implementation detail: the encoder doesn't compute `(a + b) / 2` with a
wider integer. It uses the branchless, overflow-free identity
`(a & b) + ((a ^ b) >> 1)` - see
[aggregate.org/MAGIC](http://aggregate.org/MAGIC/#Average%20of%20Integers).

<details class="impl-ref" open>
<summary>Implementation reference</summary>

[`filter_internal`, Avg arm L440–442](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L440-L442),
with the comment linking to aggregate.org/MAGIC.

</details>

### 4. Paeth - predict from the "closest" neighbour

Paeth picks whichever of `a`, `b`, `c` is closest to the linear estimate
`p = a + b − c`, breaking ties in the order `a`, then `b`, then `c`
([spec §9.4](https://www.w3.org/TR/png/#9Filter-type-4-Paeth)):

<pre>
previous   = [16, 18, 11, 20]
x          = [21, 22, 23, 25]
prediction = [16, 21, 18, 23]
res        = <b>[ 5,  1,  5,  2]</b>
</pre>

Byte by byte (`p = a + b − c`; `pa`/`pb`/`pc` are the distances of `p` from
`a`/`b`/`c`; the smallest distance - the neighbour Paeth picks - is in bold):

| `i`, `x[i]` | `a`, `b`, `c` | `p` | `pa`, `pb`, `pc` | picks | `res = x[i] − pick` |
| --- | --- | --- | --- | --- | --- |
| i=0, x[i]=21 | a=0, b=16, c=0 | 16 | pa=16, pb=**0**, pc=16 | b = 16 | 21 − 16 = **5** |
| i=1, x[i]=22 | a=21, b=18, c=16 | 23 | pa=**2**, pb=5, pc=7 | a = 21 | 22 − 21 = **1** |
| i=2, x[i]=23 | a=22, b=11, c=18 | 15 | pa=7, pb=4, pc=**3** | c = 18 | 23 − 18 = **5** |
| i=3, x[i]=25 | a=23, b=20, c=11 | 32 | pa=**9**, pb=12, pc=21 | a = 23 | 25 − 23 = **2** |

On this input Paeth uses all three neighbours - `b` at the left edge, `a` at
`i = 1` and `i = 3`, `c` (above-left) at `i = 2` - and beats every other filter
(see the comparison at the end of this page).

**Wins on:** most natural photos - edges and textures in arbitrary directions.
The encoder computes it with an optimised **all-unsigned, vectorization-friendly
variant**.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

[`filter_paeth_fpnge`](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L45-L84).
The file contains three provably equivalent implementations - there's even an
(exhaustive, `#[ignore]`d) test that checks all 256³ inputs
([`paeth.rs` L360–375](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L360-L375)).

</details>

<details class="aside">
<summary>Why <code>p = a + b − c</code>?</summary>

It is the value `x` would take if the 2x2 neighbourhood lay on a flat
plane: a plane has zero mixed second difference (`x − a − b + c = 0`), so
`x = a + b − c`. Read it as `p = a + (b − c)` - the left neighbour plus the
vertical gradient measured one row up - or equivalently as
`p = b + (a − c)` - the byte above plus the horizontal gradient. Paeth then
snaps this planar estimate to the closest of the three real neighbours: on
a smooth gradient the residual is near zero, and on an edge it falls back
to whichever neighbour best continues the local trend. 

Paeth never predicts
with `p` itself: an actually observed neighbour is more robust when the
planar estimate is off (edges, noise), and snapping `p` to that neighbour
is exactly what makes the distances simplify to the plain differences
`pa = |b − c|`, `pb = |a − c|`, `pc = |a + b − 2c|` - so the predictor stays
cheap.

</details>

### 0. None - no prediction

Store `x` raw (`previous` is ignored):

<pre>
previous   = [16, 18, 11, 20]
x          = [21, 22, 23, 25]
prediction = [ 0,  0,  0,  0]
res        = <b>[21, 22, 23, 25]</b>
</pre>

Sometimes a *predictive* filter produces residuals larger than the original
bytes (e.g. noisy data), and None is the honest choice.

## All five filters on the same input

The residuals above side by side, with the MSAD cost (`sum |res|`, residuals
read as signed bytes) that `Adaptive` uses on the next page:

<pre>
filter  residuals         sum |res|
None    [21, 22, 23, 25]   91
Sub     [21,  1,  1,  2]   25
Up      [ 5,  4, 12,  5]   26
Avg     [13,  3,  7,  4]   27
Paeth   [<b> 5,  1,  5,  2</b>]   <b>13</b>
</pre>

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
