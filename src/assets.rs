//! Download and normalize known game artwork for embedding in PSP EBOOT files.
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
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
    let pic1 = game.pic1.as_deref().and_then(|url| {
        cached_image(
            &client,
            url,
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
    let pic0 = game
        .pic0
        .as_deref()
        .and_then(|url| cached_pic0(&client, url, &directory, game_id, game, Some(cancelled)));
    if cancelled.load(Ordering::Relaxed) {
        return PbpAssets::default();
    }
    let icon_url = game.icon0.as_deref();
    let icon0 = icon_url
        .and_then(|url| {
            cached_image(
                &client,
                url,
                &directory,
                game_id,
                "icon0",
                (80, 80),
                Some(cancelled),
            )
        })
        .or_else(|| {
            let source = pic1.as_ref()?;
            let destination = directory.join(format!("{}-icon0.png", safe_id(game_id)));
            normalize_cached(source, &destination, (80, 80)).then_some(destination)
        });
    let snd0 = game
        .snd0
        .as_deref()
        .and_then(|link| cached_snd0(&client, link, &directory, game_id, Some(cancelled)));
    PbpAssets {
        icon0,
        pic0,
        pic1,
        snd0,
        ..PbpAssets::default()
    }
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
        ("pic0", "PIC0.PNG", (310, 180)),
        ("pic1", "PIC1.PNG", (480, 272)),
    ] {
        let url = format!(
            "{}/raw/main/data/{}/{}",
            repository.trim_end_matches('/'),
            game_id,
            file
        );
        if let Some(path) = cached_image(
            &client,
            &url,
            &directory,
            &format!("{game_id}-{theme}"),
            key,
            size,
            Some(cancelled),
        ) {
            match key {
                "pic0" => result.pic0 = Some(path),
                "pic1" => result.pic1 = Some(path),
                "icon0" => result.icon0 = Some(path),
                _ => {}
            }
        }
        if cancelled.load(Ordering::Relaxed) {
            return base.clone();
        }
    }
    result
}

pub fn load_theme_assets_offline(game_id: &str, theme: &str, base: &PbpAssets) -> PbpAssets {
    procedural_theme_assets(game_id, theme, base)
}

fn procedural_theme_assets(game_id: &str, theme: &str, base: &PbpAssets) -> PbpAssets {
    let directory = cache_directory().join("themes").join("generated-v2");
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
    let mut surface = ImageSurface::create(Format::ARgb32, 310, 180)?;
    let context = Context::new(&surface)?;
    context.select_font_face("monospace", FontSlant::Normal, FontWeight::Bold);
    context.set_font_size(10.0);
    let ascii = theme == "ASCIIART";
    let (r, g, b) = if ascii {
        (0.50, 1.0, 0.50)
    } else {
        (1.0, 0.50, 0.50)
    };
    let rows: Vec<String> = title
        .split(" - ")
        .map(|s| s.chars().take(32).collect())
        .collect();
    let mut lines = vec!["##############################".to_string()];
    lines.extend(rows.into_iter().take(8).map(|line| format!("# {line}")));
    lines.push("##############################".to_string());
    let mut y = 13.0;
    for (index, line) in lines.iter().enumerate() {
        if index == 0 || index + 1 == lines.len() || (ascii && index > 0) {
            context.set_source_rgb(r, g, b);
            context.move_to(if ascii { 50.0 } else { 62.0 }, y);
            context.show_text(line)?;
        }
        if index > 0 && index + 1 < lines.len() {
            context.set_source_rgb(0.0, 0.0, 0.0);
            context.move_to(if ascii { 57.0 } else { 68.0 }, y);
            context.show_text(&line[2..])?;
            context.set_source_rgb(1.0, 1.0, 1.0);
            context.move_to(if ascii { 56.0 } else { 67.0 }, y - 1.0);
            context.show_text(&line[2..])?;
        }
        y += 12.0;
    }
    if !ascii {
        surface.flush();
        let stride = surface.stride() as usize;
        let pixels = surface.data()?.to_vec();
        let dots = ImageSurface::create(Format::ARgb32, 310, 180)?;
        let dot_context = Context::new(&dots)?;
        for y in (0..180usize).step_by(2) {
            for x in (0..310usize).step_by(2) {
                let offset = y * stride + x * 4;
                if pixels[offset + 3] == 0 {
                    continue;
                }
                dot_context.set_source_rgba(
                    pixels[offset + 2] as f64 / 255.0,
                    pixels[offset + 1] as f64 / 255.0,
                    pixels[offset] as f64 / 255.0,
                    pixels[offset + 3] as f64 / 255.0,
                );
                dot_context.arc(x as f64, y as f64, 1.0, 0.0, std::f64::consts::TAU);
                dot_context.fill()?;
            }
        }
        write_png(dots, path)
    } else {
        write_png(surface, path)
    }
}

fn render_theme_pic1(
    path: &Path,
    source: &Path,
    theme: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let original = Pixbuf::from_file(source)?;
    let icon = original
        .scale_simple(120, 120, InterpType::Bilinear)
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
        let painterly = matches!(theme, "OILPAINTING" | "WATERCOLOR" | "COLORSKETCH");
        let pixel_size = 250.0 / 120.0;
        for y in 0..120usize {
            for x in 0..120usize {
                let mut p = pixel(pixels, stride, channels, x, y);
                if painterly {
                    p = painterly_pixel(pixels, stride, channels, x, y, theme);
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
    theme: &str,
) -> [u8; 3] {
    let mut sum = [0u32; 3];
    let mut count = 0u32;
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            let nx = (x as i32 + dx).clamp(0, 119) as usize;
            let ny = (y as i32 + dy).clamp(0, 119) as usize;
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

fn cached_image(
    client: &Client,
    url: &str,
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
    let response = client.get(url).send().ok()?.error_for_status().ok()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    read_response(response, MAX_IMAGE_BYTES, cancelled, &mut bytes)?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let pixbuf = Pixbuf::from_read(Cursor::new(bytes)).ok()?;
    if pixbuf.width() > 8000
        || pixbuf.height() > 8000
        || pixbuf.width() as i64 * pixbuf.height() as i64 > 50_000_000
    {
        return None;
    }
    save_scaled(&pixbuf, &path, size).ok()?;
    Some(path)
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
    let path = directory.join(format!("{}-pic0-layout-v2.png", safe_id(game_id)));
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
    let mut bytes = Vec::new();
    read_response(response, MAX_IMAGE_BYTES, cancelled, &mut bytes)?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let source = Pixbuf::from_read(Cursor::new(bytes))
        .ok()?
        .apply_embedded_orientation()?;
    let (width, height) = (source.width(), source.height());
    if width <= 0
        || height <= 0
        || width > 8000
        || height > 8000
        || width as i64 * height as i64 > 50_000_000
    {
        return None;
    }
    let fit_scale = (560.0 / height as f64).min(1000.0 / width as f64);
    let scale = fit_scale * game.pic0_scaling.unwrap_or(0.9).clamp(0.1, 3.0);
    let offset = game.pic0_offset.unwrap_or((0.1, 0.1));
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
    let resized = canvas.scale_simple(310, 180, InterpType::Hyper)?;
    let encoded = resized.save_to_bufferv("png", &[]).ok()?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, encoded).ok()?;
    fs::rename(temporary, &path).ok()?;
    Some(path)
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
