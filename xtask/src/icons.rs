//! The window launcher's icons, drawn from code: an arrow out of a rounded square, which `cargo xtask gen` writes and `gen --check` compares.
//!
//! Each icon is a PNG of the drawing at one size, and the Windows icon holds several of them, so no binary file in the repository lacks a source.

use anyhow::{Context, Result, ensure};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::io::Write as _;
use std::path::Path;

/// The directory of the launcher's icons.
const DIRECTORY: &str = "app/icons";
/// The PNG icons and their sizes in pixels.
const PNGS: [(&str, u32); 3] = [("32x32.png", 32), ("128x128.png", 128), ("icon.png", 512)];
/// The Windows icon and the sizes it holds.
const ICO: &str = "icon.ico";
const ICO_SIZES: [u32; 4] = [16, 32, 48, 256];
/// The samples along each axis of a pixel, whose share inside the drawing sets the pixel's coverage.
const SAMPLES: u32 = 4;
/// The colors of the square and of the arrow, as red, green, and blue.
const SQUARE: [u8; 3] = [0x1d, 0x4e, 0xd8];
const ARROW: [u8; 3] = [0xff, 0xff, 0xff];
/// The drawing in units of the icon's side: the square's margin and corner radius, the arrow's ends, and its stroke.
const MARGIN: f64 = 0.06;
const RADIUS: f64 = 0.22;
const TAIL: (f64, f64) = (0.31, 0.69);
const HEAD: (f64, f64) = (0.69, 0.31);
const BARB: f64 = 0.27;
const STROKE: f64 = 0.095;
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

/// The shares of a segment at its start and at its end.
const START: f64 = 0.0;
const END: f64 = 1.0;
/// The icon's side in its own units.
const SIDE: f64 = 1.0;
/// The distance from the arrow's middle line within which a point lies on the arrow.
const HALF_STROKE: f64 = STROKE / 2.0;
/// Where a sample stands within its cell: the middle.
const SAMPLE_MIDDLE: f64 = 0.5;
/// The color channels of a pixel before its alpha.
const CHANNELS: usize = 3;

/// The distance from `point` to the segment from `start` to `end`.
fn segment_distance(point: (f64, f64), start: (f64, f64), end: (f64, f64)) -> f64 {
    let (along_x, along_y) = (end.0 - start.0, end.1 - start.1);
    let (from_x, from_y) = (point.0 - start.0, point.1 - start.1);
    let length = along_x.mul_add(along_x, along_y * along_y);
    let share = if length > START {
        (from_x.mul_add(along_x, from_y * along_y) / length).clamp(START, END)
    } else {
        START
    };
    let (gap_x, gap_y) = (
        share.mul_add(-along_x, from_x),
        share.mul_add(-along_y, from_y),
    );
    gap_x.hypot(gap_y)
}

/// Whether `point`, in units of the side, lies inside the rounded square.
fn in_square(point: (f64, f64)) -> bool {
    let low = MARGIN + RADIUS;
    let high = SIDE - MARGIN - RADIUS;
    let nearest = (point.0.clamp(low, high), point.1.clamp(low, high));
    (point.0 - nearest.0).hypot(point.1 - nearest.1) <= RADIUS
}

/// Whether `point` lies on the arrow: its shaft and the two barbs of its head.
fn on_arrow(point: (f64, f64)) -> bool {
    let barbs = [(HEAD.0 - BARB, HEAD.1), (HEAD.0, HEAD.1 + BARB)];
    segment_distance(point, TAIL, HEAD) <= HALF_STROKE
        || barbs
            .iter()
            .any(|barb| segment_distance(point, *barb, HEAD) <= HALF_STROKE)
}

/// The drawing at `side` pixels, as rows of RGBA pixels.
fn draw(side: u32) -> Result<Vec<u8>> {
    let count = usize::try_from(side)?;
    let mut pixels = Vec::with_capacity(count.saturating_mul(count).saturating_mul(PIXEL));
    let samples_per_pixel = SAMPLES.saturating_mul(SAMPLES);
    let fine = f64::from(side.saturating_mul(SAMPLES));
    for row in 0..side {
        for column in 0..side {
            let (mut square, mut arrow) = (0_u32, 0_u32);
            for sample_row in 0..SAMPLES {
                for sample_column in 0..SAMPLES {
                    let at = |pixel: u32, sample: u32| {
                        (f64::from(pixel.saturating_mul(SAMPLES).saturating_add(sample))
                            + SAMPLE_MIDDLE)
                            / fine
                    };
                    let point = (at(column, sample_column), at(row, sample_row));
                    if in_square(point) {
                        square = square.saturating_add(1);
                        if on_arrow(point) {
                            arrow = arrow.saturating_add(1);
                        }
                    }
                }
            }
            for channel in 0..CHANNELS {
                let blended = SQUARE
                    .get(channel)
                    .zip(ARROW.get(channel))
                    .map(|(base, mark)| {
                        let plain = u32::from(*base).saturating_mul(square.saturating_sub(arrow));
                        let marked = u32::from(*mark).saturating_mul(arrow);
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

/// The Windows icon: a header, an entry per size, and each size's PNG.
fn ico() -> Result<Vec<u8>> {
    let images = ICO_SIZES
        .iter()
        .map(|side| png(*side))
        .collect::<Result<Vec<_>>>()?;
    let count = u16::try_from(images.len())?;
    let mut out = Vec::new();
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&ICON_TYPE.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = ICO_HEADER.saturating_add(ICO_ENTRY.saturating_mul(u32::from(count)));
    for (side, image) in ICO_SIZES.iter().zip(&images) {
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
    for image in images {
        out.extend_from_slice(&image);
    }
    Ok(out)
}

/// Every icon file, by its name, with its bytes.
fn icons() -> Result<Vec<(&'static str, Vec<u8>)>> {
    let mut files = PNGS
        .iter()
        .map(|(name, side)| Ok((*name, png(*side)?)))
        .collect::<Result<Vec<_>>>()?;
    files.push((ICO, ico()?));
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
    use super::{draw, ico, in_square, on_arrow, png};

    #[test]
    fn the_drawing_puts_the_arrow_inside_the_square_and_nothing_outside() {
        assert!(in_square((0.5, 0.5)));
        assert!(!in_square((0.07, 0.07)), "the corner is round");
        assert!(on_arrow((0.5, 0.5)));
        assert!(!on_arrow((0.3, 0.3)));
        let pixels = draw(32).unwrap();
        assert_eq!(pixels.len(), 32 * 32 * 4);
        assert_eq!(pixels.get(3), Some(&0), "a corner pixel is clear");
        let center = (16 * 32 + 16) * 4;
        assert_eq!(
            pixels.get(center..center + 4),
            Some(&[0xff, 0xff, 0xff, 0xff][..]),
            "the arrow crosses the middle"
        );
    }

    #[test]
    fn a_png_and_an_ico_carry_their_signatures_and_sizes() {
        let image = png(16).unwrap();
        assert_eq!(image.get(..8), Some(&super::SIGNATURE[..]));
        assert_eq!(image.get(16..24), Some(&[0, 0, 0, 16, 0, 0, 0, 16][..]));
        let icon = ico().unwrap();
        assert_eq!(icon.get(..6), Some(&[0, 0, 1, 0, 4, 0][..]));
        assert_eq!(icon.get(6..8), Some(&[16, 16][..]));
    }
}
