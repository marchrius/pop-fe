//! Download and normalize known game artwork for embedding in PSP EBOOT files.
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cairo::{Context, FontSlant, FontWeight, Format, ImageSurface};
use gdk_pixbuf::{InterpType, Pixbuf};
use md5::{Digest, Md5};
use reqwest::blocking::Client;

use crate::create::PbpAssets;
use crate::database::Database;

const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_SND0_BYTES: usize = 500_000;
const MAX_THEME_AUDIO_BYTES: usize = 32 * 1024 * 1024;
const NTSC_U_ICON0_FRAME: &[u8] = include_bytes!("../data/psp/icon0_ntsc_u.png");

pub fn frame_ntsc_u_icon0(
    source_path: &Path,
    game_id: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let metadata = fs::metadata(source_path)?;
    if metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err("ICON0 image is empty or exceeds the 32 MB limit".into());
    }
    let source_bytes = fs::read(source_path)?;
    let digest = Md5::digest(&source_bytes);
    let digest = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let directory = cache_directory().join("assets");
    fs::create_dir_all(&directory)?;
    let output = directory.join(format!("{}-icon0-ntsc-u-{digest}.png", safe_id(game_id)));
    if output.is_file() {
        return Ok(output);
    }
    let frame = Pixbuf::from_read(Cursor::new(NTSC_U_ICON0_FRAME))?
        .scale_simple(80, 80, InterpType::Hyper)
        .ok_or("could not scale the NTSC-U ICON0 frame")?;
    let foreground = Pixbuf::from_read(Cursor::new(source_bytes))?
        .apply_embedded_orientation()
        .ok_or("could not read the ICON0 orientation")?
        .scale_simple(60, 67, InterpType::Hyper)
        .ok_or("could not scale the ICON0 cover")?;
    foreground.composite(
        &frame,
        14,
        7,
        60,
        67,
        14.0,
        7.0,
        1.0,
        1.0,
        InterpType::Hyper,
        255,
    );
    let png = frame.save_to_bufferv("png", &[])?;
    let temporary = output.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, png)?;
    fs::rename(temporary, &output)?;
    Ok(output)
}

pub fn watermark_pic1_png(
    png: &[u8],
    title: &str,
    game_id: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut input = Cursor::new(png);
    let surface = ImageSurface::create_from_png(&mut input)?;
    let context = Context::new(&surface)?;
    context.select_font_face("sans", FontSlant::Normal, FontWeight::Normal);
    context.set_font_size(8.0);
    context.set_source_rgba(1.0, 1.0, 1.0, 0.92);
    let mut y = 18.0;
    for line in title.split(" - ").take(8) {
        let extents = context.text_extents(line)?;
        context.move_to(
            (surface.width() as f64 - extents.x_advance() - 4.0).max(1.0),
            y,
        );
        context.show_text(line)?;
        y += extents.height().max(8.0) + 2.0;
    }
    let extents = context.text_extents(game_id)?;
    let x = (surface.width() as f64 - extents.x_advance()).max(0.0);
    let top = (surface.height() as f64 - extents.height() - 2.0).max(0.0);
    context.set_source_rgba(0.0, 0.0, 0.0, 0.92);
    context.rectangle(
        x - 1.0,
        top,
        surface.width() as f64 - x + 1.0,
        surface.height() as f64 - top,
    );
    context.fill()?;
    context.set_source_rgb(1.0, 1.0, 1.0);
    context.move_to(x, surface.height() as f64 - 2.0);
    context.show_text(game_id)?;
    let mut output = Vec::new();
    surface.write_to_png(&mut output)?;
    Ok(output)
}

pub fn load_game_assets(game_id: &str) -> PbpAssets {
    load_game_assets_cancellable(game_id, &AtomicBool::new(false))
}

