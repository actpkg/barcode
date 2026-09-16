"""Decoding real barcode images through `act run --mcp`."""

import base64
import json
import pathlib

import pytest

FIXTURES = pathlib.Path(__file__).parent / "fixtures"


def _b(raw: bytes) -> dict:
    """Wrap raw bytes in the canonical envelope for a JSON transport.

    Passing a `bytes` object straight through fails inside the MCP client:
    pydantic serialises the request with `mode="json"` and raises
    UnicodeDecodeError on the first non-UTF-8 byte (a PNG starts with 0x89).
    ACT-SPEC's `{"$bytes": "<base64>"}` envelope is the wire form.
    """
    return {"$bytes": base64.b64encode(raw).decode()}


def _results(payload):
    """Unwrap the tool's JSON content block.

    `decode` returns a TextContent block whose `.text` is the JSON document.
    Its `mimeType` is None over MCP, so do not assert on it.
    """
    return json.loads(payload[0].text)


@pytest.mark.parametrize(
    ("fixture", "fmt", "text"),
    [
        ("ean13.png", "EAN_13", "4006381333931"),
        ("upca.png", "UPC_A", "036000291452"),
        ("pdf417.png", "PDF_417", "ACT PDF417 payload"),
        ("aztec.png", "AZTEC", "ACT Aztec payload"),
        ("datamatrix.png", "DATA_MATRIX", "ACT DataMatrix payload"),
        ("code128.png", "CODE_128", "ACT-CODE-128"),
        ("upce.png", "UPC_E", "04252614"),
    ],
)
async def test_decodes_each_symbology(client, fixture, fmt, text):
    data = (FIXTURES / fixture).read_bytes()
    res = await client.call_tool("decode", {"data": _b(data)})
    out = _results(res.content)
    assert out["count"] == 1
    assert out["results"][0]["format"] == fmt
    assert out["results"][0]["text"] == text


async def test_ean13_carries_gtin14(client):
    data = (FIXTURES / "ean13.png").read_bytes()
    res = await client.call_tool("decode", {"data": _b(data)})
    r = _results(res.content)["results"][0]
    assert r["gtin"] == "04006381333931"
    assert r["check_digit_valid"] is True


async def test_upca_normalises_to_gtin14(client):
    data = (FIXTURES / "upca.png").read_bytes()
    res = await client.call_tool("decode", {"data": _b(data)})
    r = _results(res.content)["results"][0]
    assert r["gtin"] == "00036000291452"
    assert r["check_digit_valid"] is True


async def test_two_d_formats_have_no_gtin(client):
    data = (FIXTURES / "aztec.png").read_bytes()
    res = await client.call_tool("decode", {"data": _b(data)})
    r = _results(res.content)["results"][0]
    assert "gtin" not in r


async def test_upce_expands_to_gtin_not_zero_padded(client):
    # A real Procter & Gamble code. Zero-padding this 8-digit UPC-E would
    # give 00000004252614 -- a different product's GTIN (see src/gtin.rs).
    data = (FIXTURES / "upce.png").read_bytes()
    res = await client.call_tool("decode", {"data": _b(data)})
    r = _results(res.content)["results"][0]
    assert r["gtin"] == "00042100005264"
    assert r["check_digit_valid"] is True


async def test_multiple_barcodes_are_all_returned(client):
    # An EAN-13 and a Code 128 stacked on one canvas. rxing does not promise
    # an order for multi-barcode results, so compare as a set of
    # (format, text) pairs rather than indexing into `results`.
    data = (FIXTURES / "multi.png").read_bytes()
    res = await client.call_tool("decode", {"data": _b(data)})
    out = _results(res.content)
    assert out["count"] == 2
    found = {(r["format"], r["text"]) for r in out["results"]}
    assert found == {
        ("EAN_13", "4006381333931"),
        ("CODE_128", "ACT-CODE-128"),
    }


async def test_decodes_lossless_jxl(client):
    # `src_ean13.jxl`: libjxl, -distance 0 (lossless, Modular codestream).
    # Proves .jxl input reaches the barcode decoder end-to-end over the
    # real MCP transport, not just through the in-process unit test.
    data = (FIXTURES / "src_ean13.jxl").read_bytes()
    res = await client.call_tool("decode", {"data": _b(data)})
    out = _results(res.content)
    assert out["count"] == 1
    assert out["results"][0]["format"] == "EAN_13"
    assert out["results"][0]["text"] == "4006381333931"


async def test_decodes_lossy_jxl(client):
    # `lossy_qr.jxl`: libjxl, -distance 1.5 (lossy VarDCT codestream) --
    # a different code path through the decoder than the lossless Modular
    # fixture above, so both are covered rather than just one.
    data = (FIXTURES / "lossy_qr.jxl").read_bytes()
    res = await client.call_tool("decode", {"data": _b(data)})
    out = _results(res.content)
    assert out["count"] == 1
    assert out["results"][0]["format"] == "QR_CODE"
    assert out["results"][0]["text"] == "badge-crop-test"


async def test_crop_finds_a_small_code_in_a_large_frame(client):
    """Small code, large frame -- the case `crop` exists for.

    `large_frame_small_code.png` is synthetic, not a real photo: a real
    conference-badge photo of this exact scenario (1920x2560, QR at ~8% of
    frame, occupying ~557px on the long edge) could not be committed --
    it is a personal photo of a real person's contact details, and the
    repo is destined for a public registry. This fixture reproduces the
    *geometry* instead: a 264px QR (within the 235-423px range measured
    across real badge photos that decoded) pasted into a 1920x2560 white
    canvas at a known offset, so `crop` has the same small-code-in-a-
    large-frame shape to work with.

    Unlike the real photo, this synthetic frame decodes even WITHOUT
    `crop` -- a clean QR on white is an easier target than one photographed
    on a lanyard, so this test does not assert `count: 0` for the
    whole-frame case (verified live against the real photo during
    development: it does return `count: 0` there, but that fixture is not
    committed). What this test pins is the half that generalises: `crop`
    around the known offset finds the code and returns its payload.
    """
    data = (FIXTURES / "large_frame_small_code.png").read_bytes()
    res = await client.call_tool(
        "decode", {"data": _b(data), "crop": [[860, 1150], [1124, 1414]]}
    )
    out = _results(res.content)
    assert out["count"] == 1
    assert out["results"][0]["format"] == "QR_CODE"
    assert out["results"][0]["text"] == "badge-crop-test"
