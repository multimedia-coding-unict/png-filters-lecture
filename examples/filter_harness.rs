//! Execution harness for experimenting with PNG scanline filter decision modes.
//!
//! For a given input image this runs the normal encoding process once per
//! selected filter mode (default: the encoder's own default, `adaptive`),
//! decodes the result back and verifies the round-trip, and exports everything
//! into an output folder:
//!
//! ```text
//! <out>/
//!   times.json            encoding times, one entry per mode (see below)
//!   <mode>/               only when sweeping; otherwise files land in <out>/
//!     encoded.png         image encoded with the selected filter mode
//!     decoded.png         decoder output of the above, re-encoded losslessly
//!     summary.json        encoded size and bits/pixel
//!     debug/
//!       filter_map.png    2D map: the color of scanline y is the filter chosen for it
//!       filters.json      per-scanline filter decisions
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
//! cargo run --release --example filter_harness -- [--repeat N] <input.png> [output_dir] [mode]
//! ```
//!
//! Modes are `none`, `sub`, `up`, `avg`, `paeth`, `adaptive` (default),
//! `minentropy`, or `sweep` to run every mode and compare them. The input must
//! be an 8-bit PNG (gray, gray+alpha, RGB or RGBA); non-interlaced input works
//! best since the filter map is one row per scanline.

use std::{
    env, fs,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};

use png::{BitDepth, ColorType, Decoder, Encoder, Filter};

/// All selectable modes, in filter-type-byte order.
const MODES: &[(&str, Filter)] = &[
    ("none", Filter::NoFilter),
    ("sub", Filter::Sub),
    ("up", Filter::Up),
    ("avg", Filter::Avg),
    ("paeth", Filter::Paeth),
    ("adaptive", Filter::Adaptive),
    ("minentropy", Filter::MinEntropy),
];

const DEFAULT_MODE: &str = "adaptive";

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

fn encode(
    path: &Path,
    width: u32,
    height: u32,
    color_type: ColorType,
    data: &[u8],
    mode: Filter,
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
    input: PathBuf,
    out_dir: PathBuf,
    selection: Selection,
    repeat: u32,
}

fn usage() -> String {
    format!(
        "usage: filter_harness [--repeat N] <input.png> [output_dir] [mode]\n\
         modes: {} (default: {DEFAULT_MODE}), sweep\n\
         --repeat N: encode each mode N times and report the minimum (default: 1)",
        MODES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
    )
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut repeat = 1u32;
    let mut positional = Vec::new();

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
            arg if arg.starts_with('-') => {
                return Err(format!("unknown option: {arg}\n{}", usage()));
            }
            arg => {
                positional.push(arg.to_string());
                i += 1;
            }
        }
    }

    if positional.is_empty() || positional.len() > 3 {
        return Err(usage());
    }

    let input = PathBuf::from(&positional[0]);
    let out_dir = match positional.get(1) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let stem = input
                .file_stem()
                .ok_or_else(|| format!("invalid input path: {}", input.display()))?;
            PathBuf::from("results").join(stem)
        }
    };
    let selection = match positional.get(2) {
        Some(name) => parse_selection(name)?,
        None => parse_selection(DEFAULT_MODE).unwrap(),
    };

    Ok(Args {
        input,
        out_dir,
        selection,
        repeat,
    })
}

struct RunTimes {
    encode_total: Duration,
    filter_prediction: Duration,
}

struct ModeResult {
    mode: &'static str,
    encoded_bytes: u64,
    bits_per_pixel: f64,
    runs: Vec<RunTimes>,
}

/// Encode, verify and export one mode. Returns the measurements for `times.json`.
#[allow(clippy::too_many_arguments)]
fn run_mode(
    mode_dir: &Path,
    mode_name: &'static str,
    mode: Filter,
    width: u32,
    height: u32,
    color_type: ColorType,
    raw: &[u8],
    repeat: u32,
) -> Result<ModeResult, Box<dyn std::error::Error>> {
    fs::create_dir_all(mode_dir.join("debug"))?;

    // Encode with the selected filter mode, recording the per-scanline decision.
    let encoded_path = mode_dir.join("encoded.png");
    let first = encode(&encoded_path, width, height, color_type, raw, mode, true)?;
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
            "{{\n  \"mode\": \"{mode_name}\",\n  \"encoded_bytes\": {bytes},\n  \"bits_per_pixel\": {bits_per_pixel:.4}\n}}\n"
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
        let run = encode(&encoded_path, width, height, color_type, raw, mode, false)?;
        runs.push(RunTimes {
            encode_total: run.encode_total,
            filter_prediction: run.filter_prediction,
        });
    }

    Ok(ModeResult {
        mode: mode_name,
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

fn print_table(results: &[ModeResult]) {
    println!();
    println!(
        "{:<12} {:>12} {:>10} {:>12} {:>12} {:>9}",
        "mode", "bytes", "bits/px", "encode ms", "filter ms", "filter %"
    );
    for result in results {
        let totals: Vec<Duration> = result.runs.iter().map(|run| run.encode_total).collect();
        let filters: Vec<Duration> = result
            .runs
            .iter()
            .map(|run| run.filter_prediction)
            .collect();
        let (total_min, _, _) = min_median_mean(&totals);
        let (filter_min, _, _) = min_median_mean(&filters);
        let percent = if total_min == 0 {
            0.0
        } else {
            filter_min as f64 / total_min as f64 * 100.0
        };
        println!(
            "{:<12} {:>12} {:>10.2} {:>12.3} {:>12.3} {:>8.1}%",
            result.mode,
            result.encoded_bytes,
            result.bits_per_pixel,
            total_min as f64 / 1e6,
            filter_min as f64 / 1e6,
            percent
        );
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{}", usage());
        return Ok(());
    }
    let args = parse_args(&args)?;

    let (width, height, color_type, raw) = decode(&args.input)?;
    println!(
        "{}: {}x{}, {:?}, {} raw bytes",
        args.input.display(),
        width,
        height,
        color_type,
        raw.len()
    );

    let (selections, sweep): (Vec<(&'static str, Filter)>, bool) = match args.selection {
        Selection::Single(name, filter) => (vec![(name, filter)], false),
        Selection::Sweep => (MODES.to_vec(), true),
    };

    let mut results = Vec::with_capacity(selections.len());
    for (name, filter) in &selections {
        let mode_dir = if sweep {
            args.out_dir.join(name)
        } else {
            args.out_dir.clone()
        };
        println!("=== {name} -> {} ===", mode_dir.display());
        let result = run_mode(
            &mode_dir,
            name,
            *filter,
            width,
            height,
            color_type,
            &raw,
            args.repeat,
        )?;
        println!(
            "  encoded: {} bytes ({:.2} bits/pixel)",
            result.encoded_bytes, result.bits_per_pixel
        );
        results.push(result);
    }

    write_times_json(
        &args.out_dir.join("times.json"),
        &args.input,
        width,
        height,
        args.repeat,
        &results,
    )?;

    if sweep {
        print_table(&results);
    }
    println!("Results written under {}", args.out_dir.display());
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