pub fn load_game_assets_cancellable(game_id: &str, cancelled: &AtomicBool) -> PbpAssets {
    if cancelled.load(Ordering::Relaxed) {
        return PbpAssets::default();
    }
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
    let database = Database::embedded();
    let mut pic1 = game.pic1.as_deref().and_then(|url| {
        let candidates = [popfe_asset_cache_url("pic1", url), url.to_owned()];
        cached_image_candidates(
            &client,
            &candidates,
            &directory,
            game_id,
            "pic1",
            (480, 272),
            Some(cancelled),
        )
    });
    if cancelled.load(Ordering::Relaxed) {
        return PbpAssets::default();
    }
    let mut pic0 = game
        .pic0
        .as_deref()
        .and_then(|url| cached_pic0(&client, url, &directory, game_id, game, Some(cancelled)));
    let mut psx_page = None;
    if pic0.is_none() && (game.pic0.is_some() || !database.game_has_field(game_id, "pic0")) {
        psx_page = cached_psx_page(&client, game, game_id, &directory, Some(cancelled));
        if let Some(path) = psx_page.as_deref().and_then(first_psx_screenshot) {
            pic0 = cached_pic0(
                &client,
                &format!("https://psxdatacenter.com/{path}"),
                &directory,
                game_id,
                game,
                Some(cancelled),
            );
        }
    }
    if pic1.is_none() && (game.pic1.is_some() || !database.game_has_field(game_id, "pic1")) {
        if psx_page.is_none() {
            psx_page = cached_psx_page(&client, game, game_id, &directory, Some(cancelled));
        }
        if let Some(path) = psx_page.as_deref().and_then(first_psx_screenshot) {
            pic1 = cached_image(
                &client,
                &format!("https://psxdatacenter.com/{path}"),
                &directory,
                game_id,
                "pic1",
                (480, 272),
                Some(cancelled),
            );
        }
    }
    let pic0_source = pic0.as_ref().and_then(|_| {
        let path = directory.join(format!("{}-pic0-source.png", safe_id(game_id)));
        path.is_file().then_some(path)
    });
    if cancelled.load(Ordering::Relaxed) {
        return PbpAssets::default();
    }
    let icon_url = game.icon0.as_deref();
    let mut icon0 = icon_url.and_then(|url| {
        cached_image(
            &client,
            url,
            &directory,
            game_id,
            "icon0-v2",
            (80, 80),
            Some(cancelled),
        )
    });
    if icon0.is_none() && icon_url.is_none() && !database.game_has_field(game_id, "icon0") {
        if psx_page.is_none() {
            psx_page = cached_psx_page(&client, game, game_id, &directory, Some(cancelled));
        }
        if let Some(path) = psx_page.as_deref().and_then(first_psx_cover) {
            icon0 = cached_image(
                &client,
                &format!("https://psxdatacenter.com/{path}"),
                &directory,
                game_id,
                "icon0-v2",
                (80, 80),
                Some(cancelled),
            );
        }
    }
    let snd0 = game
        .snd0
        .as_deref()
        .and_then(|link| cached_snd0(&client, link, &directory, game_id, Some(cancelled)));
    PbpAssets {
        icon0,
        pic0,
        pic0_source,
        pic1,
        snd0,
        ..PbpAssets::default()
    }
}

pub fn load_game_assets_for_image_cancellable(
    game_id: &str,
    image_path: &Path,
    cancelled: &AtomicBool,
) -> PbpAssets {
    let mut assets = load_game_assets_cancellable(game_id, cancelled);
    if !cancelled.load(Ordering::Relaxed) {
        apply_local_sidecars(game_id, image_path, &mut assets, cancelled);
    }
    assets
}

pub fn load_game_assets_cached(game_id: &str) -> PbpAssets {
    let Some(game) = Database::embedded().games.get(game_id) else {
        return PbpAssets::default();
    };
    let directory = cache_directory().join("assets");
    let pic1 = cached_image_path(&directory, game_id, "pic1", (480, 272));
    let pic0_path = directory.join(format!("{}-pic0-layout-v3.png", safe_id(game_id)));
    let pic0 = Pixbuf::from_file(&pic0_path)
        .ok()
        .filter(|image| image.width() == 1000 && image.height() == 560)
        .map(|_| pic0_path);
    let pic0_source = directory
        .join(format!("{}-pic0-source.png", safe_id(game_id)))
        .is_file()
        .then(|| directory.join(format!("{}-pic0-source.png", safe_id(game_id))));
    let icon0 = cached_image_path(&directory, game_id, "icon0-v2", (80, 80));
    let snd0 = game.snd0.as_deref().and_then(|_| {
        let path = directory.join(format!("{}-snd0.at3", safe_id(game_id)));
        fs::read(&path)
            .ok()
            .filter(|bytes| valid_snd0(bytes))
            .map(|_| path)
    });
    PbpAssets {
        icon0,
        pic0,
        pic0_source,
        pic1,
        snd0,
        ..PbpAssets::default()
    }
}

pub fn load_game_assets_cached_for_image(game_id: &str, image_path: &Path) -> PbpAssets {
    let mut assets = load_game_assets_cached(game_id);
    apply_local_sidecars(game_id, image_path, &mut assets, &AtomicBool::new(false));
    assets
}

fn apply_local_sidecars(
    game_id: &str,
    image_path: &Path,
    assets: &mut PbpAssets,
    cancelled: &AtomicBool,
) {
    let Some(stem) = image_path.file_stem().map(|stem| stem.to_string_lossy()) else {
        return;
    };
    let sibling = |suffix: &str| image_path.with_file_name(format!("{stem}_{suffix}.png"));
    let icon = sibling("cover");
    if let Some(path) = local_artwork_png(&icon, game_id, "local-icon0", cancelled) {
        assets.icon0 = Some(path);
    }
    if cancelled.load(Ordering::Relaxed) {
        return;
    }
    let pic0_source = sibling("pic0");
    if pic0_source.is_file() {
        let game = Database::embedded().games.get(game_id);
        let scaling = game.and_then(|record| record.pic0_scaling).unwrap_or(0.9);
        let offset = game
            .and_then(|record| record.pic0_offset)
            .unwrap_or((0.1, 0.1));
        if let Some(path) = render_pic0_layout(&pic0_source, game_id, scaling, offset) {
            assets.pic0_source = Some(pic0_source);
            assets.pic0 = Some(path);
        }
    }
    if cancelled.load(Ordering::Relaxed) {
        return;
    }
    let pic1 = sibling("pic1");
    if let Some(path) = local_artwork_png(&pic1, game_id, "local-pic1", cancelled) {
        assets.pic1 = Some(path);
    }
    let local_snd0 = image_path.with_extension("snd0");
    if local_snd0.is_file() {
        assets.snd0 = Some(local_snd0);
        assets.local_snd0 = true;
    }
}

