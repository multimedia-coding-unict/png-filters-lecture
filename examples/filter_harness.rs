//! Execution harness for experimenting with PNG scanline filter decision modes.
//!
//! For each input image this runs the normal encoding process once per selected
//! filter mode (default: the encoder's own default, `adaptive`), decodes the
//! result back and verifies the round-trip, and exports everything into a
//! per-image output folder:
//!
//! ```text
//! <out>/<image-stem>/
//!   times.json            encoding times, one entry per mode (see below)
//!   <mode>/               only when sweeping; otherwise files land here directly
//!     encoded.png         image encoded with the selected filter mode
//!     decoded.png         decoder output of the above, re-encoded losslessly
//!     summary.json        encoded size and bits/pixel
//!     debug/
//!       filter_map.png    2D map: the color of scanline y is the filter chosen for it
//!       filters.json      per-scanline filter decisions
//! ```
//!
//! Inputs may be PNG files or directories. A directory is searched recursively
//! for `*.png` and its images keep their relative layout under the folder name:
//!
//! ```text
//! <out>/<folder>/<relative-path-without-extension>/
//! ```
//!
//! `times.json` reports, for every mode:
//!
//! - `encode_total_ns`: the whole encode (file creation, header, filtering,
//!   compression and IEND), measured in the harness;
//! - `filter_prediction_ns`: only the per-scanline filter selection and
//!   prediction, measured inside the encoder via `Encoder::set_filter_timing`;
//! - `runs`: the individual measurements, plus min/median/mean aggregates.
//!
//! Usage:
//!
//! ```text
//! cargo run --release --example filter_harness -- \
//!     [--out DIR] [--mode MODE] [--threshold T[,T...]] [--repeat N] INPUT [INPUT...]
//! ```
//!
//! Modes are `none`, `sub`, `up`, `avg`, `paeth`, `adaptive` (default),
//! `minentropy`, `greedy`, or `sweep` to run every mode and compare them.
//! `greedy` uses [`Filter::GreedyAdaptive`], which reuses the previous
//! scanline's filter when its cost is at or below `--threshold` (a mean
//! absolute residual per byte). `--threshold` accepts a comma-separated list,
//! so the greedy mode can be swept over several thresholds in one run; in a
//! sweep the greedy rows are named `greedy@<threshold>`, and a full sweep
//! sweeps greedy over [`SWEEP_GREEDY_THRESHOLDS`] by default. Inputs must be
//! 8-bit PNGs (gray, gray+alpha, RGB or RGBA); non-interlaced input works best
//! since the filter map is one row per scanline. Unsupported images in a
//! folder are reported and skipped.
//!
//! A sweep prints one table per image and, after all images, a cumulative
//! table that sums the encoded bytes and times over the images and averages
//! bits/pixel, so the whole input set can be compared mode by mode. In every
//! numeric table column the percentage difference from the best mode is
//! reported in parentheses (the best mode itself is marked). A sweep also ends
//! with a per-image greedy-vs-`adaptive` gain summary, sorted from the largest
//! gain to the largest loss, using the best greedy threshold per image.

use std::{
    env, fs,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};

use png::{BitDepth, ColorType, Decoder, Encoder, Filter, DEFAULT_GREEDY_THRESHOLD};

/// All selectable modes, in filter-type-byte order.
const MODES: &[(&str, Filter)] = &[
    ("none", Filter::NoFilter),
    ("sub", Filter::Sub),
    ("up", Filter::Up),
    ("avg", Filter::Avg),
    ("paeth", Filter::Paeth),
    ("adaptive", Filter::Adaptive),
    ("minentropy", Filter::MinEntropy),
    ("greedy", Filter::GreedyAdaptive),
];

const DEFAULT_MODE: &str = "adaptive";

/// Greedy thresholds swept by a full sweep unless `--threshold` overrides them.
const SWEEP_GREEDY_THRESHOLDS: &[f64] = &[2.0, 4.0, 8.0, 16.0];

/// One concrete encoding configuration: a filter mode plus the threshold used
/// by [`Filter::GreedyAdaptive`] (ignored by the other modes).
#[derive(Clone)]
struct ModeConfig {
    name: String,
    filter: Filter,
    threshold: f64,
}

impl ModeConfig {
    fn new(name: impl Into<String>, filter: Filter, threshold: f64) -> Self {
        ModeConfig {
            name: name.into(),
            filter,
            threshold,
        }
    }
}

/// Name used for a greedy run, including the threshold so that a threshold
/// sweep produces distinct output folders and table rows.
fn greedy_name(threshold: f64) -> String {
    format!("greedy@{threshold}")
}

