# Greedy-adaptive: a custom filter mode that reuses the previous row's choice

Everything so far has described how the PNG codec filters scanlines in its
standard configuration. This page shows what improving an existing codec
looks like in practice: we take the `Adaptive` filter mode of `image-png` and
modify it into `GreedyAdaptive`, which reuses the previous row's filter
whenever that choice is still good enough for the current row.

The stock `Adaptive` mode runs `filter_internal` on all of
`[Up, Sub, Avg, Paeth]` for every scanline and keeps the cheapest - about 4x
the filter work of a fixed filter. Yet **consecutive rows of a natural image usually want the
same filter**: if the previous row's filter already leaves small residuals on
the current row, re-evaluating the other three candidates is unlikely to pay
off.

## The idea in one picture

```rust,noplayground
greedy_adaptive_filter(current_row, previous_row, state):
    if state.previous is Some(p):                  # not the first row of the pass
        residuals = filter_internal(p, current_row, previous_row)
        cost      = sum_buffer(residuals)          # MSAD, same heuristic as Adaptive
        if cost <= threshold * len(current_row):   # mean residual per byte <= threshold
            return (p, residuals)                  # reuse: 1 prediction instead of 4
        else:
            return adaptive_filter_seeded(         # search over [Up, Sub, Avg, Paeth]
                current_row, previous_row,         # with p as the incumbent: p is not
                seed = (p, cost))                  # re-evaluated, and the loop exits as
                                                   # soon as a checked candidate has
                                                   # cost <= p's cost (see below)
    else:
        return adaptive_filter(current_row, previous_row)   # first row: plain Adaptive

adaptive_filter_seeded(..., seed = (p, cost)):
    min_cost, choice = cost, p                     # the checked filter starts as incumbent
    for f in [Up, Sub, Avg, Paeth] \ {p}:          # usual trial order, seed skipped
        residuals = filter_internal(f, current_row, previous_row)
        c = sum_buffer(residuals)
        if c <= min_cost:                          # the checked candidate is selected
            return (f, residuals)                  # EARLY EXIT: replaces Adaptive's cost == 0
    return (p, recompute(p))                       # nothing beat the seed: keep it
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

All of the greedy code is a local commit, not upstream - the paths and
line numbers below are repo-relative, from this checkout:

- `Filter::GreedyAdaptive` variant - `src/filter/mod.rs` L27–39 (variant at L39)
- `DEFAULT_GREEDY_THRESHOLD = 2.0` - `src/filter/mod.rs` L115–119; re-exported
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
(`src/filter/mod.rs` L659–691, local commit): after choosing, it stores
`state.previous = Some(chosen)` (L689). `FilterState` (L123–148) holds just
the previous row's `RowFilter` and the threshold, and `reset()` forgets the
previous choice - called exactly where prediction itself resets: at each
Adam7 pass in `Writer::zlib_encode_image_data` (`src/encoder.rs` L1086, L1141)
and at each new frame in `StreamWriter::start_frame` (L2028). The first row
of a pass or frame therefore behaves exactly like `Adaptive`.

## The algorithm: `greedy_adaptive_filter` + a seeded search

<details class="impl-ref" open>
<summary>Implementation reference</summary>

`greedy_adaptive_filter` - `src/filter/mod.rs` L625–657 (local commit):

```rust
if let Some(filter) = previous_filter {
    filter_internal(filter, bpp, len, previous, current, output);
    let cost = f(output);
    // `threshold` is a mean per byte: scale it by the row length before comparing
    if cost as f64 <= threshold * len as f64 {
        return filter;                    // reuse: only 1 filter evaluated
    }
    // Reuse the cost just computed: the fallback starts from `filter` as the
    // incumbent, so it does not pay for it a second time
    return adaptive_filter_seeded(f, bpp, len, previous, current, output,
                                  Some((filter, cost)));
}
adaptive_filter(f, bpp, len, previous, current, output)   // first row: plain Adaptive
```

The cost function is `sum_buffer` (MSAD) - the same heuristic as `Adaptive`,
not entropy (dispatch at `src/filter/mod.rs` L670–687).

`adaptive_filter_seeded` (L570–614) is the upstream
[`adaptive_filter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L498-L526)
generalized with an `initial: Option<(RowFilter, u64)>` seed. **Without a
seed** it is the plain adaptive search, unchanged: trial order
`[Up, Sub, Avg, Paeth]`, `<=` tie-break, early exit at cost 0 (L604). **With
a seed** the checked filter becomes the incumbent: `min_cost` starts at the
seed's cost and `filter_choice` at the seed (L584–585), the loop skips the
seed itself (L590–593) - its filtered row is already in `output` and its
cost is already known - and the early exit becomes *selection-based* instead
of cost-based: as soon as a checked candidate has `cost <= min_cost`, it is
selected and the loop breaks (L598–606). That selection-based early exit
**replaces** the cost 0 early exit inside the greedy fallback. The "re-run
the winner into `output`" step still checks which filter's output is actually
sitting in the buffer (`output_filter`, L587, L610–612) instead of assuming
Paeth was last.

</details>

Things to point out in the lecture:

- On a row where the previous filter's mean residual is at or below the
  threshold, greedy evaluates **one** candidate instead of four - that is
  where most of the saved time comes from.
- When the threshold fails, the fallback no longer runs all four candidates:
  the previously checked filter is the incumbent, and the search exits as
  soon as a checked candidate is selected over it (`cost <= min_cost`,
  L604). The fallback therefore evaluates between one and three candidates
  instead of always four.
