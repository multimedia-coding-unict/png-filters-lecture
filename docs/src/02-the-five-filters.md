# The five PNG scanline filters

Examples use **8-bit grayscale** (`bpp = 1`). PNG filters bytes, not whole
pixels or separate channel planes. RGB8 is interleaved `RGBRGB...`, so its left
neighbour is **three bytes back** (`bpp = 3`). One filter choice covers the
**entire scanline**, including all channels.

For the byte `x`, the predictor can use original bytes `a`, `b`, `c`:

```text
                  i-bpp     i
previous row:  +-----+---+-----+
               |  c  |...|  b  |
               +-----+---+-----+
current row:   |  a  |...|  x  |
               +-----+---+-----+
```

The decoder has already reconstructed these neighbours when it reaches `x`.
For packed bit depths below 8, `bpp = 1`; for 16-bit samples, filtering still
operates on individual bytes, comparing corresponding high or low bytes.

## Wrapping does not lose information

Only the residual subtraction and reconstruction addition wrap modulo 256.
Predictor calculations must preserve their exact mathematical result:
Average must not overflow `a + b`, and Paeth's estimate must not wrap.

```text
x = 10, prediction = 20
stored residual:  (10 - 20) mod 256 = 246
reconstructed x: (246 + 20) mod 256 = 10
```

Here `246` represents a difference of `-10`, not a large prediction error.

## The five filters (spec §9.2, filter-type bytes 0–4)

| Byte | Name | Residual stored |
| ---: | --- | --- |
| 0 | None | `x` (raw byte) |
| 1 | Sub | `x − a` |
| 2 | Up | `x − b` |
| 3 | Avg | `x − ⌊(a + b) / 2⌋` |
| 4 | Paeth | `x − PaethPredictor(a, b, c)` |

Full definitions: [spec §9.2 "Filter types for filter method 0"](https://www.w3.org/TR/png/#9Filter-types).

All five produce the same number of row bytes. What changes is how well those
bytes compress. In the examples, `res` is the stored residual row (in bold).

### 0. None — no prediction

Store `x` unchanged. The specification usually recommends None for
indexed-colour images and packed bit depths below 8: predicting palette
indices or packed samples often does not help.

### 1. Sub — predict from the left

`residual = x − a`. The first `bpp` bytes of the scanline have no left
neighbour, so `a = 0` and they are stored raw.

**Often effective on:** horizontal gradients and smooth colour transitions
(`previous` is never read):

<pre>
previous   = [ 7,  7,  7,  7]
x          = [10, 20, 30, 40]
prediction = [ 0, 10, 20, 30]
res        = <b>[10, 10, 10, 10]</b>
</pre>

### 2. Up — predict from above

`residual = x − b`, with `b = previous[i]` the byte at the same index of the
previous scanline. **Often effective on:** repeated rows and flat regions:

<pre>
previous   = [ 8, 20, 28, 36]
x          = [10, 20, 30, 40]
prediction = [ 8, 20, 28, 36]
res        = <b>[ 2,  0,  2,  4]</b>
</pre>

With `previous = x` the residuals would be all `0`, which DEFLATE compresses
very efficiently—not to zero bytes.

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

For larger inputs, the same rule applies: averaging `200` and `200` gives
`200`, not an overflowing byte sum divided by two.

### 4. Paeth — predict from the "closest" neighbour

Paeth picks whichever of `a`, `b`, `c` is closest to the linear estimate
`p = a + b − c`, breaking ties in the order `a`, then `b`, then `c`
([spec §9.4](https://www.w3.org/TR/png/#9Filter-type-4-Paeth)):

<pre>
previous   = [10, 10, 10, 10]
x          = [10, 20, 30, 40]
prediction = [10, 10, 20, 30]
res        = <b>[ 0, 10, 10, 10]</b>
</pre>

At `i = 0` the edges make `a = c = 0`, so Paeth picks `b = 10`. From `i = 1` on
`b = c = 10`, hence `p = a` and Paeth picks `a` — the same residuals as Sub
**after the first byte**. Sub would give `[10, 10, 10, 10]`.

Paeth can also pick the diagonal neighbour:

```text
a = 100, b = 120, c = 110
p = 100 + 120 - 110 = 110
distances to p: a = 10, b = 10, c = 0  ->  pick c
```

**Often effective on:** natural images, where the useful prediction direction
can change from byte to byte. It is not guaranteed to beat the simpler filters.

## Edge behaviour

Out-of-bounds bytes are treated as 0 ([spec §9.2](https://www.w3.org/TR/png/#9Filter-types)),
so on the first scanline `b = c = 0`: Up degenerates to None and Paeth to Sub.
For the first `bpp` bytes of every row, `a = c = 0`. These rules also apply
independently to each Adam7 pass.

Implementation optimisations and source links are in the
[appendix](05-reference.md#filter-implementation-details).