/// Name of a filter type byte, as recorded in the filter log.
fn filter_name(id: u8) -> &'static str {
    match id {
        0 => "none",
        1 => "sub",
        2 => "up",
        3 => "avg",
        4 => "paeth",
        _ => "unknown",
    }
}

/// Color used for the filter-decision map, indexed by filter type byte.
fn filter_color(id: u8) -> [u8; 3] {
    match id {
        0 => [0, 0, 0],     // None    - black
        1 => [255, 0, 0],   // Sub     - red
        2 => [0, 255, 0],   // Up      - green
        3 => [0, 0, 255],   // Avg     - blue
        4 => [255, 255, 0], // Paeth   - yellow
        _ => [255, 0, 255], // unknown - magenta
    }
}

fn decode(path: &Path) -> Result<(u32, u32, ColorType, Vec<u8>), Box<dyn std::error::Error>> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = Decoder::new(BufReader::new(file))
        .read_info()
        .map_err(|e| format!("{}: {e}", path.display()))?;

    if reader.info().bit_depth != BitDepth::Eight {
        return Err(format!(
            "{}: only 8-bit images are supported (found {:?})",
            path.display(),
            reader.info().bit_depth
        )
        .into());
    }

    let color_type = reader.info().color_type;
    match color_type {
        ColorType::Grayscale | ColorType::GrayscaleAlpha | ColorType::Rgb | ColorType::Rgba => {}
        ColorType::Indexed => {
            return Err(format!("{}: indexed color input is not supported", path.display()).into())
        }
    }

    let buf_len = reader
        .output_buffer_size()
        .ok_or_else(|| format!("{}: image exceeds size limits", path.display()))?;
    let mut buf = vec![0; buf_len];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("{}: {e}", path.display()))?;

    Ok((info.width, info.height, info.color_type, buf))
}

struct Encoded {
    bytes: u64,
    /// The per-scanline filter decision, empty when recording was not enabled.
    filter_log: Vec<u8>,
    /// Wall-clock time for the whole encode call.
    encode_total: Duration,
    /// Time spent inside the encoder's filter prediction path.
    filter_prediction: Duration,
}

#[allow(clippy::too_many_arguments)]
fn encode(
    path: &Path,
    width: u32,
    height: u32,
    color_type: ColorType,
    data: &[u8],
    mode: Filter,
    threshold: f64,
    record: bool,
) -> Result<Encoded, Box<dyn std::error::Error>> {
    let start = Instant::now();

    let file = fs::File::create(path)?;
    let mut encoder = Encoder::new(
        Box::new(BufWriter::new(file)) as Box<dyn std::io::Write>,
        width,
        height,
    );
    encoder.set_color(color_type);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_filter(mode);
    encoder.set_filter_threshold(threshold);
    encoder.set_filter_log(record);
    encoder.set_filter_timing(true);

    let mut writer = encoder.write_header()?;
    writer.write_image_data(data)?;
    let filter_log = writer.filter_log().unwrap_or_default().to_vec();
    let filter_prediction = writer.filter_time().unwrap_or_default();
    writer.finish()?;

    let encode_total = start.elapsed();
    let bytes = fs::metadata(path)?.len();
    Ok(Encoded {
        bytes,
        filter_log,
        encode_total,
        filter_prediction,
    })
}

fn write_filter_map(
    path: &Path,
    width: u32,
    height: u32,
    log: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    if log.len() != height as usize {
        return Err(format!(
            "filter log has {} entries but the image has {height} scanlines; \
             filter maps require non-interlaced input",
            log.len()
        )
        .into());
    }

    let mut map = Vec::with_capacity(width as usize * height as usize * 3);
    for &id in log {
        map.extend(filter_color(id).repeat(width as usize));
    }
    encode(
        path,
        width,
        height,
        ColorType::Rgb,
        &map,
        Filter::NoFilter,
        DEFAULT_GREEDY_THRESHOLD,
        false,
    )?;
    Ok(())
}

fn write_filter_json(
    path: &Path,
    width: u32,
    height: u32,
    log: &[u8],
) -> Result<(), std::io::Error> {
    let mut json =
        format!("{{\n  \"width\": {width},\n  \"height\": {height},\n  \"filters\": [\n");
    for (row, &id) in log.iter().enumerate() {
        json.push_str(&format!(
            "    {{\"row\": {row}, \"filter_id\": {id}, \"filter_name\": \"{}\"}}{}\n",
            filter_name(id),
            if row + 1 < log.len() { "," } else { "" }
        ));
    }
    json.push_str("  ]\n}\n");
    fs::write(path, json)
}

