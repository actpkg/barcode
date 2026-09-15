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
        };
        let err = s.read().unwrap_err();
        assert!(format!("{err:?}").contains("not both"));
    }

    #[test]
    fn rejects_neither_source() {
        let s = Source {
            data: None,
            path: None,
        };
        assert!(s.read().is_err());
    }

    #[test]
    fn returns_inline_data() {
        let s = Source {
            data: Some(Bytes(vec![1, 2, 3])),
            path: None,
        };
        assert_eq!(s.read().unwrap(), vec![1, 2, 3]);
    }
}
