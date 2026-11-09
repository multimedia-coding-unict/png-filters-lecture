//! Execution harness for experimenting with PNG scanline filter decision modes.
//!
//! For a given input image this encodes it once per requested filter mode with
//! this crate's encoder, decodes each result back and verifies the round-trip,
//! and exports everything into an output folder:
//!
//! ```text
//! <out>/
//!   <mode>.png                 image encoded with that filter mode
//!   decoded_<mode>.png         decoder output of the above, re-encoded losslessly
//!   summary.csv                per-mode encoded size
//!   debug/
//!     filter_map_<mode>.png    2D map: the color of scanline y is the filter chosen for it
//!     filters_<mode>.csv       row,filter_id,filter_name
//! ```
//!
//! Usage:
//!
//! ```text
//! cargo run --release --example filter_harness -- <input.png> [output_dir] [modes...]
//! ```
//!
//! Modes are `none`, `sub`, `up`, `avg`, `paeth`, `adaptive`, `minentropy`, or
//! `all` (default). The input must be an 8-bit PNG (gray, gray+alpha, RGB or
//! RGBA); non-interlaced input works best since the filter maps are one row per
//! scanline.

use std::{
    env, fs,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    process::ExitCode,
};

use png::{BitDepth, ColorType, Decoder, Encoder, Filter};

const ALL_MODES: &[(&str, Filter)] = &[
    ("none", Filter::NoFilter),
    ("sub", Filter::Sub),
    ("up", Filter::Up),
    ("avg", Filter::Avg),
    ("paeth", Filter::Paeth),
    ("adaptive", Filter::Adaptive),
    ("minentropy", Filter::MinEntropy),
];

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
    let mut encoder = Encoder::new(Box::new(BufWriter::new(file)) as Box<dyn std::io::Write>, width, height);
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

fn write_filter_csv(path: &Path, log: &[u8]) -> Result<(), std::io::Error> {
    let mut csv = String::from("row,filter_id,filter_name\n");
    for (row, &id) in log.iter().enumerate() {
        csv.push_str(&format!("{},{},{}\n", row, id, filter_name(id)));
    }
    fs::write(path, csv)
}

fn parse_args(args: &[String]) -> Result<(PathBuf, PathBuf, Vec<(&'static str, Filter)>), String> {
    if args.is_empty() || args.len() > 3 {
        return Err(format!(
            "usage: filter_harness <input.png> [output_dir] [modes...]\n\
             modes: {}all",
            ALL_MODES
                .iter()
                .map(|(name, _)| format!("{name},"))
                .collect::<String>()
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

    let mode_names: Vec<&str> = match args.get(2) {
        Some(spec) if spec != "all" => spec.split(',').map(|m| m.trim()).collect(),
        _ => ALL_MODES.iter().map(|(name, _)| *name).collect(),
    };

    let mut modes = Vec::new();
    for name in mode_names {
        let mode = ALL_MODES
            .iter()
            .find(|(known, _)| *known == name)
            .ok_or_else(|| format!("unknown filter mode: {name}"))?;
        modes.push(*mode);
    }

    Ok((input, out_dir, modes))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let (input, out_dir, modes) = parse_args(&args)?;

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

    let mut summary = String::from("mode,encoded_bytes,bits_per_pixel\n");
    for (name, mode) in modes {
        let encoded_path = out_dir.join(format!("{name}.png"));
        let encoded = encode(
            &encoded_path,
            width,
            height,
            color_type,
            &raw,
            mode,
            true,
        )?;

        // Decode the result again and verify the round-trip against the source.
        let (dwidth, dheight, dcolor, decoded) = decode(&encoded_path)?;
        if (dwidth, dheight, dcolor) != (width, height, color_type) || decoded != raw {
            return Err(format!("{name}: round-trip through {} mismatched the input; the exported filter map would be misleading", encoded_path.display()).into());
        }

        // Export the decoder's output as its own image, re-encoded without any
        // filtering so it is a plain lossless representation of the decoded data.
        let decoded_path = out_dir.join(format!("decoded_{name}.png"));
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
            &out_dir.join("debug").join(format!("filter_map_{name}.png")),
            width,
            height,
            &encoded.filter_log,
        )?;
        write_filter_csv(
            &out_dir.join("debug").join(format!("filters_{name}.csv")),
            &encoded.filter_log,
        )?;

        let bits_per_pixel = encoded.bytes as f64 * 8.0 / (width as f64 * height as f64);
        summary.push_str(&format!("{name},{},{}\n", encoded.bytes, bits_per_pixel));
        println!(
            "  {name:>10}: {:>10} bytes  ({bits_per_pixel:.2} bits/pixel)",
            encoded.bytes
        );
    }

    fs::write(out_dir.join("summary.csv"), summary)?;
    println!("results written to {}", out_dir.display());
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