/// Which filter modes to run.
#[derive(Clone, Copy)]
enum Selection {
    Single(&'static str, Filter),
    Sweep,
}

fn parse_selection(name: &str) -> Result<Selection, String> {
    if name == "sweep" || name == "all" {
        return Ok(Selection::Sweep);
    }
    MODES
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(known, filter)| Selection::Single(known, *filter))
        .ok_or_else(|| {
            format!(
                "unknown filter mode: {name} (expected one of {}, sweep)",
                MODES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
            )
        })
}

struct Args {
    inputs: Vec<PathBuf>,
    out_dir: PathBuf,
    selection: Selection,
    /// Thresholds used by the greedy mode; `None` means the built-in default
    /// ([`DEFAULT_GREEDY_THRESHOLD`], or a full sweep's
    /// [`SWEEP_GREEDY_THRESHOLDS`]).
    thresholds: Option<Vec<f64>>,
    repeat: u32,
}

fn usage() -> String {
    format!(
        "usage: filter_harness [--out DIR] [--mode MODE] [--threshold T[,T...]] [--repeat N] INPUT [INPUT...]\n\
         INPUT may be a PNG file or a directory (searched recursively for *.png)\n\
         modes: {} (default: {DEFAULT_MODE}), sweep\n\
         --out DIR: base output directory (default: results)\n\
         --threshold T[,T...]: greedy threshold(s), mean residual per byte\n\
             (default: {DEFAULT_GREEDY_THRESHOLD}; a sweep sweeps 2, 4, 8, 16)\n\
         --repeat N: encode each mode N times and report the minimum (default: 1)",
        MODES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
    )
}

fn parse_thresholds(value: &str) -> Result<Vec<f64>, String> {
    let mut thresholds = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        let threshold: f64 = part
            .parse()
            .map_err(|_| format!("invalid threshold: {part}"))?;
        if !threshold.is_finite() || threshold < 0.0 {
            return Err(format!(
                "threshold must be a finite, non-negative number: {part}"
            ));
        }
        thresholds.push(threshold);
    }
    if thresholds.is_empty() {
        return Err("--threshold needs at least one value".to_string());
    }
    Ok(thresholds)
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut repeat = 1u32;
    let mut out_dir = PathBuf::from("results");
    let mut selection = parse_selection(DEFAULT_MODE).unwrap();
    let mut thresholds: Option<Vec<f64>> = None;
    let mut inputs = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repeat" | "-n" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| format!("--repeat needs a value\n{}", usage()))?;
                repeat = value
                    .parse()
                    .map_err(|_| format!("invalid repeat count: {value}"))?;
                if repeat == 0 {
                    return Err("--repeat must be at least 1".to_string());
                }
                i += 2;
            }
            "--out" | "-o" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| format!("--out needs a value\n{}", usage()))?;
                out_dir = PathBuf::from(value);
                i += 2;
            }
            "--mode" | "-m" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| format!("--mode needs a value\n{}", usage()))?;
                selection = parse_selection(value)?;
                i += 2;
            }
            "--threshold" | "-t" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| format!("--threshold needs a value\n{}", usage()))?;
                thresholds = Some(parse_thresholds(value)?);
                i += 2;
            }
            arg if arg.starts_with('-') => {
                return Err(format!("unknown option: {arg}\n{}", usage()));
            }
            arg => {
                inputs.push(PathBuf::from(arg));
                i += 1;
            }
        }
    }

    if inputs.is_empty() {
        return Err(usage());
    }

    Ok(Args {
        inputs,
        out_dir,
        selection,
        thresholds,
        repeat,
    })
}

/// Expand the CLI selection into the concrete runs for one image. A greedy
/// mode expands to one run per threshold; the other modes ignore thresholds.
fn mode_configs(selection: Selection, thresholds: &[f64]) -> Vec<ModeConfig> {
    let mut configs = Vec::new();
    let mut push = |name: &str, filter: Filter| {
        if filter == Filter::GreedyAdaptive {
            for &threshold in thresholds {
                configs.push(ModeConfig::new(greedy_name(threshold), filter, threshold));
            }
        } else {
            configs.push(ModeConfig::new(name, filter, DEFAULT_GREEDY_THRESHOLD));
        }
    };

    match selection {
        Selection::Single(name, filter) => push(name, filter),
        Selection::Sweep => {
            for &(name, filter) in MODES {
                push(name, filter);
            }
        }
    }
    configs
}

