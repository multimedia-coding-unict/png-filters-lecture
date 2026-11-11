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

The rationale: filter choice is sticky. On natural images the filter that won
the previous row usually wins the current one too, so `Adaptive` spends most
of its time re-confirming a decision that was already made. Greedy spends one
prediction on the previous row's filter and pays for a search only when that
choice is measurably bad - and even then the fallback is seeded with the
filter just measured, so it is never evaluated twice and stops at the first
candidate that beats it.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

`greedy_adaptive_filter` - `src/filter/mod.rs` L625–657:

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

`adaptive_filter_seeded` (L570–614)
generalizes [`adaptive_filter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L498-L526)
with an `initial: Option<(RowFilter, u64)>` seed. The seed carries a filter
whose filtered row is already in `output`, together with its cost, and the
search starts from it (L584–587):

```rust
let mut min_cost: u64 = initial.map_or(u64::MAX, |(_, cost)| cost);
let mut filter_choice = initial.map_or(RowFilter::NoFilter, |(filter, _)| filter);
let mut output_filter = initial.map(|(filter, _)| filter);  // what sits in `output` now
```

**Without a seed**, `min_cost` starts at `u64::MAX` and the choice at
`NoFilter`: this is the plain adaptive search, unchanged. 
**With a seed**, the previously checked filter becomes the incumbent, and the loop skips the
seed itself (L589–593):

```rust
for &filter in [Up, Sub, Avg, Paeth].iter() {
    if Some(filter) == seed {
        continue;   // incumbent: its row is already in `output`, its cost is known
    }
    filter_internal(filter, bpp, len, previous, current, output);
    output_filter = Some(filter);
    let cost = f(output);
```

The seed is never re-evaluated: the caller already paid for one prediction
with it, and its residual row is still sitting in `output`.

Inside the loop the early exit changes meaning (L598–606):

```rust
    if cost <= min_cost {
        min_cost = cost;
        filter_choice = filter;
        if seed.is_some() || cost == 0 {
            break;   // seeded: exit as soon as a checked candidate is selected
                     // over the incumbent; unseeded: only at cost 0
        }
    }
```

In the greedy fallback this *selection-based* early exit **replaces** the
cost 0 early exit: the loop stops at the first checked candidate with
`cost <= min_cost` instead of running all four to the end. Finally, the
winner's row is re-run into `output` only if it is not already there - the
code checks which filter's output is actually in the buffer (`output_filter`,
L610–612) instead of assuming Paeth was last:

```rust
if output_filter != Some(filter_choice) {
    filter_internal(filter_choice, bpp, len, previous, current, output);
}
```

</details>

Notably:

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
  previous filter was bad.

## Experimental evaluation

```bash
harness/run_harness.sh --sweep --repeat 5 tests/benches/
```

A sweep runs every mode - the five fixed filters, `adaptive`, `minentropy` and
`greedy` at thresholds 2/4/8/16 - on each `*.png` under `tests/benches/`. Every
configuration is encoded 5 times and the minimum is reported, and each result
is decoded again and verified against the input. The folder holds 15 PNGs, but
3 of them are indexed (palette) and the harness skips those (it accepts 8-bit
gray/RGB/RGBA only), so the numbers below cover the remaining 12 images, from
16x16 RGBA icons up to a 3840x2160 RGB diagram.

### Whole bench summed (12 images)

Bytes and times are summed over the 12 images, bits/pixel is their mean;
parenthesized values are the % difference from `adaptive`:

| mode | total bytes | bits/px | encode ms | filter ms | filter % of encode |
| --- | ---: | ---: | ---: | ---: | ---: |
| none | 14 448 865 (-0.3%) | 9.99 (+9.1%) | 965.286 (-23.9%) | 2.539 (-81.3%) | 0.3% |
| sub | 14 617 058 (+0.9%) | 9.71 (+6.0%) | 1 175.938 (-7.3%) | 2.934 (-78.4%) | 0.2% |
| up | 14 899 261 (+2.8%) | 9.71 (+6.0%) | 1 183.362 (-6.7%) | 2.607 (-80.8%) | 0.2% |
| avg | 15 491 911 (+6.9%) | 10.27 (+12.1%) | 1 140.114 (-10.1%) | 3.067 (-77.4%) | 0.3% |
| paeth | 14 835 608 (+2.4%) | 9.30 (+1.5%) | 1 226.103 (-3.3%) | 5.828 (-57.0%) | 0.5% |
| adaptive | 14 489 876 (ref) | 9.16 (ref) | 1 267.907 (ref) | 13.554 (ref) | 1.1% (ref) |
| minentropy | 14 506 534 (+0.1%) | 9.14 (-0.2%) | 1 311.674 (+3.5%) | 46.589 (+243.7%) | 3.6% |
| greedy@2 | 14 485 089 (-0.03%) | 9.17 (+0.1%) | 1 264.165 (-0.3%) | 9.425 (-30.5%) | 0.7% |
| greedy@4 | 14 534 798 (+0.3%) | 9.22 (+0.7%) | 1 270.306 (+0.2%) | 8.618 (-36.4%) | 0.7% |
| greedy@8 | 14 535 147 (+0.3%) | 9.28 (+1.3%) | 1 254.493 (-1.1%) | 7.435 (-45.1%) | 0.6% |
| greedy@16 | 14 612 107 (+0.8%) | 9.41 (+2.7%) | 1 250.774 (-1.4%) | 7.382 (-45.5%) | 0.6% |

(`none` only looks competitive in total bytes because the grayscale newsprint
in the set compresses best unfiltered: on that one image `none` is -14.8%
against `adaptive`.)

### Reading the numbers

- **Threshold 2 is the near-free point.** Summed over the bench, greedy@2 is
  -0.03% in bytes with the filter path already down -30.5% (13.554 -> 9.425 ms).
  On the smoothest and largest image (the 3840x2160 diagram) greedy@2 is both
  -1.85% smaller and -60.9% faster in the filter path (5.360 -> 2.094 ms):
  there the reuse check passes on most rows, so most rows cost one prediction
  instead of four.
- **Higher thresholds trade bytes for filter time.** greedy@8 cuts the filter
  path -45.1% but costs +0.3% bytes summed; greedy@16 -45.5% for +0.8%. The
  losses concentrate in small, graphics-like images: the 128x128 icon grows
  +8.0% at @8 and +24.0% at @16, and Transparency +7.3% at every threshold,
  while the photos and screenshots stay within -1.85%...+2.43%.
- **At high thresholds greedy degenerates into a fixed filter.** At @8,
  kodim02/kodim07/kodim23 pick Paeth on every row and come out byte-identical
  to the fixed `paeth` mode (644 163 / 584 195 / 592 829 bytes); at @16 the
  Exoplanet diagram picks Up on all 2160 rows and matches fixed `up`
  (864 742 bytes). The share of rows that keep the previous row's filter rises
  from 78% on average under `adaptive` to 94% under greedy@8, hitting 100% on
  several images (computed from `debug/filters.json`).
- **Greedy can also win on size.** With the best threshold per image, the
  sweep's greedy-vs-adaptive summary ranges from a +1.85% gain (Exoplanet at
  greedy@2) to a -7.33% loss (Transparency at greedy@2). Part of this is the
  selection-based early exit picking a different filter than adaptive's global
  `<=` argmin: on kodim17, greedy@8 chose Avg on 78% of rows where adaptive
  chose it on 51%, and disagrees with adaptive on 47% of that image's rows.
- **Threshold 2 can come out slightly slower than adaptive on noisy images.**
  kodim07: 0.482 vs 0.374 ms (+28.9%) at @2; kodim17 +1.8%, kodim02 +0.5%. On a
  row where the reuse check fails, greedy pays the check plus the seeded
  search, so when the check almost never passes, the savings only start at
  threshold 4 (kodim23: -49.2% at @4, -53.0% at @8).
- **The whole-encode effect is small on this bench because DEFLATE
  dominates.** The filter path is 1.1% of the whole encode for `adaptive`
  (0.6% for greedy@8), so the summed encode time only moves -1.4%...+0.2%
  (1 267.9 -> 1 250.8...1 270.3 ms). The largest absolute saving is on the
  largest image (5.360 -> 1.595 ms at @16). On small images the filter share
  is bigger (Transparency 7.2%, the screenshot 3.6%, the icons 2-3%) but the
  times are tens of microseconds: the 128x128 icon goes 0.025 -> 0.007 ms at
  @8 (-71.6%) while its whole encode moves 1.115 -> 1.020 ms (-8.5%).
- **One image barely reacts: the Lohengrin newsprint**, saving only
  -4.2%...-4.9% of filter time at every threshold. Halftone newsprint keeps
  the residual mean above even threshold 2 on most rows, so nearly every row
  still runs the fallback search.
- **Contrast with `minentropy`:** same search but an entropy cost, far more
  expensive per candidate: +243.7% filter time summed (46.589 ms) for +0.1%
  bytes. Greedy's win comes from evaluating fewer candidates per row, not from
  a cheaper cost function.

> **Note:** the size numbers above are whole-file bytes, not the heuristic
> cost - the heuristic only guided the choice; DEFLATE had the final word.
