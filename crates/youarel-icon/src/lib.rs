//! The icons youarel shows: an image a page offers, in any common format, decoded within bounds and drawn again as one small PNG.
//!
//! The image comes from the network, so the decoder runs under limits on the bytes it reads, the sides it accepts, and the memory it allocates.
//! Every icon leaves as a square PNG of [`SIDE`] pixels, so a device shows the same pixels for one image and keeps no more than [`LONGEST_ICON`] bytes for it.

use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::{self, FilterType};
use image::{DynamicImage, ExtendedColorType, ImageEncoder as _, ImageReader, Limits, RgbaImage};
use std::io::Cursor;

/// The side of every icon, in pixels.
pub const SIDE: u32 = 64;
/// The most bytes a drawn icon holds, which a square of [`SIDE`] pixels fits many times over.
pub const LONGEST_ICON: usize = 32 * 1024;
/// The most bytes of an image the adapter decodes.
pub const LONGEST_SOURCE: usize = 512 * 1024;
/// The widest and the tallest image the adapter decodes.
const LARGEST_SIDE: u32 = 4096;
/// The most memory the decoder allocates.
const MOST_DECODED: u64 = 64 * 1024 * 1024;
/// The margins that share the room around a centered image, one on each side.
const MARGINS: u32 = 2;

/// Why an image yields no icon.
#[derive(Debug)]
pub enum IconError {
    /// The image holds more than [`LONGEST_SOURCE`] bytes.
    TooLong,
    /// The image is in no format the adapter reads, is malformed, or passes the decoder's limits.
    Unreadable(image::ImageError),
    /// The drawn icon would pass [`LONGEST_ICON`], which no square of [`SIDE`] pixels reaches.
    TooLarge,
}

impl core::fmt::Display for IconError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooLong => write!(
                formatter,
                "the image holds more than {LONGEST_SOURCE} bytes"
            ),
            Self::Unreadable(error) => write!(formatter, "the image cannot be read: {error}"),
            Self::TooLarge => formatter.write_str("the icon would be too large"),
        }
    }
}

impl std::error::Error for IconError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable(error) => Some(error),
            Self::TooLong | Self::TooLarge => None,
        }
    }
}

/// The image `bytes` hold, decoded under the adapter's limits.
fn decode(bytes: &[u8]) -> Result<DynamicImage, IconError> {
    if bytes.len() > LONGEST_SOURCE {
        return Err(IconError::TooLong);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| IconError::Unreadable(image::ImageError::IoError(error)))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(LARGEST_SIDE);
    limits.max_image_height = Some(LARGEST_SIDE);
    limits.max_alloc = Some(MOST_DECODED);
    reader.limits(limits);
    reader.decode().map_err(IconError::Unreadable)
}

/// The image fitted into a square of [`SIDE`] pixels and centered on a clear one.
///
/// A small image grows by a whole factor with its pixels kept sharp, and a large one shrinks smoothly, each keeping its proportions.
fn fit(image: &DynamicImage) -> RgbaImage {
    let pixels = image.to_rgba8();
    let (width, height) = pixels.dimensions();
    let longer = width.max(height).max(1);
    let fitted = if longer <= SIDE {
        let factor = SIDE.checked_div(longer).unwrap_or(1);
        imageops::resize(
            &pixels,
            width.saturating_mul(factor),
            height.saturating_mul(factor),
            FilterType::Nearest,
        )
    } else {
        DynamicImage::ImageRgba8(pixels)
            .resize(SIDE, SIDE, FilterType::Lanczos3)
            .to_rgba8()
    };
    let mut square = RgbaImage::new(SIDE, SIDE);
    let (fitted_width, fitted_height) = fitted.dimensions();
    let left = SIDE
        .saturating_sub(fitted_width)
        .checked_div(MARGINS)
        .unwrap_or(0);
    let top = SIDE
        .saturating_sub(fitted_height)
        .checked_div(MARGINS)
        .unwrap_or(0);
    imageops::overlay(&mut square, &fitted, i64::from(left), i64::from(top));
    square
}