struct RunTimes {
    encode_total: Duration,
    filter_prediction: Duration,
}

struct ModeResult {
    mode: String,
    encoded_bytes: u64,
    bits_per_pixel: f64,
    runs: Vec<RunTimes>,
}

/// Encode, verify and export one mode. Returns the measurements for `times.json`.
#[allow(clippy::too_many_arguments)]
fn run_mode(
    mode_dir: &Path,
    config: &ModeConfig,
    width: u32,
    height: u32,
    color_type: ColorType,
    raw: &[u8],
    repeat: u32,
) -> Result<ModeResult, Box<dyn std::error::Error>> {
    fs::create_dir_all(mode_dir.join("debug"))?;

    // Encode with the selected filter mode, recording the per-scanline decision.
    let encoded_path = mode_dir.join("encoded.png");
    let first = encode(
        &encoded_path,
        width,
        height,
        color_type,
        raw,
        config.filter,
        config.threshold,
        true,
    )?;
    let bytes = first.bytes;

    // Decode the result again and verify the round-trip against the source.
    let (dwidth, dheight, dcolor, decoded) = decode(&encoded_path)?;
    if (dwidth, dheight, dcolor) != (width, height, color_type) || decoded != raw {
        return Err(format!(
            "round-trip through {} mismatched the input; the exported filter map would be misleading",
            encoded_path.display()
        )
        .into());
    }

    // Export the decoder's output as its own image, re-encoded without any
    // filtering so it is a plain lossless representation of the decoded data.
    encode(
        &mode_dir.join("decoded.png"),
        width,
        height,
        color_type,
        &decoded,
        Filter::NoFilter,
        DEFAULT_GREEDY_THRESHOLD,
        false,
    )?;

    write_filter_map(
        &mode_dir.join("debug").join("filter_map.png"),
        width,
        height,
        &first.filter_log,
    )?;
    write_filter_json(
        &mode_dir.join("debug").join("filters.json"),
        width,
        height,
        &first.filter_log,
    )?;

    let bits_per_pixel = bytes as f64 * 8.0 / (width as f64 * height as f64);
    fs::write(
        mode_dir.join("summary.json"),
        format!(
            "{{\n  \"mode\": \"{}\",\n  \"encoded_bytes\": {bytes},\n  \"bits_per_pixel\": {bits_per_pixel:.4}\n}}\n",
            config.name
        ),
    )?;

    // The first run above already produced the output files; repeat it for
    // timing stability. The encoded bytes are deterministic, so overwriting is
    // harmless.
    let mut runs = vec![RunTimes {
        encode_total: first.encode_total,
        filter_prediction: first.filter_prediction,
    }];
    for _ in 1..repeat {
        let run = encode(
            &encoded_path,
            width,
            height,
            color_type,
            raw,
            config.filter,
            config.threshold,
            false,
        )?;
        runs.push(RunTimes {
            encode_total: run.encode_total,
            filter_prediction: run.filter_prediction,
        });
    }

    Ok(ModeResult {
        mode: config.name.clone(),
        encoded_bytes: bytes,
        bits_per_pixel,
        runs,
    })
}

