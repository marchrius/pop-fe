//! Native PBP and PSISOIMG writer for normalized PlayStation disc images.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};

use flate2::Compression;
use flate2::write::DeflateEncoder;
use sha1::{Digest, Sha1};
use thiserror::Error;

use crate::disc::{CueSheet, TrackMode};

const SECTOR_SIZE: usize = 0x9300;
const PSISO_PAYLOAD_OFFSET: u64 = 0x100000;
const PSP_BODY: &[u8] = include_bytes!("../data/psp/datapspbody.bin");
const PSTITLE_TEMPLATE: &[u8] = include_bytes!("../data/psp/pstitledata.bin");
const STARTDAT_HEADER: &[u8] = include_bytes!("../data/psp/startdatheader.bin");
const STARTDAT_FOOTER: &[u8] = include_bytes!("../data/psp/startdatfooter.bin");
const DEFAULT_LOGO: &[u8] = include_bytes!("../data/psp/logo_buffer.bin");
const EMPTY_PSP_CONFIG: &[u8] = include_bytes!("../data/psp/empty_config.bin");
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum CreateError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid conversion input: {0}")]
    Invalid(String),
    #[error("operation cancelled")]
    Cancelled,
}

#[derive(Clone, Debug, Default)]
pub struct PbpAssets {
    pub icon0: Option<PathBuf>,
    pub icon1: Option<PathBuf>,
    pub pic0: Option<PathBuf>,
    pub pic1: Option<PathBuf>,
    pub snd0: Option<PathBuf>,
    pub logo: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct PbpCreateConfig {
    pub images: Vec<PathBuf>,
    pub output: PathBuf,
    pub game_id: String,
    pub title: String,
    pub manual_source: Option<String>,
    pub assets: PbpAssets,
    pub include_title_image: bool,
    pub use_cdda: bool,
    pub disable_libcrypt_patching: bool,
    pub force_ntsc: bool,
    pub allow_disc_swapping: bool,
    pub watermark: bool,
    pub psx_undither: bool,
}

pub fn create_from_image(config: &PbpCreateConfig) -> Result<(), CreateError> {
    create_from_image_cancellable(config, &AtomicBool::new(false))
}

pub fn create_from_image_cancellable(
    config: &PbpCreateConfig,
    cancelled: &AtomicBool,
) -> Result<(), CreateError> {
    create_from_image_with_progress(config, cancelled, |_| {})
}

pub fn create_from_image_with_progress(
    config: &PbpCreateConfig,
    cancelled: &AtomicBool,
    progress: impl Fn(f64) + Send + Sync,
) -> Result<(), CreateError> {
    create_from_image_with_progress_and_warnings(config, cancelled, progress, |_| {})
}

pub fn create_from_image_with_progress_and_warnings(
    config: &PbpCreateConfig,
    cancelled: &AtomicBool,
    progress: impl Fn(f64) + Send + Sync,
    warning: impl Fn(String) + Send + Sync,
) -> Result<(), CreateError> {
    if config.game_id.len() != 9 || !config.game_id.is_ascii() {
        return Err(CreateError::Invalid(
            "game ID must contain nine ASCII characters".into(),
        ));
    }
    if config.images.is_empty() || config.images.len() > 32 {
        return Err(CreateError::Invalid(
            "select between one and 32 disc images".into(),
        ));
    }
    // POP-FE always keeps PSTITLEIMG for multi-disc EBOOTs so POPS can expose
    // the disc-switching UI, even when the single-disc option is disabled.
    let include_title_image = config.include_title_image || config.images.len() > 1;
    progress(0.0);
    #[cfg(feature = "gui")]
    let manual_document = if let Some(source) = config.manual_source.as_deref() {
        match crate::manual::create_psp_document(source, &config.game_id, cancelled, |fraction| {
            progress(0.02 * fraction);
        }) {
            Ok(document) => document,
            Err(crate::manual::ManualError::Cancelled) => return Err(CreateError::Cancelled),
            Err(error) => {
                warning(error.to_string());
                None
            }
        }
    } else {
        None
    };
    #[cfg(not(feature = "gui"))]
    let manual_document: Option<Vec<u8>> = None;
    #[cfg(not(feature = "gui"))]
    let _ = warning;
    progress(0.02);
    let mut psiso_discs = Vec::with_capacity(config.images.len());
    for (index, image_path) in config.images.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CreateError::Cancelled);
        }
        progress(0.02 + 0.10 * index as f64 / config.images.len() as f64);
        let extension = image_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        // Even raw BIN/IMG and ISO inputs need a synthetic single-track CUE:
        // PSISO TOC lead-out values are CD sectors, not 0x9300-byte blocks.
        let cue_sheet = matches!(
            extension.as_str(),
            "cue" | "chd" | "ccd" | "iso" | "bin" | "img"
        )
        .then(|| CueSheet::open_image_cancellable(image_path, cancelled))
        .transpose()
        .map_err(|error| CreateError::Invalid(error.to_string()))?;
        let audio_tracks = if config.use_cdda {
            Vec::new()
        } else if let Some(sheet) = &cue_sheet {
            prepare_audio_tracks(sheet, cancelled)?
        } else {
            Vec::new()
        };
        let normalized = if matches!(extension.as_str(), "cue" | "chd" | "ccd" | "iso" | "zip") {
            let image = TempFile::new("raw-disc")?;
            if config.use_cdda {
                if let Some(sheet) = cue_sheet.as_ref() {
                    normalize_cdda_image(sheet, &image.file, cancelled)?;
                } else {
                    normalize_image(image_path, cue_sheet.as_ref(), &image.file, cancelled)?;
                }
            } else {
                normalize_image(image_path, cue_sheet.as_ref(), &image.file, cancelled)?;
            }
            Some(image)
        } else {
            None
        };
        let disc = match &normalized {
            Some(temp) => temp.file.try_clone()?,
            None => File::open(image_path)?,
        };
        let disc_len = disc.metadata()?.len();
        if disc_len == 0 {
            return Err(CreateError::Invalid("disc image is empty".into()));
        }
        let disc_id = if index == 0 {
            config.game_id.clone()
        } else {
            crate::database::Database::embedded()
                .analyze_image(image_path)
                .map(|metadata| metadata.id)
                .unwrap_or_else(|_| "UNKN00000".into())
        };
        let source_md5 = CueSheet::open_image_cancellable(image_path, cancelled)
            .and_then(|sheet| sheet.first_track_md5_cancellable(cancelled))
            .ok();
        let ppf_compatible = matches!(extension.as_str(), "bin" | "img")
            || cue_sheet.as_ref().is_some_and(|sheet| {
                sheet.tracks.len() == 1
                    && matches!(
                        sheet.tracks[0].mode,
                        TrackMode::Mode1_2352 | TrackMode::Mode2_2352
                    )
            });
        let patch = ppf_compatible
            .then_some(source_md5.as_deref())
            .flatten()
            .and_then(|md5| crate::database::Database::embedded().psp_patch(&disc_id, md5));
        let xdelta = ppf_compatible
            .then_some(source_md5.as_deref())
            .flatten()
            .and_then(|md5| crate::database::Database::embedded().psp_xdelta(&disc_id, md5));
        let database = crate::database::Database::embedded();
        let libcrypt_patch = if config.disable_libcrypt_patching || !ppf_compatible {
            None
        } else {
            database.libcrypt_patch(&disc_id)
        };
        let lcp = if config.disable_libcrypt_patching
            || !ppf_compatible
            || !database.libcrypt.contains_key(&disc_id)
            || libcrypt_patch.is_some()
        {
            None
        } else {
            executable_path("POPFE_LCP", "lcp")
        };
        if !config.disable_libcrypt_patching
            && ppf_compatible
            && database.libcrypt.contains_key(&disc_id)
            && libcrypt_patch.is_none()
            && lcp.is_none()
        {
            warning(format!(
                "LibCrypt patch unavailable for {disc_id}; install lcp to patch this game"
            ));
        }
        let patched = if patch.is_some() || libcrypt_patch.is_some() || lcp.is_some() {
            let temp = TempFile::new("patched-disc")?;
            let mut source = disc.try_clone()?;
            source.seek(SeekFrom::Start(0))?;
            io::copy(&mut source, &mut temp.file.try_clone()?)?;
            if let Some(patch) = patch {
                crate::ppf::apply(&temp.file, &patch).map_err(|error| {
                    CreateError::Invalid(format!(
                        "automatic PPF compatibility patch failed: {error}"
                    ))
                })?;
            }
            if let Some(patch) = libcrypt_patch {
                crate::ppf::apply(&temp.file, &patch).map_err(|error| {
                    CreateError::Invalid(format!("automatic LibCrypt patch failed: {error}"))
                })?;
            }
            if let Some(lcp) = lcp {
                let mut command = Command::new(lcp);
                command.arg(&temp.path);
                run_cancellable(&mut command, cancelled, "LibCrypt patcher")?;
            }
            Some(temp)
        } else {
            None
        };
        let disc = match &patched {
            Some(temp) => temp.file.try_clone()?,
            None => disc,
        };
        let xdelta_patched = if patched.is_none() {
            if let Some(patch_data) = xdelta {
                let source = TempFile::new("xdelta-source")?;
                let mut input = disc.try_clone()?;
                input.seek(SeekFrom::Start(0))?;
                let mut target = source.file.try_clone()?;
                io::copy(&mut input, &mut target)?;
                let patch_file = TempFile::new("xdelta-patch")?;
                fs::write(&patch_file.path, patch_data)?;
                let output = TempFile::new("xdelta-output")?;
                let mut command = Command::new("xdelta3");
                command
                    .arg("decode")
                    .arg("-s")
                    .arg(&source.path)
                    .arg(&patch_file.path)
                    .arg(&output.path);
                run_cancellable(
                    &mut command,
                    cancelled,
                    "automatic Xdelta compatibility patch",
                )
                .map_err(|error| match error {
                    CreateError::Io(error) if error.kind() == io::ErrorKind::NotFound => {
                        CreateError::Invalid(format!(
                            "automatic Xdelta fix requires xdelta3: {error}"
                        ))
                    }
                    error => error,
                })?;
                if fs::metadata(&output.path)?.len() == 0 {
                    return Err(CreateError::Invalid(
                        "automatic Xdelta compatibility patch produced no disc image".into(),
                    ));
                }
                Some(output)
            } else {
                None
            }
        } else {
            None
        };
        let disc = match &xdelta_patched {
            Some(temp) => temp.file.try_clone()?,
            None => disc,
        };
        let undithered = if config.psx_undither {
            let first = 0.10 + 0.02 * index as f64 / config.images.len() as f64;
            let span = 0.02 / config.images.len() as f64;
            Some(apply_psx_undither(&disc, cancelled, |fraction| {
                progress(first + span * fraction);
            })?)
        } else {
            None
        };
        let disc = match &undithered {
            Some(temp) => temp.file.try_clone()?,
            None => disc,
        };
        let disc_len = disc.metadata()?.len();
        let psiso = TempFile::new("psiso")?;
        let mut psp_config = crate::database::Database::embedded()
            .psp_config(&disc_id)
            .unwrap_or_else(|| EMPTY_PSP_CONFIG.to_vec());
        if config.use_cdda {
            if psp_config.len() > 0x09 {
                psp_config[0x09] |= 0x20;
            }
            if psp_config.len() > 0x8d {
                psp_config[0x8d] |= 0x20;
            }
        }
        if config.force_ntsc {
            if psp_config.len() > 0x0b {
                psp_config[0x0b] |= 0x10;
            }
            if psp_config.len() > 0x8f {
                psp_config[0x8f] |= 0x10;
            }
        }
        if config.allow_disc_swapping {
            if psp_config.len() > 0x09 {
                psp_config[0x09] |= 0x80;
            }
            if psp_config.len() > 0x8d {
                psp_config[0x8d] |= 0x80;
            }
        }
        let magic_word = crate::database::Database::embedded()
            .libcrypt
            .get(&disc_id)
            .map(|record| record.magic_word);
        write_psiso(
            &disc,
            disc_len,
            &psiso.file,
            &disc_id,
            &config.title,
            &psp_config,
            magic_word,
            cue_sheet.as_ref(),
            &audio_tracks,
            index,
            config.images.len(),
            &progress,
            cancelled,
        )?;
        let psiso_len = psiso.file.metadata()?.len();
        psiso_discs.push((psiso, psiso_len));
    }

    let mut assets = Vec::new();
    for path in [
        config.assets.icon0.as_ref(),
        config.assets.icon1.as_ref(),
        config.assets.pic0.as_ref(),
        config.assets.pic1.as_ref(),
    ] {
        assets.push(match path {
            Some(path) => fs::read(path)?,
            None => Vec::new(),
        });
    }
    let snd0 = match config.assets.snd0.as_deref() {
        Some(path) => prepare_snd0(path, cancelled, |fraction| progress(0.74 + 0.06 * fraction))?,
        None => Vec::new(),
    };
    assets.push(snd0);
    #[cfg(feature = "gui")]
    if config.watermark && !assets[3].is_empty() {
        assets[3] = crate::assets::watermark_pic1_png(&assets[3], &config.title, &config.game_id)
            .map_err(|error| {
            CreateError::Invalid(format!("could not watermark PIC1: {error}"))
        })?;
    }
    let logo = match &config.assets.logo {
        Some(path) => fs::read(path)?,
        None => DEFAULT_LOGO.to_vec(),
    };
    let sfo = generate_sfo(&config.game_id, &config.title);
    let mut offsets = [0u32; 8];
    let mut cursor = 0x28u64;
    offsets[0] = format_u32(cursor, "PBP section offset")?;
    cursor += sfo.len() as u64;
    for index in 0..5 {
        offsets[index + 1] = format_u32(cursor, "PBP section offset")?;
        cursor += assets[index].len() as u64;
    }
    offsets[6] = format_u32(cursor, "PBP section offset")?;
    let psar = align_up(
        cursor
            .checked_add(30_632)
            .ok_or_else(|| CreateError::Invalid("PBP section offset overflow".into()))?,
        0x10000,
    )?;
    offsets[7] = format_u32(psar, "PBP PSAR offset")?;

    let output_dir = config
        .output
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    fs::create_dir_all(output_dir)?;
    let staged_output = TempFile::new_in(output_dir, "eboot")?;
    let mut output = staged_output.file.try_clone()?;
    let mut header = [0u8; 0x28];
    header[..4].copy_from_slice(b"\0PBP");
    put_u32(&mut header, 4, 0x10000);
    for (index, offset) in offsets.iter().enumerate() {
        put_u32(&mut header, 8 + index * 4, *offset);
    }
    output.write_all(&header)?;
    output.write_all(&sfo)?;
    for asset in &assets {
        output.write_all(asset)?;
    }
    output.write_all(PSP_BODY)?;
    output.seek(SeekFrom::Start(psar))?;

    if include_title_image {
        let mut title_image = PSTITLE_TEMPLATE.to_vec();
        title_image[..16].copy_from_slice(b"PSTITLEIMG000000");
        let first_psiso_relative = format_u32(
            align_up(
                psar.checked_add(0x400)
                    .ok_or_else(|| CreateError::Invalid("PBP PSAR offset overflow".into()))?,
                0x8000,
            )? - psar,
            "PBP title image PSISO offset",
        )?;
        put_u32(&mut title_image, 0x200, first_psiso_relative);
        let game_id = format!("_{}_{}", &config.game_id[..4], &config.game_id[4..]);
        title_image[0x264..0x26f].copy_from_slice(game_id.as_bytes());
        copy_text(&mut title_image[0x30c..], &config.title);
        output.write_all(&title_image)?;
        output.seek(SeekFrom::Start(align_up(
            psar.checked_add(0x400)
                .ok_or_else(|| CreateError::Invalid("PBP PSAR offset overflow".into()))?,
            0x8000,
        )?))?;
    } else {
        output.seek(SeekFrom::Start(align_up(psar, 0x10000)?))?;
    }
    let mut psiso_offsets = Vec::with_capacity(psiso_discs.len());
    let mut buffer = vec![0u8; 1024 * 1024];
    let total_psiso_bytes = psiso_discs
        .iter()
        .map(|(_, size)| *size)
        .sum::<u64>()
        .max(1);
    let mut copied_psiso_bytes = 0u64;
    for (psiso, psiso_len) in &psiso_discs {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CreateError::Cancelled);
        }
        let current = output.stream_position()?;
        output.seek(SeekFrom::Start(align_up(current, 0x8000)?))?;
        psiso_offsets.push(format_u32(
            output
                .stream_position()?
                .checked_sub(psar)
                .ok_or_else(|| CreateError::Invalid("invalid PBP PSISO location".into()))?,
            "PBP PSISO offset",
        )?);
        let mut psiso_reader = psiso.file.try_clone()?;
        psiso_reader.seek(SeekFrom::Start(0))?;
        let mut remaining = *psiso_len;
        while remaining > 0 {
            if cancelled.load(Ordering::Relaxed) {
                return Err(CreateError::Cancelled);
            }
            let read_len = buffer.len().min(remaining as usize);
            let count = psiso_reader.read(&mut buffer[..read_len])?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
            remaining -= count as u64;
            copied_psiso_bytes += count as u64;
            progress(0.80 + 0.19 * copied_psiso_bytes as f64 / total_psiso_bytes as f64);
        }
        let current = output.stream_position()?;
        output.seek(SeekFrom::Start(align_up(current, 16)?))?;
    }
    let startdat = align_up(output.stream_position()?, 16)?;
    output.seek(SeekFrom::Start(startdat))?;
    let mut start_header = STARTDAT_HEADER.to_vec();
    put_u32(
        &mut start_header,
        20,
        format_u32(logo.len() as u64, "PBP logo size")?,
    );
    output.write_all(&start_header)?;
    output.write_all(&logo)?;
    output.write_all(STARTDAT_FOOTER)?;

    if include_title_image {
        let mut title_image = PSTITLE_TEMPLATE.to_vec();
        title_image[..16].copy_from_slice(b"PSTITLEIMG000000");
        for (index, offset) in psiso_offsets.iter().enumerate() {
            put_u32(&mut title_image, 0x200 + index * 4, *offset);
        }
        let startdat_relative = startdat
            .checked_sub(psar)
            .ok_or_else(|| CreateError::Invalid("invalid PBP STARTDAT location".into()))?;
        put_u32(
            &mut title_image,
            0x10,
            format_u32(startdat_relative, "PBP STARTDAT offset")?,
        );
        put_u32(
            &mut title_image,
            0x284,
            format_u32(
                startdat_relative
                    .checked_add(0x2d31)
                    .ok_or_else(|| CreateError::Invalid("PBP STARTDAT end overflow".into()))?,
                "PBP STARTDAT end",
            )?,
        );
        let game_id = format!("_{}_{}", &config.game_id[..4], &config.game_id[4..]);
        title_image[0x264..0x26f].copy_from_slice(game_id.as_bytes());
        copy_text(&mut title_image[0x30c..], &config.title);
        output.seek(SeekFrom::Start(psar))?;
        output.write_all(&title_image)?;
    }
    output.sync_all()?;
    drop(output);
    fs::rename(&staged_output.path, &config.output)?;
    if let Some(document) = manual_document {
        let document_path = output_dir.join("DOCUMENT.DAT");
        let temporary = document_path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary, document)?;
        fs::rename(temporary, document_path)?;
    }
    progress(1.0);
    Ok(())
}

