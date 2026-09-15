//! Where the image bytes come from.

use std::io;

use act_sdk::prelude::*;

/// Supply exactly one of `data` or `path`.
#[derive(Deserialize, JsonSchema)]
pub struct Source {
    /// Inline image bytes, as a CBOR byte string — or the canonical
    /// `{"$bytes": "<base64>"}` envelope over JSON transports.
    pub data: Option<Bytes>,
    /// Path to an image file on the host. Requires a `wasi:filesystem` read
    /// grant covering this path.
    pub path: Option<String>,
    /// Region to decode, as `[[x1, y1], [x2, y2]]` pixel coordinates (top-left
    /// and bottom-right corners) in the source image. Optional; omitting it
    /// decodes the whole image, exactly as before. A vision-capable agent
    /// that can see roughly where a code sits in a photo can pass its bounds
    /// here instead of sending the whole frame — useful when a small code in
    /// a large photo fails to decode at full resolution (see SKILL.md).
    pub crop: Option<[[i64; 2]; 2]>,
}

/// A validated, in-bounds crop region: `(x, y, width, height)` in pixels.
pub type CropRegion = (u32, u32, u32, u32);

/// Validate and clamp a `crop` argument against an image's actual dimensions.
///
/// This takes untrusted input (an agent's visual estimate of a region), so it
/// is deliberately forgiving about *overshoot* — clamping a box that runs a
/// few pixels past an edge rather than failing the whole call — and strict
/// about a box that cannot mean anything: inverted, degenerate, or entirely
/// outside the image.
pub fn resolve_crop(crop: [[i64; 2]; 2], img_w: u32, img_h: u32) -> ActResult<CropRegion> {
    let [[x1, y1], [x2, y2]] = crop;
    if x2 <= x1 || y2 <= y1 {
        return Err(ActError::invalid_args(format!(
            "crop box must have x2 > x1 and y2 > y1, got [[{x1},{y1}],[{x2},{y2}]]"
        )));
    }
    if x2 <= 0 || y2 <= 0 || x1 >= i64::from(img_w) || y1 >= i64::from(img_h) {
        return Err(ActError::invalid_args(format!(
            "crop box [[{x1},{y1}],[{x2},{y2}]] is entirely outside the {img_w}x{img_h} image"
        )));
    }

    // Clamp to bounds rather than rejecting a box that merely overshoots an
    // edge — an agent's visual estimate routinely does, by a few pixels.
    let cx1 = x1.clamp(0, i64::from(img_w)) as u32;
    let cy1 = y1.clamp(0, i64::from(img_h)) as u32;
    let cx2 = x2.clamp(0, i64::from(img_w)) as u32;
    let cy2 = y2.clamp(0, i64::from(img_h)) as u32;

    let (width, height) = (cx2 - cx1, cy2 - cy1);
    if width == 0 || height == 0 {
        return Err(ActError::invalid_args(format!(
            "crop box [[{x1},{y1}],[{x2},{y2}]] has zero area after clamping to the {img_w}x{img_h} image"
        )));
    }
    Ok((cx1, cy1, width, height))
}

/// The long edge to upscale a cropped region towards, and the caps that keep
/// a tiny crop from becoming a memory bomb.
///
/// Measured against a real phone photo (1920x2560, QR at ~8% of frame): a
/// bare crop (557px long edge) does not decode; upscaling the same crop to
/// an 800px long edge does. The root cause is pixels-per-module, not frame
/// size, so 1024 gives margin over the measured 800px threshold rather than
/// sitting right at it. This is not an edge case: across a batch of real
/// conference-badge photos, every QR that decoded occupied only 235-423px on
/// the long edge — well below 557 — so the auto-upscale is not a nicety, it
/// is what makes `crop` decode this kind of input at all.
const UPSCALE_TARGET: u32 = 1024;
const UPSCALE_MAX_FACTOR: u32 = 4;
const UPSCALE_MAX_EDGE: u32 = 4096;

/// If a cropped region's long edge is below [`UPSCALE_TARGET`], compute the
/// new `(width, height)` to resize it to — preserving aspect ratio, capped
/// at [`UPSCALE_MAX_FACTOR`]x and [`UPSCALE_MAX_EDGE`] px. Returns `None` if
/// the region is already large enough and needs no resize.
pub fn upscale_target(width: u32, height: u32) -> Option<(u32, u32)> {
    let long_edge = width.max(height);
    if long_edge == 0 || long_edge >= UPSCALE_TARGET {
        return None;
    }
    let wanted_factor = f64::from(UPSCALE_TARGET) / f64::from(long_edge);
    let factor = wanted_factor.min(f64::from(UPSCALE_MAX_FACTOR));
    let new_long_edge = ((f64::from(long_edge) * factor).round() as u32).min(UPSCALE_MAX_EDGE);
    if new_long_edge <= long_edge {
        return None;
    }
    let new_w = (u64::from(width) * u64::from(new_long_edge) / u64::from(long_edge)) as u32;
    let new_h = (u64::from(height) * u64::from(new_long_edge) / u64::from(long_edge)) as u32;
    Some((new_w.max(1), new_h.max(1)))
}

