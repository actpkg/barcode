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
//! `multi.png` additionally covers the multi-barcode case: two symbologies
//! composited onto one canvas.
//!
//! `upce.png` uses 255x150, not the 300x150 used for the other 1D fixtures:
//! at 300x150 rxing's own decoder fails to read back a UPC-E it just wrote
//! (`NotFoundException`), an aliasing artifact of that exact aspect ratio
//! against UPC-E's much narrower 51-module symbol (vs EAN-13's 95).
//! 255x150, 400x200 and 153x80 all decode correctly; measured, not guessed.
//!
//! `large_frame_small_code.png` is not produced by this script (it uses the
//! `qrcode` crate's `module_dimensions`/`quiet_zone` builder, which this
//! `MultiFormatWriter`-based generator does not have a matching knob for).
//! It is a small QR (264px, via `qrcode::QrCode::render().module_dimensions
//! (8, 8).quiet_zone(true)`) encoding `"badge-crop-test"`, pasted with
//! `image::imageops::replace` into a 1920x2560 white canvas at (860, 1150).
//! It exists to test `decode`'s `crop` argument on a small-code-in-a-
//! large-frame shape without committing a real photo — see `test_decode.py`
//! for why a genuine photo of this scenario could not be used.

use image::{GenericImage, GrayImage, Luma};
use rxing::{BarcodeFormat, MultiFormatWriter, Writer};

fn main() {
    let cases: Vec<(&str, BarcodeFormat, &str, i32, i32)> = vec![
        ("ean13", BarcodeFormat::EAN_13, "4006381333931", 300, 150),
        ("upca", BarcodeFormat::UPC_A, "036000291452", 300, 150),
        ("code128", BarcodeFormat::CODE_128, "ACT-CODE-128", 400, 150),
        ("pdf417", BarcodeFormat::PDF_417, "ACT PDF417 payload", 400, 200),
        ("aztec", BarcodeFormat::AZTEC, "ACT Aztec payload", 300, 300),
        ("datamatrix", BarcodeFormat::DATA_MATRIX, "ACT DataMatrix payload", 300, 300),
        // See the module doc comment for why this is 255x150, not 300x150.
        ("upce", BarcodeFormat::UPC_E, "04252614", 255, 150),
    ];
    let writer = MultiFormatWriter;

    let encode_gray = |data: &str, format: BarcodeFormat, width: i32, height: i32| -> GrayImage {
        let matrix = writer
            .encode(data, &format, width, height)
            .unwrap_or_else(|e| panic!("encode {data}: {e:?}"));
        let img: image::DynamicImage = (&matrix).into();
        img.to_luma8()
    };

    for (name, format, data, width, height) in &cases {
        let img = encode_gray(data, *format, *width, *height);
        let path = format!("e2e/fixtures/{name}.png");
        img.save(&path).unwrap();
        println!("wrote {path}");
    }

    // multi.png: an EAN-13 and a Code 128 stacked vertically on one white
    // canvas, so `decode` must return both from a single call.
    let a = encode_gray("4006381333931", BarcodeFormat::EAN_13, 300, 150);
    let b = encode_gray("ACT-CODE-128", BarcodeFormat::CODE_128, 400, 150);
    let (aw, ah) = (a.width(), a.height());
    let (bw, bh) = (b.width(), b.height());
    let gap = 40;
    let mut canvas = GrayImage::from_pixel(aw.max(bw) + 40, ah + bh + gap + 40, Luma([255u8]));
    canvas.copy_from(&a, 20, 20).unwrap();
    canvas.copy_from(&b, 20, 20 + ah + gap).unwrap();
    canvas.save("e2e/fixtures/multi.png").unwrap();
    println!("wrote e2e/fixtures/multi.png");
}
