use std::collections::HashMap;
use std::sync::OnceLock;

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
    pub libcrypt: HashMap<String, LibCryptRecord>,
    pub themes: HashMap<String, ThemeRecord>,
    pub ppf_fixes: HashMap<String, PatchRecord>,
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
            libcrypt: serde_json::from_str(include_str!("../data/libcrypt.json"))
                .expect("LibCrypt database JSON must be valid"),
            themes: serde_json::from_str(include_str!("../data/themes.json"))
                .expect("theme database JSON must be valid"),
            ppf_fixes: serde_json::from_str(include_str!("../data/ppf_fixes.json"))
                .expect("PPF database JSON must be valid"),
        })
    }

    pub fn identify(&self, cue: &CueSheet) -> Result<String, DiscError> {
        let hash = cue.first_track_md5()?;
        if let Some(record) = self.discs.get(&hash) {
            return Ok(record.id.clone());
        }

        let cnf = cue.system_cnf()?;
        let ids: Vec<String> = self.games.keys().cloned().collect();
        game_id_from_system_cnf(&cnf, &ids).ok_or(DiscError::UnknownId)
    }

    pub fn analyze_image(&self, path: &std::path::Path) -> Result<DiscMetadata, DiscError> {
        let cue = CueSheet::open_image(path)?;
        let id = self.identify(&cue)?;
        let game = self.games.get(&id);
        let title = self
            .title(&id)
            .map(str::to_owned)
            .unwrap_or_else(|| id.clone());
        Ok(DiscMetadata {
            id,
            title,
            manual: game.and_then(|record| record.manual.clone()),
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
}
