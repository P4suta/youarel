//! One blackletter L supplies desktop bitmaps and the layered macOS icon.

use anyhow::{Context, Result, ensure};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

/// The directory of the launcher's icons.
const DIRECTORY: &str = "app/icons";
/// The PNG icons and their sizes in pixels.
const PNGS: [(&str, u32); 4] = [
    ("32x32.png", 32),
    ("128x128.png", 128),
    ("128x128@2x.png", 256),
    ("icon.png", 1024),
];
/// The Windows icon and the sizes it holds.
const ICO: &str = "icon.ico";
const ICO_SIZES: [u32; 6] = [32, 16, 24, 48, 64, 256];
const ICNS: &str = "icon.icns";
const ICNS_HEADER: usize = 8;
const ICNS_FRAMES: [([u8; 4], u32); 11] = [
    (*b"icp4", 16),
    (*b"icp5", 32),
    (*b"icp6", 64),
    (*b"ic07", 128),
    (*b"ic08", 256),
    (*b"ic09", 512),
    (*b"ic10", 1024),
    (*b"ic11", 32),
    (*b"ic12", 64),
    (*b"ic13", 256),
    (*b"ic14", 512),
];
const COMPOSER: &str = "L.icon/icon.json";
const LAYER: &str = "L.icon/Assets/L.svg";
const SVG_SIDE: u32 = 1024;
const CURVE_STEPS: u32 = 24;
/// The samples along each axis of a pixel, whose share inside the drawing sets the pixel's coverage.
const SAMPLES: u32 = 4;
const SQUARE: [u8; 3] = [u8::MAX; 3];
const MARK: [u8; 3] = [0; 3];
/// Legacy bitmaps include their own tile; Icon Composer supplies the native mask.
const MARGIN: f64 = 0.06;
const RADIUS: f64 = 0.22;
/// The PNG signature, and the color type and depth of eight-bit RGBA.
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
const RGBA: u8 = 6;
const DEPTH: u8 = 8;
/// The bytes of one RGBA pixel, and the scanline filter that leaves a line as it is.
const PIXEL: usize = 4;
const UNFILTERED: u8 = 0;
/// The ICO header's type of an icon, the bytes of its header and of each entry, and the side that its byte of zero stands for.
const ICON_TYPE: u16 = 1;
const ICO_HEADER: u32 = 6;
const ICO_ENTRY: u32 = 16;
const ICO_ZERO_SIDE: u32 = 256;
/// The planes and bits per pixel an ICO entry declares.
const PLANES: u16 = 1;
const BITS_PER_PIXEL: u16 = 32;

/// The icon's side in its own units.
const SIDE: f64 = 1.0;
/// Where a sample stands within its cell: the middle.
const SAMPLE_MIDDLE: f64 = 0.5;
/// The color channels of a pixel before its alpha.
const CHANNELS: usize = 3;

type Point = (f64, f64);

#[derive(Clone, Copy)]
enum Edge {
    Line(Point),
    Curve(Point, Point, Point),
}

struct Contour {
    start: Point,
    edges: &'static [Edge],
}

const LETTER: [Contour; 2] = [
    Contour {
        start: (0.265, 0.290),
        edges: &[
            Edge::Curve((0.285, 0.205), (0.330, 0.180), (0.380, 0.195)),
            Edge::Curve((0.445, 0.220), (0.490, 0.245), (0.570, 0.195)),
            Edge::Curve((0.515, 0.260), (0.490, 0.305), (0.480, 0.405)),
            Edge::Line((0.460, 0.575)),
            Edge::Curve((0.450, 0.650), (0.430, 0.690), (0.395, 0.725)),
            Edge::Curve((0.490, 0.660), (0.545, 0.660), (0.625, 0.705)),
            Edge::Curve((0.680, 0.740), (0.700, 0.770), (0.755, 0.725)),
            Edge::Line((0.685, 0.825)),
            Edge::Curve((0.610, 0.780), (0.540, 0.730), (0.460, 0.755)),
            Edge::Curve((0.405, 0.770), (0.345, 0.800), (0.290, 0.835)),
            Edge::Curve((0.375, 0.755), (0.410, 0.645), (0.370, 0.595)),
            Edge::Curve((0.350, 0.570), (0.325, 0.565), (0.305, 0.570)),
            Edge::Curve((0.330, 0.525), (0.365, 0.510), (0.400, 0.530)),
            Edge::Curve((0.400, 0.435), (0.430, 0.335), (0.480, 0.280)),
            Edge::Curve((0.400, 0.300), (0.340, 0.230), (0.300, 0.260)),
            Edge::Line((0.265, 0.290)),
        ],
    },
    Contour {
        start: (0.670, 0.180),
        edges: &[
            Edge::Curve((0.590, 0.215), (0.565, 0.265), (0.555, 0.375)),
            Edge::Line((0.540, 0.530)),
            Edge::Curve((0.530, 0.595), (0.505, 0.645), (0.470, 0.675)),
            Edge::Curve((0.555, 0.630), (0.585, 0.555), (0.595, 0.440)),
            Edge::Line((0.610, 0.295)),
            Edge::Curve((0.615, 0.240), (0.635, 0.205), (0.670, 0.180)),
        ],
    },
];

