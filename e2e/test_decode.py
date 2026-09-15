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
