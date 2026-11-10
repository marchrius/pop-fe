use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use crate::disc::{CueSheet, DiscError, game_id_from_system_cnf};

#[derive(Clone, Debug, Deserialize)]
pub struct DiscRecord {
    pub id: String,
    pub title: Option<String>,
    pub exe: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GameRecord {
    pub url: Option<String>,
    pub id: Option<String>,
    pub title: Option<String>,
    pub icon0: Option<String>,
    pub pic0: Option<String>,
    #[serde(rename = "pic0-scaling")]
    pub pic0_scaling: Option<f64>,
    #[serde(rename = "pic0-offset")]
    pub pic0_offset: Option<(f64, f64)>,
    pub pic1: Option<String>,
    pub snd0: Option<String>,
    pub manual: Option<String>,
    pub pspconfig: Option<String>,
    #[serde(rename = "psp-use-cdda")]
    pub psp_use_cdda: Option<bool>,
    pub ppf: Option<String>,
    pub tags: Option<Vec<String>>,
    pub hashes: Option<HashMap<String, HashMap<String, String>>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LibCryptRecord {
    pub magic_word: u16,
    pub ppf: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct LibCryptRevisionSignature {
    offset: u64,
    old_byte: u8,
    old_patch: Option<String>,
    new_byte: u8,
    new_patch: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ThemeRecord {
    pub url: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PatchRecord {
    pub ppf: Option<String>,
    pub tags: Option<Vec<String>>,
    pub desc: Option<String>,
    pub hashes: Option<HashMap<String, HashMap<String, String>>>,
}

#[derive(Debug, Deserialize)]
pub struct Database {
    pub discs: HashMap<String, DiscRecord>,
    pub games: HashMap<String, GameRecord>,
    game_fields: HashMap<String, HashSet<String>>,
    psp_configs: HashMap<String, String>,
    pub libcrypt: HashMap<String, LibCryptRecord>,
    pub themes: HashMap<String, ThemeRecord>,
    pub ppf_fixes: HashMap<String, PatchRecord>,
    ppf_data: HashMap<String, String>,
    libcrypt_patches: HashMap<String, String>,
    libcrypt_hash_patches: HashMap<String, HashMap<String, Option<String>>>,
    libcrypt_revision_signatures: HashMap<String, LibCryptRevisionSignature>,
}

#[derive(Clone, Debug)]
pub struct DiscMetadata {
    pub id: String,
    pub title: String,
    pub manual: Option<String>,
    pub use_cdda: bool,
    pub pic0_scaling: f64,
    pub pic0_offset: (f64, f64),
}

impl Database {
    pub fn embedded() -> &'static Self {
        static DATABASE: OnceLock<Database> = OnceLock::new();
        DATABASE.get_or_init(|| {
            let games_json = include_str!("../data/games.json");
            let games: HashMap<String, GameRecord> =
                serde_json::from_str(games_json).expect("game database JSON must be valid");
            let game_fields: HashMap<String, HashSet<String>> =
                serde_json::from_str::<HashMap<String, serde_json::Value>>(games_json)
                    .expect("game database JSON must be valid")
                    .into_iter()
                    .map(|(id, record)| {
                        let fields = record
                            .as_object()
                            .map(|object| object.keys().cloned().collect())
                            .unwrap_or_default();
                        (id, fields)
                    })
                    .collect();
            Self {
                discs: serde_json::from_str(include_str!("../data/disc_by_md5.json"))
                    .expect("disc database JSON must be valid"),
                games,
                game_fields,
                psp_configs: serde_json::from_str(include_str!("../data/psp_configs.json"))
                    .expect("PSP config database JSON must be valid"),
                libcrypt: serde_json::from_str(include_str!("../data/libcrypt.json"))
                    .expect("LibCrypt database JSON must be valid"),
                themes: serde_json::from_str(include_str!("../data/themes.json"))
                    .expect("theme database JSON must be valid"),
                ppf_fixes: serde_json::from_str(include_str!("../data/ppf_fixes.json"))
                    .expect("PPF database JSON must be valid"),
                ppf_data: serde_json::from_str(include_str!("../data/ppf_data.json"))
                    .expect("PPF data JSON must be valid"),
                libcrypt_patches: serde_json::from_str(include_str!(
                    "../data/libcrypt_patches.json"
                ))
                .expect("LibCrypt patch map JSON must be valid"),
                libcrypt_hash_patches: serde_json::from_str(include_str!(
                    "../data/libcrypt_hash_patches.json"
                ))
                .expect("LibCrypt revision patch map JSON must be valid"),
                libcrypt_revision_signatures: serde_json::from_str(include_str!(
                    "../data/libcrypt_revision_signatures.json"
                ))
                .expect("LibCrypt revision signatures JSON must be valid"),
            }
        })
    }

    pub fn game_has_field(&self, game_id: &str, field: &str) -> bool {
        self.game_fields
            .get(game_id)
            .is_some_and(|fields| fields.contains(field))
    }

    pub fn identify(&self, cue: &CueSheet) -> Result<String, DiscError> {
        self.identify_cancellable(cue, &AtomicBool::new(false))
    }

    pub fn identify_cancellable(
        &self,
        cue: &CueSheet,
        cancelled: &AtomicBool,
    ) -> Result<String, DiscError> {
        let hash = cue.first_track_md5_cancellable(cancelled)?;
        if let Some(record) = self.discs.get(&hash) {
            return Ok(record.id.clone());
        }

        let cnf = cue.system_cnf()?;
        let ids: Vec<String> = self.games.keys().cloned().collect();
        game_id_from_system_cnf(&cnf, &ids).ok_or(DiscError::UnknownId)
    }

    pub fn analyze_image(&self, path: &std::path::Path) -> Result<DiscMetadata, DiscError> {
        self.analyze_image_cancellable(path, &AtomicBool::new(false))
    }

    pub fn analyze_image_cancellable(
        &self,
        path: &std::path::Path,
        cancelled: &AtomicBool,
    ) -> Result<DiscMetadata, DiscError> {
        let cue = CueSheet::open_image_cancellable(path, cancelled)?;
        let id = match self.identify_cancellable(&cue, cancelled) {
            Ok(id) => id,
            Err(DiscError::UnknownId) => "UNKN00000".to_string(),
            Err(error) => return Err(error),
        };
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(DiscError::Cancelled);
        }
        let game = self.games.get(&id);
        let title = self
            .title(&id)
            .map(str::to_owned)
            .unwrap_or_else(|| id.clone());
        Ok(DiscMetadata {
            id,
            title,
            manual: local_manual_for_image(path)
                .or_else(|| game.and_then(|record| record.manual.clone())),
            use_cdda: game.and_then(|record| record.psp_use_cdda).unwrap_or(false),
            pic0_scaling: game
                .and_then(|record| record.pic0_scaling)
                .unwrap_or(0.9)
                .clamp(0.1, 3.0),
            pic0_offset: game
                .and_then(|record| record.pic0_offset)
                .unwrap_or((0.1, 0.1)),
        })
    }

    pub fn title(&self, id: &str) -> Option<&str> {
        self.games
            .get(id)
            .and_then(|record| record.title.as_deref())
            .or_else(|| {
                self.discs
                    .values()
                    .find(|record| record.id == id)
                    .and_then(|record| record.title.as_deref())
            })
    }

    pub fn psp_config(&self, id: &str) -> Option<Vec<u8>> {
        let encoded = self.psp_configs.get(id)?;
        decode_hex(encoded)
    }

    pub fn psp_patch(&self, id: &str, md5: &str) -> Option<Vec<u8>> {
        let record = self.ppf_fixes.get(id)?;
        if record
            .tags
            .as_ref()
            .is_some_and(|tags| !tags.iter().any(|tag| tag == "psp"))
        {
            return None;
        }
        let mut path = record.ppf.as_deref();
        if let Some(override_record) = record.hashes.as_ref().and_then(|hashes| hashes.get(md5)) {
            if let Some(override_path) = override_record.get("ppf") {
                path = Some(override_path);
            }
        }
        self.ppf_data.get(path?).and_then(|hex| decode_hex(hex))
    }

    pub fn libcrypt_patch(&self, id: &str) -> Option<Vec<u8>> {
        let path = self
            .libcrypt
            .get(id)
            .and_then(|record| record.ppf.as_deref())
            .or_else(|| self.libcrypt_patches.get(id).map(String::as_str))?;
        self.ppf_data.get(path).and_then(|hex| decode_hex(hex))
    }

    pub fn has_libcrypt_revision_data(&self, id: &str) -> bool {
        self.libcrypt_hash_patches.contains_key(id)
            || self.libcrypt_revision_signatures.contains_key(id)
    }

    /// Returns `Some(None)` when the signature identifies a revision that needs no patch.
    pub fn libcrypt_patch_for_signature(
        &self,
        id: &str,
        image: &File,
    ) -> io::Result<Option<Option<Vec<u8>>>> {
        let Some(signature) = self.libcrypt_revision_signatures.get(id) else {
            return Ok(None);
        };
        let mut image = image.try_clone()?;
        image.seek(SeekFrom::Start(signature.offset))?;
        let mut byte = [0u8; 1];
        match image.read_exact(&mut byte) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }
        let patch_path = if byte[0] == signature.old_byte {
            &signature.old_patch
        } else if byte[0] == signature.new_byte {
            &signature.new_patch
        } else {
            return Ok(None);
        };
        Ok(Some(patch_path.as_deref().and_then(|path| {
            self.ppf_data.get(path).and_then(|hex| decode_hex(hex))
        })))
    }

    /// Returns `Some(None)` when a known image revision intentionally needs no patch.
    pub fn libcrypt_patch_for_hash(&self, id: &str, md5: &str) -> Option<Option<Vec<u8>>> {
        let path = self.libcrypt_hash_patches.get(id)?.get(md5)?;
        Some(
            path.as_deref()
                .and_then(|path| self.ppf_data.get(path).and_then(|hex| decode_hex(hex))),
        )
    }

    pub fn psp_xdelta(&self, id: &str, md5: &str) -> Option<Vec<u8>> {
        let record = self.ppf_fixes.get(id)?;
        if record
            .tags
            .as_ref()
            .is_some_and(|tags| !tags.iter().any(|tag| tag == "psp"))
        {
            return None;
        }
        let patch = record.hashes.as_ref()?.get(md5)?.get("xdelta")?;
        self.ppf_data.get(patch).and_then(|hex| decode_hex(hex))
    }
}

fn local_manual_for_image(image: &std::path::Path) -> Option<String> {
    let stem = image.file_stem()?;
    let mut base = image.to_path_buf();
    base.set_file_name(stem);
    for suffix in [
        ".manual.zip",
        ".manual.cbz",
        ".manual.cbr",
        ".manual.rar",
        ".manual.pdf",
        ".manual.dat",
        ".manual",
    ] {
        let candidate = base.with_file_name(format!("{}{}", stem.to_string_lossy(), suffix));
        if candidate.exists() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

fn decode_hex(input: &str) -> Option<Vec<u8>> {
    if input.len() % 2 != 0 {
        return None;
    }
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)? as u8;
            let low = (pair[1] as char).to_digit(16)? as u8;
            Some((high << 4) | low)
        })
        .collect()
}