fn normalize_image(
    path: &PathBuf,
    cue_sheet: Option<&CueSheet>,
    output: &File,
    cancelled: &AtomicBool,
) -> Result<(), CreateError> {
    let mut target = output.try_clone()?;
    target.set_len(0)?;
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("iso"))
    {
        let mut source = File::open(path)?;
        let mut lba = 0u32;
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err(CreateError::Cancelled);
            }
            let mut payload = [0u8; 2048];
            let mut read = 0;
            while read < payload.len() {
                let count = source.read(&mut payload[read..])?;
                if count == 0 {
                    break;
                }
                read += count;
            }
            if read == 0 {
                break;
            }
            let sector = mode1_sector(lba, payload);
            target.write_all(&sector)?;
            lba = lba
                .checked_add(1)
                .ok_or_else(|| CreateError::Invalid("ISO is too large".into()))?;
        }
        return Ok(());
    }
    let sheet = cue_sheet.ok_or_else(|| CreateError::Invalid("CUE metadata is missing".into()))?;
    let track = sheet
        .data_track()
        .ok_or_else(|| CreateError::Invalid("CUE sheet does not contain a data track".into()))?;
    if track.mode == TrackMode::Audio {
        return Err(CreateError::Invalid(
            "audio-only CUE tracks cannot be converted as a PSX data disc".into(),
        ));
    }
    let mut source = File::open(&track.path)?;
    let sector_size = track.mode.sector_size();
    let start = track.index01.saturating_mul(sector_size);
    let end = sheet
        .tracks
        .iter()
        .skip_while(|next| next.number != track.number)
        .skip(1)
        .find(|next| next.path == track.path)
        .map(|next| next.index00.unwrap_or(next.index01) * next.mode.sector_size())
        .unwrap_or(source.metadata()?.len());
    let length = end.saturating_sub(start);
    source.seek(SeekFrom::Start(start))?;
    match track.mode {
        TrackMode::Mode1_2048 => {
            let mut lba = 0u32;
            let mut remaining = length;
            while remaining > 0 {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(CreateError::Cancelled);
                }
                let mut payload = [0u8; 2048];
                let wanted = remaining.min(2048) as usize;
                source.read_exact(&mut payload[..wanted])?;
                target.write_all(&mode1_sector(lba, payload))?;
                remaining -= wanted as u64;
                lba = lba
                    .checked_add(1)
                    .ok_or_else(|| CreateError::Invalid("CUE image is too large".into()))?;
            }
        }
        TrackMode::Mode2_2336 => {
            let mut lba = 0u32;
            let mut remaining = length;
            while remaining > 0 {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(CreateError::Cancelled);
                }
                let mut payload = [0u8; 2336];
                let wanted = remaining.min(2336) as usize;
                source.read_exact(&mut payload[..wanted])?;
                let mut sector = sector_prefix(lba, 2);
                sector[16..].copy_from_slice(&payload);
                target.write_all(&sector)?;
                remaining -= wanted as u64;
                lba = lba
                    .checked_add(1)
                    .ok_or_else(|| CreateError::Invalid("CUE image is too large".into()))?;
            }
        }
        TrackMode::Mode1_2352 | TrackMode::Mode2_2352 | TrackMode::Audio => {
            io::copy(&mut source.take(length), &mut target)?;
        }
    }
    Ok(())
}

