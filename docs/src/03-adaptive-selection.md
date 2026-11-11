# Adaptive selection: how this encoder picks a filter per row

## The public API: the `Filter` modes

Users of the crate choose a **mode** through the encoder's filter setting:
a fixed filter, or a per-row heuristic. `Adaptive` - pick per row with a
heuristic - is the default; `MinEntropy` picks per row with an entropy
heuristic.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

[`Encoder::set_filter`](https://github.com/image-rs/image-png/blob/master/src/encoder.rs#L336)
(and `StreamWriter::set_filter`):

```rust
pub enum Filter {
    NoFilter,   // always None
    Sub,        // always Sub
    Up,         // always Up
    Avg,        // always Avg
    Paeth,      // always Paeth
    Adaptive,   // pick per row with a heuristic   ← default
    MinEntropy, // pick per row with an entropy heuristic
}
```

([`src/filter/mod.rs` L19–27](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L19-L27);
the default is `Adaptive`, L29–33.)

Fixed modes map 1:1 onto the internal
[`RowFilter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L88-L97)
(`from_method`). The convenience
[`Compression`](https://github.com/image-rs/image-png/blob/master/src/common.rs#L356-L377)
setting also picks a filter for you, via
[`Filter::from_simple`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L48-L57)
(called from `Encoder::set_compression`, `src/encoder.rs` L315–318 in this
checkout).

</details>

`Adaptive` and `MinEntropy` don't have a single internal filter type - they
decide **per scanline**, and that decision is what gets stored in the filter
byte.

The `Compression` convenience setting picks a filter for you:

| `Compression` | filter chosen | Rationale |
| --- | --- | --- |
| `NoCompression` | `NoFilter` | without DEFLATE, filtering only adds work |
| `Fastest` | `Up` | a good match for FdeflateUltraFast |
| `Fast` / `Balanced` / `High` | `Adaptive` | best ratio |

## The core loop: per-row choice

Both adaptive modes share one function: it **tries the four predictive filters
on the row and keeps the cheapest**.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

[`adaptive_filter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L498-L526):

```rust
fn adaptive_filter(f: impl Fn(&[u8]) -> u64, /* cost function */
                    bpp, len, previous, current, output) -> RowFilter {
    let mut min_cost: u64 = u64::MAX;
    let mut filter_choice = RowFilter::NoFilter;
    for &filter in [Up, Sub, Avg, Paeth].iter() {          // trial order
        filter_internal(filter, bpp, len, previous, current, output);
        let cost = f(output);                              // score the residuals
        if cost <= min_cost {                              // note: <=
            min_cost = cost;
            filter_choice = filter;
            if cost == 0 {
                return filter_choice;                      // perfect row: stop early
            }
        }
    }
    if filter_choice != Paeth {
        filter_internal(filter_choice, bpp, len, previous, current, output); // re-run winner
    }
    filter_choice
}
```

The only difference between the two adaptive modes is the **cost function**
`f`, wired up in
[`filter`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L528-L546):

```rust
Filter::Adaptive   => adaptive_filter(sum_buffer, ...),
Filter::MinEntropy => adaptive_filter(entropy, ...),
```

</details>

The logic of the algorithm can be summarized as:

- **All four candidates are actually computed** (`filter_internal` writes into
  `output`), then scored. This is "try everything, keep the best" - simple and
  robust, at the cost of ~4× the filtering work per row.
- **Trial order is `Up, Sub, Avg, Paeth` and ties go to the *later* candidate**
  because of `cost <= min_cost`. So on a tie, Paeth beats Avg beats Sub beats
  Up - a deliberate bias towards Paeth, which is best on typical photos.
- **`NoFilter` is never tried** by the adaptive modes - the loop starts at `Up`.
- If the winner isn't Paeth, its output has been overwritten by the later
  trials, so it's recomputed once at the end (the `if filter_choice != Paeth`
  branch). Paeth is last, so its result is still sitting in `output`.
- **Early exit at cost 0**: a perfectly predicted row (all-zero residuals,
  e.g. a solid-colour region with Up) short-circuits the loop.

## Heuristic 1 (default): minimum sum of absolute differences

`Adaptive` scores each candidate row with the classic **"minimum sum of absolute
differences"** heuristic: sum `|b as i8|`
over the filtered row - each residual byte reinterpreted as a signed value, its
magnitude added. Small-magnitude residuals tend to form the concentrated
distribution DEFLATE likes, so a lower score is a good proxy for a smaller file. 

## Heuristic 2: minimum estimated entropy (`MinEntropy`)

`MinEntropy` estimates *how many bits* the residual row would need, without
floating point (implementation originally from **oxipng**):

1. Count occurrences of each byte value in a 256-entry histogram.
2. **Runs of zeros are free**: an all-zero chunk is skipped entirely, and an all-zero row short-circuits
   to cost 0 - same early-exit effect.
3. Estimate `n · H = Σ countᵢ · log2(n / countᵢ)` as `ilog2i(n) − Σ ilog2i(countᵢ)`,
   using the integer approximation
   `ilog2i(i) = i·⌊log2 i⌋ + 2·(i − 2^⌊log2 i⌋)`.

Intuition: entropy directly models what DEFLATE's Huffman stage cares about -
the *distribution* of residual bytes - rather than just their magnitude. 

<details class="impl-ref" open>
<summary>Implementation reference</summary>

First the integer log helper,
[`ilog2i`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L548-L553)
(this computes `i * log2(i)`, so that `ilog2i(n) - sum ilog2i(count)` approximates
`n * H`):

```rust
/// Estimate the value of i * log2(i) without using floating point operations,
/// implementation originally from oxipng.
fn ilog2i(i: u32) -> u32 {
    let log = 32 - i.leading_zeros() - 1;
    i * log + ((i - (1 << log)) << 1)
}
```

Then the cost function itself,
[`entropy`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L555-L603):
histogram scattered into 4 arrays to reduce contention (L566–574), all-zero
8-byte chunks skipped (L562–565), all-zero row short-circuits to cost 0
(L582–584), and the estimate computed via `ilog2i` (L595–600):

```rust
fn entropy(buf: &[u8]) -> u64 {
    let mut counts = [[0_u32; 256]; 4];
    let mut total = 0;

    // Count the number of occurrences of each byte value.
    let mut chunks = buf.chunks_exact(8);
    for chunk in &mut chunks {
        // Runs of zeros are common and very compressible, so treat them as free.
        if chunk == [0; 8] {
            continue;
        }

        // Scatter the counts into 4 separate arrays to reduce contention.
        for j in 0..2 {
            counts[0][chunk[j * 4] as usize] += 1;
            counts[1][chunk[1 + j * 4] as usize] += 1;
            counts[2][chunk[2 + j * 4] as usize] += 1;
            counts[3][chunk[3 + j * 4] as usize] += 1;
        }
        total += 8;
    }
    for &lit in chunks.remainder() {
        counts[0][lit as usize] += 1;
        total += 1;
    }

    // If the input is entirely zeros, short-circuit the entropy calculation.
    if counts[0][0] == total {
        return 0;
    }

    // Consolidate the counts.
    for i in 0..256 {
        counts[0][i] += counts[1][i] + counts[2][i] + counts[3][i];
    }

    // Compute the entropy: ilog2i(n) - sum of ilog2i(count) over non-zero counts.
    let mut entropy = ilog2i(total);
    for &count in &counts[0] {
        if count > 0 {
            entropy = entropy.saturating_sub(ilog2i(count));
        }
    }

    entropy as u64
}
```

</details>
