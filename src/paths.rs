use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ConversionPreferences {
    pub cdda: bool,
    pub disable_libcrypt_patching: bool,
    pub force_ntsc: bool,
    pub allow_disc_swapping: bool,
    pub disable_title_image: bool,
    pub disable_pic0: bool,
    pub disable_pic1: bool,
    pub use_pic1_logo: bool,
    pub watermark: bool,
    pub psx_undither: bool,
    pub disable_asset_downloads: bool,
    pub disable_snd0: bool,
    pub youtube_cookies: String,
    pub ntsc_u_icon0: bool,
    pub theme: u32,
    pub psp_root: String,
}

impl ConversionPreferences {
    pub fn load() -> Self {
        fs::read(preferences_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> bool {
        let settings = preferences_path();
        let Some(parent) = settings.parent() else {
            return false;
        };
        if fs::create_dir_all(parent).is_err() {
            return false;
        }
        let temporary = settings.with_extension(format!("{}.tmp", std::process::id()));
        if serde_json::to_vec(self).is_ok_and(|bytes| fs::write(&temporary, bytes).is_ok())
            && fs::rename(&temporary, &settings).is_ok()
        {
            true
        } else {
            let _ = fs::remove_file(temporary);
            false
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Preferences {
    last_directory: PathBuf,
}

#[derive(Clone, Debug)]
pub struct FolderHistory {
    settings: PathBuf,
    home: PathBuf,
    last_directory: PathBuf,
}

impl FolderHistory {
    pub fn load() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| PathBuf::from("/"));
        let settings = app_config_dir(&home).join("dialogs.json");
        let saved = fs::read(&settings)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Preferences>(&bytes).ok())
            .map(|preferences| preferences.last_directory)
            .filter(|path| path.is_absolute() && Self::accessible(path));
        let last_directory = saved.unwrap_or_else(|| home.clone());
        Self {
            settings,
            home,
            last_directory,
        }
    }

    pub fn current(&self) -> &Path {
        if Self::accessible(&self.last_directory) {
            &self.last_directory
        } else {
            &self.home
        }
    }

    pub fn remember_file(&mut self, selected: &Path) -> bool {
        let Some(directory) = selected.parent() else {
            return false;
        };
        self.remember_directory(directory)
    }

    pub fn remember_directory(&mut self, directory: &Path) -> bool {
        if !directory.is_absolute() || !Self::accessible(directory) {
            return false;
        }
        self.last_directory = directory.to_path_buf();
        if let Some(parent) = self.settings.parent()
            && fs::create_dir_all(parent).is_ok()
        {
            let temporary = self
                .settings
                .with_extension(format!("{}.tmp", std::process::id()));
            let preferences = Preferences {
                last_directory: self.last_directory.clone(),
            };
            if let Ok(bytes) = serde_json::to_vec(&preferences)
                && fs::write(&temporary, bytes).is_ok()
                && fs::rename(&temporary, &self.settings).is_ok()
            {
                return true;
            }
            let _ = fs::remove_file(temporary);
        }
        false
    }

    fn accessible(path: &Path) -> bool {
        path.is_dir() && fs::read_dir(path).is_ok()
    }
}

fn preferences_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"));
    app_config_dir(&home).join("preferences.json")
}

fn app_config_dir(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("org.popfe.PSPManager")
}
