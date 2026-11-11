# Greedy-adaptive: a custom filter mode that reuses the previous row's choice

Everything on the previous pages is upstream `image-png`. This page describes
the one filter mode that is **not upstream**: `Filter::GreedyAdaptive`, added
in local commit `4d6b45d` ("feat: greedy adaptive mode") in this checkout.

## Why: `Adaptive` pays for four predictions per row

The previous page showed that `Adaptive` runs `filter_internal` on all of
`[Up, Sub, Avg, Paeth]` for **every** scanline and keeps the cheapest - about
4× the filter work of a fixed filter. On the 768×512 kodim photos the filter
path measured ~0.5–0.6% of the whole encode (DEFLATE dominates); on a small
128×128 icon it measured ~2.3% (results below).

The observation behind `GreedyAdaptive`: **consecutive rows of a natural
image usually want the same filter.** If the filter chosen for the previous
row already leaves small residuals on the current row, re-evaluating the
other three candidates is unlikely to pay off.

## The idea in one picture

```text
row i-1:   filter p chosen  -->  remembered in FilterState.previous
row i:     residuals = filter_internal(p, row)
           cost      = sum_buffer(residuals)     # same MSAD heuristic as Adaptive
           mean      = cost / row_len            # per byte
           if mean <= threshold:  keep p         # 1 prediction instead of 4
           else:  full adaptive search, seeded with (p, cost)   # p not recomputed
```

## API: the mode and its threshold

```rust
encoder.set_filter(Filter::GreedyAdaptive);
encoder.set_filter_threshold(2.0);   // optional; the default is 2.0
```

The **threshold** is the *mean absolute residual per byte* at or below which
the previous row's filter is reused. It is compared against the total cost
scaled by the row length (`cost <= threshold * len`). Edge values:

- `0` (negative values are clamped to 0): reuse only a *perfect* row (cost 0,
  e.g. a flat region predicted by `Up`);
- infinity: always reuse the previous filter (except on the first row);
- the default `DEFAULT_GREEDY_THRESHOLD = 2.0` is deliberately conservative.

The setting has no effect on the other filter modes. `StreamWriter` has the
same setter.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

All of the greedy code is local commit `4d6b45d`, not upstream - the paths and
line numbers below are repo-relative, from this checkout:

- `Filter::GreedyAdaptive` variant - `src/filter/mod.rs` L24–36 (variant at L35)
- `DEFAULT_GREEDY_THRESHOLD = 2.0` - `src/filter/mod.rs` L109–115; re-exported
  in `src/lib.rs` L91
- `Encoder::set_filter_threshold` - `src/encoder.rs` L363–376 (clamped at
  L375); option field `filter_threshold` L170, default at L187;
  `StreamWriter::set_filter_threshold` - L1778–1783
- harness wiring: mode `"greedy"` and `--threshold T[,T...]` -
  `examples/filter_harness.rs` L81, L204, L413–418 (a sweep runs the
  thresholds 2/4/8/16 and names the rows `greedy@<t>`)

</details>

## State carried between rows: `FilterState`

The per-row entry point `filter` takes a mutable state
(`src/filter/mod.rs` L641–673, local commit): after choosing, it stores
`state.previous = Some(chosen)` (L671). `FilterState` (L117–144) holds just
the previous row's `RowFilter` and the threshold, and `reset()` forgets the
previous choice - called exactly where prediction itself resets: at each
Adam7 pass in `Writer::zlib_encode_image_data` (`src/encoder.rs` L1086, L1141)
and at each new frame in `StreamWriter::start_frame` (L2028). The first row
of a pass or frame therefore behaves exactly like `Adaptive`.

## The algorithm: `greedy_adaptive_filter` + a seeded search

<details class="impl-ref" open>
<summary>Implementation reference</summary>

`greedy_adaptive_filter` - `src/filter/mod.rs` L601–639 (local commit
`4d6b45d`):

```rust
if let Some(filter) = previous_filter {
    filter_internal(filter, bpp, len, previous, current, output);
    let cost = f(output);
    // `threshold` is a mean per byte: scale it by the row length before comparing
    if cost as f64 <= threshold * len as f64 {
        return filter;                    // reuse: only 1 filter evaluated
    }
    // Reuse the cost just computed, so the fallback does not pay for `filter` twice
    return adaptive_filter_seeded(f, bpp, len, previous, current, output,
                                  Some((filter, cost)));
}
adaptive_filter(f, bpp, len, previous, current, output)   // first row: plain Adaptive
```

The cost function is `sum_buffer` (MSAD) - the same heuristic as `Adaptive`,
not entropy (dispatch at `src/filter/mod.rs` L652–664).