fn between(from: Point, to: Point, share: f64) -> Point {
    (
        (to.0 - from.0).mul_add(share, from.0),
        (to.1 - from.1).mul_add(share, from.1),
    )
}

fn outlines() -> Vec<Vec<Point>> {
    LETTER
        .iter()
        .map(|contour| {
            let mut points = vec![contour.start];
            let mut from = contour.start;
            for edge in contour.edges {
                match *edge {
                    Edge::Line(to) => points.push(to),
                    Edge::Curve(first, second, to) => {
                        for step in 1..=CURVE_STEPS {
                            let share = f64::from(step) / f64::from(CURVE_STEPS);
                            let left = between(from, first, share);
                            let middle = between(first, second, share);
                            let right = between(second, to, share);
                            points.push(between(
                                between(left, middle, share),
                                between(middle, right, share),
                                share,
                            ));
                        }
                    }
                }
                from = match *edge {
                    Edge::Line(to) | Edge::Curve(_, _, to) => to,
                };
            }
            points
        })
        .collect()
}

fn crossings(outlines: &[Vec<Point>], y: f64) -> Vec<Vec<f64>> {
    outlines
        .iter()
        .map(|points| {
            let mut intersections = points
                .windows(2)
                .filter_map(|edge| {
                    let [from, to] = edge else { return None };
                    ((from.1 > y) != (to.1 > y))
                        .then(|| (y - from.1).mul_add((to.0 - from.0) / (to.1 - from.1), from.0))
                })
                .collect::<Vec<_>>();
            intersections.sort_by(f64::total_cmp);
            intersections
        })
        .collect()
}

/// Whether `point`, in units of the side, lies inside the rounded square.
fn in_square(point: (f64, f64)) -> bool {
    let low = MARGIN + RADIUS;
    let high = SIDE - MARGIN - RADIUS;
    let nearest = (point.0.clamp(low, high), point.1.clamp(low, high));
    (point.0 - nearest.0).hypot(point.1 - nearest.1) <= RADIUS
}

/// The drawing at `side` pixels, as rows of RGBA pixels.
fn draw(side: u32) -> Result<Vec<u8>> {
    let count = usize::try_from(side)?;
    let mut pixels = Vec::with_capacity(count.saturating_mul(count).saturating_mul(PIXEL));
    let samples_per_pixel = SAMPLES.saturating_mul(SAMPLES);
    let fine = f64::from(side.saturating_mul(SAMPLES));
    let outlines = outlines();
    let at = |pixel: u32, sample: u32| {
        (f64::from(pixel.saturating_mul(SAMPLES).saturating_add(sample)) + SAMPLE_MIDDLE) / fine
    };
    for row in 0..side {
        let rows = (0..SAMPLES)
            .map(|sample| crossings(&outlines, at(row, sample)))
            .collect::<Vec<_>>();
        for column in 0..side {
            let (mut square, mut mark) = (0_u32, 0_u32);
            for (sample_row, bands) in (0..SAMPLES).zip(&rows) {
                for sample_column in 0..SAMPLES {
                    let point = (at(column, sample_column), at(row, sample_row));
                    if in_square(point) {
                        square = square.saturating_add(1);
                        if bands.iter().any(|edges| {
                            edges
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .any(|[left, right]| *left <= point.0 && point.0 < *right)
                        }) {
                            mark = mark.saturating_add(1);
                        }
                    }
                }
            }
            for channel in 0..CHANNELS {
                let blended = SQUARE
                    .get(channel)
                    .zip(MARK.get(channel))
                    .map(|(base, ink)| {
                        let plain = u32::from(*base).saturating_mul(square.saturating_sub(mark));
                        let marked = u32::from(*ink).saturating_mul(mark);
                        plain
                            .saturating_add(marked)
                            .checked_div(square)
                            .unwrap_or_default()
                    })
                    .context("a color has three channels")?;
                pixels.push(u8::try_from(blended)?);
            }
            let alpha = square
                .saturating_mul(u32::from(u8::MAX))
                .checked_div(samples_per_pixel)
                .context("a pixel holds samples")?;
            pixels.push(u8::try_from(alpha)?);
        }
    }
    Ok(pixels)
}

