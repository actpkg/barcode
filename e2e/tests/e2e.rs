//! Drive the packed component through `act run --mcp` with a real MCP client.
//!
//! This replaces the python fastmcp/pytest suite that used to live beside
//! these sources: the tests observe exactly what an agent observes, over the
//! same client stack (`rmcp`) the host bridge itself is built on.
//!
//! Env: WASM — path to the packed component (default: the component's
//!      release build output);
//!      ACT  — the act invocation (default `act`; `npx @actcore/act`, the
//!             component justfile's default, also works — whitespace-split,
//!             like the shlex.split the python conftest did).

use std::path::PathBuf;

use base64::Engine as _;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ContentBlock},
    transport::TokioChildProcess,
};
use serde_json::{Value, json};

/// `().serve(transport)` hands back the client-role service running over the
/// child process: role first, the unit client handler second.
type Client = rmcp::service::RunningService<rmcp::service::RoleClient, ()>;

fn wasm_path() -> PathBuf {
    PathBuf::from(std::env::var("WASM").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/wasm32-wasip2/release/component_barcode.wasm"
        )
        .into()
    }))
}

/// The ACT invocation, honouring the same override the component justfile
/// uses. Its default there is `npx @actcore/act` — two words — which cannot
/// be `argv[0]` for a non-shell spawn, so the value is whitespace-split into
/// program + leading args.
fn act_argv() -> Vec<String> {
    std::env::var("ACT")
        .unwrap_or_else(|_| "act".into())
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Spawn `act run <wasm> --mcp` with the grant this component needs.
///
/// Grants are NOT optional: the default policy mode is `ask` and a headless
/// run degrades it to deny. The ceiling is read-only `path = "**"`
/// (act.toml: "Read-only by design"), so opening the `wasi:filesystem` class
/// grants exactly what the python conftest granted, nothing wider.
fn act_command() -> tokio::process::Command {
    act_command_inner(true)
}

/// The same component with NO capability grants — `test_hardening`'s
/// `ungranted_client`. The ask-by-default policy degrades to deny in this
/// headless run, so any `path` at all is outside the (empty) grant.
fn act_command_ungranted() -> tokio::process::Command {
    act_command_inner(false)
}

fn act_command_inner(granted: bool) -> tokio::process::Command {
    let argv = act_argv();
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.arg("run").arg(wasm_path()).arg("--mcp");
    if granted {
        cmd.args(["--allow", "wasi:filesystem"]);
    }
    cmd
}

async fn connect() -> Client {
    connect_over(act_command()).await
}

async fn connect_ungranted() -> Client {
    connect_over(act_command_ungranted()).await
}

async fn connect_over(command: tokio::process::Command) -> Client {
    // No timeout around the handshake: `act run --mcp` instantiates the
    // component before it answers `initialize`, and that cost is the
    // connect (the python conftest bounded it at 120s for exactly this
    // reason and still called the bound "deliberately loose").
    ()
        .serve(TokioChildProcess::new(command).expect("spawn act run --mcp"))
        .await
        .expect("rmcp handshake with act run --mcp")
}

fn first_text_block(result: &rmcp::model::CallToolResult) -> &rmcp::model::TextContent {
    match result.content.first() {
        Some(ContentBlock::Text(t)) => t,
        other => panic!("expected the first content block to be Text, got: {other:?}"),
    }
}

/// Wrap raw bytes in the canonical envelope for a JSON transport.
///
/// Passing raw bytes inside a JSON argument is not a thing: ACT-SPEC's
/// `{"$bytes": "<base64>"}` envelope is the wire form (the python suite
/// wrapped every fixture the same way).
fn b64_envelope(bytes: &[u8]) -> Value {
    json!({ "$bytes": base64::engine::general_purpose::STANDARD.encode(bytes) })
}

/// `decode` args with the image inline.
fn data_args(bytes: &[u8]) -> Value {
    json!({ "data": b64_envelope(bytes) })
}

/// Call `decode` and return the parsed JSON document, or a failure string.
///
/// `decode` returns a TextContent block whose `.text` is the JSON document.
/// Its mimeType is None over MCP, so do not assert on it.
async fn try_decode(client: &Client, args: Value) -> Result<Value, String> {
    let result = client
        .call_tool(CallToolRequestParams::new("decode").with_arguments(
            args.as_object()
                .expect("decode args are a JSON object")
                .clone(),
        ))
        .await
        .map_err(|e| format!("call_tool failed: {e:?}"))?;
    if result.is_error == Some(true) {
        return Err(format!("decode failed: {result:?}"));
    }
    serde_json::from_str(&first_text_block(&result).text)
        .map_err(|e| format!("decode did not return a JSON document: {e}"))
}

/// The panicking flavour the single-case tests read best with.
async fn decode_ok(client: &Client, args: Value) -> Value {
    try_decode(client, args)
        .await
        .unwrap_or_else(|e| panic!("decode failed: {e}"))
}

/// Call `generate_qr` and return the PNG bytes from the image content block,
/// or a failure string.
///
/// `generate_qr` returns an MCP ImageContent block: `.data` is a base64
/// string and `.mimeType` is "image/png". (`decode`, by contrast, returns
/// TextContent whose mimeType is None — never assert a mime-type there.)
async fn try_generate_png(client: &Client, args: Value) -> Result<Vec<u8>, String> {
    let result = client
        .call_tool(CallToolRequestParams::new("generate_qr").with_arguments(
            args.as_object()
                .expect("generate_qr args are a JSON object")
                .clone(),
        ))
        .await
        .map_err(|e| format!("call_tool failed: {e:?}"))?;
    if result.is_error == Some(true) {
        return Err(format!("generate_qr failed: {result:?}"));
    }
    match result.content.first() {
        Some(ContentBlock::Image(img)) => {
            if img.mime_type != "image/png" {
                return Err(format!("expected image/png, got {}", img.mime_type));
            }
            base64::engine::general_purpose::STANDARD
                .decode(&img.data)
                .map_err(|e| format!("image data is not base64: {e}"))
        }
        other => Err(format!("expected an image content block, got: {other:?}")),
    }
}

/// The panicking flavour the single-case tests read best with.
async fn generate_png(client: &Client, args: Value) -> Vec<u8> {
    try_generate_png(client, args)
        .await
        .unwrap_or_else(|e| panic!("generate_qr failed: {e}"))
}

/// A tool-level failure surfaces through the MCP client either as a JSON-RPC
/// error response (`ErrorData.data` / `message`) or as an isError result
/// (`_meta` / text content) — fastmcp raised `ToolError` from the latter, so
/// that is the path every guest-produced failure takes here. Returns
/// `(kind, message)`; panics if the call SUCCEEDED.
///
/// The kind is `dev.actcore/error-kind` from wherever the failure surfaced.
/// A missing kind reads back as the empty string, so an assert on a specific
/// kind fails loudly rather than passing on a vanished key.
async fn expect_failure(client: &Client, tool: &'static str, args: Value) -> (String, String) {
    let params = CallToolRequestParams::new(tool).with_arguments(
        args.as_object()
            .expect("args are a JSON object")
            .clone(),
    );
    match client.call_tool(params).await {
        Err(rmcp::ServiceError::McpError(e)) => {
            let kind = e
                .data
                .as_ref()
                .and_then(|d| d.get("dev.actcore/error-kind"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            (kind.unwrap_or_default(), e.message.to_string())
        }
        Ok(result) => {
            assert_eq!(result.is_error, Some(true), "the call must fail: {result:?}");
            let kind = result
                .meta
                .as_ref()
                .and_then(|m| m.0.get("dev.actcore/error-kind"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let message = result
                .content
                .first()
                .and_then(|b| match b {
                    ContentBlock::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            (kind.unwrap_or_default(), message)
        }
        Err(other) => panic!("unexpected transport failure: {other:?}"),
    }
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/").to_owned() + name)
        .unwrap_or_else(|e| panic!("fixture {name} is missing: {e}"))
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex digits"))
        .collect()
}

// ── test_tools.py ───────────────────────────────────────────────────────────

#[tokio::test]
async fn component_exposes_its_tools() {
    let client = connect().await;
    let tools = client.list_all_tools().await.expect("list_all_tools");
    assert!(
        tools.len() >= 1,
        "a component with no tools is almost always a packaging mistake"
    );
    client.cancel().await.ok();
}

// ── test_info.py ────────────────────────────────────────────────────────────

/// The packed artifact carries the metadata `act-build pack` embedded. Also
/// the fast-fail the python `wasm_path` fixture provided — an unpacked wasm
/// (raw `cargo build` output, no `act:component` section) declares no
/// ceiling, every grant is refused as "outside ceiling", and the failures
/// point anywhere but at the missing metadata. The justfile's `test: build`
/// ordering exists so this test finds a packed artifact.
#[test]
fn manifest_reports_name_and_version() {
    let output = {
        let argv = act_argv();
        let mut cmd = std::process::Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        cmd.args(["inspect", "component-manifest"])
            .arg(wasm_path())
            .output()
            .expect("run act inspect component-manifest")
    };
    assert!(
        output.status.success(),
        "inspect failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: Value = serde_json::from_slice(&output.stdout).expect("manifest is JSON");
    assert_eq!(
        manifest["std"]["name"], "barcode",
        "packed manifest must carry the component name"
    );
    assert!(
        manifest["std"]["version"].is_string(),
        "packed manifest must carry a version, got: {}",
        manifest["std"]["version"]
    );
}

// ── test_decode.py ──────────────────────────────────────────────────────────

const SYMBOLOGIES: &[(&str, &str, &str)] = &[
    ("ean13.png", "EAN_13", "4006381333931"),
    ("upca.png", "UPC_A", "036000291452"),
    ("pdf417.png", "PDF_417", "ACT PDF417 payload"),
    ("aztec.png", "AZTEC", "ACT Aztec payload"),
    ("datamatrix.png", "DATA_MATRIX", "ACT DataMatrix payload"),
    ("code128.png", "CODE_128", "ACT-CODE-128"),
    ("upce.png", "UPC_E", "04252614"),
];

/// One parametrized case, reported as a failure string instead of a panic so
/// a bad fixture does not hide the verdict on the rest (pytest ran each
/// parametrization as its own test).
type Case = Result<(), String>;

async fn decode_symbology_case(client: &Client, fixture: &str, fmt: &str, text: &str) -> Case {
    let data = fixture_bytes(fixture);
    let out = try_decode(client, data_args(&data))
        .await
        .map_err(|e| format!("{fixture}: {e}"))?;
    if out["count"] != 1 {
        return Err(format!("{fixture}: count = {}, want 1", out["count"]));
    }
    let got = (
        out["results"][0]["format"].as_str().unwrap_or(""),
        out["results"][0]["text"].as_str().unwrap_or(""),
    );
    if got != (fmt, text) {
        return Err(format!(
            "{fixture}: decoded ({:?}, {:?}), want ({:?}, {:?})",
            got.0, got.1, fmt, text
        ));
    }
    Ok(())
}

#[tokio::test]
async fn decodes_each_symbology() {
    let client = connect().await;
    let mut failures = Vec::new();
    for (fixture, fmt, text) in SYMBOLOGIES {
        if let Err(e) = decode_symbology_case(&client, fixture, fmt, text).await {
            failures.push(e);
        }
    }
    assert!(
        failures.is_empty(),
        "symbology round trips failed:\n  {}",
        failures.join("\n  ")
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn ean13_carries_gtin14() {
    let client = connect().await;
    let data = fixture_bytes("ean13.png");
    let out = decode_ok(&client, data_args(&data)).await;
    let r = &out["results"][0];
    assert_eq!(r["gtin"], "04006381333931");
    assert_eq!(r["check_digit_valid"], true);
    client.cancel().await.ok();
}

#[tokio::test]
async fn upca_normalizes_to_gtin14() {
    let client = connect().await;
    let data = fixture_bytes("upca.png");
    let out = decode_ok(&client, data_args(&data)).await;
    let r = &out["results"][0];
    assert_eq!(r["gtin"], "00036000291452");
    assert_eq!(r["check_digit_valid"], true);
    client.cancel().await.ok();
}

#[tokio::test]
async fn two_d_formats_have_no_gtin() {
    let client = connect().await;
    let data = fixture_bytes("aztec.png");
    let out = decode_ok(&client, data_args(&data)).await;
    let r = &out["results"][0];
    assert_eq!(
        r.get("gtin"),
        None,
        "2D results must not carry a gtin key at all"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn upce_expands_to_gtin_not_zero_padded() {
    // A real Procter & Gamble code. Zero-padding this 8-digit UPC-E would
    // give 00000004252614 -- a different product's GTIN (see src/gtin.rs).
    let client = connect().await;
    let data = fixture_bytes("upce.png");
    let out = decode_ok(&client, data_args(&data)).await;
    let r = &out["results"][0];
    assert_eq!(r["gtin"], "00042100005264");
    assert_eq!(r["check_digit_valid"], true);
    client.cancel().await.ok();
}

#[tokio::test]
async fn multiple_barcodes_are_all_returned() {
    // An EAN-13 and a Code 128 stacked on one canvas. rxing does not promise
    // an order for multi-barcode results, so compare as a set of
    // (format, text) pairs rather than indexing into `results`.
    let client = connect().await;
    let data = fixture_bytes("multi.png");
    let out = decode_ok(&client, data_args(&data)).await;
    assert_eq!(out["count"], 2);
    let mut found: Vec<(String, String)> = out["results"]
        .as_array()
        .expect("results is an array")
        .iter()
        .map(|r| {
            (
                r["format"].as_str().unwrap_or_default().to_string(),
                r["text"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    found.sort();

    let mut want = vec![
        ("EAN_13".to_string(), "4006381333931".to_string()),
        ("CODE_128".to_string(), "ACT-CODE-128".to_string()),
    ];
    want.sort();

    assert_eq!(found, want);
    client.cancel().await.ok();
}

#[tokio::test]
async fn decodes_lossless_jxl() {
    // `src_ean13.jxl`: libjxl, -distance 0 (lossless, Modular codestream).
    // Proves .jxl input reaches the barcode decoder end-to-end over the
    // real MCP transport, not just through the in-process unit test.
    let client = connect().await;
    let data = fixture_bytes("src_ean13.jxl");
    let out = decode_ok(&client, data_args(&data)).await;
    assert_eq!(out["count"], 1);
    assert_eq!(out["results"][0]["format"], "EAN_13");
    assert_eq!(out["results"][0]["text"], "4006381333931");
    client.cancel().await.ok();
}

#[tokio::test]
async fn decodes_lossy_jxl() {
    // `lossy_qr.jxl`: libjxl, -distance 1.5 (lossy VarDCT codestream) --
    // a different code path through the decoder than the lossless Modular
    // fixture above, so both are covered rather than just one.
    let client = connect().await;
    let data = fixture_bytes("lossy_qr.jxl");
    let out = decode_ok(&client, data_args(&data)).await;
    assert_eq!(out["count"], 1);
    assert_eq!(out["results"][0]["format"], "QR_CODE");
    assert_eq!(out["results"][0]["text"], "badge-crop-test");
    client.cancel().await.ok();
}

#[tokio::test]
async fn crop_finds_a_small_code_in_a_large_frame() {
    // Small code, large frame -- the case `crop` exists for.
    // `large_frame_small_code.png` is synthetic, not a real photo: a real
    // conference-badge photo of this exact scenario (1920x2560, QR at ~8% of
    // frame) could not be committed -- it is a personal photo of a real
    // person's contact details, and the repo is destined for a public
    // registry. This fixture reproduces the *geometry* instead. Unlike the
    // real photo, it decodes even WITHOUT `crop`, so what this test pins is
    // the half that generalises: `crop` around the known offset finds the
    // code and returns its payload.
    let client = connect().await;
    let data = fixture_bytes("large_frame_small_code.png");
    let out = decode_ok(
        &client,
        json!({
            "data": b64_envelope(&data),
            "crop": [[860, 1150], [1124, 1414]],
        }),
    )
    .await;
    assert_eq!(out["count"], 1);
    assert_eq!(out["results"][0]["format"], "QR_CODE");
    assert_eq!(out["results"][0]["text"], "badge-crop-test");
    client.cancel().await.ok();
}

// ── test_generate.py ────────────────────────────────────────────────────────

#[tokio::test]
async fn generates_a_png() {
    let client = connect().await;
    let png = generate_png(&client, json!({ "text": "hello" })).await;
    assert!(
        png.starts_with(b"\x89PNG\r\n\x1a\n"),
        "not a PNG: {:?}",
        &png[..png.len().min(16)]
    );
    client.cancel().await.ok();
}

async fn round_trip_case(client: &Client, text: &str, ecc: &str) -> Case {
    let label = format!("ecc={ecc} text={:.20}", text);
    let png = try_generate_png(client, json!({ "text": text, "ecc": ecc }))
        .await
        .map_err(|e| format!("{label}: {e}"))?;
    let out = try_decode(client, data_args(&png))
        .await
        .map_err(|e| format!("{label}: {e}"))?;
    if out["count"] != 1 {
        return Err(format!("{label}: count = {}, want 1", out["count"]));
    }
    if out["results"][0]["format"] != "QR_CODE" {
        return Err(format!(
            "{label}: format = {}, want QR_CODE",
            out["results"][0]["format"]
        ));
    }
    if out["results"][0]["text"] != text {
        return Err(format!(
            "{label}: decoded {:?}, want {text:?}",
            out["results"][0]["text"]
        ));
    }
    Ok(())
}

#[tokio::test]
async fn round_trip() {
    let client = connect().await;
    let long = "x".repeat(400);
    let texts: Vec<&str> = vec![
        "hello",
        "https://actcore.dev/python",
        "Привет, мир",
        &long,
    ];
    let mut failures = Vec::new();
    for ecc in ["l", "m", "q", "h"] {
        for text in &texts {
            if let Err(e) = round_trip_case(&client, text, ecc).await {
                failures.push(e);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "generate -> decode round trips failed:\n  {}",
        failures.join("\n  ")
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn scale_changes_image_size() {
    let client = connect().await;
    let small = generate_png(&client, json!({ "text": "x", "scale": 2 })).await;
    let large = generate_png(&client, json!({ "text": "x", "scale": 16 })).await;
    assert!(
        large.len() > small.len(),
        "scale 16 ({}) must produce a bigger PNG than scale 2 ({})",
        large.len(),
        small.len()
    );
    client.cancel().await.ok();
}

// ── test_hardening.py ───────────────────────────────────────────────────────

#[tokio::test]
async fn blank_image_is_empty_not_an_error() {
    // A 1x1 white PNG (8-bit grayscale): valid image, no barcode.
    let client = connect().await;
    let png = unhex(concat!(
        "89504e470d0a1a0a0000000d49484452000000010000000108000000003a7e9b55",
        "0000000a49444154789c63f80f0001010100b138f6140000000049454e44ae426082",
    ));
    let out = decode_ok(&client, data_args(&png)).await;
    assert_eq!(out["count"], 0, "zero barcodes is a successful result");
    assert_eq!(
        out["results"],
        json!([]),
        "zero barcodes is an empty list, not an error"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn truncated_image_is_rejected() {
    let client = connect().await;
    let (kind, message) =
        expect_failure(&client, "decode", data_args(b"\x89PNG\r\n\x1a\ntruncated")).await;
    assert_eq!(
        kind, "std:invalid-args",
        "an undecodable image is a guest invalid-args error, got {kind:?}: {message}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn truncated_jxl_is_rejected() {
    // Cut a real, valid .jxl file in half. libjxl's own error path surfaces
    // cleanly through `image::load_from_memory` as std:invalid-args, not a
    // panic.
    let client = connect().await;
    let data = fixture_bytes("src_qr.jxl");
    let truncated = &data[..data.len() / 2];
    let (kind, message) = expect_failure(&client, "decode", data_args(truncated)).await;
    assert_eq!(kind, "std:invalid-args", "got {kind:?}: {message}");
    assert!(
        message.contains("Cannot decode image"),
        "expected the decoder's message, got: {message}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn both_sources_is_an_error() {
    let client = connect().await;
    let (kind, message) = expect_failure(
        &client,
        "decode",
        json!({ "data": b64_envelope(b"x"), "path": "/etc/hostname" }),
    )
    .await;
    assert_eq!(kind, "std:invalid-args", "got {kind:?}: {message}");
    assert!(
        message.contains("not both"),
        "expected the exactly-one-source message, got: {message}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn neither_source_is_an_error() {
    let client = connect().await;
    let (kind, message) = expect_failure(&client, "decode", json!({})).await;
    assert_eq!(kind, "std:invalid-args", "got {kind:?}: {message}");
    assert!(
        message.contains("provide the image as"),
        "expected the no-source message, got: {message}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn oversized_payload_is_rejected() {
    // Beyond the capacity of even a version-40 L QR code.
    let client = connect().await;
    let huge = "x".repeat(10_000);
    let (kind, message) =
        expect_failure(&client, "generate_qr", json!({ "text": huge })).await;
    assert_eq!(kind, "std:invalid-args", "got {kind:?}: {message}");
    assert!(
        message.contains("Cannot encode as QR"),
        "expected the capacity message, got: {message}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn bad_color_is_rejected() {
    let client = connect().await;
    let (kind, message) = expect_failure(
        &client,
        "generate_qr",
        json!({ "text": "x", "dark": "red" }),
    )
    .await;
    assert_eq!(kind, "std:invalid-args", "got {kind:?}: {message}");
    assert!(
        message.contains("Color must be #rrggbb"),
        "expected the color-format message, got: {message}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn path_outside_grant_is_capability_denied() {
    // The granted `connect()` cannot exercise this case -- any readable path
    // would be permitted under `--allow wasi:filesystem`. `connect_ungranted`
    // starts the same component with no grants at all: the ask-by-default
    // policy degrades to deny in this headless run, so any `path` at all is
    // outside the (empty) grant.
    let client = connect_ungranted().await;
    let (kind, message) =
        expect_failure(&client, "decode", json!({ "path": "/etc/hostname" })).await;
    assert_eq!(
        kind, "std:capability-denied",
        "a path outside the grant is a capability denial, got {kind:?}: {message}"
    );
    assert!(
        message.contains("Permission denied"),
        "expected the permission message, got: {message}"
    );
    client.cancel().await.ok();
}
