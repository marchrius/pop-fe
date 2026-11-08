use std::collections::HashMap;
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
    psp_configs: HashMap<String, String>,
    pub libcrypt: HashMap<String, LibCryptRecord>,
    pub themes: HashMap<String, ThemeRecord>,
    pub ppf_fixes: HashMap<String, PatchRecord>,
    ppf_data: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct DiscMetadata {
    pub id: String,
    pub title: String,
    pub manual: Option<String>,
    pub use_cdda: bool,
}

impl Database {
    pub fn embedded() -> &'static Self {
        static DATABASE: OnceLock<Database> = OnceLock::new();
        DATABASE.get_or_init(|| Self {
            discs: serde_json::from_str(include_str!("../data/disc_by_md5.json"))
                .expect("disc database JSON must be valid"),
            games: serde_json::from_str(include_str!("../data/games.json"))
                .expect("game database JSON must be valid"),
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
        })
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
        let path = self.libcrypt.get(id)?.ppf.as_deref()?;
        self.ppf_data.get(path).and_then(|hex| decode_hex(hex))
    }

    pub fn psp_xdelta(&self, id: &str, md5: &str) -> Option<&'static [u8]> {
        let record = self.ppf_fixes.get(id)?;
        if record
            .tags
            .as_ref()
            .is_some_and(|tags| !tags.iter().any(|tag| tag == "psp"))
        {
            return None;
        }
        let patch = record.hashes.as_ref()?.get(md5)?.get("xdelta")?;
        match patch.as_str() {
            "romhacks/Septentrion_videofix_pspvita.xdelta" => Some(include_bytes!(
                "../romhacks/Septentrion_videofix_pspvita.xdelta"
            )),
            _ => None,
        }
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
