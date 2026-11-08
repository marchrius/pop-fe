//! PlayStation disc identification primitives used by the Rust backend.

use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use md5::{Digest, Md5};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DiscError {
    #[error("cannot read disc image: {0}")]
    Io(#[from] io::Error),
    #[error("invalid or unsupported CUE file: {0}")]
    Cue(String),
    #[error("disc ID could not be determined")]
    UnknownId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackMode {
    Audio,
    Mode1_2048,
    Mode1_2352,
    Mode2_2336,
    Mode2_2352,
}

impl TrackMode {
    fn parse(value: &str) -> Result<Self, DiscError> {
        match value.to_ascii_uppercase().as_str() {
            "AUDIO" => Ok(Self::Audio),
            "MODE1/2048" => Ok(Self::Mode1_2048),
            "MODE1/2352" => Ok(Self::Mode1_2352),
            "MODE2/2336" => Ok(Self::Mode2_2336),
            "MODE2/2352" => Ok(Self::Mode2_2352),
            other => Err(DiscError::Cue(format!("unknown track mode {other}"))),
        }
    }

    pub fn sector_size(self) -> u64 {
        match self {
            Self::Mode1_2048 => 2048,
            Self::Mode2_2336 => 2336,
            Self::Audio | Self::Mode1_2352 | Self::Mode2_2352 => 2352,
        }
    }

    fn data_offset(self) -> usize {
        match self {
            Self::Mode1_2048 => 0,
            Self::Mode1_2352 => 16,
            Self::Mode2_2336 => 8,
            Self::Mode2_2352 => 24,
            Self::Audio => 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Track {
    pub number: u8,
    pub path: PathBuf,
    pub mode: TrackMode,
    pub index01: u64,
    pub index00: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct CueSheet {
    pub tracks: Vec<Track>,
}

impl CueSheet {
    pub fn open_image(path: &Path) -> Result<Self, DiscError> {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "chd" => return Self::parse(&materialize_chd(path)?),
            "ccd" => return Self::parse_ccd(path),
            _ => {}
        }
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cue"))
        {
            return Self::parse(path);
        }
        let mode = match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "iso" => TrackMode::Mode1_2048,
            "bin" | "img" => TrackMode::Mode2_2352,
            other => {
                return Err(DiscError::Cue(format!(
                    "unsupported image extension .{other}"
                )));
            }
        };
        if !path.is_file() {
            return Err(DiscError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                path.display().to_string(),
            )));
        }
        Ok(Self {
            tracks: vec![Track {
                number: 1,
                path: path.to_path_buf(),
                mode,
                index01: 0,
                index00: None,
            }],
        })
    }

    pub fn parse(path: &Path) -> Result<Self, DiscError> {
        let source = std::fs::read_to_string(path)?;
        let mut current_file: Option<PathBuf> = None;
        let mut tracks: Vec<Track> = Vec::new();

        for (line_number, line) in source.lines().enumerate() {
            let line = line.trim();
            let mut words = line.split_whitespace();
            let Some(command) = words.next() else {
                continue;
            };
            match command.to_ascii_uppercase().as_str() {
                "FILE" => {
                    let filename = parse_cue_filename(line).ok_or_else(|| {
                        DiscError::Cue(format!("malformed FILE entry at line {}", line_number + 1))
                    })?;
                    let image = PathBuf::from(filename);
                    current_file = Some(if image.is_absolute() {
                        image
                    } else {
                        path.parent().unwrap_or_else(|| Path::new(".")).join(image)
                    });
                }
                "TRACK" => {
                    let number =
                        words
                            .next()
                            .and_then(|s| s.parse::<u8>().ok())
                            .ok_or_else(|| {
                                DiscError::Cue(format!("bad TRACK at line {}", line_number + 1))
                            })?;
                    let mode = words.next().ok_or_else(|| {
                        DiscError::Cue(format!("missing track mode at line {}", line_number + 1))
                    })?;
                    let image = current_file.clone().ok_or_else(|| {
                        DiscError::Cue(format!(
                            "TRACK appears before FILE at line {}",
                            line_number + 1
                        ))
                    })?;
                    tracks.push(Track {
                        number,
                        path: image,
                        mode: TrackMode::parse(mode)?,
                        index01: 0,
                        index00: None,
                    });
                }
                "INDEX" => {
                    let index = words.next();
                    let time = words.next().ok_or_else(|| {
                        DiscError::Cue(format!("missing INDEX time at line {}", line_number + 1))
                    })?;
                    let track = tracks.last_mut().ok_or_else(|| {
                        DiscError::Cue(format!(
                            "INDEX appears before TRACK at line {}",
                            line_number + 1
                        ))
                    })?;
                    let frames = cue_time_to_frames(time).ok_or_else(|| {
                        DiscError::Cue(format!("invalid INDEX time at line {}", line_number + 1))
                    })?;
                    match index {
                        Some("00") => track.index00 = Some(frames),
                        Some("01") => track.index01 = frames,
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if tracks.is_empty() {
            return Err(DiscError::Cue("no tracks found".into()));
        }
        for track in &tracks {
            if !track.path.is_file() {
                return Err(DiscError::Cue(format!(
                    "missing image file {}",
                    track.path.display()
                )));
            }
        }
        Ok(Self { tracks })
    }

    fn parse_ccd(path: &Path) -> Result<Self, DiscError> {
        let source = std::fs::read_to_string(path)?;
        let image = ccd_image_path(path)?;
        let mut section: Option<u8> = None;
        let mut parsed: std::collections::BTreeMap<u8, Track> = std::collections::BTreeMap::new();
        let mut modes = std::collections::BTreeSet::new();
        let mut indexes01 = std::collections::BTreeSet::new();
        for (line_number, raw) in source.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let name = &line[1..line.len() - 1];
                section = if let Some(number) = name.to_ascii_uppercase().strip_prefix("TRACK ") {
                    let number = number.parse::<u8>().map_err(|_| {
                        DiscError::Cue(format!(
                            "invalid CCD track section at line {}",
                            line_number + 1
                        ))
                    })?;
                    parsed.insert(
                        number,
                        Track {
                            number,
                            path: image.clone(),
                            mode: TrackMode::Audio,
                            index01: 0,
                            index00: None,
                        },
                    );
                    Some(number)
                } else {
                    None
                };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let Some(number) = section else { continue };
            let track = parsed.get_mut(&number).expect("track section was inserted");
            let key = key.trim();
            let value = value.trim();
            if key.eq_ignore_ascii_case("MODE") {
                let mode = parse_ccd_integer(value).ok_or_else(|| {
                    DiscError::Cue(format!("invalid CCD MODE at line {}", line_number + 1))
                })?;
                track.mode = match mode {
                    0 => TrackMode::Audio,
                    1 => TrackMode::Mode1_2352,
                    2 => TrackMode::Mode2_2352,
                    other => {
                        return Err(DiscError::Cue(format!(
                            "unsupported CCD track mode {other}"
                        )));
                    }
                };
                modes.insert(number);
                section = Some(number);
            } else if let Some(index) = key.to_ascii_uppercase().strip_prefix("INDEX ") {
                let sector = parse_ccd_integer(value).ok_or_else(|| {
                    DiscError::Cue(format!("invalid CCD index at line {}", line_number + 1))
                })?;
                match index.trim().parse::<u8>().ok() {
                    Some(0) => track.index00 = Some(sector),
                    Some(1) => track.index01 = sector,
                    _ => {}
                }
                if index.trim() == "1" {
                    indexes01.insert(number);
                }
            }
        }
        let tracks: Vec<Track> = parsed.into_values().collect();
        if tracks.is_empty() {
            return Err(DiscError::Cue("CCD contains no tracks".into()));
        }
        if tracks
            .iter()
            .any(|track| !modes.contains(&track.number) || !indexes01.contains(&track.number))
        {
            return Err(DiscError::Cue(
                "each CCD track must define MODE and INDEX 1".into(),
            ));
        }
        let image_size = image.metadata()?.len();
        if image_size == 0 || image_size % 2352 != 0 {
            return Err(DiscError::Cue(
                "CCD IMG size is not a whole number of 2352-byte sectors".into(),
            ));
        }
        if tracks.iter().any(|track| {
            track.index01 >= image_size / 2352
                || track
                    .index00
                    .is_some_and(|index| index >= image_size / 2352)
        }) {
            return Err(DiscError::Cue(
                "CCD track index lies beyond the IMG data".into(),
            ));
        }
        Ok(Self { tracks })
    }

    pub fn data_track(&self) -> Option<&Track> {
        self.tracks
            .iter()
            .find(|track| track.mode != TrackMode::Audio)
    }

    /// Match the upstream POP-FE first-track checksum format used by `db.py`.
    pub fn first_track_md5(&self) -> Result<String, DiscError> {
        let track = self
            .tracks
            .first()
            .ok_or_else(|| DiscError::Cue("no tracks found".into()))?;
        let mut file = File::open(&track.path)?;
        let offset = track.index01 * track.mode.sector_size();
        file.seek(SeekFrom::Start(offset))?;
        let file_end = file.metadata()?.len();
        let end_sector = self
            .tracks
            .iter()
            .skip(1)
            .find(|next| next.path == track.path)
            .map(|next| next.index01)
            .unwrap_or_else(|| file_end / track.mode.sector_size());
        let byte_count = end_sector
            .saturating_sub(track.index01)
            .saturating_mul(track.mode.sector_size());
        let mut digest = Md5::new();
        let mut buffer = [0u8; 1024 * 1024];
        let mut remaining = byte_count;
        while remaining > 0 {
            let read_length = buffer.len().min(remaining as usize);
            let count = file.read(&mut buffer[..read_length])?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
            remaining -= count as u64;
        }
        Ok(digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    /// Read the root SYSTEM.CNF file from a standard ISO9660 data track.
    pub fn system_cnf(&self) -> Result<Vec<u8>, DiscError> {
        let track = self.data_track().ok_or(DiscError::UnknownId)?;
        let mut image = File::open(&track.path)?;
        let pvd = read_user_sector(&mut image, track, 16)?;
        if pvd.len() < 190 || &pvd[1..6] != b"CD001" {
            return Err(DiscError::Cue(
                "ISO9660 primary volume descriptor not found".into(),
            ));
        }
        let root = &pvd[156..];
        let root_extent = u32::from_le_bytes(root[2..6].try_into().expect("fixed slice")) as u64;
        let root_size = u32::from_le_bytes(root[10..14].try_into().expect("fixed slice")) as usize;
        let directory = read_iso_extent(&mut image, track, root_extent, root_size)?;

        for record in directory_records(&directory) {
            let Some(record) = record else { continue };
            let name = std::str::from_utf8(&record.name).unwrap_or_default();
            if name
                .split(';')
                .next()
                .unwrap_or(name)
                .eq_ignore_ascii_case("SYSTEM.CNF")
            {
                return read_iso_extent(&mut image, track, record.extent, record.size);
            }
        }
        Err(DiscError::Cue(
            "SYSTEM.CNF not found in ISO9660 root".into(),
        ))
    }
}

fn parse_ccd_integer(value: &str) -> Option<u64> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()
    } else {
        value.parse().ok()
    }
}

fn ccd_image_path(ccd: &Path) -> Result<PathBuf, DiscError> {
    let candidate = ccd.with_extension("img");
    if candidate.is_file() {
        return Ok(candidate);
    }
    let stem = ccd.file_stem().unwrap_or_default().to_string_lossy();
    let directory = ccd.parent().unwrap_or_else(|| Path::new("."));
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("img"))
            && entry
                .path()
                .file_stem()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(&stem))
        {
            return Ok(entry.path());
        }
    }
    Err(DiscError::Cue(format!(
        "CCD companion IMG file not found for {}",
        ccd.display()
    )))
}

fn materialize_chd(path: &Path) -> Result<PathBuf, DiscError> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let metadata = std::fs::metadata(path)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.canonicalize()?.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata.modified().ok().hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    let cache_root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".cache"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("org.popfe.PSPManager/disc-images");
    std::fs::create_dir_all(&cache_root)?;
    let cached_dir = cache_root.join(key);
    let cached_cue = cached_dir.join("disc.cue");
    if cached_cue.is_file() && cached_dir.join("disc.bin").is_file() {
        return Ok(cached_cue);
    }
    if cached_dir.exists() {
        std::fs::remove_dir_all(&cached_dir)?;
    }
    let temporary = cache_root.join(format!(
        "extract-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&temporary)?;
    let image = temporary.join("disc.bin");
    let cue = temporary.join("disc.cue");
    let bundled_chdman = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("chdman")))
        .filter(|exe| exe.is_file());
    let mut command = bundled_chdman
        .as_ref()
        .map(Command::new)
        .unwrap_or_else(|| Command::new("chdman"));
    if let Some(executable) = &bundled_chdman {
        if let Some(app_dir) = executable.parent().and_then(Path::parent) {
            let library_dir = app_dir.join("lib");
            let mut library_path = library_dir.into_os_string();
            if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
                library_path.push(":");
                library_path.push(existing);
            }
            command.env("LD_LIBRARY_PATH", library_path);
        }
    }
    let extraction = command
        .args(["extractcd", "-f", "-i"])
        .arg(path)
        .arg("-ob")
        .arg(&image)
        .arg("-o")
        .arg(&cue)
        .output();
    let result = match extraction {
        Ok(output) if output.status.success() && image.is_file() && cue.is_file() => Ok(()),
        Ok(output) => Err(DiscError::Cue(format!(
            "chdman could not extract {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
        Err(error) => Err(DiscError::Cue(format!(
            "CHD input requires chdman (MAME tools): {error}"
        ))),
    };
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&temporary);
        return Err(error);
    }
    match std::fs::rename(&temporary, &cached_dir) {
        Ok(()) => Ok(cached_cue),
        Err(_) if cached_cue.is_file() && cached_dir.join("disc.bin").is_file() => {
            let _ = std::fs::remove_dir_all(&temporary);
            Ok(cached_cue)
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&temporary);
            Err(DiscError::Io(error))
        }
    }
}

