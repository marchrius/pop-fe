//! Automatic PSP DOCUMENT.DAT creation from database linked manuals.
use std::cmp::Ordering;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::thread;
use std::time::Duration;

use gdk_pixbuf::{InterpType, Pixbuf};
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
    let contents = read_source(source, cancelled, &progress)?;
    if contents.starts_with(b"DOC ") {
        return Ok(Some(contents));
    }
    if contents.starts_with(b"\0PGD") {
        return Ok(None);
    }
    let mut pages = if contents.starts_with(b"%PDF") {
        pdf_pages(&contents, &temporary.0, cancelled, &progress)?
    } else {
        archive_pages(&contents, &temporary.0, cancelled, &progress)?
    };
    if pages.is_empty() {
        return Ok(None);
    }
    if pages.len() > MAX_MANUAL_PAGES {
        pages.truncate(MAX_MANUAL_PAGES);
    }
    progress(0.95);
    create_document(game_id, &pages).map(Some)
}

fn read_source(
    source: &str,
    cancelled: &AtomicBool,
    progress: &dyn Fn(f64),
) -> Result<Vec<u8>, ManualError> {
    let mut reader: Box<dyn Read> =
        if source.starts_with("https://") || source.starts_with("http://") {
            let client = reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(90))
                .user_agent(concat!("POP-FE-PSP/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|error| ManualError::Invalid(error.to_string()))?;
            let response = client
                .get(source)
                .send()
                .and_then(reqwest::blocking::Response::error_for_status)
                .map_err(|error| ManualError::Invalid(error.to_string()))?;
            if response
                .content_length()
                .is_some_and(|length| length > MAX_MANUAL_BYTES as u64)
            {
                return Err(ManualError::Invalid(
                    "manual download exceeds size limit".into(),
                ));
            }
            Box::new(response)
        } else {
            Box::new(fs::File::open(source)?)
        };
    let mut contents = Vec::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        if cancelled.load(AtomicOrdering::Relaxed) {
            return Err(ManualError::Cancelled);
        }
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        contents.extend_from_slice(&buffer[..count]);
        if contents.len() > MAX_MANUAL_BYTES {
            return Err(ManualError::Invalid("manual exceeds size limit".into()));
        }
        progress((contents.len() as f64 / MAX_MANUAL_BYTES as f64).min(0.35));
    }
    Ok(contents)
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
        entries.push((index, name));
    }
    entries.sort_by(|left, right| natural_cmp(&left.1, &right.1));
    let count = entries.len().max(1) as f64;
    let mut pages = Vec::new();
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
    let mut child = Command::new("pdftoppm")
        .arg("-png")
        .arg("-scale-to")
        .arg("960")
        .arg(&input)
        .arg(&prefix)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| ManualError::Invalid(format!("pdftoppm is unavailable: {error}")))?;
    loop {
        if cancelled.load(AtomicOrdering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ManualError::Cancelled);
        }
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(ManualError::Invalid("could not render manual PDF".into()));
            }
            break;
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
    let mut child = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-y", "-i"])
        .arg(&source)
        .args(["-frames:v", "1"])
        .arg(&output)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| ManualError::Invalid(format!("ffmpeg is unavailable: {error}")))?;
    let status = loop {
        if cancelled.load(AtomicOrdering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ManualError::Cancelled);
        }
        if let Some(status) = child.try_wait()? {
            break status;
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