fn local_artwork_png(
    source: &Path,
    game_id: &str,
    role: &str,
    cancelled: &AtomicBool,
) -> Option<PathBuf> {
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let metadata = fs::metadata(source).ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES as u64 {
        return None;
    }
    let bytes = fs::read(source).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let digest = Md5::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let pixbuf = Pixbuf::from_read(Cursor::new(bytes)).ok()?;
    if pixbuf.width() <= 0
        || pixbuf.height() <= 0
        || pixbuf.width() > 8000
        || pixbuf.height() > 8000
        || pixbuf.width() as i64 * pixbuf.height() as i64 > 50_000_000
    {
        return None;
    }
    let directory = cache_directory().join("assets");
    fs::create_dir_all(&directory).ok()?;
    let path = directory.join(format!("{}-{role}-{digest}.png", safe_id(game_id)));
    if path.is_file() {
        return Some(path);
    }
    let png = pixbuf.save_to_bufferv("png", &[]).ok()?;
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, png).ok()?;
    fs::rename(&temporary, &path).ok()?;
    Some(path)
}

pub fn load_theme_assets(game_id: &str, theme: &str, base: &PbpAssets) -> PbpAssets {
    load_theme_assets_cancellable(game_id, theme, base, &AtomicBool::new(false))
}

pub fn load_theme_assets_cancellable(
    game_id: &str,
    theme: &str,
    base: &PbpAssets,
    cancelled: &AtomicBool,
) -> PbpAssets {
    if cancelled.load(Ordering::Relaxed) {
        return base.clone();
    }
    let Some(theme_info) = Database::embedded().themes.get(theme) else {
        return base.clone();
    };
    let Some(repository) = theme_info.url.as_deref() else {
        return procedural_theme_assets(game_id, theme, base);
    };
    let directory = cache_directory().join("themes");
    if fs::create_dir_all(&directory).is_err() {
        return base.clone();
    }
    let Ok(client) = Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("POP-FE-PSP/", env!("CARGO_PKG_VERSION")))
        .build()
    else {
        return base.clone();
    };
    let mut result = base.clone();
    for (key, file, size) in [
        ("icon0", "ICON0.PNG", (80, 80)),
        ("pic0-v3", "PIC0.PNG", (1000, 560)),
        ("pic1", "PIC1.PNG", (480, 272)),
    ] {
        let lowercase = file.to_ascii_lowercase();
        let mut loaded = None;
        for filename in [file, lowercase.as_str()] {
            let url = format!(
                "{}/raw/main/data/{}/{}",
                repository.trim_end_matches('/'),
                game_id,
                filename
            );
            loaded = cached_image(
                &client,
                &url,
                &directory,
                &format!("{game_id}-{theme}"),
                key,
                size,
                Some(cancelled),
            );
            if loaded.is_some() || cancelled.load(Ordering::Relaxed) {
                break;
            }
        }
        if let Some(path) = loaded {
            match key {
                "pic0-v3" => result.pic0 = Some(path),
                "pic1" => result.pic1 = Some(path),
                "icon0" => result.icon0 = Some(path),
                _ => {}
            }
        }
        if cancelled.load(Ordering::Relaxed) {
            return base.clone();
        }
    }
    if !base.local_snd0 {
        if let Some(path) =
            cached_theme_snd0(&client, repository, game_id, theme, &directory, cancelled)
        {
            result.snd0 = Some(path);
        }
    }
    result
}

pub fn load_theme_assets_offline(game_id: &str, theme: &str, base: &PbpAssets) -> PbpAssets {
    let mut result = base.clone();
    let cached_game_assets = load_game_assets_cached(game_id);
    merge_missing_assets(&mut result, &cached_game_assets);
    let Some(theme_info) = Database::embedded().themes.get(theme) else {
        return result;
    };
    if theme_info.url.is_none() {
        return procedural_theme_assets(game_id, theme, &result);
    }
    let directory = cache_directory().join("themes");
    for (key, size) in [
        ("icon0", (80, 80)),
        ("pic0-v3", (1000, 560)),
        ("pic1", (480, 272)),
    ] {
        if let Some(path) = cached_image_path(&directory, &format!("{game_id}-{theme}"), key, size)
        {
            match key {
                "icon0" => result.icon0 = Some(path),
                "pic0-v3" => result.pic0 = Some(path),
                "pic1" => result.pic1 = Some(path),
                _ => {}
            }
        }
    }
    let audio = directory.join(format!("{}-{}-snd0.wav", safe_id(game_id), safe_id(theme)));
    if !base.local_snd0 && valid_wave_file(&audio) {
        result.snd0 = Some(audio);
    }
    result
}