fn normalize_cdda_image(
    sheet: &CueSheet,
    output: &File,
    cancelled: &AtomicBool,
) -> Result<(), CreateError> {
    let mut target = output.try_clone()?;
    target.set_len(0)?;
    let mut seen = std::collections::HashSet::new();
    for track in &sheet.tracks {
        if !seen.insert(track.path.clone()) {
            continue;
        }
        if sheet
            .tracks
            .iter()
            .filter(|other| other.path == track.path)
            .any(|other| other.mode.sector_size() != track.mode.sector_size())
        {
            return Err(CreateError::Invalid(format!(
                "CDDA mode cannot merge mixed sector sizes in {}",
                track.path.display()
            )));
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err(CreateError::Cancelled);
        }
        let mut source = File::open(&track.path)?;
        let sector_size = track.mode.sector_size() as usize;
        let length = source.metadata()?.len();
        if length == 0 || length % sector_size as u64 != 0 {
            return Err(CreateError::Invalid(format!(
                "CDDA image {} is not aligned to {}-byte sectors",
                track.path.display(),
                sector_size
            )));
        }
        if sector_size == 2352 {
            let mut buffer = [0u8; 1024 * 1024];
            loop {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(CreateError::Cancelled);
                }
                let count = source.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                target.write_all(&buffer[..count])?;
            }
            continue;
        }
        let mut lba = 0u32;
        while source.stream_position()? < length {
            if cancelled.load(Ordering::Relaxed) {
                return Err(CreateError::Cancelled);
            }
            match track.mode {
                TrackMode::Mode1_2048 => {
                    let mut payload = [0u8; 2048];
                    source.read_exact(&mut payload)?;
                    target.write_all(&mode1_sector(lba, payload))?;
                }
                TrackMode::Mode2_2336 => {
                    let mut payload = [0u8; 2336];
                    source.read_exact(&mut payload)?;
                    let mut sector = sector_prefix(lba, 2);
                    sector[16..].copy_from_slice(&payload);
                    target.write_all(&sector)?;
                }
                _ => unreachable!("2352-byte sectors handled above"),
            }
            lba = lba
                .checked_add(1)
                .ok_or_else(|| CreateError::Invalid("CDDA disc has too many sectors".into()))?;
        }
    }
    target.sync_all()?;
    Ok(())
}

