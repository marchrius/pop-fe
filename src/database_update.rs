//! Download and validate database-only GitHub releases.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;

use crate::database_store::{self, DATABASE_FILES, DatabaseReleaseManifest, MAX_ARCHIVE_BYTES};

const RELEASES_URL: &str = "https://api.github.com/repos/marchrius/pop-fe/releases?per_page=100";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// Checks for and installs the newest compatible database release.
/// Returns `Ok(None)` when the local database already matches or is newer.
pub fn check_and_install(cancelled: &AtomicBool) -> Result<Option<String>, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("POP-FE-PSP-Manager/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| format!("cannot initialize database updater: {error}"))?;
    check_cancelled(cancelled)?;
    let response = client
        .get(RELEASES_URL)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("cannot check database releases: {error}"))?;
    let releases: Vec<Release> = read_response(response, 4 * 1024 * 1024)?;
    check_cancelled(cancelled)?;
    let release = releases
        .into_iter()
        .find(|release| !release.draft && release.tag_name.starts_with("database-"))
        .ok_or_else(|| "no database release is available".to_owned())?;
    let manifest_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == "database-manifest.json")
        .ok_or_else(|| "database release has no manifest asset".to_owned())?;
    if manifest_asset.size > 64 * 1024 {
        return Err("database release manifest exceeds the size limit".into());
    }
    let manifest: DatabaseReleaseManifest = download_json(&client, manifest_asset, 64 * 1024)?;
    manifest.validate()?;
    let revision = release
        .tag_name
        .strip_prefix("database-")
        .ok_or_else(|| "invalid database release tag".to_owned())?;
    if !revision.eq_ignore_ascii_case(&manifest.revision) {
        return Err("database release tag does not match its manifest".into());
    }
    if database_store::is_current(&manifest) {
        return Ok(None);
    }
    if let Some(active_sequence) = database_store::active_sequence()
        && active_sequence >= manifest.sequence
    {
        return Ok(None);
    }
    check_cancelled(cancelled)?;
    let archive_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == manifest.archive.name)
        .ok_or_else(|| "database release archive is missing".to_owned())?;
    if archive_asset.size != manifest.archive.size || archive_asset.size > MAX_ARCHIVE_BYTES {
        return Err("database archive size does not match its manifest".into());
    }
    let archive_bytes = download_bytes(&client, archive_asset, MAX_ARCHIVE_BYTES)?;
    if database_store::digest_bytes(&archive_bytes) != manifest.archive.sha256 {
        return Err("database archive SHA-256 checksum does not match".into());
    }
    check_cancelled(cancelled)?;
    let files = extract_database_files(&archive_bytes, &manifest)?;
    check_cancelled(cancelled)?;
    let revision = database_store::install_files(&manifest, files)?;
    Ok(Some(revision))
}

fn extract_database_files(
    bytes: &[u8],
    manifest: &DatabaseReleaseManifest,
) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("invalid database ZIP archive: {error}"))?;
    if archive.len() != DATABASE_FILES.len() {
        return Err("database archive contains an unexpected number of files".into());
    }
    let mut names = HashSet::with_capacity(DATABASE_FILES.len());
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|error| format!("cannot inspect database ZIP archive: {error}"))?;
        if file.is_dir() || !DATABASE_FILES.contains(&file.name()) {
            return Err("database archive contains an unsupported path".into());
        }
        if !names.insert(file.name().to_owned()) {
            return Err("database archive contains duplicate files".into());
        }
    }
    if names.len() != DATABASE_FILES.len() {
        return Err("database archive is missing required files".into());
    }
    let mut files = HashMap::with_capacity(DATABASE_FILES.len());
    for name in DATABASE_FILES {
        let expected = manifest
            .files
            .get(name)
            .expect("validated release manifest");
        let file = archive
            .by_name(name)
            .map_err(|error| format!("database archive is missing {name}: {error}"))?;
        if file.size() != expected.size {
            return Err(format!("database archive size mismatch for {name}"));
        }
        let mut contents = Vec::with_capacity(expected.size as usize);
        file.take(expected.size + 1)
            .read_to_end(&mut contents)
            .map_err(|error| format!("cannot read database archive file {name}: {error}"))?;
        if contents.len() as u64 != expected.size {
            return Err(format!("database archive size mismatch for {name}"));
        }
        files.insert(name.to_owned(), contents);
    }
    Ok(files)
}

fn download_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::blocking::Client,
    asset: &ReleaseAsset,
    max_size: u64,
) -> Result<T, String> {
    let response = client
        .get(&asset.browser_download_url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("cannot download database manifest: {error}"))?;
    read_response(response, max_size)
}

fn download_bytes(
    client: &reqwest::blocking::Client,
    asset: &ReleaseAsset,
    max_size: u64,
) -> Result<Vec<u8>, String> {
    let response = client
        .get(&asset.browser_download_url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("cannot download database archive: {error}"))?;
    read_response_bytes(response, max_size)
}

fn read_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::blocking::Response,
    max_size: u64,
) -> Result<T, String> {
    serde_json::from_slice(&read_response_bytes(response, max_size)?)
        .map_err(|error| format!("invalid database release response: {error}"))
}

fn read_response_bytes(
    response: reqwest::blocking::Response,
    max_size: u64,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > max_size)
    {
        return Err("database download exceeds the size limit".into());
    }
    let mut bytes = Vec::new();
    response
        .take(max_size + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read database download: {error}"))?;
    if bytes.len() as u64 > max_size {
        return Err("database download exceeds the size limit".into());
    }
    Ok(bytes)
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        Err("database update cancelled".into())
    } else {
        Ok(())
    }
}