fn merge_missing_assets(target: &mut PbpAssets, fallback: &PbpAssets) {
    if target.icon0.is_none() {
        target.icon0.clone_from(&fallback.icon0);
    }
    if target.icon1.is_none() {
        target.icon1.clone_from(&fallback.icon1);
    }
    if target.pic0.is_none() {
        target.pic0.clone_from(&fallback.pic0);
    }
    if target.pic0_source.is_none() {
        target.pic0_source.clone_from(&fallback.pic0_source);
    }
    if target.pic1.is_none() {
        target.pic1.clone_from(&fallback.pic1);
    }
    if target.snd0.is_none() {
        target.snd0.clone_from(&fallback.snd0);
    }
    if target.logo.is_none() {
        target.logo.clone_from(&fallback.logo);
    }
}

fn cached_image_path(
    directory: &Path,
    game_id: &str,
    name: &str,
    size: (i32, i32),
) -> Option<PathBuf> {
    let path = directory.join(format!("{}-{name}.png", safe_id(game_id)));
    let image = Pixbuf::from_file(&path).ok()?;
    (image.width() == size.0 && image.height() == size.1).then_some(path)
}

fn valid_wave_file(path: &Path) -> bool {
    let mut header = [0u8; 12];
    fs::metadata(path).ok().is_some_and(|metadata| {
        metadata.len() >= 12 && metadata.len() <= MAX_THEME_AUDIO_BYTES as u64
    }) && fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok()
        && &header[..4] == b"RIFF"
        && &header[8..] == b"WAVE"
}

fn procedural_theme_assets(game_id: &str, theme: &str, base: &PbpAssets) -> PbpAssets {
    let directory = cache_directory().join("themes").join("generated-v4");
    if fs::create_dir_all(&directory).is_err() {
        return base.clone();
    }
    let title = Database::embedded().title(game_id).unwrap_or(game_id);
    let pic0_path = directory.join(format!("{}-{theme}-pic0.png", safe_id(game_id)));
    let pic1_path = directory.join(format!("{}-{theme}-pic1.png", safe_id(game_id)));
    let mut result = base.clone();
    if !pic0_path.is_file() && render_theme_pic0(&pic0_path, title, theme).is_err() {
        return result;
    }
    if !pic1_path.is_file() {
        let icon = base.icon0.as_deref().or(base.pic1.as_deref());
        if let Some(icon) = icon {
            if render_theme_pic1(&pic1_path, icon, theme).is_err() {
                return result;
            }
        }
    }
    result.pic0 = Some(pic0_path);
    if pic1_path.is_file() {
        result.pic1 = Some(pic1_path);
    }
    result
}

fn render_theme_pic0(
    path: &Path,
    title: &str,
    theme: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if theme == "DOTPAINTING" {
        return render_dotpainting_pic0(path, title);
    }
    let surface = ImageSurface::create(Format::ARgb32, 1000, 560)?;
    let context = Context::new(&surface)?;
    context.scale(4.0, 4.0);
    context.set_operator(cairo::Operator::Source);
    context.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    context.paint()?;
    context.set_operator(cairo::Operator::Over);
    context.select_font_face("monospace", FontSlant::Normal, FontWeight::Normal);
    context.set_font_size(12.0);
    let ascii = theme == "ASCIIART";
    let (r, g, b, origin) = if ascii {
        (128.0 / 255.0, 1.0, 128.0 / 255.0, 80.0)
    } else {
        (
            1.0,
            128.0 / 255.0,
            128.0 / 255.0,
            if theme == "DOTPAINTING" { 80.0 } else { 100.0 },
        )
    };
    let line_height = context.font_extents()?.height() + 2.0;
    let draw_text = |text: &str, x: f64, y: f64| -> Result<(), cairo::Error> {
        let extents = context.text_extents(text)?;
        context.move_to(x - extents.x_bearing(), y - extents.y_bearing());
        context.show_text(text)
    };
    context.set_source_rgb(r, g, b);
    draw_text("##############################", origin, 1.0)?;
    let mut y = 1.0 + line_height;
    for line in title.split(" - ") {
        if ascii {
            context.set_source_rgb(r, g, b);
            draw_text("#", origin, y)?;
            context.set_source_rgb(0.0, 0.0, 0.0);
            draw_text(line, origin + 12.0, y)?;
            context.set_source_rgb(1.0, 1.0, 1.0);
            draw_text(line, origin + 11.0, y - 1.0)?;
        } else {
            context.set_source_rgb(r, g, b);
            draw_text("#", origin, y)?;
            context.set_source_rgb(1.0, 1.0, 1.0);
            draw_text(line, origin + 11.0, y)?;
        }
        y += line_height;
    }
    context.set_source_rgb(r, g, b);
    draw_text("##############################", origin, y)?;

    let mut png = Vec::new();
    surface.write_to_png(&mut png)?;
    fs::write(path, png)?;
    Ok(())
}

