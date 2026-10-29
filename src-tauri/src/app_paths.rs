//! Shared user-data location for the desktop process and isolated workers.
use std::{
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

const MIGRATION_MARKER: &str = ".legacy-imported";

/// Resolve once per process so the desktop and its workers use the same layout.
/// Keep the original files as a backup and retry an incomplete import next launch.
pub fn config_dir() -> Option<PathBuf> {
    static CONFIG_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    CONFIG_DIR
        .get_or_init(|| {
            let current = default_config_dir()?;
            if let Some(legacy) = directories::ProjectDirs::from("com", "OSCMIDI", "OSCMIDI") {
                let legacy = legacy.config_dir();
                if let Err(error) = migrate_legacy_config(legacy, &current) {
                    log::warn!(
                        "Unable to migrate user data to {}: {error}",
                        current.display()
                    );
                    // Never replace inaccessible saved settings with fresh defaults.
                    if !current.join("config.yaml").exists() && legacy.join("config.yaml").exists()
                    {
                        return Some(legacy.to_path_buf());
                    }
                }
            }
            Some(current)
        })
        .clone()
}

fn default_config_dir() -> Option<PathBuf> {
    // On Windows, ProjectDirs includes both organization and application.
    directories::ProjectDirs::from("com", "", "OSCMIDI").map(|dirs| dirs.config_dir().to_path_buf())
}

fn migrate_legacy_config(legacy: &Path, current: &Path) -> io::Result<()> {
    if legacy == current || current.join(MIGRATION_MARKER).try_exists()? || !legacy.try_exists()? {
        return Ok(());
    }
    // Copy configuration last: its presence means the supporting state was imported.
    for name in [
        "app.log",
        "panic.log",
        "vst_cache.json",
        "vst_state",
        "logs",
        "config.yaml",
    ] {
        copy_missing(&legacy.join(name), &current.join(name))?;
    }
    fs::create_dir_all(current)?;
    // Remember completion even if a user later deletes config.yaml to reset settings.
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(current.join(MIGRATION_MARKER))
    {
        Ok(file) => file.sync_all(),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

fn copy_missing(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other(
            "user-data migration cannot follow symbolic links",
        ));
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other(
                "user-data migration cannot follow reparse points",
            ));
        }
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_missing(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.is_file() && !destination.try_exists()? {
        copy_file_without_replacement(source, destination)?;
    }
    Ok(())
}

fn copy_file_without_replacement(source: &Path, destination: &Path) -> io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("missing parent directory"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".migration-{}.tmp", uuid::Uuid::new_v4()));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        io::copy(&mut fs::File::open(source)?, &mut output)?;
        output.sync_all()
    })();
    drop(output);
    let result = result.and_then(|()| publish_without_replacement(&temporary, destination));
    let _ = fs::remove_file(&temporary);
    result
}

fn publish_without_replacement(temporary: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    let result = {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
        let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // Omit REPLACE_EXISTING: another process may have created the file meanwhile.
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(io::Error::other)
    };
    #[cfg(not(target_os = "windows"))]
    let result = fs::hard_link(temporary, destination);
    if result.is_err() && destination.is_file() {
        return Ok(());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(target_os = "windows")]
    #[test]
    fn default_directory_contains_only_one_oscmidi_component() {
        let roaming = directories::BaseDirs::new()
            .unwrap()
            .config_dir()
            .to_path_buf();
        assert_eq!(
            default_config_dir().unwrap(),
            roaming.join("OSCMIDI/config")
        );
    }

    #[test]
    fn migration_preserves_configuration_logs_cache_and_plugin_state() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("OSCMIDI/OSCMIDI/config");
        let current = root.path().join("OSCMIDI/config");
        fs::create_dir_all(legacy.join("vst_state")).unwrap();
        fs::write(legacy.join("config.yaml"), b"version: 4\naudio: saved").unwrap();
        fs::write(legacy.join("app.log"), b"previous log").unwrap();
        fs::write(legacy.join("vst_cache.json"), b"[]").unwrap();
        fs::write(legacy.join("vst_state/instrument.state"), b"preset").unwrap();
        migrate_legacy_config(&legacy, &current).unwrap();
        for name in [
            "config.yaml",
            "app.log",
            "vst_cache.json",
            "vst_state/instrument.state",
        ] {
            assert_eq!(
                fs::read(current.join(name)).unwrap(),
                fs::read(legacy.join(name)).unwrap()
            );
        }
    }

    #[test]
    fn migration_keeps_newer_destination_files_and_fills_missing_state() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("legacy");
        let current = root.path().join("current");
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&current).unwrap();
        fs::write(legacy.join("config.yaml"), b"old settings").unwrap();
        fs::write(current.join("config.yaml"), b"new settings").unwrap();
        fs::write(legacy.join("app.log"), b"old log").unwrap();
        migrate_legacy_config(&legacy, &current).unwrap();
        assert_eq!(
            fs::read(current.join("config.yaml")).unwrap(),
            b"new settings"
        );
        assert_eq!(fs::read(current.join("app.log")).unwrap(), b"old log");
    }

    #[test]
    fn completed_migration_does_not_restore_a_config_deleted_for_reset() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("legacy");
        let current = root.path().join("current");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("config.yaml"), b"old settings").unwrap();
        migrate_legacy_config(&legacy, &current).unwrap();
        fs::remove_file(current.join("config.yaml")).unwrap();
        migrate_legacy_config(&legacy, &current).unwrap();
        assert!(!current.join("config.yaml").exists());
        assert!(legacy.join("config.yaml").exists());
    }

    #[test]
    fn absent_legacy_directory_does_not_create_user_data() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("current");
        migrate_legacy_config(&root.path().join("absent"), &current).unwrap();
        assert!(!current.exists());
    }

    #[test]
    fn publishing_a_copy_never_replaces_a_file_created_by_another_process() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::write(&source, b"old settings").unwrap();
        fs::write(&destination, b"new settings").unwrap();
        copy_file_without_replacement(&source, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new settings");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn interrupted_import_keeps_originals_and_can_be_retried() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("legacy");
        let current = root.path().join("current");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("config.yaml"), b"saved settings").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(legacy.join("config.yaml"))
            .unwrap();
        assert!(migrate_legacy_config(&legacy, &current).is_err());
        assert!(!current.join("config.yaml").exists());
        assert!(!current.join(MIGRATION_MARKER).exists());
        assert_eq!(fs::read_dir(&current).unwrap().count(), 0);
        drop(held);
        migrate_legacy_config(&legacy, &current).unwrap();
        assert_eq!(
            fs::read(current.join("config.yaml")).unwrap(),
            b"saved settings"
        );
        assert_eq!(
            fs::read(legacy.join("config.yaml")).unwrap(),
            b"saved settings"
        );
    }
}