`adaptive_filter_seeded` (L559–599) is the upstream
[`adaptive_filter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L498-L526)
generalized with an `initial: Option<(RowFilter, u64)>` seed: when the loop
over `[Up, Sub, Avg, Paeth]` reaches the seeded filter, it uses the
precomputed cost instead of re-running `filter_internal` (L575–583). The
trial order and the `<=` tie-break are unchanged, so a seeded search returns
the same filter as an unseeded one; the "re-run the winner into `output`"
step now checks which filter's output is actually sitting in the buffer
(`output_filter`, L573, L595–597) instead of assuming Paeth was last.

</details>

Things to point out in the lecture:

- On a row where the previous filter's mean residual is at or below the
  threshold, greedy evaluates **one** candidate instead of four - that is
  where the saved time comes from.
- The fallback is still the full adaptive search, so a bad reuse is always
  corrected: greedy can differ from `Adaptive` only when the previous filter
  was "good enough", never when it was bad.
- The early exit at cost 0 still applies inside the seeded loop.

## Measured with this repo's harness

```bash
harness/run_harness.sh --sweep --repeat 5 \
    tests/benches/kodim02.png tests/benches/kodim07.png \
    tests/benches/kodim17.png tests/benches/kodim23.png
```

A sweep runs greedy at thresholds 2/4/8/16 by default. The numbers below are
the minimum of 5 runs, read from `results/kodim*/times.json` (gitignored).

### Encoded size (bytes; % difference from `adaptive` in parentheses)

| image | adaptive | greedy@2 | greedy@4 | greedy@8 | greedy@16 |
| --- | ---: | ---: | ---: | ---: | ---: |
| kodim02 | 631 220 (ref) | 631 351 (+0.02%) | 639 592 (+1.33%) | 644 163 (+2.05%) | 644 163 (+2.05%) |
| kodim07 | 584 509 (ref) | 584 717 (+0.04%) | 585 112 (+0.10%) | 584 195 (−0.05%) | 584 195 (−0.05%) |
| kodim17 | 625 639 (ref) | 626 039 (+0.06%) | 627 290 (+0.26%) | 625 559 (−0.01%) | 633 101 (+1.19%) |
| kodim23 | 578 737 (ref) | 581 884 (+0.54%) | 588 880 (+1.75%) | 592 829 (+2.44%) | 592 829 (+2.44%) |

### Filter-prediction time only (ms; compression excluded)

| image | adaptive | greedy@2 | greedy@4 | greedy@8 | greedy@16 |
| --- | ---: | ---: | ---: | ---: | ---: |
| kodim02 | 0.487 | 0.490 | 0.358 | 0.233 | 0.247 |
| kodim07 | 0.447 | 0.548 | 0.342 | 0.232 | 0.234 |
| kodim17 | 0.480 | 0.470 | 0.380 | 0.190 | 0.236 |
| kodim23 | 0.490 | 0.422 | 0.260 | 0.228 | 0.224 |

And on a small image, where the filter path is a visible share of the encode
(the 128×128 RGBA icon
`tests/benches/tango-icon-address-book-new-128-rsvg-convert.png`):

| mode | encoded bytes | filter ms | filter % of encode |
| --- | ---: | ---: | ---: |
| adaptive | 12 581 | 0.026 | 2.3 |
| greedy@2 | 12 550 (−0.25%) | 0.019 | 1.7 |

What the measurements show:

- At the default threshold 2, encoded sizes stayed within +0.02%…+0.54% of
  `adaptive` on these four photos; on the icon greedy@2 came out −0.25%
  smaller and its filter path was ~27% faster.
- At threshold 8, filter-prediction time dropped ~48–60% on the photos, with
  encoded sizes ranging from −0.05% to +2.44% relative to `adaptive` depending
  on the image.
- greedy@8 came out *smaller* than `adaptive` on kodim07 and kodim17 - the
  MSAD argmin is only a proxy for the compressed size.
- On these photos the filter path stayed below ~0.7% of the whole encode for
  both modes, so the absolute whole-encode difference was small there; on the
  small icon the filter path was a larger share (2.3% for `adaptive`, 1.7% for
  greedy@2).

> **Note:** the size numbers above are whole-file bytes, not the heuristic
> cost - the heuristic only guided the choice; DEFLATE had the final word.

## Tests

<details class="impl-ref" open>
<summary>Implementation reference</summary>

`src/filter/mod.rs`, bottom of the file (local commit `4d6b45d`):

- `greedy_adaptive_reuses_previous_filter_below_threshold` L863–885 -
  identical rows, threshold 0, previous `Up` → `Up` kept, output all zeros;
- `greedy_adaptive_keeps_previous_filter_with_high_threshold` L887–906 -
  threshold infinity keeps the previous filter unconditionally;
- `greedy_adaptive_falls_back_to_adaptive` L908–939 - threshold 0 with a bad
  previous filter → same choice *and* same output as `Filter::Adaptive`;
- `seeded_adaptive_matches_unseeded` L941–972 - the seeded search returns the
  same filter and output as plain `adaptive_filter` for all four seeds.

</details>