/// One PNG chunk: its length, its kind, its data, and the CRC of the kind and the data.
fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) -> Result<()> {
    out.extend_from_slice(&u32::try_from(data.len())?.to_be_bytes());
    let mut crc = flate2::Crc::new();
    crc.update(&kind);
    crc.update(data);
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
    Ok(())
}

/// The PNG of the drawing at `side` pixels, with every scanline unfiltered.
pub(crate) fn png(side: u32) -> Result<Vec<u8>> {
    let pixels = draw(side)?;
    let line = usize::try_from(side)?.saturating_mul(PIXEL);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    for scanline in pixels.chunks(line) {
        encoder.write_all(&[UNFILTERED])?;
        encoder.write_all(scanline)?;
    }
    let compressed = encoder.finish()?;
    let mut header = Vec::new();
    header.extend_from_slice(&side.to_be_bytes());
    header.extend_from_slice(&side.to_be_bytes());
    header.extend_from_slice(&[DEPTH, RGBA, 0, 0, 0]);
    let mut out = SIGNATURE.to_vec();
    chunk(&mut out, *b"IHDR", &header)?;
    chunk(&mut out, *b"IDAT", &compressed)?;
    chunk(&mut out, *b"IEND", &[])?;
    Ok(out)
}

fn bitmap(images: &BTreeMap<u32, Vec<u8>>, side: u32) -> Result<&[u8]> {
    images
        .get(&side)
        .map(Vec::as_slice)
        .with_context(|| format!("the {side}-pixel icon is missing"))
}

fn bitmaps() -> Result<BTreeMap<u32, Vec<u8>>> {
    let sizes = PNGS
        .iter()
        .map(|(_, side)| *side)
        .chain(ICO_SIZES)
        .chain(ICNS_FRAMES.map(|(_, side)| side))
        .collect::<BTreeSet<_>>();
    sizes
        .into_iter()
        .map(|side| Ok((side, png(side)?)))
        .collect()
}

/// The Windows icon: a header, an entry per size, and each size's PNG.
fn ico(images: &BTreeMap<u32, Vec<u8>>) -> Result<Vec<u8>> {
    let frames = ICO_SIZES
        .iter()
        .map(|side| bitmap(images, *side))
        .collect::<Result<Vec<_>>>()?;
    let count = u16::try_from(frames.len())?;
    let mut out = Vec::new();
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&ICON_TYPE.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = ICO_HEADER.saturating_add(ICO_ENTRY.saturating_mul(u32::from(count)));
    for (side, image) in ICO_SIZES.iter().zip(&frames) {
        let byte = if *side == ICO_ZERO_SIDE {
            0
        } else {
            u8::try_from(*side)?
        };
        let size = u32::try_from(image.len())?;
        out.extend_from_slice(&[byte, byte, 0, 0]);
        out.extend_from_slice(&PLANES.to_le_bytes());
        out.extend_from_slice(&BITS_PER_PIXEL.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset = offset.saturating_add(size);
    }
    for image in frames {
        out.extend_from_slice(image);
    }
    Ok(out)
}

fn icns(images: &BTreeMap<u32, Vec<u8>>) -> Result<Vec<u8>> {
    let mut chunks = Vec::new();
    for (kind, side) in ICNS_FRAMES {
        let image = bitmap(images, side)?;
        let length = u32::try_from(image.len().saturating_add(ICNS_HEADER))?;
        chunks.extend_from_slice(&kind);
        chunks.extend_from_slice(&length.to_be_bytes());
        chunks.extend_from_slice(image);
    }
    let length = u32::try_from(chunks.len().saturating_add(ICNS_HEADER))?;
    let mut out = b"icns".to_vec();
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&chunks);
    Ok(out)
}

fn svg() -> Result<Vec<u8>> {
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{SVG_SIDE}\" height=\"{SVG_SIDE}\" viewBox=\"0 0 1 1\">\n<path fill=\"#000\" d=\""
    );
    for contour in &LETTER {
        write!(out, "M{} {}", contour.start.0, contour.start.1)?;
        for edge in contour.edges {
            match *edge {
                Edge::Line(to) => write!(out, "L{} {}", to.0, to.1)?,
                Edge::Curve(first, second, to) => write!(
                    out,
                    "C{} {} {} {} {} {}",
                    first.0, first.1, second.0, second.1, to.0, to.1
                )?,
            }
        }
        out.push('Z');
    }
    out.push_str("\"/>\n</svg>\n");
    Ok(out.into_bytes())
}

fn composer() -> Result<Vec<u8>> {
    let mut out = serde_json::to_vec_pretty(&serde_json::json!({
        "fill": { "solid": "extended-srgb:1,1,1,1" },
        "groups": [{
            "layers": [{
                "fill-specializations": [
                    { "value": "automatic" },
                    { "appearance": "dark", "value": "system-light" },
                    { "appearance": "tinted", "value": "system-light" }
                ],
                "glass": false,
                "image-name": "L.svg",
                "name": "L",
                "position": { "scale": 1, "translation-in-points": [0, 0] }
            }],
            "shadow": { "kind": "neutral", "opacity": 0 },
            "translucency": { "enabled": false, "value": 0 }
        }],
        "supported-platforms": { "squares": ["macOS"] }
    }))?;
    out.push(b'\n');
    Ok(out)
}

