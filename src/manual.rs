//! Automatic PSP DOCUMENT.DAT creation from database linked manuals.
use std::cmp::Ordering;
use std::ffi::CString;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::thread;
use std::time::Duration;

use gdk_pixbuf::{InterpType, Pixbuf};
use md5::{Digest, Md5};
use thiserror::Error;
use zip::ZipArchive;

const MAX_MANUAL_BYTES: usize = 256 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_MANUAL_PAGES: usize = 1000;
const MAX_ARCHIVE_ENTRIES: usize = 10000;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum ManualError {
    #[error("manual I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("manual operation cancelled")]
    Cancelled,
    #[error("manual processing failed: {0}")]
    Invalid(String),
}

pub fn create_psp_document(
    source: &str,
    game_id: &str,
    cancelled: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<Option<Vec<u8>>, ManualError> {
    if source.is_empty() {
        return Ok(None);
    }
    let temporary = TemporaryDirectory::create()?;
    if Path::new(source).is_dir() {
        let pages = directory_pages(Path::new(source), &temporary.0, cancelled, &progress)?;
        if pages.is_empty() {
            return Err(ManualError::Invalid(
                "no readable pages were found in the manual folder".into(),
            ));
        }
        progress(0.95);
        return create_document(game_id, &pages).map(Some);
    }
    let contents = read_source(source, cancelled, &progress)?;
    if contents.starts_with(b"DOC ") {
        return Ok(Some(contents));
    }
    if contents.starts_with(b"\0PGD") {
        let pages = decrypt_ps3_document(&contents, cancelled)?;
        if pages.is_empty() {
            return Err(ManualError::Invalid(
                "encrypted PS3 manual contains no readable PSP pages".into(),
            ));
        }
        progress(0.95);
        return create_document(game_id, &pages).map(Some);
    }
    let mut pages = if contents.starts_with(b"Rar!") {
        let extracted = extract_rar(&contents, &temporary.0, cancelled, &progress)?;
        directory_pages(&extracted, &temporary.0, cancelled, &|fraction| {
            progress(0.55 + fraction * 0.35);
        })?
    } else if contents.starts_with(b"%PDF") {
        pdf_pages(&contents, &temporary.0, cancelled, &progress)?
    } else {
        archive_pages(&contents, &temporary.0, cancelled, &progress)?
    };
    if pages.is_empty() {
        return Err(ManualError::Invalid(
            "no readable pages were found in the manual".into(),
        ));
    }
    if pages.len() > MAX_MANUAL_PAGES {
        pages.truncate(MAX_MANUAL_PAGES);
    }
    progress(0.95);
    create_document(game_id, &pages).map(Some)
}

fn decrypt_ps3_document(data: &[u8], cancelled: &AtomicBool) -> Result<Vec<Vec<u8>>, ManualError> {
    const PGD_HEADER: &[u8; 16] = b"\0PGD\x01\0\0\0\x01\0\0\0\0\0\0\0";
    if data.get(..16) != Some(PGD_HEADER) || data.len() < 0x90 {
        return Err(ManualError::Invalid("invalid PS3 PGD manual header".into()));
    }
    let header = checked_slice(data, 0x10, 0x60)?;
    verify_sha1(
        header,
        checked_slice(data, 0x80, 0x10)?,
        "PS3 document header",
    )?;
    let decoded_header = des_cbc_decrypt(header)?;
    if decoded_header.get(..4) != Some(b"DOC ")
        || decoded_header.get(4..12) != Some(b"\0\0\x01\0\0\0\x01\0")
    {
        return Err(ManualError::Invalid("invalid encrypted DOC header".into()));
    }
    let large = read_u32(&decoded_header, 0x1c)? != 0;
    let info_size = if large { 0x1f3e8 } else { 0x31e8 };
    let info_end = 0x90usize
        .checked_add(info_size)
        .ok_or_else(|| ManualError::Invalid("PS3 manual INFO block overflow".into()))?;
    let info = checked_slice(data, 0x90, info_size)?;
    let info_sha = checked_slice(data, info_end + 0x10, 0x10)?;
    verify_sha1(info, info_sha, "PS3 manual INFO block")?;
    let decoded_info = des_cbc_decrypt(info)?;
    if decoded_info.get(..4) != Some(&[0xff; 4]) {
        return Err(ManualError::Invalid(
            "invalid PS3 manual INFO marker".into(),
        ));
    }
    let page_count = read_u32(&decoded_info, 4)? as usize;
    let max_pages = ((info_size.saturating_sub(8)) / 0x80).min(MAX_MANUAL_PAGES);
    if page_count == 0 || page_count > max_pages {
        return Err(ManualError::Invalid(format!(
            "invalid PS3 manual page count: {page_count}"
        )));
    }

    let mut pages = Vec::with_capacity(page_count);
    for index in 0..page_count {
        check_cancelled(cancelled)?;
        let record = 8 + index * 0x80;
        let offset = read_u32(&decoded_info, record)? as usize;
        let length = read_u32(&decoded_info, record + 0x0c)? as usize;
        if length < 0x40 || length > MAX_PAGE_BYTES + 0x1000 {
            continue;
        }
        let encrypted_page = match data.get(offset..offset.saturating_add(length)) {
            Some(page) => page,
            None => continue,
        };
        if encrypted_page.len() < 0x40 || encrypted_page.len() % 8 != 0 {
            continue;
        }
        let mac_offset = encrypted_page.len() - 0x20;
        let encrypted_payload = &encrypted_page[..mac_offset];
        let page_sha = &encrypted_page[mac_offset + 0x10..];
        if !sha1_matches(encrypted_payload, page_sha) {
            continue;
        }
        let page_header = des_cbc_decrypt(checked_slice(encrypted_payload, 0, 0x20)?)?;
        if read_u32(&page_header, 0)? as usize != length {
            continue;
        }
        let chunks = read_u32(&page_header, 8)? as usize;
        let table_size = chunks
            .checked_mul(8)
            .and_then(|size| size.checked_add(0x20))
            .ok_or_else(|| ManualError::Invalid("PS3 page table overflow".into()))?;
        if chunks > 4096 || table_size > encrypted_payload.len() {
            continue;
        }
        let decoded_table = des_cbc_decrypt(&encrypted_payload[0x20..table_size])?;
        let mut page = encrypted_payload[table_size..].to_vec();
        let mut valid = true;
        for chunk in 0..chunks {
            check_cancelled(cancelled)?;
            let offset = read_u32(&decoded_table, chunk * 8)? as usize;
            let size = read_u32(&decoded_table, chunk * 8 + 4)? as usize;
            let end = match offset.checked_add(size) {
                Some(end) if size != 0 && size % 8 == 0 && end <= page.len() => end,
                _ => {
                    valid = false;
                    break;
                }
            };
            let decoded_chunk = des_cbc_decrypt(&page[offset..end])?;
            page[offset..end].copy_from_slice(&decoded_chunk);
        }
        if !valid {
            continue;
        }
        const PNG_END: &[u8; 8] = b"IEND\xaeB`\x82";
        let Some(end) = page
            .windows(PNG_END.len())
            .rposition(|window| window == PNG_END)
        else {
            continue;
        };
        let png_end = end + PNG_END.len();
        if png_end >= 0x43 && png_end <= MAX_PAGE_BYTES {
            page.truncate(png_end);
            pages.push(page);
        }
    }
    Ok(pages)
}

fn verify_sha1(data: &[u8], expected: &[u8], label: &str) -> Result<(), ManualError> {
    if sha1_matches(data, expected) {
        Ok(())
    } else {
        Err(ManualError::Invalid(format!("{label} SHA-1 mismatch")))
    }
}

fn sha1_matches(data: &[u8], expected: &[u8]) -> bool {
    use sha1::{Digest, Sha1};
    let digest = Sha1::digest(data);
    expected.len() == 16 && digest[..16] == *expected
}

fn checked_slice(data: &[u8], offset: usize, length: usize) -> Result<&[u8], ManualError> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| ManualError::Invalid("PS3 manual offset overflow".into()))?;
    data.get(offset..end)
        .ok_or_else(|| ManualError::Invalid("truncated PS3 manual".into()))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, ManualError> {
    let bytes = checked_slice(data, offset, 4)?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

#[repr(C)]
struct EvpCipherCtx {
    _private: [u8; 0],
}

#[repr(C)]
struct OsslProvider {
    _private: [u8; 0],
}

#[link(name = "crypto")]
unsafe extern "C" {
    fn OSSL_PROVIDER_load(
        context: *mut std::ffi::c_void,
        name: *const std::ffi::c_char,
    ) -> *mut OsslProvider;
    fn EVP_CIPHER_CTX_new() -> *mut EvpCipherCtx;
    fn EVP_CIPHER_CTX_free(context: *mut EvpCipherCtx);
    fn EVP_des_cbc() -> *const std::ffi::c_void;
    fn EVP_DecryptInit_ex(
        context: *mut EvpCipherCtx,
        cipher: *const std::ffi::c_void,
        engine: *mut std::ffi::c_void,
        key: *const u8,
        iv: *const u8,
    ) -> i32;
    fn EVP_CIPHER_CTX_set_padding(context: *mut EvpCipherCtx, padding: i32) -> i32;
    fn EVP_DecryptUpdate(
        context: *mut EvpCipherCtx,
        output: *mut u8,
        output_length: *mut i32,
        input: *const u8,
        input_length: i32,
    ) -> i32;
    fn EVP_DecryptFinal_ex(
        context: *mut EvpCipherCtx,
        output: *mut u8,
        output_length: *mut i32,
    ) -> i32;
}

fn des_cbc_decrypt(data: &[u8]) -> Result<Vec<u8>, ManualError> {
    const KEY: [u8; 8] = [0x39, 0xf7, 0xef, 0xa1, 0x6c, 0xce, 0x5f, 0x4c];
    const IV: [u8; 8] = [0xa8, 0x19, 0xc4, 0xf5, 0xe1, 0x54, 0xe3, 0x0b];
    // The original implementation accepts empty encrypted chunk tables when
    // a PS3 manual page has no separately encrypted payload chunks.
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if data.len() % 8 != 0 || data.len() > i32::MAX as usize {
        return Err(ManualError::Invalid(
            "invalid encrypted PS3 manual block".into(),
        ));
    }
    static LEGACY_PROVIDER: OnceLock<bool> = OnceLock::new();
    let provider_loaded = *LEGACY_PROVIDER.get_or_init(|| {
        let name = CString::new("legacy").unwrap();
        // OpenSSL keeps the provider loaded for the process lifetime.
        unsafe { !OSSL_PROVIDER_load(std::ptr::null_mut(), name.as_ptr()).is_null() }
    });
    if !provider_loaded {
        return Err(ManualError::Invalid(
            "OpenSSL DES legacy provider is unavailable".into(),
        ));
    }
    let context = unsafe { EVP_CIPHER_CTX_new() };
    if context.is_null() {
        return Err(ManualError::Invalid(
            "could not allocate DES context".into(),
        ));
    }
    let mut output = vec![0u8; data.len() + 8];
    let mut first_length = 0;
    let mut final_length = 0;
    let result = unsafe {
        let initialized = EVP_DecryptInit_ex(
            context,
            EVP_des_cbc(),
            std::ptr::null_mut(),
            KEY.as_ptr(),
            IV.as_ptr(),
        ) == 1
            && EVP_CIPHER_CTX_set_padding(context, 0) == 1
            && EVP_DecryptUpdate(
                context,
                output.as_mut_ptr(),
                &mut first_length,
                data.as_ptr(),
                data.len() as i32,
            ) == 1
            && EVP_DecryptFinal_ex(
                context,
                output.as_mut_ptr().add(first_length as usize),
                &mut final_length,
            ) == 1;
        EVP_CIPHER_CTX_free(context);
        initialized
    };
    if !result {
        return Err(ManualError::Invalid("DES decryption failed".into()));
    }
    output.truncate((first_length + final_length) as usize);
    Ok(output)
}

fn extract_rar(
    contents: &[u8],
    temporary: &Path,
    cancelled: &AtomicBool,
    progress: &dyn Fn(f64),
) -> Result<PathBuf, ManualError> {
    let archive = temporary.join("manual.rar");
    let destination = temporary.join("rar-pages");
    fs::write(&archive, contents)?;
    fs::create_dir(&destination)?;
    let output_option = format!("-o{}", destination.display());
    let mut command = Command::new("7z");
    command
        .args(["x", "-y", "-bd", "-bso0", "-bsp0"])
        .arg(output_option)
        .arg(&archive)
        .current_dir(temporary)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::process::isolate(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| ManualError::Invalid(format!("7-Zip is unavailable: {error}")))?;
    loop {
        if cancelled.load(AtomicOrdering::Relaxed) {
            crate::process::terminate(&mut child);
            let _ = child.wait();
            return Err(ManualError::Cancelled);
        }
        let (size, entries) = match directory_usage(&destination) {
            Ok(usage) => usage,
            Err(error) => {
                crate::process::terminate(&mut child);
                let _ = child.wait();
                return Err(error.into());
            }
        };
        if size > MAX_MANUAL_BYTES as u64 || entries > MAX_ARCHIVE_ENTRIES {
            crate::process::terminate(&mut child);
            let _ = child.wait();
            return Err(ManualError::Invalid(
                "RAR manual exceeds extraction limits".into(),
            ));
        }
        progress(0.35 + 0.2 * (size as f64 / MAX_MANUAL_BYTES as f64).min(1.0));
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(ManualError::Invalid("could not extract RAR manual".into()));
                }
                break;
            }
            Ok(None) => {}
            Err(error) => {
                crate::process::terminate(&mut child);
                let _ = child.wait();
                return Err(error.into());
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(destination)
}

fn directory_usage(root: &Path) -> Result<(u64, usize), ManualError> {
    let mut pending = vec![root.to_path_buf()];
    let mut size = 0u64;
    let mut entries = 0usize;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            entries += 1;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                size = size.saturating_add(entry.metadata()?.len());
            }
            if size > MAX_MANUAL_BYTES as u64 || entries > MAX_ARCHIVE_ENTRIES {
                return Ok((size, entries));
            }
        }
    }
    Ok((size, entries))
}

