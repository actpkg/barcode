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

/// Register JPEG XL support with the `image` crate, once.
///
/// `jxl-image-rs-integration` works by installing a decoding hook into
/// `image` itself, so after this call `image::load_from_memory` below
/// transparently handles `.jxl` — no separate decode branch needed. The
/// registration function returns `false` on every call after the first;
/// that's an ordinary, harmless result, not a signal to stop calling it, so
/// this only exists to avoid redoing the (cheap but pointless) work on
/// every decode.
fn ensure_jxl_registered() {
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| {
        jxl_image_rs_integration::register_image_decoding_hook();
    });
}

/// Decode every barcode in an already-read image, optionally restricted to a
/// `[[x1,y1],[x2,y2]]` pixel region. A small cropped region is upscaled
/// before decoding — see [`source::upscale_target`] for why a bare crop is
/// not enough on its own.
pub fn decode_bytes(bytes: &[u8], crop: Option<[[i64; 2]; 2]>) -> ActResult<DecodeOutput> {
    ensure_jxl_registered();
    let img = image::load_from_memory(bytes)
        .map_err(|e| ActError::invalid_args(format!("Cannot decode image: {e}")))?;

    let mut luma = img.to_luma8();

    if let Some(crop) = crop {
        let (img_w, img_h) = luma.dimensions();
        let (x, y, width, height) = source::resolve_crop(crop, img_w, img_h)?;
        luma = image::imageops::crop_imm(&luma, x, y, width, height).to_image();

        // A bare crop is often not enough: rxing's binariser needs enough
        // pixels per module, and a small region has too few even though the
        // barcode is now dominant in the frame. Upscale towards a long edge
        // that is known to work, capped so a tiny crop cannot become a
        // memory bomb.
        if let Some((new_w, new_h)) = source::upscale_target(width, height) {
            luma =
                image::imageops::resize(&luma, new_w, new_h, image::imageops::FilterType::Lanczos3);
        }
    }

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
            let g = gtin::normalize(format, &text);
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
fn parse_color(s: &str) -> ActResult<image::Rgba<u8>> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ActError::invalid_args(format!(
            "Color must be #rrggbb, got {s:?}"
        )));
    }
    let n = u32::from_str_radix(hex, 16)
        .map_err(|e| ActError::invalid_args(format!("Cannot parse color {s:?}: {e}")))?;
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
    let dark = parse_color(dark)?;
    let light = parse_color(light)?;
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
        description = "Decode every barcode in an image. Recognizes QR, Aztec, PDF417, DataMatrix and the 1D families (EAN-8/13, UPC-A/E, Code 39/93/128, ITF, Codabar). Supply exactly one of `data` or `path`. An optional `crop` region ([[x1,y1],[x2,y2]] pixel bounds) decodes just that part of the image, upscaling it first if it is small — useful for a small code in a large photo. Retail 1D results also carry a normalized 14-digit `gtin` and `check_digit_valid`.",
        read_only
    )]
    fn decode(#[args] source: Source) -> ActResult<Json<DecodeOutput>> {
        let crop = source.crop;
        let bytes = source.read()?;
        Ok(Json(decode_bytes(&bytes, crop)?))
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
        #[doc = "Foreground color as #rrggbb (default #000000)"] dark: Option<String>,
        #[doc = "Background color as #rrggbb (default #ffffff)"] light: Option<String>,
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

        let out = decode_bytes(&png.into_inner(), None).unwrap();
        assert_eq!(out.count, 0);
        assert!(out.results.is_empty());
    }

    #[test]
    fn undecodable_bytes_are_invalid_args() {
        assert!(decode_bytes(b"not an image at all", None).is_err());
    }

    #[test]
    fn generated_qr_round_trips_through_decode() {
        let png = render_qr("https://actcore.dev", Ecc::M, 8, true, "#000000", "#ffffff").unwrap();
        let out = decode_bytes(&png, None).unwrap();
        assert_eq!(out.count, 1);
        assert_eq!(out.results[0].format, "QR_CODE");
        assert_eq!(out.results[0].text, "https://actcore.dev");
    }

    #[test]
    fn round_trips_utf8_payload() {
        let png = render_qr("Привет, мир", Ecc::H, 6, true, "#000000", "#ffffff").unwrap();
        let out = decode_bytes(&png, None).unwrap();
        assert_eq!(out.results[0].text, "Привет, мир");
    }

    #[test]
    fn rejects_a_bad_color() {
        assert!(render_qr("x", Ecc::M, 8, true, "not-a-color", "#ffffff").is_err());
    }

    #[test]
    fn rejects_an_oversized_payload() {
        // Beyond the capacity of even a version-40 L QR code.
        let huge = "x".repeat(10_000);
        assert!(render_qr(&huge, Ecc::M, 8, true, "#000000", "#ffffff").is_err());
    }

    #[test]
    fn crop_decodes_a_code_pasted_at_a_known_offset() {
        // A small QR pasted into a much larger blank canvas at a known
        // offset. Proves `crop` finds it and returns the right payload --
        // not a claim that the whole-image path would fail to find the same
        // code (rxing's TryHarder is quite capable), just that `crop` works.
        let qr_png = render_qr("crop-me", Ecc::M, 4, true, "#000000", "#ffffff").unwrap();
        let qr_img = image::load_from_memory(&qr_png).unwrap().to_luma8();
        let (qw, qh) = qr_img.dimensions();

        let (canvas_w, canvas_h) = (qw + 400, qh + 400);
        let mut canvas = image::GrayImage::from_pixel(canvas_w, canvas_h, image::Luma([255u8]));
        let (ox, oy) = (200u32, 200u32);
        image::imageops::replace(&mut canvas, &qr_img, i64::from(ox), i64::from(oy));

        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(canvas)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let bytes = png.into_inner();

        let crop = [
            [i64::from(ox), i64::from(oy)],
            [i64::from(ox + qw), i64::from(oy + qh)],
        ];
        let out = decode_bytes(&bytes, Some(crop)).unwrap();
        assert_eq!(out.count, 1);
        assert_eq!(out.results[0].format, "QR_CODE");
        assert_eq!(out.results[0].text, "crop-me");
    }

    #[test]
    fn crop_box_x2_le_x1_is_invalid_args() {
        let png = render_qr("x", Ecc::M, 4, true, "#000000", "#ffffff").unwrap();
        assert!(decode_bytes(&png, Some([[50, 10], [10, 90]])).is_err());
    }

    #[test]
    fn crop_box_outside_image_is_invalid_args() {
        let png = render_qr("x", Ecc::M, 4, true, "#000000", "#ffffff").unwrap();
        assert!(decode_bytes(&png, Some([[9000, 9000], [9100, 9100]])).is_err());
    }

    #[test]
    fn crop_box_degenerate_after_clamp_is_invalid_args() {
        let png = render_qr("x", Ecc::M, 4, true, "#000000", "#ffffff").unwrap();
        // x1 == x2 after clamping both to the same in-bounds value is caught
        // by the same x2 <= x1 guard before clamping is ever applied.
        assert!(decode_bytes(&png, Some([[10, 10], [10, 20]])).is_err());
    }

    #[test]
    fn decodes_a_lossless_jxl() {
        // A real JPEG XL file (libjxl, -distance 0 / lossless), not a PNG
        // relabelled: this is what proves the registration hook actually
        // reaches `image::load_from_memory`, not just that JXL bytes exist.
        let jxl = include_bytes!("../e2e/fixtures/src_qr.jxl");
        let out = decode_bytes(jxl, None).unwrap();
        assert_eq!(out.count, 1);
        assert_eq!(out.results[0].format, "QR_CODE");
        assert_eq!(out.results[0].text, "badge-crop-test");
    }

    #[test]
    fn png_still_decodes_after_jxl_registration() {
        // The hook registers globally into the `image` crate; this proves
        // it doesn't shadow or otherwise break the PNG path it shares
        // `load_from_memory` with. Runs after the JXL test above in the
        // same process, which is exactly the ordering that would expose a
        // regression here.
        let png = render_qr("still-png", Ecc::M, 4, true, "#000000", "#ffffff").unwrap();
        let out = decode_bytes(&png, None).unwrap();
        assert_eq!(out.results[0].text, "still-png");
    }
}
