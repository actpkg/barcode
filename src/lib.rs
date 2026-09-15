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
}