fn read_source(
    source: &str,
    cancelled: &AtomicBool,
    progress: &dyn Fn(f64),
) -> Result<Vec<u8>, ManualError> {
    let remote = source.starts_with("https://") || source.starts_with("http://");
    let cache = remote.then(|| manual_cache_path(source));
    if let Some(path) = cache.as_ref()
        && let Ok(metadata) = fs::metadata(path)
        && metadata.is_file()
        && metadata.len() <= MAX_MANUAL_BYTES as u64
    {
        let contents = read_limited_file(path, cancelled, 0.9, progress)?;
        return Ok(contents);
    }
    if remote {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| ManualError::Invalid(error.to_string()))?;
        let contents = runtime.block_on(download_remote_source(source, cancelled, progress))?;
        if let Some(path) = cache
            && let Some(directory) = path.parent()
            && fs::create_dir_all(directory).is_ok()
        {
            let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
            if fs::write(&temporary, &contents).is_ok() {
                if fs::rename(&temporary, &path).is_err() {
                    let _ = fs::remove_file(temporary);
                }
            }
        }
        return Ok(contents);
    }
    read_limited_file(Path::new(source), cancelled, 0.35, progress)
}

fn read_limited_file(
    path: &Path,
    cancelled: &AtomicBool,
    progress_limit: f64,
    progress: &dyn Fn(f64),
) -> Result<Vec<u8>, ManualError> {
    let mut reader = fs::File::open(path)?;
    let mut contents = Vec::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        if cancelled.load(AtomicOrdering::Relaxed) {
            return Err(ManualError::Cancelled);
        }
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            progress(progress_limit);
            return Ok(contents);
        }
        contents.extend_from_slice(&buffer[..count]);
        if contents.len() > MAX_MANUAL_BYTES {
            return Err(ManualError::Invalid("manual exceeds size limit".into()));
        }
        progress(progress_limit * (contents.len() as f64 / MAX_MANUAL_BYTES as f64).min(1.0));
    }
}

