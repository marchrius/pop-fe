//! Versioned, verified database bundles stored outside the application binary.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DATABASE_FORMAT_VERSION: u32 = 1;
pub const DATABASE_SCHEMA_VERSION: u32 = 1;
pub const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_DATABASE_FILE_BYTES: u64 = 16 * 1024 * 1024;

pub const DATABASE_FILES: [&str; 10] = [
    "disc_by_md5.json",
    "games.json",
    "libcrypt.json",
    "libcrypt_hash_patches.json",
    "libcrypt_patches.json",
    "libcrypt_revision_signatures.json",
    "ppf_data.json",
    "ppf_fixes.json",
    "psp_configs.json",
    "themes.json",
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FileDigest {
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ArchiveDigest {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

/// Sidecar manifest published next to each database-only release archive.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DatabaseReleaseManifest {
    pub format_version: u32,
    pub schema_version: u32,
    pub revision: String,
    pub sequence: u64,
    pub archive: ArchiveDigest,
    pub files: BTreeMap<String, FileDigest>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "revision", rename_all = "snake_case")]
enum DatabaseVersion {
    Bundled,
    Release(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DatabaseState {
    active: DatabaseVersion,
    previous: Option<DatabaseVersion>,
}

impl DatabaseReleaseManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.format_version != DATABASE_FORMAT_VERSION {
            return Err(format!(
                "unsupported database bundle format {}",
                self.format_version
            ));
        }
        if self.schema_version != DATABASE_SCHEMA_VERSION {
            return Err(format!(
                "database schema {} is incompatible with this application",
                self.schema_version
            ));
        }
        if self.sequence == 0 {
            return Err("database release sequence must be positive".into());
        }
        if self.revision.len() != 40 || !self.revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("database revision must be a full hexadecimal Git commit ID".into());
        }
        let expected_archive = format!("pop-fe-database-{}.zip", &self.revision[..12]);
        if self.archive.name != expected_archive {
            return Err("database archive name does not match its revision".into());
        }
        if self.archive.size == 0 || self.archive.size > MAX_ARCHIVE_BYTES {
            return Err("database archive size is outside the accepted limit".into());
        }
        validate_digest(&self.archive.sha256)?;
        if self.files.len() != DATABASE_FILES.len() {
            return Err("database manifest has an unexpected set of files".into());
        }
        for name in DATABASE_FILES {
            let entry = self
                .files
                .get(name)
                .ok_or_else(|| format!("database manifest is missing {name}"))?;
            if entry.size == 0 || entry.size > MAX_DATABASE_FILE_BYTES {
                return Err(format!("database file size is invalid for {name}"));
            }
            validate_digest(&entry.sha256)?;
        }
        if self
            .files
            .keys()
            .any(|name| !DATABASE_FILES.contains(&name.as_str()))
        {
            return Err("database manifest contains an unsupported file".into());
        }
        Ok(())
    }
}

pub(crate) fn embedded_files() -> HashMap<String, String> {
    [
        ("disc_by_md5.json", include_str!("../data/disc_by_md5.json")),
        ("games.json", include_str!("../data/games.json")),
        ("libcrypt.json", include_str!("../data/libcrypt.json")),
        (
            "libcrypt_hash_patches.json",
            include_str!("../data/libcrypt_hash_patches.json"),
        ),
        (
            "libcrypt_patches.json",
            include_str!("../data/libcrypt_patches.json"),
        ),
        (
            "libcrypt_revision_signatures.json",
            include_str!("../data/libcrypt_revision_signatures.json"),
        ),
        ("ppf_data.json", include_str!("../data/ppf_data.json")),
        ("ppf_fixes.json", include_str!("../data/ppf_fixes.json")),
        ("psp_configs.json", include_str!("../data/psp_configs.json")),
        ("themes.json", include_str!("../data/themes.json")),
    ]
    .into_iter()
    .map(|(name, contents)| (name.to_owned(), contents.to_owned()))
    .collect()
}

pub fn validate_database_files(files: &HashMap<String, String>) -> Result<(), String> {
    if files.len() != DATABASE_FILES.len()
        || DATABASE_FILES.iter().any(|name| !files.contains_key(*name))
    {
        return Err("database bundle does not contain the complete supported file set".into());
    }
    if files
        .keys()
        .any(|name| !DATABASE_FILES.contains(&name.as_str()))
    {
        return Err("database bundle contains an unsupported file".into());
    }
    crate::database::Database::validate_json_files(files)
}