fn apply_psx_undither(
    source: &File,
    cancelled: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<TempFile, CreateError> {
    const RAW_SECTOR: usize = 2352;
    const USER_DATA: usize = 2048;
    const PATTERN: [u8; 6] = [0x00, 0xe1, 0x00, 0x3c, 0x00, 0x02];
    let input_len = source.metadata()?.len();
    if input_len == 0 || input_len % RAW_SECTOR as u64 != 0 {
        return Err(CreateError::Invalid(
            "PSX-undither needs a raw image aligned to 2352-byte sectors".into(),
        ));
    }
    let copy = TempFile::new("undither-source")?;
    let mut input = source.try_clone()?;
    input.seek(SeekFrom::Start(0))?;
    let mut destination = copy.file.try_clone()?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut copied = 0u64;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CreateError::Cancelled);
        }
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        destination.write_all(&buffer[..count])?;
        copied += count as u64;
        progress(0.4 * copied as f64 / input_len as f64);
    }
    destination.sync_all()?;
    let sector_count = input_len / RAW_SECTOR as u64;
    let mut image = copy.file.try_clone()?;
    let mut sector_index = 0u64;
    while sector_index < sector_count {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CreateError::Cancelled);
        }
        let count = (sector_count - sector_index).min(2) as usize;
        let mut pair = vec![0u8; count * RAW_SECTOR];
        image.seek(SeekFrom::Start(sector_index * RAW_SECTOR as u64))?;
        image.read_exact(&mut pair)?;
        let mut regions = Vec::with_capacity(count);
        let mut flattened = vec![0u8; count * USER_DATA];
        for sector in 0..count {
            let base = sector * RAW_SECTOR;
            let raw = &pair[base..base + RAW_SECTOR];
            let mode = raw[15];
            let data_offset = match mode {
                1 if valid_raw_sector(raw) => Some(16),
                2 if valid_raw_sector(raw) && raw[0x12] & 0x20 == 0 => Some(24),
                _ => None,
            };
            regions.push(data_offset);
            if let Some(offset) = data_offset {
                flattened[sector * USER_DATA..(sector + 1) * USER_DATA]
                    .copy_from_slice(&raw[offset..offset + USER_DATA]);
            }
        }
        let mut changed = false;
        if flattened.len() >= PATTERN.len() {
            for position in 0..=flattened.len() - PATTERN.len() {
                let patched_byte = position + 5;
                let first_sector = position / USER_DATA;
                let last_sector = patched_byte / USER_DATA;
                if regions[first_sector].is_none()
                    || regions[last_sector].is_none()
                    || flattened[position] != PATTERN[0]
                    || flattened[position + 1] != PATTERN[1]
                    || flattened[position + 3] != PATTERN[3]
                    || flattened[position + 4] != PATTERN[4]
                    || flattened[position + 5] != PATTERN[5]
                {
                    continue;
                }
                let target_sector = patched_byte / USER_DATA;
                let target_offset = patched_byte % USER_DATA;
                let raw_offset = target_sector * RAW_SECTOR
                    + regions[target_sector].expect("validated sector region")
                    + target_offset;
                pair[raw_offset] = 0;
                flattened[patched_byte] = 0;
                changed = true;
            }
        }
        if changed {
            for sector in 0..count {
                if regions[sector].is_some() {
                    let start = sector * RAW_SECTOR;
                    repair_raw_sector(&mut pair[start..start + RAW_SECTOR]);
                }
            }
            image.seek(SeekFrom::Start(sector_index * RAW_SECTOR as u64))?;
            image.write_all(&pair)?;
        }
        sector_index += 1;
        progress(0.4 + 0.6 * sector_index as f64 / sector_count as f64);
    }
    image.sync_all()?;
    Ok(copy)
}

fn valid_raw_sector(sector: &[u8]) -> bool {
    sector.len() == 2352
        && sector[0] == 0
        && sector[1..11].iter().all(|byte| *byte == 0xff)
        && sector[11] == 0
}

fn repair_raw_sector(sector: &mut [u8]) {
    match sector[15] {
        1 => {
            let checksum = edc(&sector[..0x810]);
            sector[0x810..0x814].copy_from_slice(&checksum.to_le_bytes());
            sector[0x814..0x81c].fill(0);
            let raw: &[u8; 2352] = (&*sector).try_into().expect("raw sector size");
            let (p, q) = ecc(raw);
            sector[0x81c..0x8c8].copy_from_slice(&p);
            sector[0x8c8..0x930].copy_from_slice(&q);
        }
        2 => {
            let subheader = [sector[0x10], sector[0x11], sector[0x12], sector[0x13]];
            sector[0x14..0x18].copy_from_slice(&subheader);
            if sector[0x12] & 0x20 == 0 {
                let checksum = edc(&sector[0x10..0x818]);
                sector[0x818..0x81c].copy_from_slice(&checksum.to_le_bytes());
                let address = [sector[12], sector[13], sector[14], sector[15]];
                sector[12..16].fill(0);
                let raw: &[u8; 2352] = (&*sector).try_into().expect("raw sector size");
                let (p, q) = ecc(raw);
                sector[0x81c..0x8c8].copy_from_slice(&p);
                sector[0x8c8..0x930].copy_from_slice(&q);
                sector[12..16].copy_from_slice(&address);
            } else {
                let checksum = edc(&sector[0x10..0x92c]);
                sector[0x92c..0x930].copy_from_slice(&checksum.to_le_bytes());
            }
        }
        _ => {}
    }
}

struct EncodedAudioTrack {
    file: TempFile,
    data_len: u64,
}