/// The icon an image becomes: a PNG of [`SIDE`] by [`SIDE`] pixels.
///
/// Drawing an icon again yields the same bytes, so an icon's digest names its pixels.
///
/// # Errors
///
/// Returns an [`IconError`] when the image is too long, unreadable, or passes the decoder's limits.
pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>, IconError> {
    let square = fit(&decode(bytes)?);
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Best, PngFilter::Adaptive)
        .write_image(square.as_raw(), SIDE, SIDE, ExtendedColorType::Rgba8)
        .map_err(IconError::Unreadable)?;
    within_limit(png)
}

/// An icon that a device may keep: no more than [`LONGEST_ICON`] bytes.
fn within_limit(png: Vec<u8>) -> Result<Vec<u8>, IconError> {
    if png.len() > LONGEST_ICON {
        return Err(IconError::TooLarge);
    }
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::{IconError, LONGEST_ICON, LONGEST_SOURCE, SIDE, normalize, within_limit};
    use image::codecs::bmp::BmpEncoder;
    use image::codecs::gif::GifEncoder;
    use image::codecs::jpeg::JpegEncoder;
    use image::codecs::png::PngEncoder;
    use image::{ExtendedColorType, ImageEncoder as _, ImageReader, Rgba, RgbaImage};
    use std::io::Cursor;

    /// A gradient of `width` by `height` pixels, opaque except its first column.
    fn gradient(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            let shade = |value: u32| u8::try_from(value & 0xff).unwrap();
            let alpha = if x == 0 { 0 } else { 255 };
            Rgba([
                shade(x.wrapping_mul(7)),
                shade(y.wrapping_mul(5)),
                shade(x.wrapping_add(y)),
                alpha,
            ])
        })
    }

    fn png(image: &RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    }

    fn decoded(bytes: &[u8]) -> RgbaImage {
        ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8()
    }

    #[test]
    fn a_small_icon_grows_sharply_and_an_icon_redrawn_stays_the_same() {
        let small = gradient(16, 16);
        let icon = normalize(&png(&small)).unwrap();
        let drawn = decoded(&icon);
        assert_eq!(drawn.dimensions(), (SIDE, SIDE));
        assert_eq!(drawn.get_pixel(5, 9), small.get_pixel(1, 2));
        assert_eq!(drawn.get_pixel(63, 63), small.get_pixel(15, 15));
        assert_eq!(
            normalize(&icon).unwrap(),
            icon,
            "drawing again changes nothing"
        );
    }

    #[test]
    fn a_wide_image_shrinks_into_the_middle_of_a_clear_square() {
        let wide = gradient(200, 100);
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 90)
            .encode(
                &image::DynamicImage::ImageRgba8(wide).to_rgb8(),
                200,
                100,
                ExtendedColorType::Rgb8,
            )
            .unwrap();
        let drawn = decoded(&normalize(&jpeg).unwrap());
        assert_eq!(drawn.dimensions(), (SIDE, SIDE));
        assert_eq!(
            drawn.get_pixel(32, 2).0[3],
            0,
            "above the image stays clear"
        );
        assert_eq!(drawn.get_pixel(32, 32).0[3], 255);
        let odd = decoded(&normalize(&png(&gradient(3, 2))).unwrap());
        assert_eq!(odd.get_pixel(31, 31).0[3], 255);
        assert_eq!(
            odd.get_pixel(31, 1).0[3],
            0,
            "a short image sits in the middle"
        );
    }

    #[test]
    fn every_common_format_reads() {
        let image = gradient(32, 32);
        let mut bmp = Vec::new();
        BmpEncoder::new(&mut bmp)
            .write_image(image.as_raw(), 32, 32, ExtendedColorType::Rgba8)
            .unwrap();
        let mut gif = Vec::new();
        GifEncoder::new(&mut gif)
            .encode(image.as_raw(), 32, 32, ExtendedColorType::Rgba8)
            .unwrap();
        for bytes in [bmp, gif, png(&image)] {
            assert_eq!(
                decoded(&normalize(&bytes).unwrap()).dimensions(),
                (SIDE, SIDE)
            );
        }
    }

    #[test]
    fn what_no_icon_can_come_from_is_refused() {
        assert!(matches!(
            normalize(b"not an image"),
            Err(IconError::Unreadable(_))
        ));
        assert!(matches!(normalize(&[]), Err(IconError::Unreadable(_))));
        assert!(matches!(
            normalize(&vec![0; LONGEST_SOURCE + 1]),
            Err(IconError::TooLong)
        ));
        assert!(
            matches!(
                normalize(&vec![0; LONGEST_SOURCE]),
                Err(IconError::Unreadable(_))
            ),
            "the longest image the adapter takes reaches the decoder"
        );
        assert_eq!(
            within_limit(vec![0; LONGEST_ICON]).unwrap().len(),
            LONGEST_ICON
        );
        assert!(matches!(
            within_limit(vec![0; LONGEST_ICON + 1]),
            Err(IconError::TooLarge)
        ));
        assert_eq!(
            (LONGEST_SOURCE, super::MOST_DECODED, LONGEST_ICON),
            (512 * 1024, 64 * 1024 * 1024, 32 * 1024),
            "the limits hold their values"
        );
        let mut huge = png(&gradient(1, 1));
        // The header's width, after the signature and the chunk's length and kind, claims a side past the limit.
        huge.splice(16..20, 100_000_u32.to_be_bytes());
        let refused = normalize(&huge).unwrap_err();
        assert!(matches!(refused, IconError::Unreadable(_)), "{refused}");
        assert!(std::error::Error::source(&refused).is_some());
        assert_ne!(
            IconError::TooLong.to_string(),
            IconError::TooLarge.to_string()
        );
    }

    /// Writes one small image of each format into the seed corpus of the icon fuzz target, which `cargo nextest run --run-ignored only seeds` regenerates.
    #[test]
    #[ignore = "writes the committed seeds of the fuzz corpus; run it to regenerate them"]
    #[expect(
        clippy::disallowed_methods,
        reason = "the seed writer writes the committed corpus, which this crate has no file owner for"
    )]
    fn seeds() {
        let image = gradient(24, 12);
        let (width, height) = image.dimensions();
        let mut seeds: Vec<(&str, Vec<u8>)> = vec![("png", png(&image))];
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 80)
            .encode(
                &image::DynamicImage::ImageRgba8(image.clone()).to_rgb8(),
                width,
                height,
                ExtendedColorType::Rgb8,
            )
            .unwrap();
        seeds.push(("jpeg", jpeg));
        let mut gif = Vec::new();
        GifEncoder::new(&mut gif)
            .encode(image.as_raw(), width, height, ExtendedColorType::Rgba8)
            .unwrap();
        seeds.push(("gif", gif));
        let mut bmp = Vec::new();
        BmpEncoder::new(&mut bmp)
            .write_image(image.as_raw(), width, height, ExtendedColorType::Rgba8)
            .unwrap();
        seeds.push(("bmp", bmp));
        let mut ico = Vec::new();
        image::codecs::ico::IcoEncoder::new(&mut ico)
            .write_image(image.as_raw(), width, height, ExtendedColorType::Rgba8)
            .unwrap();
        seeds.push(("ico", ico));
        let mut webp = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
            .write_image(image.as_raw(), width, height, ExtendedColorType::Rgba8)
            .unwrap();
        seeds.push(("webp", webp));
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/corpus/icon_normalize");
        std::fs::create_dir_all(&corpus).unwrap();
        for (name, bytes) in seeds {
            assert_eq!(
                decoded(&normalize(&bytes).unwrap()).dimensions(),
                (SIDE, SIDE),
                "{name}"
            );
            std::fs::write(corpus.join(name), bytes).unwrap();
        }
    }
}