fn render_dotpainting_pic0(path: &Path, title: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut source = ImageSurface::create(Format::ARgb32, 250, 140)?;
    let context = Context::new(&source)?;
    context.set_source_rgb(0.0, 0.0, 0.0);
    context.paint()?;
    context.select_font_face("monospace", FontSlant::Normal, FontWeight::Normal);
    context.set_font_size(12.0);
    let line_height = context.font_extents()?.height() + 2.0;
    let draw_text = |text: &str, x: f64, y: f64| -> Result<(), cairo::Error> {
        let extents = context.text_extents(text)?;
        context.move_to(x - extents.x_bearing(), y - extents.y_bearing());
        context.show_text(text)
    };
    context.set_source_rgb(1.0, 128.0 / 255.0, 128.0 / 255.0);
    draw_text("##############################", 80.0, 1.0)?;
    let mut y = 1.0 + line_height;
    for line in title.split(" - ") {
        context.set_source_rgb(1.0, 128.0 / 255.0, 128.0 / 255.0);
        draw_text("#", 80.0, y)?;
        context.set_source_rgb(1.0, 1.0, 1.0);
        draw_text(line, 91.0, y)?;
        y += line_height;
    }
    context.set_source_rgb(1.0, 128.0 / 255.0, 128.0 / 255.0);
    draw_text("##############################", 80.0, y)?;
    source.flush();

    let stride = source.stride() as usize;
    let ink = {
        let data = source.data()?;
        let mut ink = Vec::new();
        for y in 0..140usize {
            for x in 0..250usize {
                let offset = y * stride + x * 4;
                let pixel = u32::from_ne_bytes(data[offset..offset + 4].try_into()?);
                let (red, green, blue) = (
                    ((pixel >> 16) & 0xff) as u8,
                    ((pixel >> 8) & 0xff) as u8,
                    (pixel & 0xff) as u8,
                );
                if red != 0 || green != 0 || blue != 0 {
                    ink.push((x, y, red, green, blue));
                }
            }
        }
        ink
    };

    let output = ImageSurface::create(Format::ARgb32, 1000, 560)?;
    let context = Context::new(&output)?;
    context.set_operator(cairo::Operator::Source);
    context.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    context.paint()?;
    context.set_operator(cairo::Operator::Over);
    for (x, y, red, green, blue) in ink {
        context.set_source_rgb(
            red as f64 / 255.0,
            green as f64 / 255.0,
            blue as f64 / 255.0,
        );
        context.arc(
            x as f64 * 4.0 + 1.5,
            y as f64 * 4.0 + 1.5,
            2.0,
            0.0,
            std::f64::consts::TAU,
        );
        context.fill()?;
    }
    write_png(output, path)
}

fn render_theme_pic1(
    path: &Path,
    source: &Path,
    theme: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let original = Pixbuf::from_file(source)?;
    let painterly = matches!(theme, "OILPAINTING" | "WATERCOLOR" | "COLORSKETCH");
    // The original filters process a 1000×1000 icon before it is composited
    // into the 1920×1080 PIC1. At the final 480×272 size that corresponds to
    // 250×250 pixels; retain that working resolution instead of filtering a
    // 120×120 thumbnail and magnifying its artifacts.
    let icon_size = if painterly { 250 } else { 120 };
    let icon = original
        .scale_simple(icon_size, icon_size, InterpType::Bilinear)
        .ok_or("could not resize theme icon")?;
    let bytes = icon.read_pixel_bytes();
    let pixels = bytes.as_ref();
    let channels = icon.n_channels() as usize;
    let stride = icon.rowstride() as usize;
    let surface = ImageSurface::create(Format::ARgb32, 480, 272)?;
    let context = Context::new(&surface)?;
    context.set_source_rgb(0.0, 0.0, 0.0);
    context.paint()?;
    if theme == "ASCIIART" {
        let shades = ["W", "#", "%", "?", "*", "+", ";", ":", ",", ".", " "];
        context.select_font_face("monospace", FontSlant::Normal, FontWeight::Normal);
        context.set_font_size(3.0);
        context.set_source_rgb(0.75, 0.75, 0.75);
        for y in 0..120usize {
            for x in 0..120usize {
                let p = pixel(pixels, stride, channels, x, y);
                let gray = (p[0] as u16 * 30 + p[1] as u16 * 59 + p[2] as u16 * 11) / 100;
                let shade = 10 - (gray as usize / 26).min(10);
                context.move_to(52.5 + x as f64 * 2.25, y as f64 * 2.25);
                context.show_text(shades[shade])?;
            }
        }
    } else if theme == "DOTPAINTING" {
        for y in 0..120usize {
            for x in 0..120usize {
                let p = pixel(pixels, stride, channels, x, y);
                context.set_source_rgb(
                    p[0] as f64 / 255.0,
                    p[1] as f64 / 255.0,
                    p[2] as f64 / 255.0,
                );
                context.arc(
                    0.125 + x as f64 * 2.25,
                    0.125 + y as f64 * 2.25,
                    0.875,
                    0.0,
                    std::f64::consts::TAU,
                );
                context.fill()?;
            }
        }
    } else {
        let pixel_size = 250.0 / icon_size as f64;
        for y in 0..icon_size as usize {
            for x in 0..icon_size as usize {
                let mut p = pixel(pixels, stride, channels, x, y);
                if painterly {
                    p = painterly_pixel(pixels, stride, channels, x, y, icon_size as usize, theme);
                }
                context.set_source_rgb(
                    p[0] as f64 / 255.0,
                    p[1] as f64 / 255.0,
                    p[2] as f64 / 255.0,
                );
                context.rectangle(
                    25.0 + x as f64 * pixel_size,
                    y as f64 * pixel_size,
                    pixel_size,
                    pixel_size,
                );
                context.fill()?;
            }
        }
    }
    write_png(surface, path)
}

