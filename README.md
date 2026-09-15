# barcode

An [ACT](https://actcore.dev) component that decodes barcodes from images and
generates QR codes as PNG.

Decodes QR, Aztec, PDF417, DataMatrix and the 1D families (EAN-8/13,
UPC-A/E, Code 39/93/128, ITF, Codabar). Generates QR.

## Why a sandbox

A barcode decoder is a hostile-input surface: reaching it means running an
image decoder over bytes someone else chose, and the decoder itself is a port
of ZXing. Here both run inside wasm under a declared ceiling of **read-only
filesystem and nothing else** — no network, no writes. A malformed image that
corrupts the parser has nowhere to go.

## Use

```bash
# Inline bytes need no grant at all.
act call actpkg.dev/library/barcode decode --args '{"data": {"$bytes": "<base64 png>"}}'

# Reading from disk needs a read grant.
act call actpkg.dev/library/barcode decode \
  --args '{"path": "/data/label.png"}' \
  --grant '{"wasi:filesystem":{"mode":"allowlist","allow":[{"path":"/data/**","mode":"ro"}]}}'

act call actpkg.dev/library/barcode generate_qr --args '{"text": "https://actcore.dev", "ecc": "q"}'
```

## Build

```bash
just init && just build && just test
```

## Licence

MIT OR Apache-2.0
