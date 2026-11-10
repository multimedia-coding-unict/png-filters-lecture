# Seeing the choices: the debug filter harness

This repo adds a small debug facility (local commits `7bc5cec` "first harness"
and `3fa225c` "refined" — not upstream) that records **which filter was chosen
for every scanline**, plus an example program that visualizes it.

## 1. The encoder-side debug output (`src/encoder.rs`)

```rust
// on Encoder:
pub fn set_filter_log(&mut self, on: bool);      // L346 — opt in

// on Writer / StreamWriter:
pub fn filter_log(&self) -> Option<&[u8]>;       // Writer: L617, StreamWriter: L1650
```

Internally: `Options.filter_log: bool` (L170–172) → `Writer::new` creates
`filter_log: Option<Vec<u8>>` (L598, field L509–511) → every place a filter byte
is written also calls `record_filter(filter_type)` (L621–625), which pushes
`filter_type as u8`:

- stored (no compression) branch: records `NoFilter` — `src/encoder.rs` L1000
- fdeflate branch: L1019
- flate2 branch: L1061
- streaming `StreamWriter::write`: L1958–1960

So `filter_log()` returns **one spec filter-type byte (0–4) per scanline**, in
row order for normal images, in Adam7 pass order for interlaced ones.

> **Warning — known gap:** in the `FdeflateUltraFast` branch, if the compressed output would
> be larger than stored, the encoder falls back to writing raw rows with filter
> byte `0` — but that fallback path (`src/encoder.rs` L1038–1041) does **not**
> call `record_filter`. In that corner case the log is shorter than the number
> of scanlines.

## 2. The harness: `examples/filter_harness.rs` + `harness/run_harness.sh`

```bash
# from the repo root:
harness/run_harness.sh [--out DIR] [--mode MODE] IMAGE [IMAGE...]
# or directly:
cargo run --release --example filter_harness -- <input.png> [output_dir] [mode]
# mode: none sub up avg paeth adaptive (default) minentropy   (also: $FILTER_MODE)
```

For each input it (see `examples/filter_harness.rs`):

1. decodes the input (8-bit gray/gray+alpha/RGB/RGBA, non-interlaced preferred),
2. encodes it once with the chosen mode **and `set_filter_log(true)`**
   (L130–131), reading back `writer.filter_log()` (L135),
3. **decodes the result and verifies the round-trip** (L241–249) so the debug
   map can't lie,
4. exports into `<out>/`:

```text
<out>/
  encoded.png          image encoded with the selected mode
  decoded.png          decoder output, re-encoded with NoFilter (plain lossless copy)
  summary.json         {"mode": ..., "encoded_bytes": ..., "bits_per_pixel": ...}
  debug/
    filter_map.png     one row per scanline: its color = the filter chosen for it
    filters.json       [{"row": 0, "filter_id": 4, "filter_name": "paeth"}, ...]
```

The color legend (`filter_color`, `examples/filter_harness.rs` L63–72):

| filter byte | name | color in `filter_map.png` |
| ---: | --- | --- |
| 0 | none | ⬛ black |
| 1 | sub | 🟥 red |
| 2 | up | 🟩 green |
| 3 | avg | 🟦 blue |
| 4 | paeth | 🟨 yellow |

## 3. Real output from this repo

### A 128×128 RGBA icon, all modes (`tests/benches/tango-icon-address-book-new-128-rsvg-convert.png`)

| mode | encoded bytes | bits/pixel |
| --- | ---: | ---: |
| none | 14 674 | 7.17 |
| sub | 15 628 | 7.63 |
| up | 13 101 | 6.40 |
| avg | 16 196 | 7.91 |
| paeth | 12 848 | 6.27 |
| **adaptive** | **12 581** | **6.14** |
| **minentropy** | **12 351** | **6.03** |

Adaptive beats every fixed filter — it gets to pick per row.

Source image vs. its filter map under `adaptive` vs. `minentropy`:

![source icon](assets/tango-icon.png)
![adaptive filter map](assets/tango-filter-map-adaptive.png)
![minentropy filter map](assets/tango-filter-map-minentropy.png)

Read the maps row by row: flat background → green (Up); the book's horizontal
spine → red (Sub); detailed edges → yellow (Paeth).

### A 768×512 photo (`tests/benches/kodim23.png`), adaptive mode

Filter distribution over its 512 rows (from `debug/filters.json`):

| filter | rows |
| --- | ---: |
| up | 129 |
| sub | 1 |
| avg | 270 |
| paeth | 112 |

![kodim23 adaptive filter map](assets/kodim23-filter-map-adaptive.png)

(Generated maps live under `results/` — gitignored — e.g. `results/kodak_x4/`.)

## 4. Reading `filters.json` in a lecture

```json
{ "row": 0, "filter_id": 4, "filter_name": "paeth" }
```
One entry per scanline — you can `jq` it live, e.g.
`jq '[.filters[].filter_name] | group_by(.) | map({(.[0]): length}) | add' debug/filters.json`
to get the distribution table above.

> **Note:** `filter_map.png` requires non-interlaced input (one log entry per row);
> `write_filter_map` errors out otherwise
> (`examples/filter_harness.rs` L148–155).