/// Every icon file, by its name, with its bytes.
fn icons() -> Result<Vec<(&'static str, Vec<u8>)>> {
    let images = bitmaps()?;
    let mut files = PNGS
        .iter()
        .map(|(name, side)| Ok((*name, bitmap(&images, *side)?.to_vec())))
        .collect::<Result<Vec<_>>>()?;
    files.push((ICO, ico(&images)?));
    files.push((ICNS, icns(&images)?));
    files.push((COMPOSER, composer()?));
    files.push((LAYER, svg()?));
    Ok(files)
}

/// Writes every icon.
///
/// # Errors
///
/// Returns an error when an icon cannot be drawn or written.
pub fn generate(root: &Path) -> Result<()> {
    let directory = root.join(DIRECTORY);
    crate::fs::create_dir_all(&directory)?;
    for (name, bytes) in icons()? {
        crate::fs::write(&directory.join(name), &bytes)?;
    }
    Ok(())
}

/// Refuses an icon that differs from its drawing.
///
/// # Errors
///
/// Returns an error naming the first icon that drifted.
pub fn check(root: &Path) -> Result<()> {
    let directory = root.join(DIRECTORY);
    for (name, bytes) in icons()? {
        let path = directory.join(name);
        let held = crate::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        ensure!(
            held == bytes,
            "{DIRECTORY}/{name} differs from its drawing; run `just gen`"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{bitmaps, draw, icns, ico, in_square, png};
    const CONFIG: &str = "app/tauri.conf.json";

    #[test]
    fn app_artwork_stays_monochrome() {
        for [red, green, blue, _] in draw(32).unwrap().as_chunks::<{ super::PIXEL }>().0 {
            assert_eq!(red, green, "the icon must have no color cast");
            assert_eq!(green, blue, "the icon must have no color cast");
        }
    }

    #[test]
    fn macos_appearance_artwork_is_bundled_with_desktop_fallbacks() {
        let root = crate::root().unwrap();
        let config: serde_json::Value =
            serde_json::from_str(&crate::fs::read_to_string(&root.join(CONFIG)).unwrap()).unwrap();
        let paths = config["bundle"]["icon"].as_array().unwrap();
        for extension in [".icon", ".icns", ".ico", ".png"] {
            assert!(
                paths
                    .iter()
                    .any(|path| path.as_str().unwrap().ends_with(extension)),
                "the bundle must include {extension} artwork"
            );
        }
        let native: serde_json::Value =
            serde_json::from_slice(&super::composer().unwrap()).unwrap();
        let fills = native["groups"][0]["layers"][0]["fill-specializations"]
            .as_array()
            .unwrap();
        for appearance in ["dark", "tinted"] {
            assert!(
                fills.iter().any(|fill| fill["appearance"] == appearance
                    && fill["value"] == "system-light"),
                "{appearance} needs a visible foreground on the system background"
            );
        }
    }

    #[test]
    fn legacy_icons_keep_a_tile_and_clear_corners() {
        assert!(in_square((0.5, 0.5)));
        assert!(!in_square((0.07, 0.07)), "the corner is round");
        let pixels = draw(32).unwrap();
        assert_eq!(pixels.len(), 32 * 32 * 4);
        assert_eq!(pixels.get(3), Some(&0), "a corner pixel is clear");
        let center = (16 * 32 + 16) * 4;
        assert_eq!(
            pixels.get(center..center + 4),
            Some(&[0xff, 0xff, 0xff, 0xff][..]),
            "the counter between the letter strokes stays clear"
        );
    }

    #[test]
    fn a_png_and_an_ico_carry_their_signatures_and_sizes() {
        let image = png(16).unwrap();
        assert_eq!(image.get(..8), Some(&super::SIGNATURE[..]));
        assert_eq!(image.get(16..24), Some(&[0, 0, 0, 16, 0, 0, 0, 16][..]));
        let images = bitmaps().unwrap();
        let icon = ico(&images).unwrap();
        assert_eq!(icon.get(..6), Some(&[0, 0, 1, 0, 6, 0][..]));
        assert_eq!(icon.get(6..8), Some(&[32, 32][..]));
        let mac = icns(&images).unwrap();
        assert_eq!(mac.get(..4), Some(&b"icns"[..]));
        assert_eq!(
            mac.get(4..8),
            Some(&u32::try_from(mac.len()).unwrap().to_be_bytes()[..])
        );
    }
}