/// Minimum, median and mean of a set of durations, in nanoseconds.
fn min_median_mean(values: &[Duration]) -> (u64, u64, f64) {
    let ns: Vec<u64> = values.iter().map(|d| d.as_nanos() as u64).collect();
    let min = ns.iter().copied().min().unwrap_or(0);
    let mut sorted = ns.clone();
    sorted.sort_unstable();
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0);
    let mean = if ns.is_empty() {
        0.0
    } else {
        ns.iter().sum::<u64>() as f64 / ns.len() as f64
    };
    (min, median, mean)
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn write_times_json(
    path: &Path,
    input: &Path,
    width: u32,
    height: u32,
    repeat: u32,
    results: &[ModeResult],
) -> Result<(), std::io::Error> {
    let mut json = String::new();
    json.push_str("{\n");
    json.push_str(&format!(
        "  \"input\": \"{}\",\n",
        json_escape(&input.display().to_string())
    ));
    json.push_str(&format!("  \"width\": {width},\n"));
    json.push_str(&format!("  \"height\": {height},\n"));
    json.push_str(&format!("  \"repeat\": {repeat},\n"));
    json.push_str("  \"aggregate\": \"min\",\n");
    json.push_str("  \"modes\": [\n");

    for (index, result) in results.iter().enumerate() {
        let totals: Vec<Duration> = result.runs.iter().map(|run| run.encode_total).collect();
        let filters: Vec<Duration> = result
            .runs
            .iter()
            .map(|run| run.filter_prediction)
            .collect();
        let (total_min, total_median, total_mean) = min_median_mean(&totals);
        let (filter_min, filter_median, filter_mean) = min_median_mean(&filters);
        let filter_fraction = if total_min == 0 {
            0.0
        } else {
            filter_min as f64 / total_min as f64
        };

        json.push_str("    {\n");
        json.push_str(&format!("      \"mode\": \"{}\",\n", result.mode));
        json.push_str(&format!(
            "      \"encoded_bytes\": {},\n",
            result.encoded_bytes
        ));
        json.push_str(&format!(
            "      \"bits_per_pixel\": {:.4},\n",
            result.bits_per_pixel
        ));
        json.push_str("      \"runs\": [\n");
        for (run_index, run) in result.runs.iter().enumerate() {
            json.push_str(&format!(
                "        {{\"encode_total_ns\": {}, \"filter_prediction_ns\": {}}}{}\n",
                run.encode_total.as_nanos(),
                run.filter_prediction.as_nanos(),
                if run_index + 1 < result.runs.len() {
                    ","
                } else {
                    ""
                }
            ));
        }
        json.push_str("      ],\n");
        json.push_str(&format!("      \"encode_total_ns\": {total_min},\n"));
        json.push_str(&format!(
            "      \"encode_total_ms\": {:.6},\n",
            total_min as f64 / 1e6
        ));
        json.push_str(&format!(
            "      \"encode_total_median_ns\": {total_median},\n"
        ));
        json.push_str(&format!(
            "      \"encode_total_mean_ns\": {total_mean:.1},\n"
        ));
        json.push_str(&format!("      \"filter_prediction_ns\": {filter_min},\n"));
        json.push_str(&format!(
            "      \"filter_prediction_ms\": {:.6},\n",
            filter_min as f64 / 1e6
        ));
        json.push_str(&format!(
            "      \"filter_prediction_median_ns\": {filter_median},\n"
        ));
        json.push_str(&format!(
            "      \"filter_prediction_mean_ns\": {filter_mean:.1},\n"
        ));
        json.push_str(&format!(
            "      \"filter_fraction\": {filter_fraction:.4}\n"
        ));
        json.push_str(&format!(
            "    }}{}\n",
            if index + 1 < results.len() { "," } else { "" }
        ));
    }

    json.push_str("  ]\n}\n");
    fs::write(path, json)
}

fn min_duration(values: impl Iterator<Item = Duration>) -> Duration {
    values.min().unwrap_or_default()
}

/// Percentage difference from a column's reference value, formatted as a
/// parenthetical; the reference row itself is marked instead of showing a
/// zero difference.
fn pct_from_reference(value: f64, reference: f64, is_reference_row: bool) -> String {
    if is_reference_row {
        " (ref)".to_string()
    } else if reference == 0.0 {
        String::new()
    } else {
        format!(" ({:+.1}%)", (value - reference) / reference * 100.0)
    }
}

/// Share of the filter prediction path in the whole encode, in percent.
fn filter_percent(result: &ModeResult) -> f64 {
    let total = min_duration(result.runs.iter().map(|run| run.encode_total));
    let filter = min_duration(result.runs.iter().map(|run| run.filter_prediction));
    if total.is_zero() {
        0.0
    } else {
        filter.as_secs_f64() / total.as_secs_f64() * 100.0
    }
}