#[derive(Debug)]
struct DirectoryRecord<'a> {
    extent: u64,
    size: usize,
    name: &'a [u8],
}

fn directory_records(mut bytes: &[u8]) -> impl Iterator<Item = Option<DirectoryRecord<'_>>> {
    let mut position = 0usize;
    std::iter::from_fn(move || {
        while !bytes.is_empty() {
            let length = bytes[0] as usize;
            if length == 0 {
                let next_sector = (2048 - position % 2048).min(bytes.len());
                bytes = &bytes[next_sector..];
                position += next_sector;
                continue;
            }
            if length > bytes.len() || length < 34 {
                bytes = &[];
                return Some(None);
            }
            let entry = &bytes[..length];
            bytes = &bytes[length..];
            position += length;
            let name_len = entry[32] as usize;
            if 33 + name_len > entry.len() {
                return Some(None);
            }
            let extent = u32::from_le_bytes(entry[2..6].try_into().ok()?) as u64;
            let size = u32::from_le_bytes(entry[10..14].try_into().ok()?) as usize;
            return Some(Some(DirectoryRecord {
                extent,
                size,
                name: &entry[33..33 + name_len],
            }));
        }
        None
    })
}

fn read_iso_extent(
    file: &mut File,
    track: &Track,
    lba: u64,
    size: usize,
) -> Result<Vec<u8>, DiscError> {
    let mut data = Vec::with_capacity(size);
    let sectors = size.div_ceil(2048);
    for sector in 0..sectors {
        let bytes = read_user_sector(file, track, lba + sector as u64)?;
        let remaining = size - data.len();
        data.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
    }
    Ok(data)
}

