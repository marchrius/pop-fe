//! Native PBP and PSISOIMG writer for normalized PlayStation disc images.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
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
    pub assets: PbpAssets,
    pub include_title_image: bool,
}

pub fn create_from_image(config: &PbpCreateConfig) -> Result<(), CreateError> {
    create_from_image_cancellable(config, &AtomicBool::new(false))
}

pub fn create_from_image_cancellable(
    config: &PbpCreateConfig,
    cancelled: &AtomicBool,
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
    let mut psiso_discs = Vec::with_capacity(config.images.len());
    for (index, image_path) in config.images.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CreateError::Cancelled);
        }
        let extension = image_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let normalized = if extension == "cue" || extension == "iso" {
            let image = TempFile::new("raw-disc")?;
            normalize_image(image_path, &image.file, cancelled)?;
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
        let source_md5 = CueSheet::open_image(image_path)
            .and_then(|sheet| sheet.first_track_md5())
            .ok();
        let patch = source_md5
            .as_deref()
            .and_then(|md5| crate::database::Database::embedded().psp_patch(&disc_id, md5));
        let patched = if let Some(patch) = patch {
            let temp = TempFile::new("patched-disc")?;
            let mut source = disc.try_clone()?;
            source.seek(SeekFrom::Start(0))?;
            io::copy(&mut source, &mut temp.file.try_clone()?)?;
            crate::ppf::apply(&temp.file, &patch).map_err(|error| {
                CreateError::Invalid(format!("automatic PPF compatibility patch failed: {error}"))
            })?;
            Some(temp)
        } else {
            None
        };
        let disc = match &patched {
            Some(temp) => temp.file.try_clone()?,
            None => disc,
        };
        let psiso = TempFile::new("psiso")?;
        let psp_config = crate::database::Database::embedded()
            .psp_config(&disc_id)
            .unwrap_or_else(|| EMPTY_PSP_CONFIG.to_vec());
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
        config.assets.snd0.as_ref(),
    ] {
        assets.push(match path {
            Some(path) => fs::read(path)?,
            None => Vec::new(),
        });
    }
    let logo = match &config.assets.logo {
        Some(path) => fs::read(path)?,
        None => DEFAULT_LOGO.to_vec(),
    };
    let sfo = generate_sfo(&config.game_id, &config.title);
    let mut offsets = [0u32; 8];
    let mut cursor = 0x28u64;
    offsets[0] = cursor as u32;
    cursor += sfo.len() as u64;
    for index in 0..5 {
        offsets[index + 1] = cursor as u32;
        cursor += assets[index].len() as u64;
    }
    offsets[6] = cursor as u32;
    let psar = align_up(cursor + 30_632, 0x10000);
    offsets[7] = psar as u32;

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

    if config.include_title_image {
        let mut title_image = PSTITLE_TEMPLATE.to_vec();
        title_image[..16].copy_from_slice(b"PSTITLEIMG000000");
        let first_psiso_relative = (align_up(psar + 0x400, 0x8000) - psar) as u32;
        put_u32(&mut title_image, 0x200, first_psiso_relative);
        let game_id = format!("_{}_{}", &config.game_id[..4], &config.game_id[4..]);
        title_image[0x264..0x26f].copy_from_slice(game_id.as_bytes());
        copy_text(&mut title_image[0x30c..], &config.title);
        output.write_all(&title_image)?;
        output.seek(SeekFrom::Start(align_up(psar + 0x400, 0x8000)))?;
    } else {
        output.seek(SeekFrom::Start(align_up(psar, 0x10000)))?;
    }
    let mut psiso_offsets = Vec::with_capacity(psiso_discs.len());
    let mut buffer = vec![0u8; 1024 * 1024];
    for (psiso, psiso_len) in &psiso_discs {
        if cancelled.load(Ordering::Relaxed) {
            return Err(CreateError::Cancelled);
        }
        let current = output.stream_position()?;
        output.seek(SeekFrom::Start(align_up(current, 0x8000)))?;
        psiso_offsets.push((output.stream_position()? - psar) as u32);
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
        }
        let current = output.stream_position()?;
        output.seek(SeekFrom::Start(align_up(current, 16)))?;
    }
    let startdat = align_up(output.stream_position()?, 16);
    output.seek(SeekFrom::Start(startdat))?;
    let mut start_header = STARTDAT_HEADER.to_vec();
    put_u32(&mut start_header, 20, logo.len() as u32);
    output.write_all(&start_header)?;
    output.write_all(&logo)?;
    output.write_all(STARTDAT_FOOTER)?;

    if config.include_title_image {
        let mut title_image = PSTITLE_TEMPLATE.to_vec();
        title_image[..16].copy_from_slice(b"PSTITLEIMG000000");
        for (index, offset) in psiso_offsets.iter().enumerate() {
            put_u32(&mut title_image, 0x200 + index * 4, *offset);
        }
        put_u32(&mut title_image, 0x10, (startdat - psar) as u32);
        put_u32(&mut title_image, 0x284, (startdat - psar + 0x2d31) as u32);
        let game_id = format!("_{}_{}", &config.game_id[..4], &config.game_id[4..]);
        title_image[0x264..0x26f].copy_from_slice(game_id.as_bytes());
        copy_text(&mut title_image[0x30c..], &config.title);
        output.seek(SeekFrom::Start(psar))?;
        output.write_all(&title_image)?;
    }
    output.sync_all()?;
    drop(output);
    fs::rename(&staged_output.path, &config.output)?;
    Ok(())
}