async fn download_remote_source(
    source: &str,
    cancelled: &AtomicBool,
    progress: &dyn Fn(f64),
) -> Result<Vec<u8>, ManualError> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(90))
        .user_agent(concat!("POP-FE-PSP/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| ManualError::Invalid(error.to_string()))?;
    let response = tokio::select! {
        _ = wait_for_cancellation(cancelled) => return Err(ManualError::Cancelled),
        result = client.get(source).send() => result
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| ManualError::Invalid(error.to_string()))?,
    };
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MANUAL_BYTES as u64)
    {
        return Err(ManualError::Invalid(
            "manual download exceeds size limit".into(),
        ));
    }
    let mut response = response;
    let mut contents = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = wait_for_cancellation(cancelled) => return Err(ManualError::Cancelled),
            result = response.chunk() => result
                .map_err(|error| ManualError::Invalid(error.to_string()))?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        contents.extend_from_slice(&chunk);
        if contents.len() > MAX_MANUAL_BYTES {
            return Err(ManualError::Invalid("manual exceeds size limit".into()));
        }
        progress((contents.len() as f64 / MAX_MANUAL_BYTES as f64).min(0.35));
    }
    Ok(contents)
}

async fn wait_for_cancellation(cancelled: &AtomicBool) {
    while !cancelled.load(AtomicOrdering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn manual_cache_path(url: &str) -> PathBuf {
    let cache_root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".cache"))
        })
        .unwrap_or_else(std::env::temp_dir);
    let digest = Md5::digest(url.as_bytes());
    let digest = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    cache_root
        .join("org.popfe.PSPManager")
        .join("manuals")
        .join(format!("{digest}.cache"))
}

