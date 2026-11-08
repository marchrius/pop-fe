use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
        let config_root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let settings = config_root
            .join("org.popfe.PSPManager")
            .join("dialogs.json");
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