fn read_user_sector(file: &mut File, track: &Track, lba: u64) -> Result<Vec<u8>, DiscError> {
    let physical_sector = track.index01 + lba;
    let offset = physical_sector * track.mode.sector_size() + track.mode.data_offset() as u64;
    file.seek(SeekFrom::Start(offset))?;
    let mut sector = vec![0u8; 2048];
    file.read_exact(&mut sector)?;
    Ok(sector)
}

fn parse_cue_filename(line: &str) -> Option<String> {
    let value = line.get(4..)?.trim();
    if let Some(quoted) = value.strip_prefix('"') {
        Some(quoted.split_once('"')?.0.to_owned())
    } else {
        value.split_whitespace().next().map(str::to_owned)
    }
}

fn cue_time_to_frames(value: &str) -> Option<u64> {
    let mut parts = value.split(':');
    let minutes = parts.next()?.parse::<u64>().ok()?;
    let seconds = parts.next()?.parse::<u64>().ok()?;
    let frames = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() || seconds >= 60 || frames >= 75 {
        return None;
    }
    Some((minutes * 60 + seconds) * 75 + frames)
}

pub fn game_id_from_system_cnf(system_cnf: &[u8], known_ids: &[String]) -> Option<String> {
    let upper = String::from_utf8_lossy(system_cnf).to_ascii_uppercase();
    let serial = regex::Regex::new(r"([A-Z]{4})[-_]?([0-9]{3})[.-]?([0-9]{2})").ok()?;
    for captures in serial.captures_iter(&upper) {
        let normalized = format!("{}{}{}", &captures[1], &captures[2], &captures[3]);
        if normalized.len() == 9 && known_ids.iter().any(|id| id == &normalized) {
            return Some(normalized);
        }
    }
    None
}
