//! Download and normalize known game artwork for embedding in PSP EBOOT files.
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gdk_pixbuf::{InterpType, Pixbuf};
use reqwest::blocking::Client;

use crate::create::PbpAssets;
use crate::database::Database;

const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

pub fn load_game_assets(game_id: &str) -> PbpAssets {
    let Some(game) = Database::embedded().games.get(game_id) else {
        return PbpAssets::default();
    };
    let directory = cache_directory().join("assets");
    if fs::create_dir_all(&directory).is_err() {
        return PbpAssets::default();
    }
    let client = match Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("POP-FE-PSP/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(_) => return PbpAssets::default(),
    };
    let pic1 = game
        .pic1
        .as_deref()
        .and_then(|url| cached_image(&client, url, &directory, game_id, "pic1", (480, 272)));
    let pic0 = game
        .pic0
        .as_deref()
        .and_then(|url| cached_image(&client, url, &directory, game_id, "pic0", (310, 180)));
    let icon_url = game.icon0.as_deref();
    let icon0 = icon_url
        .and_then(|url| cached_image(&client, url, &directory, game_id, "icon0", (80, 80)))
        .or_else(|| {
            let source = pic1.as_ref()?;
            let destination = directory.join(format!("{}-icon0.png", safe_id(game_id)));
            normalize_cached(source, &destination, (80, 80)).then_some(destination)
        });
    PbpAssets {
        icon0,
        pic0,
        pic1,
        ..PbpAssets::default()
    }
}

fn cached_image(
    client: &Client,
    url: &str,
    directory: &Path,
    game_id: &str,
    name: &str,
    size: (i32, i32),
) -> Option<PathBuf> {
    let path = directory.join(format!("{}-{name}.png", safe_id(game_id)));
    if path.is_file() {
        return Some(path);
    }
    let response = client.get(url).send().ok()?.error_for_status().ok()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
    {
        return None;
    }
    let bytes = response.bytes().ok()?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let pixbuf = Pixbuf::from_read(Cursor::new(bytes)).ok()?;
    if pixbuf.width() > 8000 || pixbuf.height() > 8000 {
        return None;
    }
    save_scaled(&pixbuf, &path, size).ok()?;
    Some(path)
}

fn normalize_cached(source: &Path, destination: &Path, size: (i32, i32)) -> bool {
    if destination.is_file() {
        return true;
    }
    let Ok(pixbuf) = Pixbuf::from_file(source) else {
        return false;
    };
    save_scaled(&pixbuf, destination, size).is_ok()
}

fn save_scaled(
    pixbuf: &Pixbuf,
    destination: &Path,
    size: (i32, i32),
) -> Result<(), Box<dyn std::error::Error>> {
    let scaled = pixbuf
        .scale_simple(size.0, size.1, InterpType::Bilinear)
        .ok_or("could not resize image")?;
    let bytes = scaled.save_to_bufferv("png", &[])?;
    let temporary = destination.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, destination)?;
    Ok(())
}

fn cache_directory() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".cache"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("org.popfe.PSPManager")
}

fn safe_id(id: &str) -> String {
    id.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}
