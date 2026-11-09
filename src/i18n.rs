use std::collections::HashMap;
use std::sync::OnceLock;

fn selected_language() -> &'static str {
    static LANGUAGE: OnceLock<&'static str> = OnceLock::new();
    LANGUAGE.get_or_init(|| {
        let candidates = std::env::var("LANGUAGE")
            .ok()
            .into_iter()
            .flat_map(|s| s.split(':').map(str::to_owned).collect::<Vec<_>>())
            .chain(
                ["LC_ALL", "LC_MESSAGES", "LANG"]
                    .into_iter()
                    .filter_map(|key| std::env::var(key).ok()),
            );
        for candidate in candidates {
            let language = candidate
                .split(['_', '-', '.'])
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if language == "it" {
                return "it";
            }
            if language == "en" {
                return "en";
            }
        }
        "en"
    })
}

fn catalog() -> &'static HashMap<String, String> {
    static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let source = if selected_language() == "it" {
            include_str!("../locales/it.json")
        } else {
            include_str!("../locales/en.json")
        };
        serde_json::from_str(source).expect("embedded translation catalog must be valid JSON")
    })
}

pub fn tr(message: &str) -> String {
    catalog()
        .get(message)
        .cloned()
        .unwrap_or_else(|| message.to_owned())
}

pub fn localize_error(message: &str) -> String {
    const PREFIXES: [(&str, &str); 11] = [
        ("I/O error: ", "I/O error: "),
        ("invalid conversion input: ", "Invalid conversion input: "),
        ("invalid PBP: ", "Invalid PBP: "),
        ("cannot read disc image: ", "Cannot read disc image: "),
        (
            "invalid or unsupported CUE file: ",
            "Invalid or unsupported CUE file: ",
        ),
        ("manual I/O failed: ", "Manual I/O failed: "),
        ("manual processing failed: ", "Manual processing failed: "),
        (
            "automatic PPF compatibility patch failed: ",
            "Automatic PPF compatibility patch failed: ",
        ),
        (
            "automatic LibCrypt patch failed: ",
            "Automatic LibCrypt patch failed: ",
        ),
        (
            "automatic Xdelta compatibility patch failed: ",
            "Automatic Xdelta compatibility patch failed: ",
        ),
        (
            "automatic Xdelta fix requires xdelta3: ",
            "Automatic Xdelta fix requires xdelta3: ",
        ),
    ];
    for (prefix, translation_key) in PREFIXES {
        if let Some(detail) = message.strip_prefix(prefix) {
            return format!("{}{}", tr(translation_key), localize_error(detail));
        }
    }
    if let Some(index) = message
        .strip_prefix("section ")
        .and_then(|value| value.strip_suffix(" is out of bounds"))
    {
        return tr("PBP section {index} is out of bounds").replace("{index}", index);
    }
    if let Some(game_id) = message
        .strip_prefix("LibCrypt patch unavailable for ")
        .and_then(|value| value.strip_suffix("; install lcp to patch this game"))
    {
        return tr("LibCrypt patch unavailable for {id}; install lcp to patch this game")
            .replace("{id}", game_id);
    }
    let key = match message {
        "operation cancelled" => "Operation stopped",
        "compressed PSISO sector is invalid" => "Compressed PSISO sector is invalid",
        "disc ID could not be determined" => "Disc ID could not be determined",
        "game ID must contain nine ASCII characters" => {
            "Game ID must contain nine ASCII characters"
        }
        "select between one and 32 disc images" => "Select between one and 32 disc images",
        "disc image is empty" => "Disc image is empty",
        "CUE audio needs atracdenc; install it or set POPFE_ATRACDENC" => {
            "CUE audio needs atracdenc; install it or set POPFE_ATRACDENC"
        }
        "YouTube SND0 download needs yt-dlp" => "YouTube SND0 download needs yt-dlp",
        "SND0 conversion needs ffmpeg" => "SND0 conversion needs ffmpeg",
        "SND0 conversion needs atracdenc" => "SND0 conversion needs atracdenc",
        "yt-dlp did not produce a WAV file" => "yt-dlp did not produce a WAV file",
        "ATRAC3 SND0 exceeds the PSP 500 KB limit" => "ATRAC3 SND0 exceeds the PSP 500 KB limit",
        "atracdenc produced an invalid SND0 stream" => "atracdenc produced an invalid SND0 stream",
        "ISO is too large" => "ISO is too large",
        "CUE image is too large" => "CUE image is too large",
        "CDDA disc has too many sectors" => "CDDA disc has too many sectors",
        "CUE metadata is missing" => "CUE metadata is missing",
        "CUE sheet does not contain a data track" => "CUE sheet does not contain a data track",
        "audio-only CUE tracks cannot be converted as a PSX data disc" => {
            "Audio-only CUE tracks cannot be converted as a PSX data disc"
        }
        "header is truncated" => "PBP header is truncated",
        "missing PBP magic" => "PBP magic is missing",
        "first section overlaps the PBP header" => "First section overlaps the PBP header",
        "section offsets are not ordered" => "PBP section offsets are not ordered",
        "PSISO image size is invalid" => "PSISO image size is invalid",
        "compressed sector is out of bounds" => "Compressed sector is out of bounds",
        "audio track is out of bounds" => "Audio track is out of bounds",
        "staged filename is missing" => "Staged filename is missing",
        "sector offset overflow" => "Sector offset overflow",
        "empty disc image" => "Disc image is empty",
        "disc has no data track for ISO export" => "Disc has no data track for ISO export",
        "CUE data track does not refer to the extracted image" => {
            "CUE data track does not refer to the extracted image"
        }
        "data track bounds exceed the extracted disc image" => {
            "Data track bounds exceed the extracted disc image"
        }
        "ZIP archive does not contain a supported disc image" => {
            "ZIP archive does not contain a supported disc image"
        }
        _ => return message.to_owned(),
    };
    tr(key)
}