impl Source {
    pub fn read(self) -> ActResult<Vec<u8>> {
        match (self.data, self.path) {
            (Some(_), Some(_)) => Err(ActError::invalid_args(
                "provide either `data` or `path`, not both",
            )),
            (None, None) => Err(ActError::invalid_args(
                "provide the image as `data` (bytes) or `path` (a file on the host)",
            )),
            (Some(data), None) => Ok(data.into()),
            (None, Some(path)) => std::fs::read(&path).map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => ActError::not_found(format!("File not found: {path}")),
                io::ErrorKind::PermissionDenied => ActError::capability_denied(format!(
                    "Permission denied: {path} — grant wasi:filesystem read access covering this path"
                )),
                _ => ActError::internal(format!("Cannot read {path}: {e}")),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_both_sources() {
        let s = Source {
            data: Some(Bytes(vec![1])),
            path: Some("/tmp/x.png".into()),
            crop: None,
        };
        let err = s.read().unwrap_err();
        assert!(format!("{err:?}").contains("not both"));
    }

    #[test]
    fn rejects_neither_source() {
        let s = Source {
            data: None,
            path: None,
            crop: None,
        };
        assert!(s.read().is_err());
    }

    #[test]
    fn returns_inline_data() {
        let s = Source {
            data: Some(Bytes(vec![1, 2, 3])),
            path: None,
            crop: None,
        };
        assert_eq!(s.read().unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn crop_clamps_overshoot_rather_than_rejecting() {
        // A box that overshoots the image by a few pixels on two edges --
        // an agent's visual estimate routinely does this. Clamped, not
        // rejected.
        let (x, y, w, h) = resolve_crop([[5, 5], [120, 120]], 100, 100).unwrap();
        assert_eq!((x, y, w, h), (5, 5, 95, 95));
    }

    #[test]
    fn crop_rejects_inverted_box() {
        assert!(resolve_crop([[50, 50], [10, 10]], 100, 100).is_err());
    }

    #[test]
    fn crop_rejects_box_entirely_outside_image() {
        assert!(resolve_crop([[200, 200], [300, 300]], 100, 100).is_err());
    }

    #[test]
    fn crop_rejects_degenerate_box() {
        // Zero width: x1 == x2 is already caught by the inverted-box check
        // (x2 <= x1), so this exercises the same guard from the other side.
        assert!(resolve_crop([[10, 10], [10, 50]], 100, 100).is_err());
    }

    #[test]
    fn crop_rejects_negative_coordinates_out_of_bounds() {
        // A box that starts before the image and never enters it.
        assert!(resolve_crop([[-50, -50], [-1, -1]], 100, 100).is_err());
    }

    #[test]
    fn upscale_target_leaves_large_regions_alone() {
        assert_eq!(upscale_target(1200, 800), None);
    }

    #[test]
    fn upscale_target_grows_a_small_region_towards_1024() {
        let (w, h) = upscale_target(557, 486).unwrap();
        assert_eq!(w.max(h), 1024);
        // Aspect ratio preserved (within integer rounding).
        assert!((w as f64 / h as f64 - 557.0 / 486.0).abs() < 0.01);
    }

    #[test]
    fn upscale_target_caps_the_factor_at_4x() {
        // A 10x10 region would need 102x to reach 1024 -- capped at 4x
        // (40px) instead, so a tiny crop cannot become a memory bomb.
        let (w, h) = upscale_target(10, 10).unwrap();
        assert_eq!((w, h), (40, 40));
    }

    #[test]
    fn upscale_target_never_exceeds_the_absolute_cap() {
        // Given the "only upscale below 1024" guard and the 4x factor cap,
        // the largest possible result is 1023*4 = 4092 -- always under the
        // 4096 absolute cap, but the cap is checked independently as a
        // second line of defence rather than relying on that arithmetic
        // holding forever.
        for edge in [1, 100, 557, 1023] {
            let (w, h) = upscale_target(edge, edge).unwrap();
            assert!(w.max(h) <= 4096, "edge {edge} produced {w}x{h}");
        }
    }
}
