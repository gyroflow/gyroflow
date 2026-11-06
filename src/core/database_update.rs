// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use sha2::{Digest, Sha256};

pub enum DatabaseFormat {
    Cameras,
    Profiles(u32),
}

fn parse_sha256(value: &str) -> io::Result<&str> {
    let hex = value.strip_prefix("sha256:").ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Unsupported database digest"))?;
    let bytes = hex.as_bytes();
    if bytes.len() != 64 || bytes.iter().any(|byte| !byte.is_ascii_hexdigit()) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Expected a 64-digit hexadecimal SHA-256 digest"));
    }
    Ok(hex)
}

/// Compare bytes with a GitHub release asset digest (`sha256:<hex>`).
fn matches_sha256(data: &[u8], expected: &str) -> io::Result<bool> {
    let expected = parse_sha256(expected)?;
    let actual = format!("{:x}", Sha256::digest(data));
    Ok(actual.eq_ignore_ascii_case(expected))
}

/// Decide whether the cached camera catalog needs replacement for the release.
pub fn camera_catalog_needs_update(
    current_version: u32,
    release_version: u32,
    cached_catalog: Option<&[u8]>,
    expected_sha256: Option<&str>,
) -> io::Result<bool> {
    if release_version < current_version {
        return Ok(false);
    }
    if release_version > current_version {
        return Ok(true);
    }
    let Some(bytes) = cached_catalog else { return Ok(true); };
    let valid = std::str::from_utf8(bytes).ok()
        .and_then(|data| crate::camera_database::CameraDatabase::parse_file(data).ok()).is_some();
    if !valid {
        return Ok(true);
    }
    if let Some(digest) = expected_sha256 {
        return Ok(!matches_sha256(bytes, digest)?);
    }
    Ok(false)
}

// The current assets are a few MiB; cap both the download and gzip expansion before parsing.
const MAX_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PROFILE_BYTES: u64 = 128 * 1024 * 1024;

fn copy_limited(reader: impl Read, writer: &mut impl Write, limit: u64) -> io::Result<()> {
    if io::copy(&mut reader.take(limit + 1), writer)? > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Database exceeds its size limit"));
    }
    Ok(())
}