fn pixel(data: &[u8], stride: usize, channels: usize, x: usize, y: usize) -> [u8; 3] {
    let i = y * stride + x * channels;
    [data[i], data[i + 1], data[i + 2]]
}

fn painterly_pixel(
    data: &[u8],
    stride: usize,
    channels: usize,
    x: usize,
    y: usize,
    size: usize,
    theme: &str,
) -> [u8; 3] {
    let mut sum = [0u32; 3];
    let mut count = 0u32;
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            let nx = (x as i32 + dx).clamp(0, size as i32 - 1) as usize;
            let ny = (y as i32 + dy).clamp(0, size as i32 - 1) as usize;
            let p = pixel(data, stride, channels, nx, ny);
            for c in 0..3 {
                sum[c] += p[c] as u32;
            }
            count += 1;
        }
    }
    let mut out = [0; 3];
    for c in 0..3 {
        let avg = (sum[c] / count) as u8;
        out[c] = match theme {
            "OILPAINTING" => (avg / 32) * 32,
            "WATERCOLOR" => {
                ((avg as u16 * 3 + pixel(data, stride, channels, x, y)[c] as u16) / 4 / 16 * 16)
                    as u8
            }
            _ => {
                ((pixel(data, stride, channels, x, y)[c] as u16 * (255 - avg as u16 / 2)) / 255)
                    as u8
            }
        };
    }
    out
}

fn write_png(surface: ImageSurface, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    surface.flush();
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = fs::File::create(&temporary)?;
    surface.write_to_png(&mut file)?;
    drop(file);
    fs::rename(temporary, path)?;
    Ok(())
}

fn cached_psx_page(
    client: &Client,
    game: &crate::database::GameRecord,
    game_id: &str,
    directory: &Path,
    cancelled: Option<&AtomicBool>,
) -> Option<String> {
    const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
    if is_cancelled(cancelled) {
        return None;
    }
    let relative = game.url.as_deref()?;
    if !relative.starts_with("games/") || relative.contains("..") {
        return None;
    }
    let cache = directory.join(format!("{}-psxdatacenter.html", safe_id(game_id)));
    let bytes = if cache.is_file() {
        fs::read(&cache)
            .ok()
            .filter(|bytes| bytes.len() <= MAX_PAGE_BYTES)?
    } else {
        let response = client
            .get(format!("https://psxdatacenter.com/{relative}"))
            .send()
            .ok()?
            .error_for_status()
            .ok()?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_PAGE_BYTES as u64)
        {
            return None;
        }
        let mut bytes = Vec::new();
        read_response(response, MAX_PAGE_BYTES, cancelled, &mut bytes)?;
        if bytes.is_empty() || bytes.len() > MAX_PAGE_BYTES {
            return None;
        }
        let temporary = cache.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary, &bytes).ok()?;
        fs::rename(temporary, &cache).ok()?;
        bytes
    };
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn first_psx_cover(page: &str) -> Option<&str> {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| regex::Regex::new(r"images/covers/./.*/.*\.jpg").unwrap())
        .find(page)
        .map(|matched| matched.as_str())
}

fn first_psx_screenshot(page: &str) -> Option<&str> {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| regex::Regex::new(r"images/screens/./.*/.*/ss..jpg").unwrap())
        .find(page)
        .map(|matched| matched.as_str())
}