fn print_table(results: &[ModeResult]) {
    println!();
    println!(
        "parenthesized: % difference from adaptive (the column best if adaptive is absent)"
    );
    // Every cell is a fixed-width, right-aligned numeric field (digits line up
    // across rows) followed by a fixed-width, left-aligned parenthetical.
    println!(
        "{:<12} {:>12}{:<12} {:>7}{:<12} {:>9}{:<12} {:>9}{:<12} {:>8}{:<12}",
        "mode", "bytes", "", "bits/px", "", "enc ms", "", "filt ms", "", "filter %", ""
    );

    // The percentages refer to `adaptive`, the encoder's default; when it is
    // not among the results (e.g. a greedy-threshold-only sweep) they refer to
    // the best (lowest) value in each column instead.
    let adaptive = results.iter().find(|r| r.mode == "adaptive");
    let (ref_bytes, ref_bits, ref_total, ref_filter, ref_percent) = match adaptive {
        Some(a) => (
            a.encoded_bytes as f64,
            a.bits_per_pixel,
            min_duration(a.runs.iter().map(|run| run.encode_total)).as_secs_f64(),
            min_duration(a.runs.iter().map(|run| run.filter_prediction)).as_secs_f64(),
            filter_percent(a),
        ),
        None => (
            results.iter().map(|r| r.encoded_bytes).min().unwrap_or(0) as f64,
            results
                .iter()
                .map(|r| r.bits_per_pixel)
                .fold(f64::INFINITY, f64::min),
            results
                .iter()
                .map(|r| min_duration(r.runs.iter().map(|run| run.encode_total)).as_secs_f64())
                .fold(f64::INFINITY, f64::min),
            results
                .iter()
                .map(|r| min_duration(r.runs.iter().map(|run| run.filter_prediction)).as_secs_f64())
                .fold(f64::INFINITY, f64::min),
            results.iter().map(filter_percent).fold(f64::INFINITY, f64::min),
        ),
    };
    let is_reference_row = |r: &ModeResult| adaptive.is_some_and(|a| a.mode == r.mode);

    for result in results {
        let total_min = min_duration(result.runs.iter().map(|run| run.encode_total));
        let filter_min = min_duration(result.runs.iter().map(|run| run.filter_prediction));
        let percent = filter_percent(result);
        let reference_row = is_reference_row(result);
        println!(
            "{:<12} {:>12}{:<12} {:>7.2}{:<12} {:>9.3}{:<12} {:>9.3}{:<12} {:>8.1}%{:<12}",
            result.mode,
            result.encoded_bytes,
            pct_from_reference(result.encoded_bytes as f64, ref_bytes, reference_row),
            result.bits_per_pixel,
            pct_from_reference(result.bits_per_pixel, ref_bits, reference_row),
            total_min.as_secs_f64() * 1e3,
            pct_from_reference(total_min.as_secs_f64(), ref_total, reference_row),
            filter_min.as_secs_f64() * 1e3,
            pct_from_reference(filter_min.as_secs_f64(), ref_filter, reference_row),
            percent,
            pct_from_reference(percent, ref_percent, reference_row),
        );
    }
}

/// One aggregated row of [`print_cumulative_table`].
struct CumulativeRow {
    mode: String,
    bytes: u64,
    bits_per_pixel: f64,
    total: Duration,
    filter: Duration,
}

/// One table for the whole input set: bytes and times are summed over the
/// images, bits/pixel is the mean of the per-image values. Each numeric column
/// carries the percentage difference from the best mode in parentheses.
fn print_cumulative_table(per_image: &[(String, Vec<ModeResult>)]) {
    let images = per_image.len();
    println!();
    println!("=== cumulative over {images} image(s) ===");
    println!(
        "parenthesized: % difference from adaptive (the column best if adaptive is absent)"
    );
    // Same cell layout as `print_table`: fixed-width numeric field, then the
    // parenthetical in its own fixed-width field, so digits stay aligned.
    println!(
        "{:<12} {:>12}{:<12} {:>7}{:<12} {:>9}{:<12} {:>9}{:<12} {:>8}{:<12}",
        "mode", "total bytes", "", "bits/px", "", "enc ms", "", "filt ms", "", "filter %", ""
    );

    let Some(first) = per_image.first() else {
        return;
    };
    let mut rows: Vec<CumulativeRow> = Vec::new();
    for (mode_index, first_mode) in first.1.iter().enumerate() {
        let mut bytes = 0u64;
        let mut bits_per_pixel = 0.0;
        let mut total = Duration::ZERO;
        let mut filter = Duration::ZERO;
        let mut count = 0usize;

        for (_, results) in per_image {
            let Some(result) = results.get(mode_index) else {
                continue;
            };
            // Modes are always run in the same order; guard against a mismatch
            // rather than silently mixing two different modes in one row.
            debug_assert_eq!(result.mode, first_mode.mode);
            bytes += result.encoded_bytes;
            bits_per_pixel += result.bits_per_pixel;
            total += min_duration(result.runs.iter().map(|run| run.encode_total));
            filter += min_duration(result.runs.iter().map(|run| run.filter_prediction));
            count += 1;
        }

        if count > 0 {
            rows.push(CumulativeRow {
                mode: first_mode.mode.clone(),
                bytes,
                bits_per_pixel: bits_per_pixel / count as f64,
                total,
                filter,
            });
        }
    }

    // The percentages refer to `adaptive` (see `print_table`).
    let percent_of = |row: &CumulativeRow| {
        if row.total.is_zero() {
            0.0
        } else {
            row.filter.as_secs_f64() / row.total.as_secs_f64() * 100.0
        }
    };
    let adaptive = rows.iter().find(|r| r.mode == "adaptive");
    let (ref_bytes, ref_bits, ref_total, ref_filter, ref_percent) = match adaptive {
        Some(a) => (
            a.bytes as f64,
            a.bits_per_pixel,
            a.total.as_secs_f64(),
            a.filter.as_secs_f64(),
            percent_of(a),
        ),
        None => (
            rows.iter().map(|r| r.bytes).min().unwrap_or(0) as f64,
            rows.iter().map(|r| r.bits_per_pixel).fold(f64::INFINITY, f64::min),
            rows.iter().map(|r| r.total.as_secs_f64()).fold(f64::INFINITY, f64::min),
            rows.iter().map(|r| r.filter.as_secs_f64()).fold(f64::INFINITY, f64::min),
            rows.iter().map(&percent_of).fold(f64::INFINITY, f64::min),
        ),
    };
    let is_reference_row = |r: &CumulativeRow| adaptive.is_some_and(|a| a.mode == r.mode);

    for row in &rows {
        let percent = percent_of(row);
        let reference_row = is_reference_row(row);
        println!(
            "{:<12} {:>12}{:<12} {:>7.2}{:<12} {:>9.3}{:<12} {:>9.3}{:<12} {:>8.1}%{:<12}",
            row.mode,
            row.bytes,
            pct_from_reference(row.bytes as f64, ref_bytes, reference_row),
            row.bits_per_pixel,
            pct_from_reference(row.bits_per_pixel, ref_bits, reference_row),
            row.total.as_secs_f64() * 1e3,
            pct_from_reference(row.total.as_secs_f64(), ref_total, reference_row),
            row.filter.as_secs_f64() * 1e3,
            pct_from_reference(row.filter.as_secs_f64(), ref_filter, reference_row),
            percent,
            pct_from_reference(percent, ref_percent, reference_row),
        );
    }
}

