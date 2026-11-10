# Introduction: where filtering sits in a PNG

## The PNG pipeline in 30 seconds

A PNG file is an 8-byte signature followed by a sequence of **chunks**. For our
purposes the interesting ones are:

```text
+------------+------------------------------------------------------+
| signature  |  89 50 4E 47 0D 0A 1A 0A                             |
+------------+------------------------------------------------------+
| IHDR       |  width, height, bit depth, color type, interlace, ...|
+------------+------------------------------------------------------+
| IDAT ...   |  one zlib (DEFLATE) stream, split over 1+ chunks     |
+------------+------------------------------------------------------+
| IEND       |  end marker                                          |
+------------+------------------------------------------------------+
```

The pixel data inside `IDAT` is **not** raw pixels. Before compression, every
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

In this encoder each filtered scanline is written exactly like that — see
`Writer::zlib_encode_image_data` in `src/encoder.rs` (lines 1017–1018 and
1060–1063), which does `current[0] = filter_type as u8;` / writes
`&[filter_type as u8]` before the row bytes, and the streaming variant in
`StreamWriter::write` (`src/encoder.rs` line 1951 ff.).

## Why filter at all?

DEFLATE (the zlib layer) compresses *bytes*, and it is very good at squeezing
**repetition and small values** (Huffman codes are short for frequent symbols,
and long runs of zeros get special treatment). Natural images are full of
large values that don't repeat — a blue sky row might be
`[135, 206, 235, 135, 206, 235, ...]` — and those compress poorly on their own.

A predictive filter replaces each byte `x` with a **residual**:

```text
residual = x − predictor(neighbouring bytes)      (all arithmetic mod 256)
```

If the predictor is good, the residuals are small numbers clustered around 0,
and DEFLATE loves that. Filtering is **lossless**: the decoder adds the
prediction back (`unfilter`), so pixels round-trip exactly.

> **Note:** filtering is a *preparation* step for compression, not compression itself.
> The code in `src/filter/mod.rs` says exactly this in its doc comment
> (lines 8–14): the filter is "a content-aware method of compressing the range
> of occurring byte values to help the compression algorithm", and it operates
> on **raw bytes of a scanline, not on pixels**.

Spec reference: [PNG specification, section 9 “Filter algorithms”](https://www.w3.org/TR/png/#9Filters)
and the classic [libpng book, chapter 9](http://www.libpng.org/pub/png/book/chapter09.html)
— both are also cited from the source docs in
[`src/filter/mod.rs`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L14).

## The cast of characters in this repo

| Concept | Where it lives |
| --- | --- |
| Public filter choice (`Filter::Adaptive`, `Paeth`, ...) | [`src/filter/mod.rs` L8–33](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L19-L27) |
| Internal per-row filter type byte (`RowFilter`, `0..=4`) | [`src/filter/mod.rs` L59–98](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L62-L68) |
| Filtering a row (encoder side) | [`filter_internal`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L359-L496), dispatched by [`filter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L528-L546) |
| Adaptive per-row choice | [`adaptive_filter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L498-L526) |
| Un-filtering (decoder side) | [`unfilter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L100-L357) |
| Paeth predictor variants | [`src/filter/paeth.rs`](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs) |
| Where the encoder calls the filter | [`src/encoder.rs`](https://github.com/image-rs/image-png/blob/master/src/encoder.rs) — `zlib_encode_image_data` (L906) and `StreamWriter::write` (L1951 in this checkout) |
| Debug output of the per-row choice | `src/encoder.rs` (`set_filter_log` / `filter_log` / `record_filter`, local commits `7bc5cec`, `3fa225c`) + `examples/filter_harness.rs` |

Next page: the five filters themselves, with formulas and worked examples.
