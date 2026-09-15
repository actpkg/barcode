use act_sdk::prelude::*;
use serde::Serialize;

mod source;
use source::Source;
mod gtin;

#[derive(Serialize)]
pub struct DecodeOutput {
    /// Number of barcodes found. Zero is a successful result, not an error.
    pub count: usize,
    pub results: Vec<DecodedBarcode>,
}

#[derive(Serialize)]
pub struct DecodedBarcode {
    /// rxing's symbology name, e.g. `QR_CODE`, `EAN_13`, `PDF_417`.
    pub format: String,
    /// The decoded payload.
    pub text: String,
    /// Corner points located in the source image, in pixels.
    pub corners: Vec<Corner>,
    /// GTIN-14 form, present only for 1D retail symbologies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gtin: Option<String>,
    /// Whether the GTIN check digit validates. Present with `gtin`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_digit_valid: Option<bool>,
}

#[derive(Serialize)]
pub struct Corner {
    pub x: f32,
    pub y: f32,
}

/// Decode every barcode in an already-read image.
pub fn decode_bytes(bytes: &[u8]) -> ActResult<DecodeOutput> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| ActError::invalid_args(format!("Cannot decode image: {e}")))?;
    let luma = img.to_luma8();
    let (w, h) = luma.dimensions();

    let mut hints = rxing::DecodeHints::default();
    let found =
        rxing::helpers::detect_multiple_in_luma_with_hints(luma.into_raw(), w, h, &mut hints);

    // rxing signals "nothing found" as an Err, which is a successful empty
    // result here — an agent asking "what is in this image" gets an answer.
    // Only NotFoundException means that. Every other variant is a real
    // failure and must surface: reporting an internal decoder fault as
    // `count: 0` would tell the agent the image is clean when it is not.
    let results = match found {
        Ok(results) => results,
        Err(rxing::Exceptions::NotFoundException(_)) => Vec::new(),
        Err(e) => {
            return Err(ActError::internal(format!("Decoder failed: {e:?}")));
        }
    };

    let results: Vec<DecodedBarcode> = results
        .into_iter()
        .map(|r| {
            let format = r.getBarcodeFormat();
            let text = r.getText().to_string();
            let g = gtin::normalise(format, &text);
            DecodedBarcode {
                format: format!("{format:?}"),
                text,
                corners: r
                    .getPoints()
                    .iter()
                    .map(|p| Corner { x: p.x, y: p.y })
                    .collect(),
                gtin: g.as_ref().map(|g| g.gtin.clone()),
                check_digit_valid: g.as_ref().map(|g| g.check_digit_valid),
            }
        })
        .collect();

    Ok(DecodeOutput {
        count: results.len(),
        results,
    })
}

/// Error-correction level. Higher levels survive more damage but hold less data.
#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Ecc {
    /// ~7% recovery.
    L,
    /// ~15% recovery (default).
    M,
    /// ~25% recovery.
    Q,
    /// ~30% recovery.
    H,
}

impl From<Ecc> for qrcode::EcLevel {
    fn from(e: Ecc) -> Self {
        match e {
            Ecc::L => qrcode::EcLevel::L,
            Ecc::M => qrcode::EcLevel::M,
            Ecc::Q => qrcode::EcLevel::Q,
            Ecc::H => qrcode::EcLevel::H,
        }
    }
}

/// Parse `#rrggbb` into an opaque RGBA pixel.
fn parse_colour(s: &str) -> ActResult<image::Rgba<u8>> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ActError::invalid_args(format!(
            "Colour must be #rrggbb, got {s:?}"
        )));
    }
    let n = u32::from_str_radix(hex, 16)
        .map_err(|e| ActError::invalid_args(format!("Cannot parse colour {s:?}: {e}")))?;
    Ok(image::Rgba([
        ((n >> 16) & 0xff) as u8,
        ((n >> 8) & 0xff) as u8,
        (n & 0xff) as u8,
        0xff,
    ]))
}

