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

In this encoder each filtered scanline is written exactly like that.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

`Writer::zlib_encode_image_data` in `src/encoder.rs` (L1017–1018 and L1060–1063
in this checkout) does `current[0] = filter_type as u8;` / writes
`&[filter_type as u8]` before the row bytes; the streaming variant is
`StreamWriter::write` (`src/encoder.rs` L1951 ff. in this checkout). Upstream:
[`src/encoder.rs`](https://github.com/image-rs/image-png/blob/master/src/encoder.rs).

</details>

## Why filter at all?

DEFLATE (the zlib layer) compresses *bytes*, and it is good at squeezing
**repetition and concentrated distributions**: Huffman codes are short for
frequent symbols, and repeated byte sequences get compact encodings. Natural
images are full of neighbouring bytes that are similar but not identical — a
blue sky row might be `[135, 206, 235, 136, 207, 236, ...]` — and those
compress poorly on their own.

A predictive filter replaces each byte `x` with a **residual**:

```text
residual = x − predictor(neighbouring bytes)
```

If the predictor is good, the residuals form a concentrated, repetitive
distribution — small numbers clustered around 0 — which is exactly what
DEFLATE compresses well. Filtering is **lossless**: the decoder adds the
prediction back (`unfilter`), so pixels round-trip exactly.

> **Note:** filtering is a *preparation* step for compression, not compression itself.

Spec reference: [PNG specification, section 9 “Filter algorithms”](https://www.w3.org/TR/png/#9Filters).

<details class="impl-ref" open>
<summary>Implementation reference</summary>

The code says exactly this in the doc comment of
[`src/filter/mod.rs` L8–14](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L8-L14):
the filter is "a content-aware method of compressing the range of occurring byte
values to help the compression algorithm".

| Concept | Where it lives |
| --- | --- |
| Public filter choice (`Filter::Adaptive`, `Paeth`, ...) | [`src/filter/mod.rs` L8–33](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L19-L27) |
| Internal per-row filter type byte (`RowFilter`, `0..=4`) | [`src/filter/mod.rs` L59–98](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L62-L68) |
| Filtering a row (encoder side) | [`filter_internal`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L359-L496), dispatched by [`filter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L528-L546) |
| Adaptive per-row choice | [`adaptive_filter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L498-L526) |
| Un-filtering (decoder side) | [`unfilter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L100-L357) |
| Paeth predictor variants | [`src/filter/paeth.rs`](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs) |
| Where the encoder calls the filter | `src/encoder.rs` — `zlib_encode_image_data` (L906) and `StreamWriter::write` (L1951 in this checkout) |
| Debug output of the per-row choice | `src/encoder.rs` (`set_filter_log` / `filter_log` / `record_filter`, local commits `7bc5cec`, `3fa225c`) + `examples/filter_harness.rs` |

</details>

