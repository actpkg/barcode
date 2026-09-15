#!/usr/bin/env -S cargo +nightly -Zscript
---
[dependencies]
rxing = { version = "0.9", default-features = false, features = [
  "encoders", "aztec", "datamatrix", "oned", "pdf417", "qrcode", "image",
] }
image = { version = "0.25", default-features = false, features = ["png"] }
---
//! Regenerate the decode fixtures. Run manually; the PNGs are committed.
//!
//!     cargo +nightly -Zscript e2e/fixtures/generate.rs
//!
//! Fixtures cover only the symbologies `qrcode` cannot produce — QR itself is
//! covered by the generate -> decode round-trip test, which needs no fixture.

use rxing::{BarcodeFormat, MultiFormatWriter, Writer};

fn main() {
    let cases: Vec<(&str, BarcodeFormat, &str, i32, i32)> = vec![
        ("ean13", BarcodeFormat::EAN_13, "4006381333931", 300, 150),
        ("upca", BarcodeFormat::UPC_A, "036000291452", 300, 150),
        ("code128", BarcodeFormat::CODE_128, "ACT-CODE-128", 400, 150),
        ("pdf417", BarcodeFormat::PDF_417, "ACT PDF417 payload", 400, 200),
        ("aztec", BarcodeFormat::AZTEC, "ACT Aztec payload", 300, 300),
        ("datamatrix", BarcodeFormat::DATA_MATRIX, "ACT DataMatrix payload", 300, 300),
    ];
    let writer = MultiFormatWriter;
    for (name, format, data, width, height) in cases {
        let matrix = writer
            .encode(data, &format, width, height)
            .unwrap_or_else(|e| panic!("encode {name}: {e:?}"));
        let img: image::DynamicImage = (&matrix).into();
        let path = format!("e2e/fixtures/{name}.png");
        img.to_luma8().save(&path).unwrap();
        println!("wrote {path}");
    }
}