/// Render a QR code to PNG bytes.
pub fn render_qr(
    text: &str,
    ecc: Ecc,
    scale: u32,
    quiet_zone: bool,
    dark: &str,
    light: &str,
) -> ActResult<Vec<u8>> {
    let dark = parse_colour(dark)?;
    let light = parse_colour(light)?;
    let scale = scale.clamp(1, 64);

    let code = qrcode::QrCode::with_error_correction_level(text, ecc.into())
        .map_err(|e| ActError::invalid_args(format!("Cannot encode as QR: {e}")))?;

    let img = code
        .render::<image::Rgba<u8>>()
        .module_dimensions(scale, scale)
        .quiet_zone(quiet_zone)
        .dark_color(dark)
        .light_color(light)
        .build();

    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| ActError::internal(format!("Cannot encode PNG: {e}")))?;
    Ok(png.into_inner())
}

#[act_component]
mod component {
    use super::*;

    /// Decode barcodes from an image.
    #[act_tool(
        description = "Decode every barcode in an image. Recognises QR, Aztec, PDF417, DataMatrix and the 1D families (EAN-8/13, UPC-A/E, Code 39/93/128, ITF, Codabar). Supply exactly one of `data` or `path`. Retail 1D results also carry a normalised 14-digit `gtin` and `check_digit_valid`.",
        read_only
    )]
    fn decode(#[args] source: Source) -> ActResult<Json<DecodeOutput>> {
        let bytes = source.read()?;
        Ok(Json(decode_bytes(&bytes)?))
    }

    /// Generate a QR code as a PNG.
    #[act_tool(
        description = "Generate a QR code from text and return it as a PNG image. The QR version is chosen automatically to fit the payload.",
        read_only
    )]
    fn generate_qr(
        #[doc = "Text to encode"] text: String,
        #[doc = "Error-correction level: l, m (default), q, h"] ecc: Option<Ecc>,
        #[doc = "Module size in pixels (default 8, clamped to 1..=64)"] scale: Option<u32>,
        #[doc = "Include the surrounding quiet zone (default true; scanners need it)"]
        quiet_zone: Option<bool>,
        #[doc = "Foreground colour as #rrggbb (default #000000)"] dark: Option<String>,
        #[doc = "Background colour as #rrggbb (default #ffffff)"] light: Option<String>,
    ) -> ActResult<Content> {
        let png = render_qr(
            &text,
            ecc.unwrap_or(Ecc::M),
            scale.unwrap_or(8),
            quiet_zone.unwrap_or(true),
            dark.as_deref().unwrap_or("#000000"),
            light.as_deref().unwrap_or("#ffffff"),
        )?;
        Ok(Content("image/png", png))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_barcode_is_an_empty_result_not_an_error() {
        // A 32x32 all-white PNG contains no barcode.
        let img = image::GrayImage::from_pixel(32, 32, image::Luma([255u8]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(img)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();

        let out = decode_bytes(&png.into_inner()).unwrap();
        assert_eq!(out.count, 0);
        assert!(out.results.is_empty());
    }

    #[test]
    fn undecodable_bytes_are_invalid_args() {
        assert!(decode_bytes(b"not an image at all").is_err());
    }

    #[test]
    fn generated_qr_round_trips_through_decode() {
        let png = render_qr("https://actcore.dev", Ecc::M, 8, true, "#000000", "#ffffff").unwrap();
        let out = decode_bytes(&png).unwrap();
        assert_eq!(out.count, 1);
        assert_eq!(out.results[0].format, "QR_CODE");
        assert_eq!(out.results[0].text, "https://actcore.dev");
    }

    #[test]
    fn round_trips_utf8_payload() {
        let png = render_qr("Привет, мир", Ecc::H, 6, true, "#000000", "#ffffff").unwrap();
        let out = decode_bytes(&png).unwrap();
        assert_eq!(out.results[0].text, "Привет, мир");
    }

    #[test]
    fn rejects_a_bad_colour() {
        assert!(render_qr("x", Ecc::M, 8, true, "not-a-colour", "#ffffff").is_err());
    }

    #[test]
    fn rejects_an_oversized_payload() {
        // Beyond the capacity of even a version-40 L QR code.
        let huge = "x".repeat(10_000);
        assert!(render_qr(&huge, Ecc::M, 8, true, "#000000", "#ffffff").is_err());
    }
}
