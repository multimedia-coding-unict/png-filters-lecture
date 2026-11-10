# Seeing the choices: the debug filter harness

This repo adds a small debug facility (local commits `7bc5cec` "first harness"
and `3fa225c` "refined" — not upstream) that records **which filter was chosen
for every scanline**, plus an example program that visualizes it.

## 1. The encoder-side debug output

Opt in on the `Encoder`, then read the log back from the `Writer` /
`StreamWriter`. Internally, every place a filter byte is written also records
it, so the log holds **one spec filter-type byte (0–4) per scanline**, in row
order for normal images, in Adam7 pass order for interlaced ones.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

```rust
// on Encoder:
pub fn set_filter_log(&mut self, on: bool);      // src/encoder.rs L346 — opt in

// on Writer / StreamWriter:
pub fn filter_log(&self) -> Option<&[u8]>;       // Writer: L617, StreamWriter: L1650
```

Internally: `Options.filter_log: bool` (L170–172) → `Writer::new` creates
`filter_log: Option<Vec<u8>>` (L598, field L509–511) → every filter-byte write
also calls `record_filter(filter_type)` (L621–625), which pushes
`filter_type as u8`:

- stored (no compression) branch: records `NoFilter` — `src/encoder.rs` L1000
- fdeflate branch: L1019
- flate2 branch: L1061
- streaming `StreamWriter::write`: L1958–1960
- stored fallback inside the fdeflate branch: L1038–1041 — **not** recorded
  (the known gap below)

All `src/encoder.rs` line numbers above are from this checkout — local commits
`7bc5cec` / `3fa225c`, not upstream.

</details>

> **Warning — known gap:** in the `FdeflateUltraFast` branch, if the compressed
> output would be larger than stored, the encoder falls back to writing raw rows
> with filter byte `0` — but that fallback path does **not** call
> `record_filter`. In that corner case the log is shorter than the number of
> scanlines.

## 2. The harness: `examples/filter_harness.rs` + `harness/run_harness.sh`

```bash
# from the repo root:
harness/run_harness.sh [--out DIR] [--mode MODE] IMAGE [IMAGE...]
# or directly:
cargo run --release --example filter_harness -- <input.png> [output_dir] [mode]
# mode: none sub up avg paeth adaptive (default) minentropy   (also: $FILTER_MODE)
```

For each input it:

1. decodes the input (8-bit gray/gray+alpha/RGB/RGBA, non-interlaced preferred),
2. encodes it once with the chosen mode **and the filter log switched on**,
   reading the log back from the writer,
3. **decodes the result and verifies the round-trip** so the debug map can't lie,
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

The color legend:

| filter byte | name | color in `filter_map.png` |
| ---: | --- | --- |
| 0 | none | ⬛ black |
| 1 | sub | 🟥 red |
| 2 | up | 🟩 green |
| 3 | avg | 🟦 blue |
| 4 | paeth | 🟨 yellow |

<details class="impl-ref" open>
<summary>Implementation reference</summary>

In `examples/filter_harness.rs` (local commits `7bc5cec` / `3fa225c`): the log
is switched on at L130–131 and read back at L135, the round-trip is verified at
L241–249, the color legend (`filter_color`) is at L63–72, and `write_filter_map`
errors out on interlaced input (L148–155).

</details>

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

> **Note:** `filter_map.png` requires non-interlaced input (one log entry per
> row); `write_filter_map` errors out otherwise.
