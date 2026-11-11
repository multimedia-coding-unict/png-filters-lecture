# Appendix: implementation and measurements

Reference material for questions during or after the lecture. The preceding
four chapters are the presentation path; consult the details here as needed.

## Source map

<details class="impl-ref" open>
<summary>Implementation reference</summary>

Links below point to upstream `master`; line numbers can drift as upstream
changes. Repo-relative citations explicitly marked **local commit** refer to
this checkout instead.

| Concept | Upstream source |
| --- | --- |
| Public filter modes and default | [`Filter`, L19–33](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L19-L33) |
| Standard filter bytes 0–4 | [`RowFilter`, L62–86](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L62-L86) |
| Encoder-side prediction | [`filter_internal`, L359–496](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L359-L496) |
| Per-row dispatch | [`filter`, L528–546](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L528-L546) |
| Decoder-side reconstruction | [`unfilter`, L100–357](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L100-L357) |
| Filter-byte prefix, fast path | [`encoder.rs`, L977–979](https://github.com/image-rs/image-png/blob/master/src/encoder.rs#L977-L979) |
| Filter-byte prefix, level-based path | [`encoder.rs`, L1019–1021](https://github.com/image-rs/image-png/blob/master/src/encoder.rs#L1019-L1021) |

The corresponding instrumented writes in this checkout are in
`src/encoder.rs` L1101–1103 and L1156–1158; the streaming writes are at
L2092–2097. Logging/timing were added in local commits `7bc5cec`, `3fa225c`
and `3035718`; greedy state was added in local commit `4d6b45d`.

Local debug API: `Encoder::set_filter_log` (L384), `set_filter_timing` (L396),
`Writer::filter_log` / `filter_time` (L672 / L681), and `record_filter` /
`record_filter_time` (L685 / L691), all in `src/encoder.rs` (local commits).

**Logging limitation:** the fast compressor's stored-data fallback writes
filter byte 0 at `src/encoder.rs` L1122 without replacing the already-recorded
trial choices (local debug commits). Its filter map can therefore disagree
with the final file. The lecture's default level-based experiments do not use
that fallback.

</details>

## Filter implementation details

<details class="impl-ref" open>
<summary>Implementation reference</summary>

- Prediction stride: [`BytesPerPixel`, L112–119](https://github.com/image-rs/image-png/blob/master/src/common.rs#L112-L119)
  and [`Info::bpp_in_prediction`, L824–833](https://github.com/image-rs/image-png/blob/master/src/common.rs#L824-L833).
  Packed low-bit-depth data is not expanded before prediction.
- The predictor loop uses 32-byte chunks to encourage autovectorisation
  ([L369–372](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L369-L372)).
- Average uses the overflow-free identity `(a & b) + ((a ^ b) >> 1)`
  ([L435–442](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L435-L442)).
  This computes the exact floored mean without a wider integer; it does **not**
  wrap the sum before division. See [Average of Integers](http://aggregate.org/MAGIC/#Average%20of%20Integers).
- Encoding uses the all-unsigned, vectorisation-friendly
  [`filter_paeth_fpnge`, L45–84](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L45-L84).
  The file contains three equivalent formulations; the ignored exhaustive test
  compares all 256³ inputs ([L360–375](https://github.com/image-rs/image-png/blob/master/src/filter/paeth.rs#L360-L375)).
- On an empty previous row the decoder reduces Paeth to Sub and Up to None
  ([L108–115](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L108-L115)).
  The encoder starts with a zeroed previous row
  ([`encoder.rs`, L924](https://github.com/image-rs/image-png/blob/master/src/encoder.rs#L924));
  in the instrumented checkout it is L1033 (local commits).

</details>

## Adaptive search and scoring

<details class="impl-ref" open>
<summary>Implementation reference</summary>

[`Encoder::set_filter`, L333–335](https://github.com/image-rs/image-png/blob/master/src/encoder.rs#L333-L335)
sets the mode. Fixed modes map to standard filter bytes through
[`RowFilter::from_method`, L88–97](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L88-L97).

The convenience setting
[`Encoder::set_compression`, L312–315](https://github.com/image-rs/image-png/blob/master/src/encoder.rs#L312-L315)
changes **both** the DEFLATE settings and the filter, using
[`Filter::from_simple`, L48–57](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L48-L57):

| Compression preset | Filter | Reason |
| --- | --- | --- |
| NoCompression | None | Stored DEFLATE blocks gain nothing from prediction |
| Fastest | Up | Cheap prediction for the fast compressor |
| Fast / Balanced / High | Adaptive | Per-row heuristic selection |

To override the preset's filter, call `set_filter` **after** `set_compression`.
NoCompression still uses a zlib/DEFLATE stream, but with uncompressed blocks.

The upstream [`adaptive_filter`, L498–526](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L498-L526)
evaluates Up, Sub, Avg, Paeth in that order. The `<=` comparison replaces the
winner on equal costs, but a zero cost returns immediately. With one shared
output buffer, a winner other than the last computed candidate must be
regenerated. The local implementation preserves this behaviour through a
seeded helper (local commit `4d6b45d`).

MSAD is [`sum_buffer`, L606–627](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L606-L627).
Its per-byte contribution is:

```rust,ignore
u64::from((b as i8).unsigned_abs())
```

`unsigned_abs()` handles `-128` correctly; the regression test covers every
possible byte ([L720–730](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L720-L730)).

[`entropy`, L555–603](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L555-L603),
originally from oxipng, skips aligned all-zero 8-byte chunks and scatters
histogram updates across four arrays to reduce contention. Trailing bytes are
counted normally. Let `n` be the number of bytes **retained after skipping**, and
`count_i` their byte frequencies. It approximates:

```text
n * H = n * log2(n) - sum(count_i * log2(count_i))
score = L(n) - sum(L(count_i)), clipped at zero
L(i)  = i * floor(log2(i)) + 2 * (i - 2^floor(log2(i)))
```

[`ilog2i`, L548–553](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L548-L553)
implements `L(i)` for positive inputs; zero-count contributions are omitted.
An all-zero row returns cost zero before the logarithm calculation. Any
constant nonzero row also has score zero because its one count equals `n`.

</details>

## Local greedy implementation

<details class="impl-ref" open>
<summary>Implementation reference</summary>

All citations in this fragment are repo-relative, **local commit `4d6b45d`**,
not upstream:

- `src/filter/mod.rs` L35: `Filter::GreedyAdaptive`; L115:
  `DEFAULT_GREEDY_THRESHOLD = 2.0`; re-exported in `src/lib.rs` L91.
- `src/encoder.rs` L363–376: `Encoder::set_filter_threshold`;
  L1778–1783: the streaming setter. Both clamp negative thresholds to zero.
- `src/filter/mod.rs` L117–144: `FilterState` stores the previous choice and
  threshold. Dispatch at L652–671 uses MSAD for greedy and remembers the choice.
- The state starts empty for each image/frame encode; it resets between Adam7
  passes at `src/encoder.rs` L1086 / L1141, and at streaming frame starts at
  L2028. The first row therefore behaves like Adaptive.
- `greedy_adaptive_filter`, `src/filter/mod.rs` L607–639, first computes and
  scores the remembered filter. It accepts when
  `cost as f64 <= threshold * len as f64`; otherwise it seeds the shared search.
- `adaptive_filter_seeded`, L559–599, reuses the supplied **cost** when the
  ordered search reaches that candidate. Its tie and zero-cost rules match
  unseeded Adaptive. `output_filter` tracks the buffer's actual contents, so
  even the seeded filter may need regenerating if it wins after being
  overwritten. Fallback is not guaranteed to save work over an unseeded search
  that would have exited early.

Tests in `src/filter/mod.rs` cover:

| Test | Lines | What it checks |
| --- | --- | --- |
| `greedy_adaptive_reuses_previous_filter_below_threshold` | 863–885 | Zero-cost Up is reused |
| `greedy_adaptive_keeps_previous_filter_with_high_threshold` | 887–906 | Infinity retains the previous filter |
| `greedy_adaptive_falls_back_to_adaptive` | 908–939 | A rejected candidate gives Adaptive's choice and output |
| `seeded_adaptive_matches_unseeded` | 941–972 | All four seeds match plain search on the test row |

</details>

## Reproducing the experiment

From the repository root, rerun all four photos **and the icon** into a separate
output directory:

```bash
harness/run_harness.sh --out results/lecture --sweep --repeat 5 \
    tests/benches/kodim02.png tests/benches/kodim07.png \
    tests/benches/kodim17.png tests/benches/kodim23.png \
    tests/benches/tango-icon-address-book-new-128-rsvg-convert.png
```

The wrapper uses a release build. The sweep covers fixed modes, Adaptive,
MinEntropy, and greedy thresholds 2/4/8/16. Each encode is checked by decoding
and comparing the pixel bytes.

The lecture tables preserve the saved experiment, not a newly run benchmark.
Its photo data is in `results/kodim*/times.json`; the icon data is in
`results/benches/tango-icon-address-book-new-128-rsvg-convert/times.json`.
Those files and rerun outputs are gitignored. The two icon filter maps in the
lecture match that saved icon run; the displayed icon has the same pixel data.

### Measurement scope

- The harness leaves compression at the default Balanced setting: flate2 at
  its default level (6). It does not request a fast-compression experiment.
- Saved JSON does **not** identify CPU, compiler, dependency versions or active
  flate2 backend. Record these, the checkout revision, features and build flags
  alongside future runs; do not infer the old environment from today's host.
- Whole-encode time includes file creation, headers, filtering, compression,
  output handling and completion. Decoding and figure export are outside it.
- Prediction time brackets each per-row filter call: it includes selection,
  scoring and prediction, with clock-read overhead, but excludes compression.
- Each displayed timing is its own minimum of five measurements. Prediction
  and total minima may come from **different runs**; their ratio is not a
  paired per-run fraction.
- Mode ordering and instrumentation can affect timings. Minima without a
  controlled benchmark and uncertainty analysis do not establish small
  end-to-end speed differences.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

Local harness/debug additions, including local commits `3035718` and `4d6b45d`:

- `examples/filter_harness.rs` L183–222: encode setup and timing; L447–546:
  round-trip check and repeated runs; L580–672: JSON aggregates.
- The first run records choices; later timing runs disable choice logging
  but retain prediction timing (L458–469 / L524–538).
- Default greedy sweep thresholds: L86–87. The CLI rejects non-finite or
  negative thresholds at L322–339, and mode expansion is at L410–430.
- Filter-map colours are defined at L126–135. Each output row is filled with
  its chosen filter's colour at L224–254.
- Default options: `src/encoder.rs` L183–194; compression mapping:
  [`src/common.rs`, L434–448](https://github.com/image-rs/image-png/blob/master/src/common.rs#L434-L448).
- Prediction timing brackets: `src/encoder.rs` L1089–1100 / L1144–1155;
  streaming L2072–2085 (local commits).

</details>

## Full saved measurements

### File size in bytes

Percentages are relative to Adaptive; a negative value means a smaller file.
These are **whole-file bytes**, not residual magnitudes or entropy scores.

| Image | Adaptive | Greedy @2 | Greedy @4 | Greedy @8 | Greedy @16 |
| --- | ---: | ---: | ---: | ---: | ---: |
| kodim02 | 631 220 | 631 351 (+0.02%) | 639 592 (+1.33%) | 644 163 (+2.05%) | 644 163 (+2.05%) |
| kodim07 | 584 509 | 584 717 (+0.04%) | 585 112 (+0.10%) | 584 195 (−0.05%) | 584 195 (−0.05%) |
| kodim17 | 625 639 | 626 039 (+0.06%) | 627 290 (+0.26%) | 625 559 (−0.01%) | 633 101 (+1.19%) |
| kodim23 | 578 737 | 581 884 (+0.54%) | 588 880 (+1.75%) | 592 829 (+2.43%) | 592 829 (+2.43%) |

### Prediction time in milliseconds

Includes selection and scoring, but excludes compression. All timings are
independent minima of five runs.

| Image | Adaptive | Greedy @2 | Greedy @4 | Greedy @8 | Greedy @16 |
| --- | ---: | ---: | ---: | ---: | ---: |
| kodim02 | 0.487 | 0.490 | 0.358 | 0.233 | 0.247 |
| kodim07 | 0.447 | 0.548 | 0.342 | 0.232 | 0.234 |
| kodim17 | 0.480 | 0.470 | 0.380 | 0.190 | 0.236 |
| kodim23 | 0.490 | 0.422 | 0.260 | 0.228 | 0.224 |

The percentage changes on the lecture page were calculated from the unrounded
nanosecond measurements, not these rounded cells.

### MinEntropy comparison and Adaptive baseline

| Image | MinEntropy bytes | MinEntropy prediction ms | Adaptive whole encode ms |
| --- | ---: | ---: | ---: |
| kodim02 | 630 171 | 2.154 | 75.924 |
| kodim07 | 584 458 | 2.110 | 84.226 |
| kodim17 | 624 060 | 2.417 | 74.720 |
| kodim23 | 579 196 | 2.047 | 88.533 |

For the icon, the unrounded prediction minima are 26,314 ns (Adaptive) and
19,175 ns (Greedy @2), a saving of 7,139 ns (27.1%). The whole-encode minima
are 1,162,995 ns and 1,147,427 ns respectively; that observed difference is
not attributable solely to prediction.