- Because the loop stops at the *first* selected candidate instead of
  letting the *last* global minimum win the `<=` tie-break, the greedy
  fallback can now pick a different filter than `Adaptive` even when the
  previous filter was bad: on kodim17, greedy@8 chose Avg on 78% of rows
  where `adaptive` chose it on 51% (histograms read from
  `results/<image>/<mode>/debug/filters.json`).
- The cost 0 early exit is unchanged for `Adaptive`/`MinEntropy` themselves
  (they call the search unseeded) and for the first row of a pass or frame.

## Measured with this repo's harness

```bash
harness/run_harness.sh --sweep --repeat 5 \
    tests/benches/kodim02.png tests/benches/kodim07.png \
    tests/benches/kodim17.png tests/benches/kodim23.png \
    tests/benches/tango-icon-address-book-new-128-rsvg-convert.png
```

A sweep runs greedy at thresholds 2/4/8/16 by default. The numbers below are
the minimum of 5 runs, read from `results/<image>/times.json` (gitignored).

### Encoded size (bytes; % difference from `adaptive` in parentheses)

| image | adaptive | greedy@2 | greedy@4 | greedy@8 | greedy@16 |
| --- | ---: | ---: | ---: | ---: | ---: |
| kodim02 | 631 220 (ref) | 631 032 (-0.03%) | 638 811 (+1.20%) | 644 163 (+2.05%) | 644 163 (+2.05%) |
| kodim07 | 584 509 (ref) | 584 717 (+0.04%) | 585 112 (+0.10%) | 584 195 (-0.05%) | 584 195 (-0.05%) |
| kodim17 | 625 639 (ref) | 626 079 (+0.07%) | 627 284 (+0.26%) | 625 559 (-0.01%) | 633 101 (+1.19%) |
| kodim23 | 578 737 (ref) | 579 902 (+0.20%) | 588 857 (+1.75%) | 592 829 (+2.44%) | 592 829 (+2.44%) |

### Filter-prediction time only (ms; compression excluded)

| image | adaptive | greedy@2 | greedy@4 | greedy@8 | greedy@16 |
| --- | ---: | ---: | ---: | ---: | ---: |
| kodim02 | 0.434 | 0.449 | 0.337 | 0.221 | 0.206 |
| kodim07 | 0.442 | 0.489 | 0.301 | 0.204 | 0.212 |
| kodim17 | 0.451 | 0.454 | 0.343 | 0.164 | 0.195 |
| kodim23 | 0.413 | 0.386 | 0.212 | 0.203 | 0.199 |

And on a small image, where the filter path is a visible share of the encode
(the 128×128 RGBA icon
`tests/benches/tango-icon-address-book-new-128-rsvg-convert.png`):

| mode | encoded bytes | filter ms | filter % of encode |
| --- | ---: | ---: | ---: |
| adaptive | 12 581 | 0.025 | 2.3 |
| greedy@2 | 12 597 (+0.13%) | 0.017 | 1.6 |

What the measurements show:

- At the default threshold 2, encoded sizes stayed within -0.03%...+0.20% of
  `adaptive` on these four photos; on the icon greedy@2 came out +0.13%
  larger and its filter path was ~32% faster (0.025 -> 0.017 ms).
- At threshold 2 the filter time on the photos is about the same as
  `adaptive` (0.386-0.489 vs 0.413-0.451 ms): the reuse check rarely passes
  on noisy photos, so most rows take the fallback search - which now
  early-exits, but often still evaluates several candidates before one is
  selected. The big savings appear at thresholds 4-16, where more rows pass
  the reuse check.
- At threshold 8, filter-prediction time dropped ~49-64% on the photos, with
  encoded sizes ranging from -0.05% to +2.44% relative to `adaptive`
  depending on the image.
- greedy@8 came out *smaller* than `adaptive` on kodim07 and kodim17 - the
  MSAD argmin is only a proxy for the compressed size.
- On these photos the filter path stayed below ~0.7% of the whole encode for
  both modes, so the absolute whole-encode difference was small there; on the
  small icon the filter path was a larger share (2.3% for `adaptive`, 1.6% for
  greedy@2).

> **Note:** the size numbers above are whole-file bytes, not the heuristic
> cost - the heuristic only guided the choice; DEFLATE had the final word.

## Tests

<details class="impl-ref" open>
<summary>Implementation reference</summary>

`src/filter/mod.rs`, bottom of the file (local commit):

- `greedy_adaptive_reuses_previous_filter_below_threshold` L881–904 -
  identical rows, threshold 0, previous `Up` -> `Up` kept, output all zeros;
- `greedy_adaptive_keeps_previous_filter_with_high_threshold` L905–925 -
  threshold infinity keeps the previous filter unconditionally;
- `greedy_adaptive_fallback_exits_at_first_selected_candidate` L926–967 -
  threshold 0, previous `Up` (cost 280 on the test row): the fallback exits
  at `Sub` (cost 70, the first checked candidate at or below the seed's
  cost), while plain `Adaptive` picks `Paeth` on the same row;
- `greedy_adaptive_first_row_matches_adaptive` L968–999 - no previous
  filter: same choice *and* same output as `Filter::Adaptive`;
- `seeded_adaptive_exits_early_on_selection` L1000–1130 - a counting cost
  function shows the seeded search stops after 1/2/3 evaluations and keeps
  the seed (recomputing its row into `output`) when no candidate is selected,
  while the unseeded search still evaluates all four candidates and still
  exits early at cost 0 on a perfect row.

</details>