fn prepare_audio_tracks(
    sheet: &CueSheet,
    cancelled: &AtomicBool,
) -> Result<Vec<EncodedAudioTrack>, CreateError> {
    let encoder = atracdenc_path().ok_or_else(|| {
        CreateError::Invalid("CUE audio needs atracdenc; install it or set POPFE_ATRACDENC".into())
    })?;
    let mut encoded_tracks = Vec::new();
    for (index, track) in sheet
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| track.mode == TrackMode::Audio)
    {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CreateError::Cancelled);
        }
        let sector_size = track.mode.sector_size();
        let start = track
            .index01
            .checked_mul(sector_size)
            .ok_or_else(|| CreateError::Invalid("CUE audio offset overflow".into()))?;
        let source = File::open(&track.path)?;
        let end = sheet
            .tracks
            .iter()
            .skip(index + 1)
            .find(|next| next.path == track.path)
            .map(|next| {
                next.index00
                    .unwrap_or(next.index01)
                    .saturating_mul(next.mode.sector_size())
            })
            .unwrap_or(source.metadata()?.len());
        let length = end.saturating_sub(start);
        let length = length - length % 2352;
        if length == 0 {
            return Err(CreateError::Invalid(format!(
                "CUE audio track {} is empty",
                track.number
            )));
        }
        if length > u32::MAX as u64 {
            return Err(CreateError::Invalid(
                "CUE audio track exceeds WAV size limit".into(),
            ));
        }
        let wav = TempFile::new_ext("audio", "wav")?;
        let mut wav_file = wav.file.try_clone()?;
        write_wav_header(&mut wav_file, length as u32)?;
        let mut audio_source = source;
        audio_source.seek(SeekFrom::Start(start))?;
        let mut remaining = length;
        let mut buffer = vec![0u8; 1024 * 1024];
        while remaining > 0 {
            if cancelled.load(Ordering::Relaxed) {
                return Err(CreateError::Cancelled);
            }
            let count = buffer.len().min(remaining as usize);
            audio_source.read_exact(&mut buffer[..count])?;
            wav_file.write_all(&buffer[..count])?;
            remaining -= count as u64;
        }
        drop(wav_file);
        let aea = TempFile::new_ext("audio", "aea")?;
        let mut child = Command::new(&encoder)
            .args(["--encode=atrac3", "-i"])
            .arg(&wav.path)
            .arg("-o")
            .arg(&aea.path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let status = loop {
            if cancelled.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CreateError::Cancelled);
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        if !status.success() {
            let result = child.wait_with_output()?;
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            return Err(CreateError::Invalid(format!(
                "atracdenc failed on audio track {}: {}",
                track.number,
                diagnostic.trim()
            )));
        }
        let encoded_len = aea.file.metadata()?.len();
        if encoded_len <= 0x60 {
            return Err(CreateError::Invalid(format!(
                "atracdenc produced an empty track {}",
                track.number
            )));
        }
        encoded_tracks.push(EncodedAudioTrack {
            file: aea,
            data_len: encoded_len - 0x60,
        });
    }
    Ok(encoded_tracks)
}

fn atracdenc_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("POPFE_ATRACDENC").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    if let Some(path) = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("atracdenc"))
        .find(|path| path.is_file())
    {
        return Some(path);
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        let sibling = directory.join("atracdenc");
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("atracdenc/src/atracdenc");
    bundled.is_file().then_some(bundled)
}

fn prepare_snd0(
    source: &std::path::Path,
    cancelled: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<Vec<u8>, CreateError> {
    const MAX_BYTES: usize = 500_000;
    const MAX_DURATION: f64 = 59.0;
    let mut _download = None;
    progress(0.0);
    let source = if source.to_str().is_some_and(|value| {
        value.starts_with("https://www.youtube.com/") || value.starts_with("https://youtu.be/")
    }) {
        let target = TempFile::new_ext("snd0-download", "wav")?;
        fs::remove_file(&target.path)?;
        let output = target.path.with_extension("");
        let yt_dlp = yt_dlp_executable()
            .ok_or_else(|| CreateError::Invalid("YouTube SND0 download needs yt-dlp".into()))?;
        let mut command = Command::new(&yt_dlp);
        expose_bundled_tools(&mut command, &yt_dlp);
        command
            .args([
                "--no-playlist",
                "--no-progress",
                "--quiet",
                "--js-runtimes",
                "deno",
                "-f",
                "bestaudio",
                "-x",
                "--audio-format",
                "wav",
                "-o",
            ])
            .arg(output)
            .arg(source);
        run_cancellable(&mut command, cancelled, "YouTube SND0 download")?;
        progress(0.25);
        if !target.path.is_file() {
            return Err(CreateError::Invalid(
                "yt-dlp did not produce a WAV file".into(),
            ));
        }
        let path = target.path.clone();
        _download = Some(target);
        path
    } else {
        source.to_path_buf()
    };
    let input = fs::read(&source)?;
    if input.len() >= 20
        && &input[..4] == b"RIFF"
        && &input[8..12] == b"WAVE"
        && (u16::from_le_bytes([input[20], input[21]]) == 0x0270
            || u16::from_le_bytes([input[20], input[21]]) == 0xfffe)
    {
        if input.len() > MAX_BYTES {
            return Err(CreateError::Invalid(
                "ATRAC3 SND0 exceeds the PSP 500 KB limit".into(),
            ));
        }
        progress(1.0);
        return Ok(input);
    }
    let ffmpeg = executable_path("POPFE_FFMPEG", "ffmpeg")
        .ok_or_else(|| CreateError::Invalid("SND0 conversion needs ffmpeg".into()))?;
    let encoder = atracdenc_path()
        .ok_or_else(|| CreateError::Invalid("SND0 conversion needs atracdenc".into()))?;
    let mut duration = MAX_DURATION;
    let mut attempts = 0usize;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CreateError::Cancelled);
        }
        let wav = TempFile::new_ext("snd0", "wav")?;
        let mut command = Command::new(&ffmpeg);
        command
            .args(["-nostdin", "-v", "error", "-y", "-i"])
            .arg(&source)
            .args(["-t"])
            .arg(format!("{duration:.3}"))
            .args(["-ar", "44100", "-ac", "2"])
            .arg(&wav.path);
        run_cancellable(&mut command, cancelled, "ffmpeg SND0 conversion")?;
        progress(0.35 + 0.2 * (attempts.min(2) as f64 / 3.0));
        let ea3 = TempFile::new_ext("snd0", "ea3")?;
        let mut command = Command::new(&encoder);
        command
            .args(["--encode=atrac3", "-i"])
            .arg(&wav.path)
            .arg("-o")
            .arg(&ea3.path);
        run_cancellable(&mut command, cancelled, "ATRAC3 SND0 encoding")?;
        progress(0.65 + 0.1 * (attempts.min(2) as f64 / 3.0));
        let encoded = fs::read(&ea3.path)?;
        if encoded.len() < 96 || &encoded[..4] != b"EA3\x01" {
            return Err(CreateError::Invalid(
                "atracdenc produced an invalid SND0 stream".into(),
            ));
        }
        let output = wrap_at3(&encoded[96..]);
        if output.len() <= MAX_BYTES {
            progress(1.0);
            return Ok(output);
        }
        let next_duration = duration * (MAX_BYTES as f64 / output.len() as f64) * 0.94;
        if next_duration < 1.0 {
            return Err(CreateError::Invalid(
                "SND0 audio cannot be reduced below the PSP 500 KB limit".into(),
            ));
        }
        duration = next_duration;
        attempts += 1;
    }
}

fn executable_path(variable: &str, name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(variable).map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
}

fn expose_bundled_tools(command: &mut Command, executable: &std::path::Path) {
    let Some(directory) = executable.parent() else {
        return;
    };
    let mut search_path = vec![directory.to_path_buf()];
    search_path.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    if let Ok(search_path) = std::env::join_paths(search_path) {
        command.env("PATH", search_path);
    }
}

pub(crate) fn yt_dlp_executable() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("POPFE_YT_DLP").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        let bundled = directory.join("yt-dlp");
        if bundled.is_file() {
            return Some(bundled);
        }
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join("yt-dlp"))
        .find(|path| path.is_file())
}

