//! PBP container inspection and PSISOIMG disc extraction.
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use flate2::Decompress;
use flate2::FlushDecompress;
use thiserror::Error;

use crate::disc::CueSheet;

const PBP_HEADER_SIZE: usize = 0x28;
const PSISO_HEADER_SIZE: u64 = 0x100000;
const SECTOR_SIZE: usize = 0x9300;
const RAW_SECTOR_SIZE: u64 = 2352;
const INDEX_OFFSET: u64 = 0x4000;
const MAX_DISC_BYTES: u64 = 2 * 1024 * 1024 * 1024;
static EXTRACTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum PbpError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid PBP: {0}")]
    Invalid(String),
    #[error("compressed PSISO sector is invalid")]
    Compression,
    #[error("operation cancelled")]
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct Pbp {
    path: PathBuf,
    sections: [u32; 8],
    file_size: u64,
}

impl Pbp {
    pub fn open(path: &Path) -> Result<Self, PbpError> {
        let mut file = File::open(path)?;
        let file_size = file.metadata()?.len();
        if file_size < PBP_HEADER_SIZE as u64 {
            return Err(PbpError::Invalid("header is truncated".into()));
        }
        let mut header = [0u8; PBP_HEADER_SIZE];
        file.read_exact(&mut header)?;
        if &header[..4] != b"\0PBP" {
            return Err(PbpError::Invalid("missing PBP magic".into()));
        }
        let mut sections = [0u32; 8];
        for (index, offset) in sections.iter_mut().enumerate() {
            let at = 8 + index * 4;
            *offset = u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
            if *offset as u64 > file_size {
                return Err(PbpError::Invalid(format!(
                    "section {} is out of bounds",
                    index
                )));
            }
        }
        if sections[0] < PBP_HEADER_SIZE as u32 {
            return Err(PbpError::Invalid(
                "first section overlaps the PBP header".into(),
            ));
        }
        if sections.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err(PbpError::Invalid("section offsets are not ordered".into()));
        }
        Ok(Self {
            path: path.to_path_buf(),
            sections,
            file_size,
        })
    }

    pub fn extract(&self, output: &Path) -> Result<Vec<PathBuf>, PbpError> {
        self.extract_cancellable(output, &AtomicBool::new(false))
    }

    pub fn extract_cancellable(
        &self,
        output: &Path,
        cancelled: &AtomicBool,
    ) -> Result<Vec<PathBuf>, PbpError> {
        self.extract_with_progress(output, cancelled, |_| {})
    }

    pub fn extract_with_progress(
        &self,
        output: &Path,
        cancelled: &AtomicBool,
        progress: impl Fn(f64),
    ) -> Result<Vec<PathBuf>, PbpError> {
        let total_work = self.estimate_work(cancelled)?.max(1);
        let mut reporter = ProgressReporter {
            done: 0,
            total: total_work,
            callback: &progress,
        };
        progress(0.0);
        fs::create_dir_all(output)?;
        let mut staging = ExtractionStage::new(output)?;
        let staging_dir = staging.path.as_path();
        let mut source = File::open(&self.path)?;
        let names = [
            "PARAM.SFO",
            "ICON0.PNG",
            "ICON1.PMF",
            "PIC0.PNG",
            "PIC1.PNG",
            "SND0.AT3",
            "DATA.PSP",
        ];
        let mut written = Vec::new();
        let header_path = staging_dir.join("HEADER.DAT");
        copy_range(
            &mut source,
            0,
            PBP_HEADER_SIZE as u64,
            &header_path,
            cancelled,
            &mut reporter,
        )?;
        written.push(header_path);
        for (index, name) in names.iter().enumerate() {
            let start = self.sections[index] as u64;
            let end = self.sections[index + 1] as u64;
            if end <= start {
                continue;
            }
            let path = staging_dir.join(name);
            copy_range(
                &mut source,
                start,
                end - start,
                &path,
                cancelled,
                &mut reporter,
            )?;
            written.push(path);
        }
        let psar = self.sections[7] as u64;
        if psar >= self.file_size {
            if cancelled.load(Ordering::Relaxed) {
                return Err(PbpError::Cancelled);
            }
            let output_files = staging.publish(written)?;
            progress(1.0);
            return Ok(output_files);
        }
        source.seek(SeekFrom::Start(psar))?;
        let mut marker = [0; 16];
        source.read_exact(&mut marker)?;
        let mut listed_psiso_offsets = HashSet::new();
        if &marker == b"PSTITLEIMG000000" {
            let mut title = [0u8; 1024];
            source.seek(SeekFrom::Start(psar))?;
            source.read_exact(&mut title)?;
            // PSTITLEIMG stores PSISO positions relative to the PSAR start.
            // Prefer those positions so arbitrary PSAR payload bytes that
            // happen to contain "PSISOIMG0000" are not treated as discs.
            for entry in title[0x200..0x280].chunks_exact(4) {
                let relative = u32::from_le_bytes(entry.try_into().unwrap()) as u64;
                if relative == 0 {
                    break;
                }
                let Some(position) = psar.checked_add(relative) else {
                    continue;
                };
                if position
                    .checked_add(12)
                    .is_none_or(|end| end > self.file_size)
                {
                    continue;
                }
                source.seek(SeekFrom::Start(position))?;
                let mut psiso_marker = [0u8; 12];
                source.read_exact(&mut psiso_marker)?;
                if &psiso_marker == b"PSISOIMG0000" {
                    listed_psiso_offsets.insert(position);
                }
            }
            let file = staging_dir.join("PSTITLEIMG.DAT");
            fs::write(&file, title)?;
            reporter.add(1024);
            written.push(file);
        }
        let mut offset = psar;
        let mut disc_index = 0;
        while offset + 12 <= self.file_size {
            if cancelled.load(Ordering::Relaxed) {
                return Err(PbpError::Cancelled);
            }
            source.seek(SeekFrom::Start(offset))?;
            let mut magic = [0u8; 12];
            source.read_exact(&mut magic)?;
            if &magic == b"PSISOIMG0000"
                && (listed_psiso_offsets.is_empty() || listed_psiso_offsets.contains(&offset))
            {
                let (image, cue, toc, iso, audio_tracks) = self.extract_disc(
                    &mut source,
                    offset,
                    staging_dir,
                    disc_index,
                    cancelled,
                    &mut reporter,
                )?;
                written.push(image);
                written.push(cue);
                written.push(toc);
                written.extend(iso);
                written.extend(audio_tracks);
                disc_index += 1;
                let mut length = [0u8; 4];
                source.seek(SeekFrom::Start(offset + 12))?;
                source.read_exact(&mut length)?;
                let extent = u32::from_le_bytes(length) as u64;
                if extent < PSISO_HEADER_SIZE || offset.checked_add(extent).is_none() {
                    break;
                }
                offset = (offset + extent + 15) & !15;
            } else if &magic[..8] == b"STARTDAT" {
                let startdat = staging_dir.join("STARTDAT.DAT");
                copy_range(
                    &mut source,
                    offset,
                    self.file_size - offset,
                    &startdat,
                    cancelled,
                    &mut reporter,
                )?;
                written.push(startdat);
                break;
            } else {
                offset = offset.saturating_add(16);
            }
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err(PbpError::Cancelled);
        }
        let output_files = staging.publish(written)?;
        progress(1.0);
        Ok(output_files)
    }

    fn estimate_work(&self, cancelled: &AtomicBool) -> Result<u64, PbpError> {
        let mut work = self
            .sections
            .windows(2)
            .map(|pair| pair[1].saturating_sub(pair[0]) as u64)
            .sum::<u64>()
            .saturating_add(PBP_HEADER_SIZE as u64);
        let mut source = File::open(&self.path)?;
        let mut offset = self.sections[7] as u64;
        while offset + 16 <= self.file_size {
            if cancelled.load(Ordering::Relaxed) {
                return Err(PbpError::Cancelled);
            }
            source.seek(SeekFrom::Start(offset))?;
            let mut magic = [0u8; 16];
            source.read_exact(&mut magic)?;
            if &magic[..12] == b"PSISOIMG0000" {
                let extent = u32::from_le_bytes(magic[12..16].try_into().unwrap()) as u64;
                if extent < PSISO_HEADER_SIZE
                    || offset
                        .checked_add(extent)
                        .is_none_or(|end| end > self.file_size)
                {
                    break;
                }
                work = work.saturating_add(1020);
                let mut index_pos = offset + INDEX_OFFSET;
                let mut scanned_sectors = 0usize;
                while index_pos + 32 <= offset + extent {
                    if scanned_sectors % 512 == 0 && cancelled.load(Ordering::Relaxed) {
                        return Err(PbpError::Cancelled);
                    }
                    source.seek(SeekFrom::Start(index_pos))?;
                    let mut entry = [0u8; 32];
                    source.read_exact(&mut entry)?;
                    let length = u16::from_le_bytes(entry[4..6].try_into().unwrap());
                    if length == 0 {
                        break;
                    }
                    // Extraction writes each decoded PSISO sector once, then
                    // scans its 2352-byte CD sectors while exporting the ISO.
                    work = work.saturating_add(SECTOR_SIZE as u64 + RAW_SECTOR_SIZE);
                    index_pos += 32;
                    scanned_sectors += 1;
                }
                for track in 0..98u64 {
                    let table = offset + 0xc00 + track * 16;
                    source.seek(SeekFrom::Start(table))?;
                    let mut entry = [0u8; 16];
                    source.read_exact(&mut entry)?;
                    let length = u32::from_le_bytes(entry[4..8].try_into().unwrap()) as u64;
                    if length == 0 {
                        break;
                    }
                    work = work.saturating_add(length);
                }
                offset = (offset + extent + 15) & !15;
            } else if &magic[..8] == b"STARTDAT" {
                work = work.saturating_add(self.file_size - offset);
                break;
            } else {
                offset += 16;
            }
        }
        Ok(work)
    }

    fn extract_disc(
        &self,
        source: &mut File,
        base: u64,
        output: &Path,
        disc: usize,
        cancelled: &AtomicBool,
        reporter: &mut ProgressReporter<'_>,
    ) -> Result<(PathBuf, PathBuf, PathBuf, Option<PathBuf>, Vec<PathBuf>), PbpError> {
        let mut image_size = [0u8; 4];
        source.seek(SeekFrom::Start(base + 12))?;
        source.read_exact(&mut image_size)?;
        let image_size = u32::from_le_bytes(image_size) as u64;
        if image_size < PSISO_HEADER_SIZE
            || image_size > MAX_DISC_BYTES
            || base
                .checked_add(image_size)
                .is_none_or(|end| end > self.file_size)
        {
            return Err(PbpError::Invalid("PSISO image size is invalid".into()));
        }
        let image = output.join(format!("PSISO{disc}.bin"));
        let cue = output.join(format!("PSISO{disc}.cue"));
        let toc = output.join(format!("PSISO{disc}.toc"));
        let mut toc_data = [0u8; 1020];
        // The TOC lives in the PSISO header at offset 0x800. The 0x100000
        // payload boundary is only for compressed disc sectors and audio.
        source.seek(SeekFrom::Start(base + 0x800))?;
        source.read_exact(&mut toc_data)?;
        fs::write(&toc, toc_data)?;
        reporter.add(toc_data.len() as u64);
        let mut target = File::create(&image)?;
        let mut index_pos = base + INDEX_OFFSET;
        let payload = base + PSISO_HEADER_SIZE;
        let mut total = 0u64;
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err(PbpError::Cancelled);
            }
            if index_pos + 32 > base + image_size {
                break;
            }
            let mut entry = [0u8; 32];
            source.seek(SeekFrom::Start(index_pos))?;
            source.read_exact(&mut entry)?;
            let offset = u32::from_le_bytes(entry[0..4].try_into().unwrap()) as u64;
            let len = u16::from_le_bytes(entry[4..6].try_into().unwrap()) as usize;
            if offset == 0 && len == 0 {
                break;
            }
            if len == 0 || total + SECTOR_SIZE as u64 > MAX_DISC_BYTES {
                break;
            }
            let mut encoded = vec![0u8; len];
            let data_pos = payload
                .checked_add(offset)
                .ok_or_else(|| PbpError::Invalid("sector offset overflow".into()))?;
            if data_pos
                .checked_add(len as u64)
                .is_none_or(|end| end > base + image_size)
            {
                return Err(PbpError::Invalid(
                    "compressed sector is out of bounds".into(),
                ));
            }
            source.seek(SeekFrom::Start(data_pos))?;
            source.read_exact(&mut encoded)?;
            let sector = if len == SECTOR_SIZE {
                encoded
            } else {
                let mut decoded = vec![0u8; SECTOR_SIZE];
                let mut inflater = Decompress::new(false);
                let status = inflater
                    .decompress(&encoded, &mut decoded, FlushDecompress::Finish)
                    .map_err(|_| PbpError::Compression)?;
                if status != flate2::Status::StreamEnd
                    || inflater.total_in() != encoded.len() as u64
                    || inflater.total_out() != SECTOR_SIZE as u64
                {
                    return Err(PbpError::Compression);
                }
                decoded
            };
            if sector.len() != SECTOR_SIZE {
                return Err(PbpError::Compression);
            }
            target.write_all(&sector)?;
            total += sector.len() as u64;
            reporter.add(sector.len() as u64);
            index_pos += 32;
        }
        let has_atrac_audio = psiso_has_atrac_tracks(source, base)?;
        let actual_size = write_cue(&toc_data, &cue, &image, total, !has_atrac_audio)?;
        target.set_len(actual_size)?;
        let iso = output.join(format!("PSISO{disc}.iso"));
        let iso = extract_iso(&image, &cue, &iso, cancelled, reporter)?.then_some(iso);
        let mut track = 2usize;
        let mut audio_paths = Vec::new();
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err(PbpError::Cancelled);
            }
            let table_offset = base + 0xc00 + ((track - 2) * 16) as u64;
            if table_offset + 8 > base + PSISO_HEADER_SIZE {
                break;
            }
            let mut entry = [0u8; 16];
            source.seek(SeekFrom::Start(table_offset))?;
            source.read_exact(&mut entry)?;
            let audio_offset = u32::from_le_bytes(entry[0..4].try_into().unwrap()) as u64;
            let audio_length = u32::from_le_bytes(entry[4..8].try_into().unwrap()) as u64;
            if audio_offset == 0 || audio_length == 0 {
                break;
            }
            let payload_start = base
                .checked_add(PSISO_HEADER_SIZE)
                .ok_or_else(|| PbpError::Invalid("PSISO audio offset overflow".into()))?;
            let start = payload_start
                .checked_add(audio_offset)
                .ok_or_else(|| PbpError::Invalid("PSISO audio offset overflow".into()))?;
            let psiso_end = base
                .checked_add(image_size)
                .ok_or_else(|| PbpError::Invalid("PSISO audio offset overflow".into()))?;
            if start < payload_start
                || start
                    .checked_add(audio_length)
                    .is_none_or(|end| end > psiso_end || end > self.file_size)
            {
                return Err(PbpError::Invalid("audio track is out of bounds".into()));
            }
            let audio_path = output.join(format!("TRACK_{disc}_{track:02}.aea"));
            let mut audio = File::create(&audio_path)?;
            let mut header = [0u8; 0x60];
            header[0..4].copy_from_slice(b"EA3\x01");
            header[5] = 0x60;
            header[6] = 0xff;
            header[7] = 0xff;
            header[34] = 0x20;
            header[35] = 0x30;
            audio.write_all(&header)?;
            source.seek(SeekFrom::Start(start))?;
            let mut remaining = audio_length;
            let mut buffer = [0u8; 64 * 1024];
            while remaining > 0 {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(PbpError::Cancelled);
                }
                let count = remaining.min(buffer.len() as u64) as usize;
                source.read_exact(&mut buffer[..count])?;
                audio.write_all(&buffer[..count])?;
                reporter.add(count as u64);
                remaining -= count as u64;
            }
            audio_paths.push(audio_path);
            track += 1;
        }
        Ok((image, cue, toc, iso, audio_paths))
    }
}