pub fn digest_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn bundled_digests() -> BTreeMap<String, FileDigest> {
    embedded_files()
        .into_iter()
        .map(|(name, contents)| {
            let bytes = contents.as_bytes();
            (
                name,
                FileDigest {
                    sha256: digest_bytes(bytes),
                    size: bytes.len() as u64,
                },
            )
        })
        .collect()
}

pub fn is_current(manifest: &DatabaseReleaseManifest) -> bool {
    if manifest.files == bundled_digests() {
        return true;
    }
    let Some((installed, _)) = load_active_version() else {
        return false;
    };
    manifest.files == installed.files || installed.sequence >= manifest.sequence
}

pub fn load_active_files() -> Option<HashMap<String, String>> {
    let state = read_state().ok().flatten()?;
    match read_source(&state.active) {
        Ok(files) => files,
        Err(_) => state
            .previous
            .as_ref()
            .and_then(|previous| read_source(previous).ok().flatten()),
    }
}

pub fn install_files(
    manifest: &DatabaseReleaseManifest,
    files: HashMap<String, Vec<u8>>,
) -> Result<String, String> {
    manifest.validate()?;
    if files.len() != DATABASE_FILES.len() {
        return Err("database archive does not contain the complete file set".into());
    }
    let mut text_files = HashMap::with_capacity(files.len());
    for name in DATABASE_FILES {
        let contents = files
            .get(name)
            .ok_or_else(|| format!("database archive is missing {name}"))?;
        let expected = manifest
            .files
            .get(name)
            .ok_or_else(|| format!("database manifest is missing {name}"))?;
        if contents.len() as u64 != expected.size || digest_bytes(contents) != expected.sha256 {
            return Err(format!("database checksum or size mismatch for {name}"));
        }
        let text = String::from_utf8(contents.clone())
            .map_err(|_| format!("database file is not valid UTF-8: {name}"))?;
        text_files.insert(name.to_owned(), text);
    }
    validate_database_files(&text_files)?;

    let versions = store_root().join("versions");
    fs::create_dir_all(&versions)
        .map_err(|error| format!("cannot create database store: {error}"))?;
    let destination = version_path(&manifest.revision);
    if destination.exists() {
        let (existing, _) = read_version(&manifest.revision)
            .map_err(|error| format!("database version already exists but is invalid: {error}"))?;
        if existing != *manifest {
            return Err("database revision already exists with different contents".into());
        }
    } else {
        let staging = staging_path(&versions);
        fs::create_dir(&staging)
            .map_err(|error| format!("cannot create database staging directory: {error}"))?;
        let staged = (|| {
            for name in DATABASE_FILES {
                write_new_file(
                    &staging.join(name),
                    files.get(name).expect("validated file set"),
                )?;
            }
            let manifest_bytes = serde_json::to_vec_pretty(manifest)
                .map_err(|error| io::Error::other(error.to_string()))?;
            write_new_file(&staging.join("manifest.json"), &manifest_bytes)?;
            sync_directory(&staging)?;
            fs::rename(&staging, &destination)?;
            sync_directory(&versions)
        })();
        if let Err(error) = staged {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("cannot install database files: {error}"));
        }
    }

    let old_state =
        read_state().map_err(|error| format!("cannot read current database state: {error}"))?;
    if old_state
        .as_ref()
        .is_some_and(|state| state.active == DatabaseVersion::Release(manifest.revision.clone()))
    {
        return Ok(manifest.revision.clone());
    }
    let state = DatabaseState {
        active: DatabaseVersion::Release(manifest.revision.clone()),
        previous: Some(
            old_state
                .map(|state| state.active)
                .unwrap_or(DatabaseVersion::Bundled),
        ),
    };
    write_state(&state).map_err(|error| format!("cannot activate database update: {error}"))?;
    Ok(manifest.revision.clone())
}

