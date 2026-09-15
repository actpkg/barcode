//! GTIN normalisation for 1D retail symbologies.
//!
//! Agents routinely get this wrong — UPC-A is not an EAN-13 by truncation —
//! so the component does it once, here.

use rxing::BarcodeFormat;

pub struct Gtin {
    /// The value left-padded to 14 digits (GTIN-14).
    pub gtin: String,
    /// Whether the trailing check digit validates under mod-10.
    pub check_digit_valid: bool,
}

/// Returns `None` for any format that is not a GTIN carrier, or whose payload
/// is not a plausible GTIN. A bad check digit is reported, not suppressed:
/// the caller asked what the image says.
pub fn normalise(format: &BarcodeFormat, text: &str) -> Option<Gtin> {
    let carries_gtin = matches!(
        format,
        BarcodeFormat::EAN_8
            | BarcodeFormat::EAN_13
            | BarcodeFormat::UPC_A
            | BarcodeFormat::UPC_E
            | BarcodeFormat::ITF
    );
    if !carries_gtin {
        return None;
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // GTIN-8/12/13/14 are the only defined widths.
    if !matches!(text.len(), 8 | 12 | 13 | 14) {
        return None;
    }

    let gtin = format!("{text:0>14}");
    Some(Gtin {
        check_digit_valid: check_digit_valid(&gtin),
        gtin,
    })
}

/// Mod-10: weight the 13 digits before the check digit 3,1,3,1,… from the
/// right, sum, and the check digit is what rounds the total up to a multiple
/// of ten.
fn check_digit_valid(gtin14: &str) -> bool {
    let digits: Vec<u32> = gtin14.bytes().map(|b| u32::from(b - b'0')).collect();
    let Some((&check, body)) = digits.split_last() else {
        return false;
    };
    let sum: u32 = body
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| if i % 2 == 0 { d * 3 } else { *d })
        .sum();
    (10 - (sum % 10)) % 10 == check
}

#[cfg(test)]
mod tests {
    use super::*;
    use rxing::BarcodeFormat;

    #[test]
    fn ean13_pads_to_fourteen() {
        let g = normalise(&BarcodeFormat::EAN_13, "4006381333931").unwrap();
        assert_eq!(g.gtin, "04006381333931");
        assert!(g.check_digit_valid);
    }

    #[test]
    fn upca_pads_to_fourteen() {
        let g = normalise(&BarcodeFormat::UPC_A, "036000291452").unwrap();
        assert_eq!(g.gtin, "00036000291452");
        assert!(g.check_digit_valid);
    }

    #[test]
    fn bad_check_digit_is_reported_not_rejected() {
        let g = normalise(&BarcodeFormat::EAN_13, "4006381333930").unwrap();
        assert_eq!(g.gtin, "04006381333930");
        assert!(!g.check_digit_valid);
    }

    #[test]
    fn two_d_formats_have_no_gtin() {
        assert!(normalise(&BarcodeFormat::QR_CODE, "4006381333931").is_none());
    }

    #[test]
    fn non_numeric_has_no_gtin() {
        assert!(normalise(&BarcodeFormat::CODE_128, "ABC-123").is_none());
    }
}