#[cfg(feature = "gui")]
pub(crate) fn search_youtube_audio(title: &str, cancelled: &AtomicBool) -> Result<String, String> {
    let executable = yt_dlp_executable()
        .ok_or_else(|| crate::i18n::tr("yt-dlp is required to search YouTube"))?;
    let mut command = Command::new(&executable);
    expose_bundled_tools(&mut command, &executable);
    let mut child = command
        .args([
            "--quiet",
            "--no-warnings",
            "--js-runtimes",
            "deno",
            "--flat-playlist",
            "--print",
            "id",
        ])
        .arg(format!("ytsearch1:{title} ps1 ost"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "failed to capture yt-dlp error output".to_string())?;
    let stderr_reader = std::thread::spawn(move || {
        const MAX_BYTES: usize = 16 * 1024;
        let mut output = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let count = match stderr.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            output.extend_from_slice(&buffer[..count]);
            if output.len() > MAX_BYTES {
                let excess = output.len() - MAX_BYTES;
                output.drain(..excess);
            }
        }
        output
    });

    let status = loop {
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_reader.join();
            return Err("operation cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_reader.join();
                return Err(error.to_string());
            }
        }
    };
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    let error = String::from_utf8_lossy(&stderr_reader.join().unwrap_or_default())
        .trim()
        .to_owned();
    if !status.success() {
        return Err(if error.is_empty() {
            format!("yt-dlp search failed with {status}")
        } else {
            error
        });
    }
    let id = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    if id.is_empty() {
        Err(crate::i18n::tr("No YouTube audio result was found"))
    } else {
        Ok(format!("https://www.youtube.com/watch?v={id}"))
    }
}

fn run_cancellable(
    command: &mut Command,
    cancelled: &AtomicBool,
    operation: &str,
) -> Result<(), CreateError> {
    const MAX_ERROR_BYTES: usize = 32 * 1024;
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| CreateError::Invalid("failed to capture command error output".into()))?;
    let stderr_reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let count = match stderr.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            output.extend_from_slice(&buffer[..count]);
            if output.len() > MAX_ERROR_BYTES {
                let excess = output.len() - MAX_ERROR_BYTES;
                output.drain(..excess);
            }
        }
        output
    });
    loop {
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_reader.join();
            return Err(CreateError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    let _ = stderr_reader.join();
                    return Ok(());
                }
                let error = stderr_reader.join().unwrap_or_default();
                let detail = String::from_utf8_lossy(&error).trim().to_owned();
                let message = if detail.is_empty() {
                    format!("{operation} failed with {status}")
                } else {
                    format!("{operation} failed with {status}: {detail}")
                };
                return Err(CreateError::Invalid(message));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_reader.join();
                return Err(error.into());
            }
        }
    }
}

fn wrap_at3(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len() + 150);
    output.extend_from_slice(b"RIFF\0\0\0\0WAVE");
    output.extend_from_slice(b"fmt \x20\0\0\0\x70\x02\x02\0\x44\xac\0\0\x99\x40\0\0\x80\x01\0\0\x0e\0\x01\0\0\x10\0\0\0\0\0\0\x01\0\0\0");
    let samples = ((data.len() as u64 / 0xc0) * 0x201) as u32;
    output.extend_from_slice(b"fact\x08\0\0\0");
    output.extend_from_slice(&((samples & !0xfff).saturating_sub(0x2000)).to_le_bytes());
    output.extend_from_slice(&0x800u32.to_le_bytes());
    output.extend_from_slice(b"smpl\x3c\0\0\0");
    output.extend_from_slice(&[0; 28]);
    output.extend_from_slice(&1u32.to_le_bytes());
    output.extend_from_slice(&24u32.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0x800u32.to_le_bytes());
    output.extend_from_slice(&((samples & !0xfff).saturating_sub(0x2000 + 0x2801)).to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(b"data");
    output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    output.extend_from_slice(data);
    output.extend_from_slice(b"LIST\x16\0\0\0INFOISFT\x0a\0\0\0ATRACDENC\0");
    let riff_size = (output.len() - 8) as u32;
    output[4..8].copy_from_slice(&riff_size.to_le_bytes());
    output
}

fn write_wav_header(file: &mut File, data_len: u32) -> io::Result<()> {
    let mut header = [0u8; 44];
    header[..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(data_len + 36).to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes());
    header[22..24].copy_from_slice(&2u16.to_le_bytes());
    header[24..28].copy_from_slice(&44_100u32.to_le_bytes());
    header[28..32].copy_from_slice(&176_400u32.to_le_bytes());
    header[32..34].copy_from_slice(&4u16.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_len.to_le_bytes());
    file.write_all(&header)
}

fn mode1_sector(lba: u32, payload: [u8; 2048]) -> [u8; 2352] {
    let mut sector = sector_prefix(lba, 1);
    sector[16..2064].copy_from_slice(&payload);
    let edc = edc(&sector[..2064]);
    sector[2064..2068].copy_from_slice(&edc.to_le_bytes());
    let (p, q) = ecc(&sector);
    sector[2076..2248].copy_from_slice(&p);
    sector[2248..2352].copy_from_slice(&q);
    sector
}

fn sector_prefix(lba: u32, mode: u8) -> [u8; 2352] {
    let mut sector = [0u8; 2352];
    sector[1..11].fill(0xff);
    let frames = lba + 150;
    sector[12] = bcd((frames / 4500) as u8);
    sector[13] = bcd(((frames / 75) % 60) as u8);
    sector[14] = bcd((frames % 75) as u8);
    sector[15] = mode;
    sector
}

fn edc(bytes: &[u8]) -> u32 {
    let mut value = 0u32;
    for byte in bytes {
        value ^= *byte as u32;
        for _ in 0..8 {
            value = (value >> 1) ^ if value & 1 != 0 { 0xd8018001 } else { 0 };
        }
    }
    value
}

fn ecc(data: &[u8; 2352]) -> ([u8; 172], [u8; 104]) {
    fn parity(
        source: &[u8],
        major_count: usize,
        minor_count: usize,
        major_mult: usize,
        minor_inc: usize,
    ) -> Vec<u8> {
        let mut forward = [0u8; 256];
        let mut backward = [0u8; 256];
        for value in 0..256 {
            let doubled = (value << 1) ^ if value & 0x80 != 0 { 0x11d } else { 0 };
            forward[value] = doubled as u8;
            backward[(value ^ doubled) as usize] = value as u8;
        }
        let size = major_count * minor_count;
        let mut output = vec![0u8; major_count * 2];
        for major in 0..major_count {
            let mut index = (major >> 1) * major_mult + (major & 1);
            let (mut a, mut b) = (0u8, 0u8);
            for _ in 0..minor_count {
                let value = source[index];
                index += minor_inc;
                if index >= size {
                    index -= size;
                }
                a ^= value;
                b ^= value;
                a = forward[a as usize];
            }
            a = backward[(forward[a as usize] ^ b) as usize];
            output[major] = a;
            output[major + major_count] = a ^ b;
        }
        output
    }
    let p = parity(&data[12..2076], 86, 24, 2, 86);
    let mut with_p = *data;
    with_p[2076..2248].copy_from_slice(&p);
    let q = parity(&with_p[12..2248], 52, 43, 86, 88);
    (p.try_into().unwrap(), q.try_into().unwrap())
}

struct TempFile {
    path: PathBuf,
    file: File,
}
impl TempFile {
    fn new(label: &str) -> io::Result<Self> {
        Self::new_ext(label, "tmp")
    }

