//! Native PBP and PSISOIMG writer for raw 2352-byte PlayStation images.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use flate2::Compression;
use flate2::write::DeflateEncoder;
use sha1::{Digest, Sha1};
use thiserror::Error;

const SECTOR_SIZE: usize = 0x9300;
const PSISO_PAYLOAD_OFFSET: u64 = 0x100000;
const PSP_BODY: &[u8] = include_bytes!("../data/psp/datapspbody.bin");
const PSTITLE_TEMPLATE: &[u8] = include_bytes!("../data/psp/pstitledata.bin");
const STARTDAT_HEADER: &[u8] = include_bytes!("../data/psp/startdatheader.bin");
const STARTDAT_FOOTER: &[u8] = include_bytes!("../data/psp/startdatfooter.bin");
const DEFAULT_LOGO: &[u8] = include_bytes!("../data/psp/logo_buffer.bin");
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum CreateError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid conversion input: {0}")]
    Invalid(String),
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
    pub image: PathBuf,
    pub output: PathBuf,
    pub game_id: String,
    pub title: String,
    pub assets: PbpAssets,
    pub include_title_image: bool,
}

pub fn create_from_raw(config: &PbpCreateConfig) -> Result<(), CreateError> {
    if config.game_id.len() != 9 || !config.game_id.is_ascii() {
        return Err(CreateError::Invalid(
            "game ID must contain nine ASCII characters".into(),
        ));
    }
    let disc = File::open(&config.image)?;
    let disc_len = disc.metadata()?.len();
    if disc_len == 0 {
        return Err(CreateError::Invalid("disc image is empty".into()));
    }
    let psiso = TempFile::new("psiso")?;
    write_psiso(&disc, disc_len, &psiso.file, &config.game_id, &config.title)?;
    let psiso_len = psiso.file.metadata()?.len();

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

    if let Some(parent) = config.output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = File::create(&config.output)?;
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
        let first_psiso_relative = 0x8000u32;
        put_u32(&mut title_image, 0x200, first_psiso_relative);
        let game_id = format!("_{}_{}", &config.game_id[..4], &config.game_id[4..]);
        title_image[0x264..0x26f].copy_from_slice(game_id.as_bytes());
        copy_text(&mut title_image[0x30c..], &config.title);
        output.write_all(&title_image)?;
        output.seek(SeekFrom::Start(align_up(psar + 0x400, 0x8000)))?;
    } else {
        output.seek(SeekFrom::Start(align_up(psar, 0x10000)))?;
    }
    let psiso_start = output.stream_position()?;
    let mut psiso_reader = psiso.file.try_clone()?;
    psiso_reader.seek(SeekFrom::Start(0))?;
    io::copy(&mut psiso_reader.take(psiso_len), &mut output)?;
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
        put_u32(&mut title_image, 0x10, (startdat - psar) as u32);
        put_u32(&mut title_image, 0x284, (startdat - psar + 0x2d31) as u32);
        let game_id = format!("_{}_{}", &config.game_id[..4], &config.game_id[4..]);
        title_image[0x264..0x26f].copy_from_slice(game_id.as_bytes());
        copy_text(&mut title_image[0x30c..], &config.title);
        output.seek(SeekFrom::Start(psar))?;
        output.write_all(&title_image)?;
    }
    let _ = psiso_start;
    output.sync_all()?;
    Ok(())
}

struct TempFile {
    path: PathBuf,
    file: File,
}
impl TempFile {
    fn new(label: &str) -> io::Result<Self> {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
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
    psiso.seek(SeekFrom::Start(PSISO_PAYLOAD_OFFSET))?;

    let mut source = disc.try_clone()?;
    source.seek(SeekFrom::Start(0))?;
    let mut indexes = Vec::with_capacity(index_bytes as usize);
    let mut data_offset = 0u32;
    let mut remaining = disc_len;
    for _ in 0..sector_count {
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
