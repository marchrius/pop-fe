use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const APP_DIRECTORY: &str = "org.popfe.PSPManager";

pub fn home_dir() -> PathBuf {
    #[cfg(windows)]
    let candidates = ["USERPROFILE", "HOME"];
    #[cfg(not(windows))]
    let candidates = ["HOME", "USERPROFILE"];

    candidates
        .iter()
        .filter_map(|name| std::env::var_os(name))
        .map(PathBuf::from)
        .find(|path| path.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
}

pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    let platform_dir = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let platform_dir = Some(home_dir().join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let platform_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| Some(home_dir().join(".config")));

    platform_dir
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home_dir())
        .join(APP_DIRECTORY)
}

pub fn cache_dir() -> PathBuf {
    #[cfg(windows)]
    let platform_dir = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let platform_dir = Some(home_dir().join("Library/Caches"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let platform_dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| Some(home_dir().join(".cache")));

    platform_dir
        .filter(|path| path.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIRECTORY)
}

pub fn executable_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if path.extension().is_none() {
        let windows_path = path.with_extension("exe");
        if windows_path.is_file() {
            return windows_path;
        }
    }
    path.to_path_buf()
}

pub fn find_executable(name: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    let name = if name == "7z" { "7zz" } else { name };

    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        let candidate = executable_path(&directory.join(name));
        if candidate.is_file() {
            return candidate;
        }
    }
    if let Some(candidate) = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| executable_path(&directory.join(name)))
        .find(|path| path.is_file())
    {
        return candidate;
    }
    PathBuf::from(name)
}

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
            && replace_file(&temporary, &settings).is_ok()
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
        let home = home_dir();
        let settings = config_dir().join("dialogs.json");
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
                && replace_file(&temporary, &self.settings).is_ok()
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
    config_dir().join("preferences.json")
}

pub(crate) fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)
}
