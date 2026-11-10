# Adaptive selection: how this encoder picks a filter per row

## The public API: the `Filter` modes

Users of the crate choose a **mode** through the encoder's filter setting:
a fixed filter, or a per-row heuristic. `Adaptive` — pick per row with a
heuristic — is the default; `MinEntropy` picks per row with an entropy
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

`Adaptive` and `MinEntropy` don't have a single internal filter type — they
decide **per scanline**, and that decision is what gets stored in the filter
byte.

The `Compression` convenience setting picks a filter for you:

| `Compression` | filter chosen | why |
| --- | --- | --- |
| `NoCompression` | `NoFilter` | "with no DEFLATE, filtering would only waste time" |
| `Fastest` | `Up` | "pairs well with FdeflateUltraFast" |
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

Things to point out in the lecture:

- **All four candidates are actually computed** (`filter_internal` writes into
  `output`), then scored. This is "try everything, keep the best" — simple and
  robust, at the cost of ~4× the filtering work per row.
- **Trial order is `Up, Sub, Avg, Paeth` and ties go to the *later* candidate**
  because of `cost <= min_cost`. So on a tie, Paeth beats Avg beats Sub beats
  Up — a deliberate bias towards Paeth, which is best on typical photos.
- **`NoFilter` is never tried** by the adaptive modes — the loop starts at `Up`.
- If the winner isn't Paeth, its output has been overwritten by the later
  trials, so it's recomputed once at the end (the `if filter_choice != Paeth`
  branch). Paeth is last, so its result is still sitting in `output`.
- **Early exit at cost 0**: a perfectly predicted row (all-zero residuals,
  e.g. a solid-colour region with Up) short-circuits the loop.

## Heuristic 1 (default): minimum sum of absolute differences

`Adaptive` scores each candidate row with the classic **"minimum sum of absolute
differences"** heuristic (the same idea libpng uses by default; the spec leaves
filter selection entirely to the encoder — see
[§12.7 "Filter selection"](https://www.w3.org/TR/png/#12Filter-selection)):
sum `|b as i8|`
over the filtered row — each residual byte reinterpreted as a signed value, its
magnitude added. Smaller-magnitude residuals → smaller Huffman-coded values →
smaller file. The cast `(b as i8)` reinterprets `128..=255` as negative, so
`unsigned_abs()` is correct for all bytes.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

[`sum_buffer`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L606-L627):

```rust
fn sum_buffer(buf: &[u8]) -> u64 {
    // per 32-byte chunk:
    acc += u64::from((b as i8).unsigned_abs());
    ...
}
```

There's a regression test for the `-128` edge case
([`sum_buffer_test`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L720-L730)).
The 32-byte chunking exists to help the compiler auto-vectorize (see the
`CHUNK_SIZE` comment at
[`filter_internal` L369–372](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L369-L372)).

</details>

## Heuristic 2: minimum estimated entropy (`MinEntropy`)

`MinEntropy` estimates *how many bits* the residual row would need, without
floating point (implementation originally from **oxipng**):

1. Count occurrences of each byte value in a 256-entry histogram.
2. **Runs of zeros are free**: an all-zero chunk is skipped entirely ("runs of
   zeros are common and very compressible"), and an all-zero row short-circuits
   to cost 0 — same early-exit effect.
3. Estimate `n · H = Σ countᵢ · log2(n / countᵢ)` as `ilog2i(n) − Σ ilog2i(countᵢ)`,
   using the integer approximation
   `ilog2i(i) = i·⌊log2 i⌋ + 2·(i − 2^⌊log2 i⌋)`.

Intuition: entropy directly models what DEFLATE's Huffman stage cares about —
the *distribution* of residual bytes — rather than just their magnitude. In
practice `MinEntropy` often wins a few percent over `Adaptive` on photos, at the
cost of building the histogram per candidate row.

<details class="impl-ref" open>
<summary>Implementation reference</summary>

[`entropy`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L555-L603):
histogram scattered into 4 arrays to reduce contention (L566–574), all-zero
8-byte chunks skipped (L562–565), all-zero row → cost 0 (L582–584), and the
estimate computed via
[`ilog2i`](https://github.com/image-rs/image-png/blob/master/src/filter/mod.rs#L548-L553)
(L595–600).

</details>

## What this means in one picture

```text
for each scanline:
    for f in [Up, Sub, Avg, Paeth]:        # NoFilter not considered
        residuals = filter_internal(f, row)
        cost[f]   = sum_buffer(residuals)  # or entropy(residuals) for MinEntropy
    choose argmin cost[f]   (ties → later in [Up, Sub, Avg, Paeth], i.e. Paeth bias)
    write filter byte = chosen RowFilter as u8, then the residuals
```

Next page: where all of this lives in the encoder, and how to *see* these
decisions thanks to the debug outputs added in this repo.