/// Replace the previous database only after verifying the format and any supplied release digest.
pub fn install(
    source: impl Read,
    destination: &Path,
    format: DatabaseFormat,
    expected_sha256: Option<&str>,
) -> io::Result<()> {
    let expected_sha256 = expected_sha256.map(parse_sha256).transpose()?;
    let parent = destination.parent().ok_or_else(|| io::Error::other("Missing database directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    copy_limited(source, &mut temporary, MAX_DOWNLOAD_BYTES)?;
    temporary.flush()?;
    if let Some(expected) = expected_sha256 {
        temporary.seek(SeekFrom::Start(0))?;
        let mut hasher = Sha256::new();
        io::copy(&mut temporary, &mut hasher)?;
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Database SHA-256 digest mismatch"));
        }
    }
    temporary.seek(SeekFrom::Start(0))?;
    match format {
        DatabaseFormat::Cameras => {
            let mut data = String::new();
            temporary.read_to_string(&mut data)?;
            crate::camera_database::CameraDatabase::parse_file(&data)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        }
        DatabaseFormat::Profiles(version) => {
            // Read to EOF before parsing, so gzip footer/truncation errors are checked too.
            let mut data = Vec::new();
            copy_limited(flate2::read::MultiGzDecoder::new(temporary.as_file_mut()), &mut data, MAX_PROFILE_BYTES)?;
            let mut bytes = data.as_slice();
            let profiles: Vec<(String, serde_json::Value)> = ciborium::from_reader(&mut bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let versions = profiles.iter().filter(|(name, _)| name == "__version").map(|(_, value)| value.as_u64());
            if !bytes.is_empty() || !versions.eq([Some(u64::from(version))]) ||
               !profiles.iter().any(|(name, value)| !name.starts_with("__") && value.is_object()) {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid profile database or release version"));
            }
        }
    }
    temporary.as_file().sync_all()?;
    temporary.persist(destination).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    const CATALOG_OLD: &[u8] = br#"{"version":1,"cameras":[{"brand":"Example","model":"Body"}],"lenses":[{"brand":"Example","model":"Lens"}]}"#;
    const CATALOG_NEW: &[u8] = br#"{"version":1,"cameras":[{"brand":"Example","model":"Body II"}],"lenses":[{"brand":"Example","model":"Lens"}]}"#;

    #[test]
    fn equal_version_catalog_with_stale_digest_needs_update() {
        assert!(!camera_catalog_needs_update(42, 42, Some(CATALOG_OLD), Some(&digest(CATALOG_OLD))).unwrap());
        assert!(camera_catalog_needs_update(42, 42, Some(CATALOG_OLD), Some(&digest(CATALOG_NEW))).unwrap());
        assert!(!camera_catalog_needs_update(42, 42, Some(CATALOG_OLD), None).unwrap());
        assert!(!camera_catalog_needs_update(42, 41, Some(CATALOG_OLD), Some(&digest(CATALOG_NEW))).unwrap());
    }

    #[test]
    fn digest_validation_handles_non_ascii_manifest_text_without_panicking() {
        assert!(matches_sha256(b"catalog", &format!("sha256:{}", "é".repeat(32))).is_err());
    }

    #[test]
    fn digest_mismatch_preserves_existing_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("camera_database.json");
        std::fs::write(&path, CATALOG_OLD).unwrap();

        assert!(install(CATALOG_NEW, &path, DatabaseFormat::Cameras, Some(&digest(CATALOG_OLD))).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), CATALOG_OLD);

        install(CATALOG_NEW, &path, DatabaseFormat::Cameras, Some(&digest(CATALOG_NEW))).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), CATALOG_NEW);
    }

    #[test]
    fn stops_reading_when_the_size_limit_is_exceeded() {
        let mut output = Vec::new();
        assert!(copy_limited(io::repeat(0), &mut output, 16).is_err());
        assert_eq!(output.len(), 17);
        output.clear();
        copy_limited(&[0; 16][..], &mut output, 16).unwrap();
    }

    #[test]
    fn invalid_updates_preserve_existing_database() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("database");
        std::fs::write(&path, b"previous database").unwrap();
        for (bytes, format) in [(b"{}".as_slice(), DatabaseFormat::Cameras), (b"broken gzip".as_slice(), DatabaseFormat::Profiles(42))] {
            assert!(install(bytes, &path, format, None).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"previous database");
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn interrupted_download_preserves_existing_database() {
        struct Interrupted;
        impl Read for Interrupted {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::ConnectionReset, "interrupted"))
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("database");
        std::fs::write(&path, b"previous database").unwrap();
        assert!(install(Interrupted, &path, DatabaseFormat::Cameras, None).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"previous database");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn validates_complete_profile_bundle_before_replacing_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("database");
        let mut cbor = Vec::new();
        ciborium::into_writer(&vec![
            ("__version", serde_json::json!(42)),
            ("example.json", serde_json::json!({"camera_brand": "Example"})),
        ], &mut cbor).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&cbor).unwrap();
        let compressed = encoder.finish().unwrap();
        install(compressed.as_slice(), &path, DatabaseFormat::Profiles(42), None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), compressed);
        assert!(install(compressed.as_slice(), &path, DatabaseFormat::Profiles(43), None).is_err());
        assert!(install(&compressed[..compressed.len() - 4], &path, DatabaseFormat::Profiles(42), None).is_err());
        let mut trailing = compressed.clone();
        trailing.extend_from_slice(b"trailing data");
        assert!(install(trailing.as_slice(), &path, DatabaseFormat::Profiles(42), None).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), compressed);
    }
}