/// Per-image summary of the greedy mode against `adaptive`, sorted from the
/// largest gain (smaller encoded bytes) to the largest loss. For each image the
/// best greedy threshold is used; gains are relative to `adaptive`.
fn print_greedy_summary(per_image: &[(String, Vec<ModeResult>)]) {
    let Some((_, first)) = per_image.first() else {
        return;
    };
    let Some(adaptive_index) = first.iter().position(|r| r.mode == "adaptive") else {
        return;
    };
    let greedy_indices: Vec<usize> = first
        .iter()
        .enumerate()
        .filter(|(_, result)| result.mode.starts_with("greedy@"))
        .map(|(index, _)| index)
        .collect();
    if greedy_indices.is_empty() {
        return;
    }

    // (image, best greedy mode, adaptive bytes, greedy bytes, gain %)
    let mut rows: Vec<(String, String, u64, u64, f64)> = Vec::new();
    for (name, results) in per_image {
        let adaptive = &results[adaptive_index];
        let mut best: Option<(usize, f64)> = None;
        for &index in &greedy_indices {
            let gain = (adaptive.encoded_bytes as f64 - results[index].encoded_bytes as f64)
                / adaptive.encoded_bytes as f64
                * 100.0;
            if best.map_or(true, |(_, best_gain)| gain > best_gain) {
                best = Some((index, gain));
            }
        }
        if let Some((index, gain)) = best {
            rows.push((
                name.clone(),
                results[index].mode.clone(),
                adaptive.encoded_bytes,
                results[index].encoded_bytes,
                gain,
            ));
        }
    }

    rows.sort_by(|a, b| b.4.total_cmp(&a.4));

    println!();
    println!(
        "=== greedy vs adaptive, best threshold per image, sorted from the largest gain to the largest loss ==="
    );
    println!(
        "{:<40} {:>12} {:>12} {:>10} {:>10} {:>10}",
        "image", "adaptive B", "greedy B", "mode", "saved B", "gain %"
    );
    for (name, mode, adaptive_bytes, greedy_bytes, gain) in &rows {
        let saved = *adaptive_bytes as i64 - *greedy_bytes as i64;
        println!(
            "{:<40.40} {:>12} {:>12} {:>10} {:>+10} {:>+9.2}%",
            name, adaptive_bytes, greedy_bytes, mode, saved, gain
        );
    }
}

fn is_png(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.to_string_lossy().eq_ignore_ascii_case("png"))
        .unwrap_or(false)
}