fn popfe_asset_cache_url(asset: &str, source_url: &str) -> String {
    let digest = Md5::digest(source_url.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("https://github.com/sahlberg/pop-fe-assets/raw/master/{asset}/{digest}")
}

fn cached_image(
    client: &Client,
    url: &str,
    directory: &Path,
    game_id: &str,
    name: &str,
    size: (i32, i32),
    cancelled: Option<&AtomicBool>,
) -> Option<PathBuf> {
    cached_image_candidates(
        client,
        &[url.to_owned()],
        directory,
        game_id,
        name,
        size,
        cancelled,
    )
}

fn cached_image_candidates(
    client: &Client,
    urls: &[String],
    directory: &Path,
    game_id: &str,
    name: &str,
    size: (i32, i32),
    cancelled: Option<&AtomicBool>,
) -> Option<PathBuf> {
    if is_cancelled(cancelled) {
        return None;
    }
    let path = directory.join(format!("{}-{name}.png", safe_id(game_id)));
    if path.is_file() {
        return Some(path);
    }
    for url in urls {
        if is_cancelled(cancelled) {
            return None;
        }
        let Some(response) = client
            .get(url)
            .send()
            .ok()
            .and_then(|r| r.error_for_status().ok())
        else {
            continue;
        };
        if response
            .content_length()
            .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
        {
            continue;
        }
        let mut bytes = Vec::new();
        if read_response(response, MAX_IMAGE_BYTES, cancelled, &mut bytes).is_none()
            || bytes.is_empty()
            || bytes.len() > MAX_IMAGE_BYTES
        {
            continue;
        }
        let Ok(pixbuf) = Pixbuf::from_read(Cursor::new(bytes)) else {
            continue;
        };
        if pixbuf.width() <= 0
            || pixbuf.height() <= 0
            || pixbuf.width() > 8000
            || pixbuf.height() > 8000
            || pixbuf.width() as i64 * pixbuf.height() as i64 > 50_000_000
        {
            continue;
        }
        if save_scaled(&pixbuf, &path, size).is_ok() {
            return Some(path);
        }
    }
    None
}

fn cached_pic0(
    client: &Client,
    url: &str,
    directory: &Path,
    game_id: &str,
    game: &crate::database::GameRecord,
    cancelled: Option<&AtomicBool>,
) -> Option<PathBuf> {
    if is_cancelled(cancelled) {
        return None;
    }
    let path = directory.join(format!("{}-pic0-layout-v3.png", safe_id(game_id)));
    let source_path = directory.join(format!("{}-pic0-source.png", safe_id(game_id)));
    if source_path.is_file() {
        if path.is_file() {
            return Some(path);
        }
        let source = Pixbuf::from_file(&source_path)
            .ok()?
            .apply_embedded_orientation()?;
        return render_pic0_pixbuf(
            &source,
            &path,
            game.pic0_scaling.unwrap_or(0.9),
            game.pic0_offset.unwrap_or((0.1, 0.1)),
        );
    }
    let candidates = [popfe_asset_cache_url("pic0", url), url.to_owned()];
    let mut source = None;
    for candidate in candidates {
        let Some(response) = client
            .get(candidate)
            .send()
            .ok()
            .and_then(|response| response.error_for_status().ok())
        else {
            continue;
        };
        if response
            .content_length()
            .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
        {
            continue;
        }
        let mut bytes = Vec::new();
        if read_response(response, MAX_IMAGE_BYTES, cancelled, &mut bytes).is_none()
            || bytes.is_empty()
            || bytes.len() > MAX_IMAGE_BYTES
        {
            continue;
        }
        if let Some(pixbuf) = Pixbuf::from_read(Cursor::new(bytes))
            .ok()
            .and_then(|pixbuf| pixbuf.apply_embedded_orientation())
        {
            source = Some(pixbuf);
            break;
        }
    }
    let source = source?;
    let (width, height) = (source.width(), source.height());
    if width <= 0
        || height <= 0
        || width > 8000
        || height > 8000
        || width as i64 * height as i64 > 50_000_000
    {
        return None;
    }
    let source_png = source.save_to_bufferv("png", &[]).ok()?;
    let temporary_source = source_path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary_source, source_png).ok()?;
    fs::rename(temporary_source, &source_path).ok()?;
    render_pic0_pixbuf(
        &source,
        &path,
        game.pic0_scaling.unwrap_or(0.9),
        game.pic0_offset.unwrap_or((0.1, 0.1)),
    )
}

pub fn render_pic0_layout(
    source_path: &Path,
    game_id: &str,
    scaling: f64,
    offset: (f64, f64),
) -> Option<PathBuf> {
    let metadata = fs::metadata(source_path).ok()?;
    if metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES as u64 {
        return None;
    }
    let source = Pixbuf::from_file(source_path)
        .ok()?
        .apply_embedded_orientation()?;
    if source.width() <= 0
        || source.height() <= 0
        || source.width() > 8000
        || source.height() > 8000
        || source.width() as i64 * source.height() as i64 > 50_000_000
    {
        return None;
    }
    let bytes = fs::read(source_path).ok()?;
    let mut key = Md5::new();
    key.update(&bytes);
    let layout = format!("{scaling:.4}:{:.4}:{:.4}", offset.0, offset.1);
    key.update(layout.as_bytes());
    let digest = key
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let directory = cache_directory().join("assets");
    fs::create_dir_all(&directory).ok()?;
    let output = directory.join(format!("{}-pic0-{digest}.png", safe_id(game_id)));
    if output.is_file() {
        return Some(output);
    }
    render_pic0_pixbuf(&source, &output, scaling, offset)
}

fn render_pic0_pixbuf(
    source: &Pixbuf,
    path: &Path,
    scaling: f64,
    offset: (f64, f64),
) -> Option<PathBuf> {
    let fit_scale = (560.0 / source.height() as f64).min(1000.0 / source.width() as f64);
    let scale = fit_scale * scaling.clamp(0.1, 3.0);
    let canvas = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 1000, 560)?;
    canvas.fill(0);
    source.composite(
        &canvas,
        0,
        0,
        1000,
        560,
        offset.0.clamp(-1.0, 2.0) * 1000.0,
        offset.1.clamp(-1.0, 2.0) * 560.0,
        scale,
        scale,
        InterpType::Hyper,
        255,
    );
    let encoded = canvas.save_to_bufferv("png", &[]).ok()?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, encoded).ok()?;
    fs::rename(temporary, &path).ok()?;
    Some(path.to_path_buf())
}

