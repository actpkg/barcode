---
name: barcode
description: Decode barcodes from images and generate QR codes as PNG
metadata:
  act: {}
---

# barcode

Decodes barcodes from raster images and generates QR codes.

Recognised on decode: QR, Aztec, PDF417, DataMatrix, and the 1D families —
EAN-8, EAN-13, UPC-A, UPC-E, Code 39, Code 93, Code 128, ITF and Codabar.
Generation is QR only.

## Supplying the image

`decode` takes exactly one of:

- `data` — the image bytes inline.
- `path` — a file on the host. Needs a `wasi:filesystem` read grant covering
  that path; without one the call fails with `capability_denied`.

Accepted image formats: PNG, JPEG, GIF, BMP, TIFF, WebP.

Pass the original image rather than a pre-processed one. Aggressive
autocontrast or sharpening can turn photo noise into edge patterns that a 1D
scanline matcher misreads as a plausible-looking barcode — a false positive
you won't get from the unmodified photo.

If a large photo comes back with `count: 0` but you can see a code in it,
pass `crop: [[x1, y1], [x2, y2]]` — the pixel bounds of the code's region —
and retry, rather than resending the whole frame. `decode` crops to that
region and upscales it if it's small before decoding, so you only need to
say roughly where the code is; you don't need to crop or resize the image
yourself first. A small code in a large frame (roughly under ~10% of the
frame area) is the case this is for.

## Reading the result

```json
{ "count": 1,
  "results": [
    { "format": "EAN_13",
      "text": "4006381333931",
      "corners": [{"x": 12.0, "y": 40.0}],
      "gtin": "04006381333931",
      "check_digit_valid": true } ] }
```

`count: 0` is a successful answer — the image had no barcode — not an error.

For retail 1D symbologies the result also carries `gtin`, the payload
normalised to 14 digits, and `check_digit_valid`. Use `gtin` when looking a
product up: a UPC-A is not an EAN-13 with a digit removed, and comparing the
raw `text` across symbologies will miss matches. A `false` check digit means
the image decoded cleanly but the number is not a valid GTIN — usually a
damaged or fabricated barcode.

## Generating a QR

`generate_qr` returns PNG bytes with an `image/png` MIME type.

- `ecc` — `l`, `m` (default), `q`, `h`. Higher survives more damage but holds
  less data. Use `q` or `h` for anything that will be printed and scanned in
  the wild.
- `scale` — module size in pixels, default 8.
- `quiet_zone` — default `true`. Leave it on: scanners need the margin. Turn
  it off only when compositing into a layout that already provides one.
- `dark` / `light` — `#rrggbb`, default black on white. Keep the contrast
  high; inverted or low-contrast codes often fail to scan.

The QR version is chosen automatically. An oversized payload is an error
rather than a silently truncated code — shorten the text or lower `ecc`.
