# Greedy-adaptive: reusing the previous row's choice

The previous page's selection modes are upstream `image-png`. **Greedy-adaptive
is an experiment local to this repository**, not an extra PNG filter type.

## Why try fewer candidates?

Consecutive rows often favour the same filter. The idea: try the previous
row's filter first; if its residuals are already small, skip the other trials.

This trades a guaranteed MSAD minimum for less prediction work—not a guarantee
about file size or encoding speed.

## The idea in one picture

```text
first row of an image, pass or frame:
    choose as Adaptive does

later rows:
    p = previous row's chosen filter
    residuals = predict(p, row, previous_row)
    cost = sum of signed residual magnitudes
    if cost <= threshold * row_length_in_bytes:
        keep p                         # one prediction and score
    else:
        run the adaptive search        # reuse p's score
    remember the chosen filter for the next row
```

If the threshold test fails, the result matches Adaptive for that row. If it
passes, another candidate might still compress better. Even in the fallback,
the winner's residuals may need regenerating after other trials overwrite them.

## The threshold

The threshold is the **mean absolute signed residual per byte**. Its default
is **2.0**; larger values allow more reuse.

- **0:** accept only all-zero residuals. Zero-cost ties can still make the
  chosen filter differ from Adaptive's first zero-cost candidate.
- **Infinity:** retain the first row's choice throughout that pass or frame.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

Local commit `4d6b45d` ("feat: greedy adaptive mode") adds this API:

```rust,ignore
encoder.set_filter(Filter::GreedyAdaptive);
encoder.set_filter_threshold(2.0); // optional; this is the default
```

The setter clamps negatives to zero and affects only greedy mode.
`StreamWriter` has the same setter. The harness CLI instead accepts only
finite, non-negative thresholds. Detailed source locations and tests are in
the [appendix](05-reference.md#local-greedy-implementation).

</details>

## Measured tradeoffs on four photos

The saved sweep used thresholds 2/4/8/16 and five runs per mode. Below are the
default threshold and threshold 8; negative percentages mean smaller or faster
**relative to Adaptive**.

| Image | Size @2 | Prediction time @2 | Size @8 | Prediction time @8 |
| --- | ---: | ---: | ---: | ---: |
| kodim02 | +0.02% | +0.5% | +2.05% | −52.1% |
| kodim07 | +0.04% | +22.7% | −0.05% | −48.0% |
| kodim17 | +0.06% | −2.1% | −0.01% | −60.3% |
| kodim23 | +0.54% | −13.8% | +2.43% | −53.5% |

- **Threshold 2 is not consistently faster:** kodim07's prediction path was
  about 23% slower, despite the small size differences.
- **Threshold 8 reduces prediction time by about 48–60%** here, with file-size
  changes ranging from −0.05% to +2.43%.
- Two files get slightly smaller at threshold 8: another reminder that the
  MSAD minimum is not necessarily the compressed-size minimum.

Adaptive's prediction path accounts for only **0.53–0.64% of the whole encode**
on these photos. They are 768×512, except portrait kodim17 (512×768).
Reducing that small component does not establish an end-to-end speedup.

## A small icon: relative and absolute savings

For the 128×128 RGBA icon shown on the previous page:

| Mode | File bytes | Prediction µs | Whole encode ms |
| --- | ---: | ---: | ---: |
| Adaptive | 12 581 | 26.3 | 1.163 |
| Greedy @2 | 12 550 (−0.25%) | 19.2 | 1.147 |

Prediction is about **27% faster**, but saves only **7 µs**: about **0.6%** of
the Adaptive whole-encode time if everything else stayed unchanged. Its share
of the measured encode falls from about 2.3% to 1.7%.

> **Note:** times are instrumented minima of five runs, not statistically
> established speedups. Whole-encode time includes file handling; it is not
> just compression CPU time. The saved results do not record the original
> machine/compiler/backend versions.

**Takeaway:** greedy can reduce prediction work, but these measurements show
small absolute savings and no consistent default-threshold win. Whether fast
compression paths benefit more needs a separate experiment.

The [appendix](05-reference.md#reproducing-the-experiment) has the rerun command,
full threshold tables, and timing caveats. Sizes are whole-file bytes, not
heuristic scores.