fn collect_pngs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_pngs(&path, out)?;
        } else if file_type.is_file() && is_png(&path) {
            out.push(path);
        }
    }
    Ok(())
}

/// Expand one CLI input into `(image, output directory)` jobs.
fn expand_input(
    input: &Path,
    out_abs: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, Box<dyn std::error::Error>> {
    if input.is_dir() {
        let root = fs::canonicalize(input)?;
        let group = root
            .file_name()
            .ok_or_else(|| format!("cannot derive a folder name from {}", input.display()))?
            .to_string_lossy()
            .into_owned();
        let group_out = out_abs.join(&group);
        if group_out == root {
            return Err(format!(
                "output for {} would be the input folder itself; choose a different --out",
                input.display()
            )
            .into());
        }

        let mut files = Vec::new();
        collect_pngs(&root, &mut files)?;
        // Do not pick up PNGs from a previous run if --out is inside the input.
        files.retain(|path| !path.starts_with(&group_out));
        files.sort();
        if files.is_empty() {
            return Err(format!("no PNG files found under {}", input.display()).into());
        }

        Ok(files
            .into_iter()
            .map(|image| {
                let rel = image.strip_prefix(&root).unwrap_or(&image);
                let rel_stem = rel.with_extension("");
                (image, group_out.join(rel_stem))
            })
            .collect())
    } else if input.is_file() {
        let stem = input
            .file_stem()
            .ok_or_else(|| format!("invalid input path: {}", input.display()))?;
        Ok(vec![(input.to_path_buf(), out_abs.join(stem))])
    } else {
        Err(format!("no such file or directory: {}", input.display()).into())
    }
}

fn run_image(
    input: &Path,
    out_dir: &Path,
    configs: &[ModeConfig],
    repeat: u32,
) -> Result<Vec<ModeResult>, Box<dyn std::error::Error>> {
    println!("=== {} -> {} ===", input.display(), out_dir.display());
    let (width, height, color_type, raw) = decode(input)?;
    println!(
        "{}: {}x{}, {:?}, {} raw bytes",
        input.display(),
        width,
        height,
        color_type,
        raw.len()
    );

    // More than one run (a mode sweep or a threshold sweep) needs one
    // subfolder per run so the outputs do not overwrite each other.
    let sweep = configs.len() > 1;

    let mut results = Vec::with_capacity(configs.len());
    for config in configs {
        let mode_dir = if sweep {
            out_dir.join(&config.name)
        } else {
            out_dir.to_path_buf()
        };
        println!("=== {} -> {} ===", config.name, mode_dir.display());
        let result = run_mode(&mode_dir, config, width, height, color_type, &raw, repeat)?;
        println!(
            "  encoded: {} bytes ({:.2} bits/pixel)",
            result.encoded_bytes, result.bits_per_pixel
        );
        results.push(result);
    }

    write_times_json(
        &out_dir.join("times.json"),
        input,
        width,
        height,
        repeat,
        &results,
    )?;

    if sweep {
        print_table(&results);
    }
    println!("Results written under {}", out_dir.display());
    Ok(results)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{}", usage());
        return Ok(());
    }
    let args = parse_args(&args)?;

    fs::create_dir_all(&args.out_dir)?;
    let out_abs = fs::canonicalize(&args.out_dir)?;

    let thresholds = match args.selection {
        Selection::Sweep => args
            .thresholds
            .unwrap_or_else(|| SWEEP_GREEDY_THRESHOLDS.to_vec()),
        _ => args
            .thresholds
            .unwrap_or_else(|| vec![DEFAULT_GREEDY_THRESHOLD]),
    };
    let configs = mode_configs(args.selection, &thresholds);
    let sweep = configs.len() > 1;

    let mut jobs = Vec::new();
    let mut failures = 0usize;
    let mut per_image: Vec<(String, Vec<ModeResult>)> = Vec::new();

    for input in &args.inputs {
        match expand_input(input, &out_abs) {
            Ok(mut expanded) => jobs.append(&mut expanded),
            Err(err) => {
                eprintln!("error: {err}");
                failures += 1;
            }
        }
    }

    for (input, out_dir) in &jobs {
        match run_image(input, out_dir, &configs, args.repeat) {
            Ok(results) => per_image.push((input.display().to_string(), results)),
            Err(err) => {
                eprintln!("error: {}: {err}", input.display());
                failures += 1;
            }
        }
    }

    if sweep && !per_image.is_empty() {
        print_cumulative_table(&per_image);
        print_greedy_summary(&per_image);
    }

    if failures != 0 {
        return Err(format!("{failures} input(s) failed").into());
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
