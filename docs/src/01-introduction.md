# Introduction: where filtering sits in a PNG

## The PNG pipeline in 30 seconds

A PNG file is an 8-byte signature followed by a sequence of **chunks**. For our
purposes the interesting ones are:

```text
+------------+-----------------------------------------------------+
| signature  | 89 50 4E 47 0D 0A 1A 0A                             |
+------------+-----------------------------------------------------+
| IHDR       | width, height, bit depth, colour type, ...          |
+------------+-----------------------------------------------------+
| IDAT ...   | one zlib (DEFLATE) stream, split over 1+ chunks     |
+------------+-----------------------------------------------------+
| IEND       | end marker                                          |
+------------+-----------------------------------------------------+
```

The `IDAT` payload is **not** a raw pixel array. Before compression, every
scanline is:

1. **filtered** — a reversible prediction step, and
2. prefixed with **one filter-type byte** (values `0..=4`) that tells the
   decoder which filter was used for that row.

```text
IDAT (after inflate) =  [filter byte][filtered row 0]
                       [filter byte][filtered row 1]
                       [filter byte][filtered row 2]
                       ...
```

We start with non-interlaced images. With Adam7 interlacing, each pass has its
own scanlines and prediction restarts at the beginning of each pass.

## Why filter at all?

DEFLATE, inside the zlib wrapper, rewards **repetition and frequent symbols**:
LZ77 encodes repeated byte sequences as matches, and Huffman coding gives
frequent symbols shorter codes. Small byte values are not inherently cheaper.

Neighbouring pixels are often similar without being identical. Prediction can
turn that similarity into repeated residual patterns and a more concentrated
byte distribution.

A predictive filter replaces each byte `x` with a **residual**:

```text
residual = (x - predictor(neighbouring bytes)) mod 256
```

Filtering is **lossless**: the decoder adds the same prediction back, modulo
256, to recover the original byte exactly. Good predictions often produce
small signed differences, but these are only a clue to compressibility—not a
guarantee of a smaller file.

> **Note:** filtering is a *preparation* step for compression, not compression itself.

Spec: [§9 “Filtering”](https://www.w3.org/TR/png/#9Filters).
Source locations are collected in the [implementation appendix](05-reference.md#source-map).