fn archive_pages(
    contents: &[u8],
    temporary: &Path,
    cancelled: &AtomicBool,
    progress: &dyn Fn(f64),
) -> Result<Vec<Vec<u8>>, ManualError> {
    let mut archive = ZipArchive::new(Cursor::new(contents))
        .map_err(|error| ManualError::Invalid(error.to_string()))?;
    let mut entries = Vec::new();
    let mut declared_bytes = 0u64;
    for index in 0..archive.len() {
        if entries.len() >= MAX_ARCHIVE_ENTRIES {
            break;
        }
        let file = archive
            .by_index(index)
            .map_err(|error| ManualError::Invalid(error.to_string()))?;
        if file.is_dir() || file.size() == 0 || file.size() > MAX_PAGE_BYTES as u64 {
            continue;
        }
        let name = file.name().replace('\\', "/");
        if name.split('/').any(|part| part.starts_with('.')) || name.contains("__MACOSX") {
            continue;
        }
        declared_bytes = declared_bytes.saturating_add(file.size());
        if declared_bytes > MAX_MANUAL_BYTES as u64 {
            return Err(ManualError::Invalid("manual exceeds size limit".into()));
        }
        entries.push((index, name));
    }
    entries.sort_by(|left, right| natural_cmp(&left.1, &right.1));
    let count = entries.len().max(1) as f64;
    let mut pages = Vec::new();
    let mut extracted_bytes = 0usize;
    for (position, (index, _name)) in entries.into_iter().enumerate() {
        check_cancelled(cancelled)?;
        let mut entry = archive
            .by_index(index)
            .map_err(|error| ManualError::Invalid(error.to_string()))?;
        let mut bytes = Vec::with_capacity(entry.size().min(MAX_PAGE_BYTES as u64) as usize);
        entry
            .by_ref()
            .take((MAX_PAGE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        extracted_bytes = extracted_bytes.saturating_add(bytes.len());
        if extracted_bytes > MAX_MANUAL_BYTES {
            return Err(ManualError::Invalid("manual exceeds size limit".into()));
        }
        if bytes.is_empty() || bytes.len() > MAX_PAGE_BYTES {
            continue;
        }
        if bytes.starts_with(b"%PDF") {
            pages.extend(pdf_pages(&bytes, temporary, cancelled, &|fraction| {
                progress(0.35 + 0.55 * (position as f64 + fraction) / count);
            })?);
        } else if let Some(pixbuf) = decode_image(&bytes, temporary, position, cancelled)? {
            pages.extend(prepare_pages(&pixbuf)?);
        }
        if pages.len() >= MAX_MANUAL_PAGES {
            break;
        }
        progress(0.35 + 0.55 * (position + 1) as f64 / count);
    }
    Ok(pages)
}

fn directory_pages(
    root: &Path,
    temporary: &Path,
    cancelled: &AtomicBool,
    progress: &dyn Fn(f64),
) -> Result<Vec<Vec<u8>>, ManualError> {
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut files = Vec::new();
    let mut declared_bytes = 0u64;
    while let Some((directory, depth)) = pending.pop() {
        check_cancelled(cancelled)?;
        if depth > 32 {
            continue;
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push((entry.path(), depth + 1));
            } else if file_type.is_file() {
                let metadata = entry.metadata()?;
                if metadata.len() > 0 && metadata.len() <= MAX_PAGE_BYTES as u64 {
                    declared_bytes = declared_bytes.saturating_add(metadata.len());
                    if declared_bytes > MAX_MANUAL_BYTES as u64 {
                        return Err(ManualError::Invalid("manual exceeds size limit".into()));
                    }
                    files.push(entry.path());
                    if files.len() >= MAX_ARCHIVE_ENTRIES {
                        break;
                    }
                }
            }
        }
        if files.len() >= MAX_ARCHIVE_ENTRIES {
            break;
        }
    }
    files.sort_by(|left, right| natural_cmp(&left.to_string_lossy(), &right.to_string_lossy()));
    let count = files.len().max(1) as f64;
    let mut pages = Vec::new();
    let mut read_bytes = 0usize;
    for (index, path) in files.iter().enumerate() {
        check_cancelled(cancelled)?;
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take((MAX_PAGE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_PAGE_BYTES {
            return Err(ManualError::Invalid("manual exceeds size limit".into()));
        }
        read_bytes = read_bytes.saturating_add(bytes.len());
        if read_bytes > MAX_MANUAL_BYTES {
            return Err(ManualError::Invalid("manual exceeds size limit".into()));
        }
        if let Some(pixbuf) = decode_image(&bytes, temporary, index, cancelled)? {
            pages.extend(prepare_pages(&pixbuf)?);
        }
        progress(0.9 * (index + 1) as f64 / count);
        if pages.len() >= MAX_MANUAL_PAGES {
            break;
        }
    }
    pages.truncate(MAX_MANUAL_PAGES);
    Ok(pages)
}

fn pdf_pages(
    contents: &[u8],
    temporary: &Path,
    cancelled: &AtomicBool,
    progress: &dyn Fn(f64),
) -> Result<Vec<Vec<u8>>, ManualError> {
    let unique = TEMP_SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed);
    let input = temporary.join(format!("source-{unique}.pdf"));
    fs::write(&input, contents)?;
    let prefix = temporary.join(format!("render-{unique}"));
    let mut command = Command::new("pdftoppm");
    command
        .arg("-png")
        .arg("-l")
        .arg(MAX_MANUAL_PAGES.to_string())
        .arg("-scale-to")
        .arg("960")
        .arg(&input)
        .arg(&prefix)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::process::isolate(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| ManualError::Invalid(format!("pdftoppm is unavailable: {error}")))?;
    loop {
        if cancelled.load(AtomicOrdering::Relaxed) {
            crate::process::terminate(&mut child);
            let _ = child.wait();
            return Err(ManualError::Cancelled);
        }
        let (rendered_pages, rendered_bytes) = match pdf_render_usage(temporary, unique) {
            Ok(usage) => usage,
            Err(error) => {
                crate::process::terminate(&mut child);
                let _ = child.wait();
                return Err(error);
            }
        };
        if rendered_pages > MAX_MANUAL_PAGES || rendered_bytes > MAX_MANUAL_BYTES as u64 {
            crate::process::terminate(&mut child);
            let _ = child.wait();
            return Err(ManualError::Invalid(
                "rendered manual exceeds extraction limits".into(),
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(ManualError::Invalid("could not render manual PDF".into()));
                }
                break;
            }
            Ok(None) => {}
            Err(error) => {
                crate::process::terminate(&mut child);
                let _ = child.wait();
                return Err(error.into());
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    let mut images = fs::read_dir(temporary)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(&format!("render-{unique}-")) && name.ends_with(".png")
                })
        })
        .collect::<Vec<_>>();
    images.sort_by(|left, right| {
        natural_cmp(
            &left.file_name().unwrap_or_default().to_string_lossy(),
            &right.file_name().unwrap_or_default().to_string_lossy(),
        )
    });
    let mut pages = Vec::new();
    let count = images.len().max(1) as f64;
    for (index, image) in images.into_iter().enumerate() {
        check_cancelled(cancelled)?;
        if let Ok(pixbuf) = Pixbuf::from_file(image) {
            pages.extend(prepare_pages(&pixbuf)?);
        }
        progress((index + 1) as f64 / count);
        if pages.len() >= MAX_MANUAL_PAGES {
            break;
        }
    }
    Ok(pages)
}

fn pdf_render_usage(temporary: &Path, unique: u64) -> Result<(usize, u64), ManualError> {
    let prefix = format!("render-{unique}-");
    let mut pages = 0usize;
    let mut bytes = 0u64;
    for entry in fs::read_dir(temporary)? {
        let entry = entry?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(&prefix)
            || entry
                .path()
                .extension()
                .is_none_or(|extension| extension != "png")
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        pages += 1;
        bytes = bytes.saturating_add(entry.metadata()?.len());
    }
    Ok((pages, bytes))
}

fn decode_image(
    contents: &[u8],
    temporary: &Path,
    index: usize,
    cancelled: &AtomicBool,
) -> Result<Option<Pixbuf>, ManualError> {
    if let Ok(pixbuf) = Pixbuf::from_read(Cursor::new(contents.to_vec())) {
        return Ok(Some(pixbuf));
    }
    let source = temporary.join(format!("page-{index}.jp2"));
    let output = temporary.join(format!("decoded-{index}.png"));
    fs::write(&source, contents)?;
    let mut command = Command::new("ffmpeg");
    command
        .args(["-nostdin", "-v", "error", "-y", "-i"])
        .arg(&source)
        .args(["-frames:v", "1"])
        .arg(&output)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::process::isolate(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| ManualError::Invalid(format!("ffmpeg is unavailable: {error}")))?;
    let status = loop {
        if cancelled.load(AtomicOrdering::Relaxed) {
            crate::process::terminate(&mut child);
            let _ = child.wait();
            return Err(ManualError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                crate::process::terminate(&mut child);
                let _ = child.wait();
                return Err(error.into());
            }
        }
        thread::sleep(Duration::from_millis(100));
    };
    if status.success() {
        return Ok(Pixbuf::from_file(output).ok());
    }
    Ok(None)
}

fn prepare_pages(pixbuf: &Pixbuf) -> Result<Vec<Vec<u8>>, ManualError> {
    let width = pixbuf.width();
    let height = pixbuf.height();
    if width <= 0
        || height <= 0
        || width > 10000
        || height > 10000
        || width as i64 * height as i64 > 50_000_000
    {
        return Ok(Vec::new());
    }
    let sources = if width as f64 > height as f64 * 1.8 {
        let half = width / 2;
        vec![
            pixbuf.new_subpixbuf(0, 0, half, height),
            pixbuf.new_subpixbuf(half, 0, width - half, height),
        ]
    } else {
        vec![pixbuf.clone()]
    };
    let mut result = Vec::with_capacity(sources.len());
    for source in sources {
        let mut target_height = (480.0 * source.height() as f64 / source.width() as f64) as i32;
        if target_height > 480 {
            target_height = 480;
        }
        let resized = source
            .scale_simple(480, target_height.max(1), InterpType::Hyper)
            .ok_or_else(|| ManualError::Invalid("could not resize manual page".into()))?;
        let png = resized
            .save_to_bufferv("png", &[])
            .map_err(|error| ManualError::Invalid(error.to_string()))?;
        result.push(png);
    }
    Ok(result)
}

fn create_document(game_id: &str, pages: &[Vec<u8>]) -> Result<Vec<u8>, ManualError> {
    if pages.len() > MAX_MANUAL_PAGES || game_id.len() != 9 || !game_id.is_ascii() {
        return Err(ManualError::Invalid("invalid manual metadata".into()));
    }
    let table_size = pages
        .len()
        .checked_mul(136)
        .and_then(|size| size.checked_add(136))
        .ok_or_else(|| ManualError::Invalid("manual is too large".into()))?;
    let data_size = pages
        .iter()
        .try_fold(0usize, |size, page| size.checked_add(page.len()))
        .ok_or_else(|| ManualError::Invalid("manual is too large".into()))?;
    if table_size.saturating_add(data_size) > MAX_MANUAL_BYTES {
        return Err(ManualError::Invalid("manual is too large".into()));
    }
    let mut output = vec![0u8; table_size];
    output[..4].copy_from_slice(b"DOC ");
    output[4..8].copy_from_slice(&0x10000u32.to_le_bytes());
    output[8..12].copy_from_slice(&0x10000u32.to_le_bytes());
    output[12..28][..game_id.len()].copy_from_slice(game_id.as_bytes());
    output[28..32].copy_from_slice(&(u32::from(pages.len() >= 100)).to_le_bytes());
    output[128..132].copy_from_slice(&u32::MAX.to_le_bytes());
    output[132..136].copy_from_slice(&(pages.len() as u32).to_le_bytes());
    let mut data_offset = table_size as u32;
    for (index, page) in pages.iter().enumerate() {
        let entry = 136 + index * 128;
        output[entry..entry + 4].copy_from_slice(&data_offset.to_le_bytes());
        output[entry + 12..entry + 16].copy_from_slice(&(page.len() as u32).to_le_bytes());
        data_offset = data_offset
            .checked_add(page.len() as u32)
            .ok_or_else(|| ManualError::Invalid("manual is too large".into()))?;
        output.extend_from_slice(page);
    }
    Ok(output)
}

fn natural_cmp(left: &str, right: &str) -> Ordering {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let (mut a, mut b) = (0, 0);
    while a < left.len() && b < right.len() {
        if left[a].is_ascii_digit() && right[b].is_ascii_digit() {
            let (a_start, b_start) = (a, b);
            while a < left.len() && left[a].is_ascii_digit() {
                a += 1;
            }
            while b < right.len() && right[b].is_ascii_digit() {
                b += 1;
            }
            let left_digits = left[a_start..a]
                .iter()
                .skip_while(|c| **c == b'0')
                .copied()
                .collect::<Vec<_>>();
            let right_digits = right[b_start..b]
                .iter()
                .skip_while(|c| **c == b'0')
                .copied()
                .collect::<Vec<_>>();
            match left_digits
                .len()
                .cmp(&right_digits.len())
                .then_with(|| left_digits.cmp(&right_digits))
            {
                Ordering::Equal => continue,
                ordering => return ordering,
            }
        } else {
            match left[a]
                .to_ascii_lowercase()
                .cmp(&right[b].to_ascii_lowercase())
            {
                Ordering::Equal => {
                    a += 1;
                    b += 1;
                }
                ordering => return ordering,
            }
        }
    }
    left.len().cmp(&right.len())
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), ManualError> {
    if cancelled.load(AtomicOrdering::Relaxed) {
        Err(ManualError::Cancelled)
    } else {
        Ok(())
    }
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn create() -> io::Result<Self> {
        for _ in 0..100 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("pop-fe-manual-{}-{sequence}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create temporary manual folder",
        ))
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
