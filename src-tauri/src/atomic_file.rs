use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "target path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        sequence
    ));
    let mut temp = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)?;
    if let Err(error) = temp.write_all(data).and_then(|_| temp.sync_all()) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    drop(temp);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };

        let temp_wide: Vec<u16> = temp_path.as_os_str().encode_wide().chain(Some(0)).collect();
        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // MOVEFILE_WRITE_THROUGH keeps the replacement durable across sudden power loss.
        let result = unsafe {
            MoveFileExW(
                PCWSTR(temp_wide.as_ptr()),
                PCWSTR(path_wide.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if let Err(error) = result {
            let _ = fs::remove_file(&temp_path);
            return Err(std::io::Error::other(error));
        }
    }

    #[cfg(not(target_os = "windows"))]
    fs::rename(&temp_path, path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_atomically;

    #[test]
    fn replaces_existing_content_without_leaving_a_temp_file() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let target = directory.path().join("config.yaml");

        write_atomically(&target, b"version: 1").expect("initial write");
        write_atomically(&target, b"version: 2").expect("replacement write");

        assert_eq!(
            std::fs::read(&target).expect("target contents"),
            b"version: 2"
        );
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("directory entries")
                .count(),
            1
        );
    }
}