fn cached_snd0(
    client: &Client,
    source_url: &str,
    directory: &Path,
    game_id: &str,
    cancelled: Option<&AtomicBool>,
) -> Option<PathBuf> {
    if is_cancelled(cancelled) {
        return None;
    }
    let hash = Md5::digest(source_url.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = directory.join(format!("{}-snd0.at3", safe_id(game_id)));
    if path.is_file() {
        if fs::read(&path).ok().is_some_and(|bytes| valid_snd0(&bytes)) {
            return Some(path);
        }
        let _ = fs::remove_file(&path);
    }
    let url = format!("https://github.com/sahlberg/pop-fe-assets/raw/master/snd0/{hash}");
    let response = client.get(url).send().ok()?.error_for_status().ok()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SND0_BYTES as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    read_response(response, MAX_SND0_BYTES, cancelled, &mut bytes)?;
    if !valid_snd0(&bytes) {
        return None;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, bytes).ok()?;
    fs::rename(&temporary, &path).ok()?;
    Some(path)
}

fn cached_theme_snd0(
    client: &Client,
    repository: &str,
    game_id: &str,
    theme: &str,
    directory: &Path,
    cancelled: &AtomicBool,
) -> Option<PathBuf> {
    if is_cancelled(Some(cancelled)) {
        return None;
    }
    let path = directory.join(format!("{}-{}-snd0.wav", safe_id(game_id), safe_id(theme)));
    if path.is_file() {
        let mut header = [0u8; 12];
        if fs::File::open(&path)
            .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut header))
            .is_ok()
            && &header[..4] == b"RIFF"
            && &header[8..] == b"WAVE"
            && fs::metadata(&path)
                .ok()
                .is_some_and(|metadata| metadata.len() <= MAX_THEME_AUDIO_BYTES as u64)
        {
            return Some(path);
        }
        let _ = fs::remove_file(&path);
    }
    let url = format!(
        "{}/raw/main/data/{}/SND0.WAV",
        repository.trim_end_matches('/'),
        game_id
    );
    let response = client.get(url).send().ok()?.error_for_status().ok()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_THEME_AUDIO_BYTES as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    read_response(response, MAX_THEME_AUDIO_BYTES, Some(cancelled), &mut bytes)?;
    if bytes.len() < 12
        || &bytes[..4] != b"RIFF"
        || &bytes[8..12] != b"WAVE"
        || bytes.len() > MAX_THEME_AUDIO_BYTES
        || is_cancelled(Some(cancelled))
    {
        return None;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, bytes).ok()?;
    if is_cancelled(Some(cancelled)) {
        let _ = fs::remove_file(&temporary);
        return None;
    }
    fs::rename(temporary, &path).ok()?;
    Some(path)
}

fn is_cancelled(cancelled: Option<&AtomicBool>) -> bool {
    cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

fn read_response(
    response: reqwest::blocking::Response,
    max_bytes: usize,
    cancelled: Option<&AtomicBool>,
    bytes: &mut Vec<u8>,
) -> Option<()> {
    let mut response = response.take((max_bytes + 1) as u64);
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if is_cancelled(cancelled) {
            return None;
        }
        let count = response.read(&mut buffer).ok()?;
        if count == 0 {
            return Some(());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > max_bytes {
            return Some(());
        }
    }
}

fn valid_snd0(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_SND0_BYTES
        || bytes.len() < 12
        || &bytes[..4] != b"RIFF"
        || &bytes[8..12] != b"WAVE"
    {
        return false;
    }
    let Ok(header_size) = <[u8; 4]>::try_from(&bytes[4..8]) else {
        return false;
    };
    let declared_size = u32::from_le_bytes(header_size) as usize;
    if declared_size
        .checked_add(8)
        .is_none_or(|end| end > bytes.len())
    {
        return false;
    }
    let mut format = None;
    let mut data_length = None;
    let mut average_bytes_per_second = None;
    let mut offset = 12usize;
    while offset + 8 <= declared_size + 8 {
        let name = &bytes[offset..offset + 4];
        let Ok(length_bytes) = <[u8; 4]>::try_from(&bytes[offset + 4..offset + 8]) else {
            return false;
        };
        let length = u32::from_le_bytes(length_bytes) as usize;
        let Some(start) = offset.checked_add(8) else {
            return false;
        };
        let Some(end) = start.checked_add(length) else {
            return false;
        };
        if end > declared_size + 8 {
            return false;
        }
        if name == b"fmt " && length >= 16 {
            format = Some(u16::from_le_bytes([bytes[start], bytes[start + 1]]));
            average_bytes_per_second = Some(u32::from_le_bytes([
                bytes[start + 8],
                bytes[start + 9],
                bytes[start + 10],
                bytes[start + 11],
            ]));
        } else if name == b"data" {
            data_length = Some(length);
        }
        let Some(next) = end.checked_add(length & 1) else {
            return false;
        };
        offset = next;
    }
    let (Some(format), Some(data_length), Some(byte_rate)) =
        (format, data_length, average_bytes_per_second)
    else {
        return false;
    };
    let byte_rate = byte_rate as usize;
    let duration_is_valid = byte_rate > 0 && data_length <= byte_rate.saturating_mul(59);
    matches!(format, 0x0270 | 0xfffe) && data_length > 0 && duration_is_valid
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
