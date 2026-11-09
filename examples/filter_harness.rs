//! Execution harness for experimenting with PNG scanline filter decision modes.
//!
//! For a given input image this runs the normal encoding process once, with the
//! selected filter mode (default: the encoder's own default, `adaptive`), decodes
//! the result back and verifies the round-trip, and exports everything into an
//! output folder:
//!
//! ```text
//! <out>/
//!   encoded.png           image encoded with the selected filter mode
//!   decoded.png           decoder output of the above, re-encoded losslessly
//!   summary.json          encoded size and bits/pixel
//!   debug/
//!     filter_map.png      2D map: the color of scanline y is the filter chosen for it
//!     filters.json        per-scanline filter decisions
//! ```
//!
//! Usage:
//!
//! ```text
//! cargo run --release --example filter_harness -- <input.png> [output_dir] [mode]
//! ```
//!
//! Modes are `none`, `sub`, `up`, `avg`, `paeth`, `adaptive` (default) or
//! `minentropy`. The input must be an 8-bit PNG (gray, gray+alpha, RGB or RGBA);
//! non-interlaced input works best since the filter map is one row per scanline.

use std::{
    env, fs,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    process::ExitCode,
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

fn decode(path: &Path) -> Result<(u32, u32, ColorType, Vec<u8>), Box<dyn std::error::Error>> {    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
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
        ColorType::Grayscale
        | ColorType::GrayscaleAlpha
        | ColorType::Rgb
        | ColorType::Rgba => {}
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
    let file = fs::File::create(path)?;
    let mut encoder =
        Encoder::new(Box::new(BufWriter::new(file)) as Box<dyn std::io::Write>, width, height);
    encoder.set_color(color_type);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_filter(mode);
    encoder.set_filter_log(record);

    let mut writer = encoder.write_header()?;
    writer.write_image_data(data)?;
    let filter_log = writer.filter_log().unwrap_or_default().to_vec();
    writer.finish()?;

    let bytes = fs::metadata(path)?.len();
    Ok(Encoded { bytes, filter_log })
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
    encode(path, width, height, ColorType::Rgb, &map, Filter::NoFilter, false)?;
    Ok(())
}

fn write_filter_json(path: &Path, width: u32, height: u32, log: &[u8]) -> Result<(), std::io::Error> {
    let mut json = format!(
        "{{\n  \"width\": {width},\n  \"height\": {height},\n  \"filters\": [\n"
    );
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

fn parse_mode(name: &str) -> Result<Filter, String> {
    MODES
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, filter)| *filter)
        .ok_or_else(|| {
            format!(
                "unknown filter mode: {name} (expected one of {})",
                MODES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
            )
        })
}

fn parse_args(args: &[String]) -> Result<(PathBuf, PathBuf, Filter), String> {
    if args.is_empty() || args.len() > 3 {
        return Err(format!(
            "usage: filter_harness <input.png> [output_dir] [mode]\n\
             modes: {} (default: {DEFAULT_MODE})",
            MODES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
        ));
    }

    let input = PathBuf::from(&args[0]);
    let out_dir = match args.get(1) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let stem = input
                .file_stem()
                .ok_or_else(|| format!("invalid input path: {}", input.display()))?;
            PathBuf::from("results").join(stem)
        }
    };
    let mode = match args.get(2) {
        Some(name) => parse_mode(name)?,
        None => parse_mode(DEFAULT_MODE).unwrap(),
    };

    Ok((input, out_dir, mode))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let (input, out_dir, mode) = parse_args(&args)?;

    let (width, height, color_type, raw) = decode(&input)?;
    println!(
        "{}: {}x{}, {:?}, {} raw bytes",
        input.display(),
        width,
        height,
        color_type,
        raw.len()
    );

    fs::create_dir_all(out_dir.join("debug"))?;

    // Encode with the selected filter mode, recording the per-scanline decision.
    let encoded_path = out_dir.join("encoded.png");
    let encoded = encode(&encoded_path, width, height, color_type, &raw, mode, true)?;
    let bytes = encoded.bytes;

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
    let decoded_path = out_dir.join("decoded.png");
    encode(
        &decoded_path,
        width,
        height,
        color_type,
        &decoded,
        Filter::NoFilter,
        false,
    )?;

    write_filter_map(
        &out_dir.join("debug").join("filter_map.png"),
        width,
        height,
        &encoded.filter_log,
    )?;
    write_filter_json(
        &out_dir.join("debug").join("filters.json"),
        width,
        height,
        &encoded.filter_log,
    )?;

    let bits_per_pixel = bytes as f64 * 8.0 / (width as f64 * height as f64);
    fs::write(
        out_dir.join("summary.json"),
        format!(
            "{{\n  \"mode\": \"{}\",\n  \"encoded_bytes\": {bytes},\n  \"bits_per_pixel\": {bits_per_pixel}\n}}\n",
            format!("{mode:?}").to_lowercase(),
        ),
    )?;
    println!(
        "  encoded: {bytes} bytes ({bits_per_pixel:.2} bits/pixel), results in {}",
        out_dir.display()
    );
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
