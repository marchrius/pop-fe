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
    const PREFIXES: [(&str, &str); 20] = [
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
        (
            "manual PPF romhack patch failed: ",
            "Manual PPF romhack patch failed: ",
        ),
        (
            "manual Xdelta romhack patch failed: ",
            "Manual Xdelta romhack patch failed: ",
        ),
        (
            "manual Xdelta romhack patch failed with ",
            "Manual Xdelta romhack patch failed with ",
        ),
        ("invalid PPF patch: ", "Invalid PPF patch: "),
        ("PPF validation failed: ", "PPF validation failed: "),
        ("could not watermark PIC1: ", "Could not watermark PIC1: "),
        ("7-Zip is unavailable: ", "7-Zip is unavailable: "),
        ("pdftoppm is unavailable: ", "pdftoppm is unavailable: "),
        ("ffmpeg is unavailable: ", "ffmpeg is unavailable: "),
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
    if let Some(game_id) = message
        .strip_prefix("PSP compatibility config unavailable for ")
        .and_then(|value| value.strip_suffix("; default settings will be used"))
    {
        return tr("PSP compatibility config unavailable for {id}; default settings will be used")
            .replace("{id}", game_id);
    }
    for (prefix, key) in [
        (
            "malformed FILE entry at line ",
            "Malformed CUE FILE entry at line {line}",
        ),
        (
            "bad TRACK at line ",
            "Invalid CUE TRACK entry at line {line}",
        ),
        (
            "missing track mode at line ",
            "Missing track mode at line {line}",
        ),
        (
            "missing INDEX time at line ",
            "Missing CUE INDEX time at line {line}",
        ),
        (
            "invalid INDEX time at line ",
            "Invalid CUE INDEX time at line {line}",
        ),
        (
            "invalid CCD MODE at line ",
            "Invalid CCD MODE at line {line}",
        ),
        (
            "invalid CCD index at line ",
            "Invalid CCD index at line {line}",
        ),
    ] {
        if let Some(line) = message.strip_prefix(prefix) {
            return tr(key).replace("{line}", line);
        }
    }
    if let Some(mode) = message.strip_prefix("unknown track mode ") {
        return tr("Unknown CUE track mode: {mode}").replace("{mode}", mode);
    }
    if let Some(extension) = message.strip_prefix("unsupported image extension .") {
        return tr("Unsupported disc image extension: .{extension}")
            .replace("{extension}", extension);
    }
    if let Some(format) = message.strip_prefix("unsupported romhack patch format: ") {
        return tr("Unsupported romhack patch format: {format}").replace("{format}", format);
    }
    if let Some(field) = message.strip_suffix(" exceeds the PBP/PSISO format limit") {
        return tr("{field} exceeds the PBP/PSISO format limit").replace("{field}", field);
    }
    if let Some(track) = message
        .strip_prefix("CUE audio track ")
        .and_then(|value| value.strip_suffix(" is empty"))
    {
        return tr("CUE audio track {track} is empty").replace("{track}", track);
    }
    if let Some(rest) = message.strip_prefix("atracdenc failed on audio track ")
        && let Some((track, detail)) = rest.split_once(": ")
    {
        return tr("atracdenc failed on audio track {track}: {detail}")
            .replace("{track}", track)
            .replace("{detail}", detail);
    }
    if let Some(track) = message.strip_prefix("atracdenc produced an empty track ") {
        return tr("atracdenc produced an empty track {track}").replace("{track}", track);
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
        "manual Xdelta romhack patch produced no disc image" => {
            "Manual Xdelta romhack patch produced no disc image"
        }
        "failed to capture command error output" => "Failed to capture command error output",
        "failed to capture atracdenc error output" => "Failed to capture atracdenc error output",
        "failed to capture yt-dlp error output" => "Failed to capture yt-dlp error output",
        "manual Xdelta romhack patch" => "Manual Xdelta romhack patch",
        "manual operation cancelled" => "Operation stopped",
        "PBP section offset overflow" => "PBP section offset overflow",
        "PBP PSAR offset overflow" => "PBP PSAR offset overflow",
        "PBP STARTDAT end overflow" => "PBP STARTDAT end overflow",
        "invalid PBP PSISO location" => "Invalid PBP PSISO location",
        "invalid PBP STARTDAT location" => "Invalid PBP STARTDAT location",
        "PBP/PSISO offset overflow" => "PBP/PSISO offset overflow",
        "CUE audio offset overflow" => "CUE audio offset overflow",
        "compressed image is too large" => "Compressed image is too large",
        "invalid PSISO audio location" => "Invalid PSISO audio location",
        "PSISO audio offset exceeds format limit" => "PSISO audio offset exceeds format limit",
        "PSISO audio track exceeds format limit" => "PSISO audio track exceeds format limit",
        "too many disc sectors" => "Too many disc sectors",
        "index table is too large" => "Index table is too large",
        "disc image is too large" => "Disc image is too large",
        "PSX-undither needs a raw image aligned to 2352-byte sectors" => {
            "PSX-undither needs a raw image aligned to 2352-byte sectors"
        }
        "PSP compatibility config is too large" => "PSP compatibility config is too large",
        "PSISO supports at most 98 audio tracks" => "PSISO supports at most 98 audio tracks",
        "CDDA image is not aligned to sectors" => "CDDA image is not aligned to sectors",
        "CUE audio track is empty" => "CUE audio track is empty",
        "CUE audio track exceeds WAV size limit" => "CUE audio track exceeds WAV size limit",
        "invalid PPF description footer" => "Invalid PPF description footer",
        "PPF1 header is truncated" => "PPF1 header is truncated",
        "PPF2 header is truncated" => "PPF2 header is truncated",
        "PPF3 header is truncated" => "PPF3 header is truncated",
        "unknown PPF version" => "Unknown PPF version",
        "disc contents do not match" => "Disc contents do not match the PPF patch",
        "truncated record" => "Truncated PPF record",
        "truncated record data" => "Truncated PPF record data",
        "truncated undo data" => "Truncated PPF undo data",
        "record offset overflow" => "PPF record offset overflow",
        "record extends beyond image" => "PPF record extends beyond the disc image",
        "truncated integer" => "Truncated integer in PPF patch",
        "invalid integer" => "Invalid integer in PPF patch",
        "manual exceeds size limit" => "Manual exceeds the size limit",
        "manual is too large" => "Manual is too large",
        "invalid manual metadata" => "Invalid manual metadata",
        "invalid PS3 PGD manual header" => "Invalid PS3 PGD manual header",
        "invalid encrypted DOC header" => "Invalid encrypted DOC header",
        "PS3 manual INFO block overflow" => "PS3 manual INFO block overflow",
        "PS3 manual offset overflow" => "PS3 manual offset overflow",
        "PS3 page table overflow" => "PS3 page table overflow",
        "truncated PS3 manual" => "Truncated PS3 manual",
        "DES decryption failed" => "DES decryption failed",
        "could not extract RAR manual" => "Could not extract RAR manual",
        "could not render manual PDF" => "Could not render the manual PDF",
        "could not resize manual page" => "Could not resize a manual page",
        "7-Zip is unavailable: " => "7-Zip is unavailable: ",
        "pdftoppm is unavailable: " => "pdftoppm is unavailable: ",
        "ffmpeg is unavailable: " => "ffmpeg is unavailable: ",
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
        "no tracks found" => "No tracks were found",
        "CCD contains no tracks" => "CCD contains no tracks",
        "ISO9660 primary volume descriptor not found" => {
            "ISO9660 primary volume descriptor not found"
        }
        "SYSTEM.CNF not found in ISO9660 root" => "SYSTEM.CNF was not found in the ISO9660 root",
        _ => return tr(message),
    };
    tr(key)
}