fn normalize_image(
    path: &PathBuf,
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
    let sheet = CueSheet::parse(path).map_err(|error| CreateError::Invalid(error.to_string()))?;
    if sheet.tracks.len() != 1 {
        return Err(CreateError::Invalid(
            "native conversion currently supports CUE sheets with one track".into(),
        ));
    }
    let track = &sheet.tracks[0];
    if track.mode == TrackMode::Audio {
        return Err(CreateError::Invalid(
            "audio-only CUE tracks cannot be converted as a PSX data disc".into(),
        ));
    }
    let mut source = File::open(&track.path)?;
    let sector_size = track.mode.sector_size();
    let start = track.index01.saturating_mul(sector_size);
    let length = source.metadata()?.len().saturating_sub(start);
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
    let q = parity(&data[12..2248], 52, 43, 86, 88);
    (p.try_into().unwrap(), q.try_into().unwrap())
}

struct TempFile {
    path: PathBuf,
    file: File,
}
impl TempFile {
    fn new(label: &str) -> io::Result<Self> {
        Self::new_in(&std::env::temp_dir(), label)
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
    put_u32(&mut header, 12, (image_size + PSISO_PAYLOAD_OFFSET) as u32);
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
    psiso.write_all(&build_toc(sector_count))?;
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
        let subchannel_offset = psiso.stream_position()? as u32;
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
    for _ in 0..sector_count {
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
    }
    let end = align_up(psiso.stream_position()?, 16);
    psiso.seek(SeekFrom::Start(end))?;
    psiso.set_len(end)?;
    psiso.seek(SeekFrom::Start(index_start))?;
    psiso.write_all(&indexes)?;
    psiso.seek(SeekFrom::Start(0))?;
    psiso.write_all(b"PSISOIMG0000")?;
    psiso.seek(SeekFrom::Start(12))?;
    psiso.write_all(&(end as u32).to_le_bytes())?;
    let p2_len = end.saturating_add(0x2d31) as u32;
    psiso.seek(SeekFrom::Start(0x0c00 + 1568))?;
    psiso.write_all(&p2_len.to_le_bytes())?;
    psiso.sync_all()?;
    Ok(())
}

fn build_toc(sectors: u64) -> [u8; 1024] {
    let mut toc = [0u8; 1024];
    let frames = sectors.saturating_add(150);
    toc[..40].copy_from_slice(&[
        0x41, 0, 0xa0, 0, 0, 0, 0, 0x01, 0x20, 0, 0x41, 0, 0xa1, 0, 0, 0, 0, 0x01, 0, 0, 0x41, 0,
        0xa2, 0, 0, 0, 0, 0, 0, 0, 0x41, 0, 0x01, 0, 0x02, 0x01, 0, 0, 0x02, 0,
    ]);
    toc[27] = bcd((frames / 4500) as u8);
    toc[28] = bcd(((frames / 75) % 60) as u8);
    toc[29] = bcd((frames % 75) as u8);
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
fn align_up(value: u64, alignment: u64) -> u64 {
    value.div_ceil(alignment) * alignment
}
fn put_u32(target: &mut [u8], offset: usize, value: u32) {
    target[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
