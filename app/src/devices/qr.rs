use anyhow::{Result, anyhow, ensure};
use qrcodegen::{QrCode, QrCodeEcc};
use serde::Serialize;
use youarel_core::device_rules::{admitted_qr_side, admitted_qr_text};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CodeImage {
    rows: Vec<Vec<bool>>,
}

impl CodeImage {
    pub(super) fn encode(text: &str) -> Result<Self> {
        ensure!(
            admitted_qr_text(text.len()),
            "The invitation code is too long or empty."
        );
        let code = QrCode::encode_text(text, QrCodeEcc::Medium)
            .map_err(|error| anyhow!("The invitation QR code could not be drawn: {error}"))?;
        ensure!(
            admitted_qr_side(u32::try_from(code.size())?),
            "The invitation QR code has an unsupported size."
        );
        let rows = (0..code.size())
            .map(|y| (0..code.size()).map(|x| code.get_module(x, y)).collect())
            .collect();
        Ok(Self { rows })
    }
}

#[cfg(test)]
mod tests {
    use super::CodeImage;

    fn decode(image: &CodeImage) -> Vec<u8> {
        const QUIET: usize = 4;
        const SCALE: usize = 4;
        let side = image.rows.len();
        let modules = side.checked_add(QUIET.checked_mul(2).unwrap()).unwrap();
        let width = modules.checked_mul(SCALE).unwrap();
        let mut pixels = vec![u8::MAX; width.checked_mul(width).unwrap()];
        for (y, row) in image.rows.iter().enumerate() {
            for (x, dark) in row.iter().enumerate() {
                if *dark {
                    let top = y.checked_add(QUIET).unwrap().checked_mul(SCALE).unwrap();
                    let left = x.checked_add(QUIET).unwrap().checked_mul(SCALE).unwrap();
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            let pixel_row = top.checked_add(dy).unwrap();
                            let column = left.checked_add(dx).unwrap();
                            let index = pixel_row
                                .checked_mul(width)
                                .unwrap()
                                .checked_add(column)
                                .unwrap();
                            *pixels.get_mut(index).unwrap() = 0;
                        }
                    }
                }
            }
        }
        let mut decoder = quircs::Quirc::default();
        let mut codes = decoder.identify(width, width, &pixels);
        let payload = codes.next().unwrap().unwrap().decode().unwrap().payload;
        assert!(codes.next().is_none());
        payload
    }

    #[test]
    fn an_independent_decoder_reads_the_exact_text_from_the_emitted_matrix() {
        let longest = "x".repeat(youarel_core::device_rules::LONGEST_QR_TEXT);
        for text in [
            "1-K7QM-2XRD",
            "65535-K7QM-2XRD",
            "招待 🖥️",
            longest.as_str(),
        ] {
            let image = CodeImage::encode(text).unwrap();
            assert!(image.rows.iter().all(|row| row.len() == image.rows.len()));
            assert_eq!(decode(&image), text.as_bytes());
        }
        CodeImage::encode("").unwrap_err();
        CodeImage::encode(&format!("{longest}x")).unwrap_err();
    }
}
