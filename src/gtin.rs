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

    // UPC-E is a *compressed* 8-digit form, not a truncated GTIN: turning it
    // into a GTIN needs the GS1 expansion table, not zero-padding. Padding
    // silently produces the wrong 14-digit number for a different product,
    // and a passing check digit on that wrong number actively conceals it
    // (zero-padding can't disturb a mod-10 checksum, so the UPC-E's own
    // valid check digit survives onto the wrong GTIN).
    if *format == BarcodeFormat::UPC_E {
        let upca = expand_upc_e(text)?;
        let gtin = format!("{upca:0>14}");
        return Some(Gtin {
            check_digit_valid: check_digit_valid(&gtin),
            gtin,
        });
    }

    // GTIN-8/12/13/14 are the only defined widths for the remaining carriers.
    if !matches!(text.len(), 8 | 12 | 13 | 14) {
        return None;
    }

    let gtin = format!("{text:0>14}");
    Some(Gtin {
        check_digit_valid: check_digit_valid(&gtin),
        gtin,
    })
}

/// Expand a UPC-E payload (number system + 6 compressed digits + check
/// digit) to the 12-digit UPC-A it stands for, per the GS1 rule table keyed
/// on the last of the six payload digits. Returns `None` unless the payload
/// is exactly 8 digits — anything else cannot be a UPC-E at all.
fn expand_upc_e(text: &str) -> Option<String> {
    if text.len() != 8 {
        return None;
    }
    let ns = &text[0..1];
    let d = &text[1..7]; // six compressed payload digits, itself indexable by d[i..j]
    let check = &text[7..8];
    let last = &d[5..6];

    let (manufacturer, item) = match last {
        "0" | "1" | "2" => (format!("{}{last}00", &d[0..2]), format!("00{}", &d[2..5])),
        "3" => (format!("{}00", &d[0..3]), format!("000{}", &d[3..5])),
        "4" => (format!("{}0", &d[0..4]), format!("0000{}", &d[4..5])),
        _ => (d[0..5].to_string(), format!("0000{last}")),
    };

    Some(format!("{ns}{manufacturer}{item}{check}"))
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

    #[test]
    fn upce_expands_via_gs1_rules_not_zero_padding() {
        // A real Procter & Gamble code. Zero-padding this 8-digit UPC-E would
        // give 00000004252614 -- a different product's GTIN.
        let g = normalise(&BarcodeFormat::UPC_E, "04252614").unwrap();
        assert_eq!(g.gtin, "00042100005264");
        assert!(g.check_digit_valid);
    }

    #[test]
    fn upce_expands_second_vector() {
        let g = normalise(&BarcodeFormat::UPC_E, "01234565").unwrap();
        assert_eq!(g.gtin, "00012345000065");
        assert!(g.check_digit_valid);
    }

    #[test]
    fn upce_wrong_width_has_no_gtin() {
        // UPC-E is always exactly 8 digits (number system + 6 compressed +
        // check); anything else cannot be expanded and must not be mangled.
        assert!(normalise(&BarcodeFormat::UPC_E, "0123456").is_none());
        assert!(normalise(&BarcodeFormat::UPC_E, "012345678").is_none());
    }
}