    fn new_ext(label: &str, extension: &str) -> io::Result<Self> {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "pop-fe-{label}-{}-{sequence}.{extension}",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self { path, file })
    }

    fn new_in(directory: &std::path::Path, label: &str) -> io::Result<Self> {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            "pop-fe-{label}-{}-{sequence}.tmp",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self { path, file })
    }
}
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn write_psiso(
    disc: &File,
    disc_len: u64,
    output: &File,
    game_id: &str,
    title: &str,
    psp_config: &[u8],
    magic_word: Option<u16>,
    cue_sheet: Option<&CueSheet>,
    audio_tracks: &[EncodedAudioTrack],
    disc_index: usize,
    disc_count: usize,
    progress: &dyn Fn(f64),
    cancelled: &AtomicBool,
) -> Result<(), CreateError> {
    let sector_count = disc_len.div_ceil(SECTOR_SIZE as u64);
    let image_size = sector_count
        .checked_mul(SECTOR_SIZE as u64)
        .ok_or_else(|| CreateError::Invalid("disc image is too large".into()))?;
    let mut psiso = output.try_clone()?;
    psiso.set_len(0)?;
    let mut header = [0u8; 1024];
    header[..12].copy_from_slice(b"PSISOIMG0000");
    let psiso_size = image_size
        .checked_add(PSISO_PAYLOAD_OFFSET)
        .ok_or_else(|| CreateError::Invalid("disc image is too large".into()))?;
    put_u32(&mut header, 12, format_u32(psiso_size, "PSISO image size")?);
    psiso.write_all(&header)?;
    let mut game = [0u8; 1024];
    let id = format!("_{}_{}", &game_id[..4], &game_id[4..]);
    game[..11].copy_from_slice(id.as_bytes());
    if psp_config.len() > game.len() - 0x20 {
        return Err(CreateError::Invalid(
            "PSP compatibility config is too large".into(),
        ));
    }
    game[0x20..0x20 + psp_config.len()].copy_from_slice(psp_config);
    psiso.write_all(&game)?;
    psiso.write_all(&build_toc(sector_count, cue_sheet))?;
    psiso.write_all(&vec![0u8; 1568])?;
    let mut title_block = [0u8; 480];
    title_block[8..10].copy_from_slice(&[0xff, 0x07]);
    copy_text(&mut title_block[12..], title);
    psiso.write_all(&title_block)?;
    psiso.write_all(&vec![0u8; 11264])?;
    let index_start = psiso.stream_position()?;
    let index_bytes = sector_count
        .checked_mul(32)
        .ok_or_else(|| CreateError::Invalid("too many disc sectors".into()))?;
    psiso.write_all(&vec![
        0u8;
        usize::try_from(index_bytes).map_err(|_| {
            CreateError::Invalid("index table is too large".into())
        })?
    ])?;
    if let Some(magic_word) = magic_word {
        let subchannels = generate_subchannels(magic_word);
        let subchannel_offset = format_u32(psiso.stream_position()?, "PSISO subchannel offset")?;
        psiso.seek(SeekFrom::Start(0x12d4))?;
        psiso.write_all(&subchannel_offset.to_le_bytes())?;
        psiso.write_all(&((subchannels.len() / 12) as u32).to_le_bytes())?;
        psiso.seek(SeekFrom::Start(subchannel_offset as u64))?;
        psiso.write_all(&subchannels)?;
    }
    psiso.seek(SeekFrom::Start(PSISO_PAYLOAD_OFFSET))?;

    let mut source = disc.try_clone()?;
    source.seek(SeekFrom::Start(0))?;
    let mut indexes = Vec::with_capacity(index_bytes as usize);
    let mut data_offset = 0u32;
    let mut remaining = disc_len;
    for sector_index in 0..sector_count {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CreateError::Cancelled);
        }
        let mut sector = vec![0u8; SECTOR_SIZE];
        let count = remaining.min(SECTOR_SIZE as u64) as usize;
        source.read_exact(&mut sector[..count])?;
        remaining -= count as u64;
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(1));
        encoder.write_all(&sector)?;
        let compressed = encoder.finish()?;
        let mut index = [0u8; 32];
        index[..4].copy_from_slice(&data_offset.to_le_bytes());
        let payload = if compressed.len() < SECTOR_SIZE {
            compressed.as_slice()
        } else {
            sector.as_slice()
        };
        index[4..6].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        let digest = Sha1::digest(&sector);
        index[8..24].copy_from_slice(&digest[..16]);
        psiso.write_all(payload)?;
        data_offset = data_offset
            .checked_add(payload.len() as u32)
            .ok_or_else(|| CreateError::Invalid("compressed image is too large".into()))?;
        indexes.extend_from_slice(&index);
        if sector_index % 64 == 0 || sector_index + 1 == sector_count {
            let disc_fraction = (sector_index + 1) as f64 / sector_count.max(1) as f64;
            progress(0.12 + 0.66 * (disc_index as f64 + disc_fraction) / disc_count as f64);
        }
    }
    if audio_tracks.len() > 98 {
        return Err(CreateError::Invalid(
            "PSISO supports at most 98 audio tracks".into(),
        ));
    }
    for (index, audio) in audio_tracks.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CreateError::Cancelled);
        }
        let aligned = align_up(psiso.stream_position()?, 16)?;
        psiso.seek(SeekFrom::Start(aligned))?;
        let relative = aligned
            .checked_sub(PSISO_PAYLOAD_OFFSET)
            .ok_or_else(|| CreateError::Invalid("invalid PSISO audio location".into()))?;
        let mut entry = [0u8; 16];
        entry[..4].copy_from_slice(
            &u32::try_from(relative)
                .map_err(|_| {
                    CreateError::Invalid("PSISO audio offset exceeds format limit".into())
                })?
                .to_le_bytes(),
        );
        entry[4..8].copy_from_slice(
            &u32::try_from(audio.data_len)
                .map_err(|_| CreateError::Invalid("PSISO audio track exceeds format limit".into()))?
                .to_le_bytes(),
        );
        psiso.seek(SeekFrom::Start(0x0c00 + (index * 16) as u64))?;
        psiso.write_all(&entry)?;
        psiso.seek(SeekFrom::Start(aligned))?;
        let mut encoded = audio.file.file.try_clone()?;
        encoded.seek(SeekFrom::Start(0x60))?;
        io::copy(&mut encoded.take(audio.data_len), &mut psiso)?;
    }
    let end = align_up(psiso.stream_position()?, 16)?;
    psiso.seek(SeekFrom::Start(end))?;
    psiso.set_len(end)?;
    psiso.seek(SeekFrom::Start(index_start))?;
    psiso.write_all(&indexes)?;
    psiso.seek(SeekFrom::Start(0))?;
    psiso.write_all(b"PSISOIMG0000")?;
    psiso.seek(SeekFrom::Start(12))?;
    psiso.write_all(&format_u32(end, "PSISO image size")?.to_le_bytes())?;
    let p2_len = format_u32(end.saturating_add(0x2d31), "PSISO image end")?;
    psiso.seek(SeekFrom::Start(0x0c00 + 1568))?;
    psiso.write_all(&p2_len.to_le_bytes())?;
    psiso.sync_all()?;
    Ok(())
}

