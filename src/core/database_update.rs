// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub enum DatabaseFormat {
    Cameras,
    Profiles,
}

/// Keep the previous database intact until the complete replacement is validated.
pub fn install(mut source: impl Read, destination: &Path, format: DatabaseFormat) -> io::Result<()> {
    let parent = destination.parent().ok_or_else(|| io::Error::other("Missing database directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    io::copy(&mut source, &mut temporary)?;
    temporary.flush()?;
    temporary.seek(SeekFrom::Start(0))?;
    match format {
        DatabaseFormat::Cameras => {
            let mut data = String::new();
            temporary.read_to_string(&mut data)?;
            crate::camera_database::CameraDatabase::parse_file(&data)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        }
        DatabaseFormat::Profiles => {
            // Read to EOF before parsing, so gzip footer/truncation errors are checked too.
            let mut data = Vec::new();
            flate2::read::GzDecoder::new(temporary.as_file_mut()).read_to_end(&mut data)?;
            let profiles: Vec<(String, serde_json::Value)> = ciborium::from_reader(data.as_slice())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if !profiles.iter().any(|(name, value)| name == "__version" && value.as_u64().is_some()) ||
               !profiles.iter().any(|(name, value)| !name.starts_with("__") && value.is_object()) {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Empty or unversioned profile database"));
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

    #[test]
    fn invalid_updates_preserve_existing_database() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("database");
        std::fs::write(&path, b"previous database").unwrap();
        for (bytes, format) in [(b"{}".as_slice(), DatabaseFormat::Cameras), (b"broken gzip".as_slice(), DatabaseFormat::Profiles)] {
            assert!(install(bytes, &path, format).is_err());
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
        assert!(install(Interrupted, &path, DatabaseFormat::Cameras).is_err());
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
        install(compressed.as_slice(), &path, DatabaseFormat::Profiles).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), compressed);
        assert!(install(&compressed[..compressed.len() - 4], &path, DatabaseFormat::Profiles).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), compressed);
    }
}
