//! Download and normalize known game artwork for embedding in PSP EBOOT files.
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use cairo::{Context, FontSlant, FontWeight, Format, ImageSurface};
use gdk_pixbuf::{InterpType, Pixbuf};
use reqwest::blocking::Client;

use crate::create::PbpAssets;
use crate::database::Database;

const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

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

pub fn load_theme_assets(game_id: &str, theme: &str, base: &PbpAssets) -> PbpAssets {
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
        ) {
            match key {
                "pic0" => result.pic0 = Some(path),
                "pic1" => result.pic1 = Some(path),
                "icon0" => result.icon0 = Some(path),
                _ => {}
            }
        }
    }
    result
}

pub fn load_theme_assets_offline(game_id: &str, theme: &str, base: &PbpAssets) -> PbpAssets {
    procedural_theme_assets(game_id, theme, base)
}

fn procedural_theme_assets(game_id: &str, theme: &str, base: &PbpAssets) -> PbpAssets {
    let directory = cache_directory().join("themes").join("generated-v1");
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
        context.set_font_size(5.2);
        context.set_source_rgb(0.75, 0.75, 0.75);
        for y in 0..120usize {
            for x in 0..120usize {
                let p = pixel(pixels, stride, channels, x, y);
                let gray = (p[0] as u16 * 30 + p[1] as u16 * 59 + p[2] as u16 * 11) / 100;
                let shade = 10 - (gray as usize / 26).min(10);
                context.move_to(105.0 + x as f64 * 3.15, 5.0 + y as f64 * 2.15);
                context.show_text(shades[shade])?;
            }
        }
    } else {
        let dots =
            theme == "DOTPAINTING" || matches!(theme, "OILPAINTING" | "WATERCOLOR" | "COLORSKETCH");
        for y in 0..120usize {
            for x in 0..120usize {
                let mut p = pixel(pixels, stride, channels, x, y);
                if matches!(theme, "OILPAINTING" | "WATERCOLOR" | "COLORSKETCH") {
                    p = painterly_pixel(pixels, stride, channels, x, y, theme);
                }
                if dots {
                    context.set_source_rgb(
                        p[0] as f64 / 255.0,
                        p[1] as f64 / 255.0,
                        p[2] as f64 / 255.0,
                    );
                    let diameter = if theme == "DOTPAINTING" { 3.1 } else { 2.0 };
                    context.arc(
                        90.0 + x as f64 * 2.4,
                        10.0 + y as f64 * 2.1,
                        diameter / 2.0,
                        0.0,
                        std::f64::consts::TAU,
                    );
                    context.fill()?;
                }
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
    let mut bytes = Vec::new();
    response
        .take((MAX_IMAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
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