fn build_toc(sectors: u64, sheet: Option<&CueSheet>) -> [u8; 1024] {
    let mut toc = [0u8; 1024];
    let mut track_starts = Vec::new();
    let mut total_sectors = sectors;
    if let Some(sheet) = sheet {
        let mut file_offsets = std::collections::HashMap::<PathBuf, u64>::new();
        let mut next_file_offset = 0u64;
        for track in &sheet.tracks {
            let base = *file_offsets.entry(track.path.clone()).or_insert_with(|| {
                let offset = next_file_offset;
                if let Ok(metadata) = track.path.metadata() {
                    next_file_offset =
                        next_file_offset.saturating_add(metadata.len() / track.mode.sector_size());
                }
                offset
            });
            track_starts.push((track.number, track.mode, base.saturating_add(track.index01)));
        }
        total_sectors = next_file_offset.max(sectors);
        let last_track = sheet.tracks.last().map(|track| track.number).unwrap_or(1);
        toc[..10].copy_from_slice(&[0x41, 0, 0xa0, 0, 0, 0, 0, 1, 0x20, 0]);
        toc[10..20].copy_from_slice(&[0x41, 0, 0xa1, 0, 0, 0, 0, bcd(last_track), 0, 0]);
    }
    // POP-FE's PSX TOC stores the frame immediately after the final sector.
    let frames = total_sectors.saturating_add(1);
    toc[..40].copy_from_slice(&[
        0x41, 0, 0xa0, 0, 0, 0, 0, 0x01, 0x20, 0, 0x41, 0, 0xa1, 0, 0, 0, 0, 0x01, 0, 0, 0x41, 0,
        0xa2, 0, 0, 0, 0, 0, 0, 0, 0x41, 0, 0x01, 0, 0x02, 0x01, 0, 0, 0x02, 0,
    ]);
    toc[27] = bcd((frames / 4500) as u8);
    toc[28] = bcd(((frames / 75) % 60) as u8);
    toc[29] = bcd((frames % 75) as u8);
    // PSISO block 3 stores the compressed-disc payload address in its last word.
    put_u32(&mut toc, 0x3fc, PSISO_PAYLOAD_OFFSET as u32);
    if let Some(sheet) = sheet {
        // Restore the A0/A1 entries after writing the standard single-track header.
        let last_track = sheet.tracks.last().map(|track| track.number).unwrap_or(1);
        toc[7] = 1;
        toc[8] = 0x20;
        toc[17] = bcd(last_track);
        for (index, (number, _mode, start)) in track_starts.iter().enumerate() {
            let offset = 30 + index * 10;
            if offset + 10 > toc.len() {
                break;
            }
            // Track 1 is encoded with POPS' fixed 00:02:01 / 00:02:00
            // addresses in the template above, independent of CUE INDEX.
            if *number == 1 {
                continue;
            }
            let mut frames = *start;
            // Keep the original PSX CUE TOC convention for subsequent tracks.
            toc[offset] = 0x01;
            toc[offset + 2] = bcd(*number);
            toc[offset + 3] = bcd((frames / 4500) as u8);
            toc[offset + 4] = bcd(((frames / 75) % 60) as u8);
            toc[offset + 5] = bcd((frames % 75) as u8);
            frames = frames.saturating_add(150);
            toc[offset + 7] = bcd((frames / 4500) as u8);
            toc[offset + 8] = bcd(((frames / 75) % 60) as u8);
            toc[offset + 9] = bcd((frames % 75) as u8);
        }
    }
    toc
}

fn generate_subchannels(magic_word: u16) -> Vec<u8> {
    const PAIRS: [(u32, u32); 16] = [
        (16167, 16172),
        (16101, 16106),
        (16031, 16036),
        (15919, 15924),
        (15628, 15633),
        (15378, 15383),
        (15312, 15317),
        (15242, 15247),
        (15130, 15135),
        (15056, 15061),
        (14899, 14904),
        (14649, 14654),
        (14579, 14584),
        (14485, 14490),
        (14231, 14236),
        (14105, 14110),
    ];
    fn bcd(value: u32) -> u8 {
        ((value / 10) as u8) << 4 | (value % 10) as u8
    }
    fn entry(sector: u32, corrupt: bool) -> [u8; 12] {
        let mut bytes = [0u8; 12];
        let mut relative = sector - 150;
        bytes[..4].copy_from_slice(&relative.to_le_bytes());
        bytes[4] = 1;
        bytes[5] = 1;
        if corrupt {
            relative -= 1;
        }
        bytes[8] = bcd(relative % 75);
        relative -= relative % 75;
        relative /= 75;
        bytes[7] = bcd(relative % 60);
        bytes[6] = bcd(relative / 60);
        let mut absolute = if corrupt { sector - 1 } else { sector };
        bytes[11] = bcd(absolute % 75);
        absolute -= absolute % 75;
        absolute /= 75;
        bytes[10] = bcd(absolute % 60);
        bytes[9] = bcd(absolute / 60);
        bytes
    }
    let mut result = Vec::with_capacity(408);
    result.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
    for (bit, (first, second)) in PAIRS.iter().enumerate().rev() {
        let corrupt = magic_word & (1 << bit) != 0;
        result.extend_from_slice(&entry(*first, corrupt));
        result.extend_from_slice(&entry(*second, corrupt));
    }
    result.extend_from_slice(&[0xff; 12]);
    result
}

fn generate_sfo(game_id: &str, title: &str) -> Vec<u8> {
    let mut entries: Vec<(&str, u16, u32, Vec<u8>)> = vec![
        ("BOOTABLE", 0x0404, 4, 1u32.to_le_bytes().to_vec()),
        ("CATEGORY", 0x0204, 4, b"ME\0\0".to_vec()),
        ("DISC_ID", 0x0204, 16, nul_string(game_id, 16)),
        ("DISC_VERSION", 0x0204, 8, nul_string("1.00", 8)),
        (
            "LICENSE",
            0x0204,
            512,
            nul_string("Copyright(C) Sony Computer Entertainment America Inc.", 512),
        ),
        ("PARENTAL_LEVEL", 0x0404, 4, 3u32.to_le_bytes().to_vec()),
        ("PSP_SYSTEM_VER", 0x0204, 8, nul_string("3.01", 8)),
        ("REGION", 0x0404, 4, 32768u32.to_le_bytes().to_vec()),
        ("TITLE", 0x0204, 128, nul_string(title, 128)),
    ];
    let mut keys = Vec::new();
    let mut data = Vec::new();
    let mut index = Vec::new();
    for (key, format, max_size, value) in entries.drain(..) {
        let key_offset = keys.len() as u16;
        keys.extend_from_slice(key.as_bytes());
        keys.push(0);
        let mut item = [0u8; 16];
        item[..2].copy_from_slice(&key_offset.to_le_bytes());
        item[2..4].copy_from_slice(&format.to_le_bytes());
        let data_length = if format == 0x0204 {
            value
                .iter()
                .position(|byte| *byte == 0)
                .map_or(value.len(), |nul| nul + 1)
        } else {
            value.len()
        };
        item[4..8].copy_from_slice(&(data_length as u32).to_le_bytes());
        item[8..12].copy_from_slice(&max_size.to_le_bytes());
        item[12..16].copy_from_slice(&(data.len() as u32).to_le_bytes());
        index.extend_from_slice(&item);
        data.extend_from_slice(&value);
    }
    while keys.len() % 4 != 0 {
        keys.push(0);
    }
    let mut result = vec![0u8; 20];
    result[..4].copy_from_slice(b"\0PSF");
    put_u32(&mut result, 4, 257);
    put_u32(&mut result, 8, 20 + index.len() as u32);
    put_u32(&mut result, 12, 20 + index.len() as u32 + keys.len() as u32);
    put_u32(&mut result, 16, 9);
    result.extend(index);
    result.extend(keys);
    result.extend(data);
    result
}

fn nul_string(value: &str, max: usize) -> Vec<u8> {
    let mut end = value.len().min(max.saturating_sub(1));
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut bytes = value.as_bytes()[..end].to_vec();
    bytes.push(0);
    bytes.resize(max, 0);
    bytes
}
fn copy_text(target: &mut [u8], value: &str) {
    let count = value.len().min(target.len());
    target[..count].copy_from_slice(&value.as_bytes()[..count]);
}
fn bcd(value: u8) -> u8 {
    (value / 10) << 4 | (value % 10)
}
fn align_up(value: u64, alignment: u64) -> Result<u64, CreateError> {
    value
        .checked_add(alignment - 1)
        .map(|rounded| rounded / alignment * alignment)
        .ok_or_else(|| CreateError::Invalid("PBP/PSISO offset overflow".into()))
}
fn format_u32(value: u64, field: &str) -> Result<u32, CreateError> {
    u32::try_from(value)
        .map_err(|_| CreateError::Invalid(format!("{field} exceeds the PBP/PSISO format limit")))
}
fn put_u32(target: &mut [u8], offset: usize, value: u32) {
    target[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