struct ExtractionStage {
    path: PathBuf,
    output: PathBuf,
    cleanup: bool,
}

impl ExtractionStage {
    fn new(output: &Path) -> Result<Self, PbpError> {
        loop {
            let sequence = EXTRACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = output.join(format!(".pop-fe-extract-{}-{sequence}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        output: output.to_path_buf(),
                        cleanup: true,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn publish(&mut self, staged_files: Vec<PathBuf>) -> Result<Vec<PathBuf>, PbpError> {
        let mut names = HashSet::with_capacity(staged_files.len());
        let mut files = Vec::with_capacity(staged_files.len());
        for staged in staged_files {
            let name = staged
                .file_name()
                .ok_or_else(|| PbpError::Invalid("staged filename is missing".into()))?;
            if staged.parent() != Some(self.path.as_path()) || !names.insert(name.to_os_string()) {
                return Err(PbpError::Invalid(
                    "staged extraction files are invalid or have duplicate names".into(),
                ));
            }
            if !fs::symlink_metadata(&staged)?.file_type().is_file() {
                return Err(PbpError::Invalid(
                    "staged extraction entry is not a regular file".into(),
                ));
            }
            let destination = self.output.join(name);
            match fs::symlink_metadata(&destination) {
                Ok(metadata) if !metadata.file_type().is_file() => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "extraction destination is not a regular file: {}",
                            destination.display()
                        ),
                    )
                    .into());
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            files.push((staged, destination));
        }

        let backup_dir = loop {
            let sequence = EXTRACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = self
                .path
                .join(format!(".backup-{}-{sequence}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        };
        let mut backups = Vec::new();
        for (_, destination) in &files {
            if !destination.exists() {
                continue;
            }
            let backup = backup_dir.join(destination.file_name().expect("checked filename"));
            if let Err(error) = fs::rename(destination, &backup) {
                return self.fail_publication(error, &[], &backups);
            }
            backups.push((destination.clone(), backup));
        }

        let mut published = Vec::new();
        for (staged, destination) in &files {
            if let Err(error) = fs::rename(staged, destination) {
                return self.fail_publication(error, &published, &backups);
            }
            published.push(destination.clone());
        }

        // The backup contains replaced destination files. Publication is
        // committed now, so discard those originals; Drop removes any
        // remaining staging data if cleanup itself fails.
        let _ = fs::remove_dir_all(backup_dir);
        Ok(published)
    }

    fn fail_publication(
        &mut self,
        cause: io::Error,
        published: &[PathBuf],
        backups: &[(PathBuf, PathBuf)],
    ) -> Result<Vec<PathBuf>, PbpError> {
        let mut rollback_errors = Vec::new();
        for destination in published.iter().rev() {
            if let Err(error) = fs::remove_file(destination) {
                rollback_errors.push(format!("remove {}: {error}", destination.display()));
            }
        }
        for (destination, backup) in backups.iter().rev() {
            if let Err(error) = fs::rename(backup, destination) {
                rollback_errors.push(format!(
                    "restore {} from {}: {error}",
                    destination.display(),
                    backup.display()
                ));
            }
        }
        if rollback_errors.is_empty() {
            Err(cause.into())
        } else {
            self.cleanup = false;
            Err(io::Error::other(format!(
                "{}; rollback was incomplete ({}); recovery files remain in {}",
                cause,
                rollback_errors.join("; "),
                self.path.display()
            ))
            .into())
        }
    }
}

impl Drop for ExtractionStage {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

struct ProgressReporter<'a> {
    done: u64,
    total: u64,
    callback: &'a dyn Fn(f64),
}

impl ProgressReporter<'_> {
    fn add(&mut self, amount: u64) {
        self.done = self.done.saturating_add(amount).min(self.total);
        (self.callback)(self.done as f64 / self.total as f64);
    }
}

fn copy_range(
    source: &mut File,
    start: u64,
    length: u64,
    output: &Path,
    cancelled: &AtomicBool,
    reporter: &mut ProgressReporter<'_>,
) -> Result<(), PbpError> {
    let mut target = File::create(output)?;
    source.seek(SeekFrom::Start(start))?;
    let mut remaining = length;
    let mut buffer = [0u8; 1024 * 1024];
    while remaining > 0 {
        if cancelled.load(Ordering::Relaxed) {
            return Err(PbpError::Cancelled);
        }
        let count = remaining.min(buffer.len() as u64) as usize;
        source.read_exact(&mut buffer[..count])?;
        target.write_all(&buffer[..count])?;
        reporter.add(count as u64);
        remaining -= count as u64;
    }
    Ok(())
}

fn write_cue(
    toc: &[u8; 1020],
    cue: &Path,
    image: &Path,
    image_len: u64,
    include_audio_tracks: bool,
) -> Result<u64, PbpError> {
    let mut disc_image = File::open(image)?;
    let mut first_sector = [0u8; RAW_SECTOR_SIZE as usize];
    disc_image.read_exact(&mut first_sector)?;
    let data_mode = if first_sector[15] == 1 {
        "MODE1/2352"
    } else {
        "MODE2/2352"
    };
    let entries: Vec<&[u8]> = toc
        .chunks_exact(10)
        .take_while(|entry| **entry != [0; 10])
        .collect();
    let tracks = entries
        .iter()
        .find(|entry| entry[2] == 0xa1)
        .map(|entry| bcd(entry[7]) as usize)
        .unwrap_or(1)
        .clamp(1, 99);
    let toc_size = entries
        .iter()
        .find(|entry| entry[2] == 0xa2)
        .map(|entry| {
            let frames =
                bcd(entry[7]) as u64 * 60 * 75 + bcd(entry[8]) as u64 * 75 + bcd(entry[9]) as u64;
            frames.saturating_sub(1) * 2352
        })
        .filter(|size| *size > 0 && *size <= image_len)
        .unwrap_or(image_len);
    let mut cue_text = format!(
        "FILE \"{}\" BINARY\n  TRACK 01 {data_mode}\n    INDEX 01 00:00:00\n",
        image.file_name().unwrap_or_default().to_string_lossy(),
    );
    for track in 2..=if include_audio_tracks { tracks } else { 1 } {
        let point = track as u8;
        if let Some(entry) = entries.iter().find(|entry| entry[2] == point) {
            let absolute =
                bcd(entry[7]) as u64 * 60 * 75 + bcd(entry[8]) as u64 * 75 + bcd(entry[9]) as u64;
            let index00 = absolute.saturating_sub(300);
            cue_text.push_str(&format!(
                "  TRACK {track:02} AUDIO\n    INDEX 00 {:02}:{:02}:{:02}\n",
                index00 / 4500,
                index00 / 75 % 60,
                index00 % 75
            ));
            let index01 = index00 + 150;
            cue_text.push_str(&format!(
                "    INDEX 01 {:02}:{:02}:{:02}\n",
                index01 / 4500,
                index01 / 75 % 60,
                index01 % 75
            ));
        }
    }
    if image_len == 0 {
        return Err(PbpError::Invalid("empty disc image".into()));
    }
    fs::write(cue, cue_text)?;
    // ATRAC tracks are stored separately in PSISO, so the data BIN must not
    // be extended with sparse zeroes to match the original CD's TOC length.
    Ok(if include_audio_tracks {
        toc_size
    } else {
        image_len
    })
}

fn psiso_has_atrac_tracks(source: &mut File, base: u64) -> Result<bool, PbpError> {
    // The audio table is terminated by its first empty entry, so its first
    // slot alone determines whether ATRAC tracks are present.
    let mut entry = [0u8; 8];
    source.seek(SeekFrom::Start(base + 0xc00))?;
    source.read_exact(&mut entry)?;
    let offset = u32::from_le_bytes(entry[..4].try_into().unwrap());
    let length = u32::from_le_bytes(entry[4..8].try_into().unwrap());
    Ok(offset != 0 && length != 0)
}

fn extract_iso(
    image: &Path,
    cue: &Path,
    iso: &Path,
    cancelled: &AtomicBool,
    reporter: &mut ProgressReporter<'_>,
) -> Result<bool, PbpError> {
    let sheet = CueSheet::parse(cue).map_err(|error| PbpError::Invalid(error.to_string()))?;
    let track = sheet
        .data_track()
        .ok_or_else(|| PbpError::Invalid("disc has no data track for ISO export".into()))?;
    if track.path != image {
        return Err(PbpError::Invalid(
            "CUE data track does not refer to the extracted image".into(),
        ));
    }
    let sector_size = 2352u64;
    let file_sectors = fs::metadata(image)?.len() / sector_size;
    let end_sector = sheet
        .tracks
        .iter()
        .skip_while(|next| next.number != track.number)
        .skip(1)
        .find(|next| next.path == track.path)
        .map(|next| next.index00.unwrap_or(next.index01))
        .unwrap_or(file_sectors);
    if end_sector < track.index01 || end_sector > file_sectors {
        return Err(PbpError::Invalid(
            "data track bounds exceed the extracted disc image".into(),
        ));
    }

    let mut source = File::open(image)?;
    source.seek(SeekFrom::Start(track.index01 * sector_size))?;
    let mut destination = File::create(iso)?;
    let mut sector = vec![0u8; sector_size as usize];
    for _ in track.index01..end_sector {
        if cancelled.load(Ordering::Relaxed) {
            return Err(PbpError::Cancelled);
        }
        source.read_exact(&mut sector)?;
        let sync =
            sector[0] == 0 && sector[1..11].iter().all(|byte| *byte == 0xff) && sector[11] == 0;
        let payload: &[u8] = match sector[15] {
            1 if sync => &sector[16..16 + 2048],
            2 if sync && sector[18] & 0x20 == 0 && sector[16..20] == sector[20..24] => {
                &sector[24..24 + 2048]
            }
            _ => {
                drop(destination);
                let _ = fs::remove_file(iso);
                return Ok(false);
            }
        };
        destination.write_all(payload)?;
        reporter.add(sector_size);
    }
    destination.sync_all()?;
    Ok(true)
}

fn bcd(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0f)
}