pub fn rollback() -> Result<String, String> {
    let mut state = read_state()
        .map_err(|error| format!("cannot read database state: {error}"))?
        .ok_or_else(|| "there is no installed database update to roll back".to_owned())?;
    let previous = state
        .previous
        .clone()
        .ok_or_else(|| "there is no previous database version to restore".to_owned())?;
    if let DatabaseVersion::Release(revision) = &previous {
        read_version(revision)
            .map_err(|error| format!("previous database version is not usable: {error}"))?;
    }
    let active = std::mem::replace(&mut state.active, previous.clone());
    state.previous = Some(active);
    write_state(&state).map_err(|error| format!("cannot restore previous database: {error}"))?;
    Ok(match previous {
        DatabaseVersion::Bundled => "bundled".to_owned(),
        DatabaseVersion::Release(revision) => revision,
    })
}

pub fn has_rollback() -> bool {
    read_state()
        .ok()
        .flatten()
        .is_some_and(|state| state.previous.is_some())
}

pub fn active_sequence() -> Option<u64> {
    load_active_version().map(|(manifest, _)| manifest.sequence)
}

fn load_active_version() -> Option<(DatabaseReleaseManifest, HashMap<String, String>)> {
    let state = read_state().ok().flatten()?;
    match read_source_version(&state.active) {
        Ok(Some(version)) => Some(version),
        Ok(None) => None,
        Err(_) => state
            .previous
            .as_ref()
            .and_then(|previous| read_source_version(previous).ok().flatten()),
    }
}

fn read_source(source: &DatabaseVersion) -> Result<Option<HashMap<String, String>>, String> {
    match source {
        DatabaseVersion::Bundled => Ok(None),
        DatabaseVersion::Release(revision) => read_version(revision).map(|(_, files)| Some(files)),
    }
}

fn read_source_version(
    source: &DatabaseVersion,
) -> Result<Option<(DatabaseReleaseManifest, HashMap<String, String>)>, String> {
    match source {
        DatabaseVersion::Bundled => Ok(None),
        DatabaseVersion::Release(revision) => read_version(revision).map(Some),
    }
}

fn read_version(
    version: &str,
) -> Result<(DatabaseReleaseManifest, HashMap<String, String>), String> {
    if version.len() != 40 || !version.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid local database version identifier".into());
    }
    let directory = version_path(version);
    let manifest_bytes = read_limited(&directory.join("manifest.json"), 64 * 1024)
        .map_err(|error| format!("cannot read database manifest: {error}"))?;
    let manifest: DatabaseReleaseManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid database manifest: {error}"))?;
    manifest.validate()?;
    if manifest.revision != version {
        return Err("local database directory does not match its manifest".into());
    }
    let mut files = HashMap::with_capacity(DATABASE_FILES.len());
    for name in DATABASE_FILES {
        let expected = manifest.files.get(name).expect("validated manifest");
        let bytes = read_limited(&directory.join(name), MAX_DATABASE_FILE_BYTES)
            .map_err(|error| format!("cannot read database file {name}: {error}"))?;
        if bytes.len() as u64 != expected.size || digest_bytes(&bytes) != expected.sha256 {
            return Err(format!("local database checksum mismatch for {name}"));
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("local database file is not UTF-8: {name}"))?;
        files.insert(name.to_owned(), text);
    }
    validate_database_files(&files)?;
    Ok((manifest, files))
}

fn validate_digest(digest: &str) -> Result<(), String> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("SHA-256 digest must contain 64 hexadecimal characters".into());
    }
    Ok(())
}

fn store_root() -> PathBuf {
    crate::paths::config_dir().join("database-updates")
}

fn version_path(version: &str) -> PathBuf {
    store_root().join("versions").join(version)
}

fn state_path() -> PathBuf {
    store_root().join("state.json")
}

fn read_state() -> io::Result<Option<DatabaseState>> {
    let path = state_path();
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = read_limited(&path, 4096)?;
    let state = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(state))
}

fn write_state(state: &DatabaseState) -> io::Result<()> {
    let path = state_path();
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("database state path has no parent"))?;
    fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec(state)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    atomic_write(&path, &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("target path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    write_new_file(&temporary, bytes)?;
    if let Err(error) = crate::paths::replace_file(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    sync_directory(parent)
}

fn staging_path(versions: &Path) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    versions.join(format!(
        ".staging-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn write_new_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn read_limited(path: &Path, max_bytes: u64) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    if size > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds the accepted size limit",
        ));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds the accepted size limit",
        ));
    }
    Ok(bytes)
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
