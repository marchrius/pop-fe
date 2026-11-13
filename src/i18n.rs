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
        ("invalid PPF patch: ", "Invalid PPF patch: "),
        ("PPF validation failed: ", "PPF validation failed: "),
        ("could not watermark PIC1: ", "Could not watermark PIC1: "),
        ("7-Zip is unavailable: ", "7-Zip is unavailable: "),
        ("pdftoppm is unavailable: ", "pdftoppm is unavailable: "),
        ("ffmpeg is unavailable: ", "ffmpeg is unavailable: "),
        (
            "could not prepare PBP image: ",
            "Could not prepare an image for the PBP: ",
        ),
    ];
    for (prefix, translation_key) in PREFIXES {
        if let Some(detail) = message.strip_prefix(prefix) {
            return format!("{}{}", tr(translation_key), localize_error(detail));
        }
    }
    const YOUTUBE_403: &str = "YouTube denied the download (HTTP 403). Select a Netscape-format cookies.txt file in the SND0 options and try again. ";
    if let Some(detail) = message.strip_prefix(YOUTUBE_403) {
        return format!("{}{}", tr(YOUTUBE_403), localize_error(detail));
    }
    if let Some(index) = message
        .strip_prefix("section ")
        .and_then(|value| value.strip_suffix(" is out of bounds"))
    {
        return tr("PBP section {index} is out of bounds").replace("{index}", index);
    }
    if let Some(count) = message.strip_prefix("invalid PS3 manual page count: ") {
        return tr("Invalid PS3 manual page count: {count}").replace("{count}", count);
    }
    if let Some(section) = message.strip_suffix(" SHA-1 mismatch") {
        let key = match section {
            "PS3 document header" => "PS3 document header SHA-1 mismatch",
            "PS3 manual INFO block" => "PS3 manual INFO block SHA-1 mismatch",
            _ => return tr(message),
        };
        return tr(key);
    }
    if let Some(error) = message.strip_prefix("cannot read ZIP disc image entry: ") {
        return tr("Cannot read ZIP disc image entry: {error}").replace("{error}", error);
    }
    if let Some(error) = message.strip_prefix("invalid ZIP disc image: ") {
        return tr("Invalid ZIP disc image: {error}").replace("{error}", error);
    }
    if let Some(status) = message.strip_prefix("chdman exited with ") {
        return tr("chdman exited with {status}").replace("{status}", status);
    }
    if let Some(error) = message.strip_prefix("CHD input requires chdman (MAME tools): ") {
        return tr("CHD input requires chdman (MAME tools): {error}")
            .replace("{error}", &localize_error(error));
    }
    if let Some(mode) = message.strip_prefix("unsupported CCD track mode ") {
        return tr("Unsupported CCD track mode: {mode}").replace("{mode}", mode);
    }
    if let Some(status) = message.strip_prefix("yt-dlp search failed with ") {
        return tr("yt-dlp search failed with {status}").replace("{status}", status);
    }
    if let Some(track) = message
        .strip_prefix("CUE audio track ")
        .and_then(|value| value.strip_suffix(" indexes are out of order"))
    {
        return tr("CUE audio track {track} indexes are out of order").replace("{track}", track);
    }
    if let Some(path) = message.strip_prefix("CDDA mode cannot merge mixed sector sizes in ") {
        return tr("CDDA mode cannot merge mixed sector sizes in {path}").replace("{path}", path);
    }
    if let Some(rest) = message.strip_prefix("CDDA image ")
        && let Some((path, sector_size)) = rest.split_once(" is not aligned to ")
        && let Some(sector_size) = sector_size.strip_suffix("-byte sectors")
    {
        return tr("CDDA image {path} is not aligned to {size}-byte sectors")
            .replace("{path}", path)
            .replace("{size}", sector_size);
    }
    if let Some(path) = message.strip_prefix("CCD companion IMG file not found for ") {
        return tr("CCD companion IMG file not found for {path}").replace("{path}", path);
    }
    if let Some(path) = message.strip_prefix("chdman did not create the extracted files for ") {
        return tr("chdman did not create extracted files for {path}").replace("{path}", path);
    }
    if let Some(path) = message.strip_prefix("extraction destination is not a regular file: ") {
        return tr("Extraction destination is not a regular file: {path}").replace("{path}", path);
    }
    if let Some(path) = message.strip_prefix("missing image file ") {
        return tr("Missing image file: {path}").replace("{path}", path);
    }
    if let Some(track) = message.strip_prefix("track ")
        && let Some(number) = track.strip_suffix(" INDEX 00 is after INDEX 01")
    {
        return tr("Track {track} INDEX 00 is after INDEX 01").replace("{track}", number);
    }
    if let Some(track) = message.strip_prefix("track ")
        && let Some(number) = track.strip_suffix(" index lies beyond the image file")
    {
        return tr("Track {track} index lies beyond the image file").replace("{track}", number);
    }
    if let Some(rest) = message.strip_prefix("CCD track number must be between 1 and 99 at line ") {
        return tr("CCD track number must be between 1 and 99 at line {line}")
            .replace("{line}", rest);
    }
    if let Some(rest) = message.strip_prefix("duplicate CCD track ")
        && let Some((track, line)) = rest.split_once(" at line ")
    {
        return tr("Duplicate CCD track {track} at line {line}")
            .replace("{track}", track)
            .replace("{line}", line);
    }
    for (prefix, key) in [
        (
            "TRACK appears before FILE at line ",
            "CUE TRACK appears before FILE at line {line}",
        ),
        (
            "INDEX appears before TRACK at line ",
            "CUE INDEX appears before TRACK at line {line}",
        ),
        (
            "invalid CCD track section at line ",
            "Invalid CCD track section at line {line}",
        ),
    ] {
        if let Some(line) = message.strip_prefix(prefix) {
            return tr(key).replace("{line}", line);
        }
    }
    if let Some((cause, restore_error)) =
        message.split_once("; could not restore previous DOCUMENT.DAT: ")
    {
        return tr("{error}; could not restore previous DOCUMENT.DAT: {restore_error}")
            .replace("{error}", &localize_error(cause))
            .replace("{restore_error}", &localize_error(restore_error));
    }
    if let Some((cause, rollback)) = message.split_once("; conversion rollback was incomplete: ") {
        return tr("{error}; conversion rollback was incomplete: {rollback}")
            .replace("{error}", &localize_error(cause))
            .replace("{rollback}", rollback);
    }
    if let Some((_, rest)) = message.split_once("; rollback was incomplete (")
        && let Some((rollback, recovery)) = rest.split_once("); recovery files remain in ")
    {
        return tr("Rollback was incomplete ({rollback}); recovery files remain in {path}")
            .replace("{rollback}", rollback)
            .replace("{path}", recovery);
    }
    if let Some(operation_error) = localize_external_command_failure(message) {
        return operation_error;
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
            "unsupported INDEX ",
            "Unsupported CUE INDEX {index} at line {line}",
        ),
        (
            "missing INDEX number at line ",
            "Missing CUE INDEX number at line {line}",
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
            if prefix == "unsupported INDEX " {
                if let Some((index, line)) = line.split_once(" at line ") {
                    return tr(key).replace("{index}", index).replace("{line}", line);
                }
            } else {
                return tr(key).replace("{line}", line);
            }
        }
    }
    if let Some(mode) = message.strip_prefix("unknown track mode ") {
        return tr("Unknown CUE track mode: {mode}").replace("{mode}", mode);
    }
    if let Some(track) = message.strip_prefix("missing INDEX 01 for track ") {
        return tr("Missing CUE INDEX 01 for track {track}").replace("{track}", track);
    }
    if let Some(rest) = message.strip_prefix("unexpected CUE TRACK ")
        && let Some((number, rest)) = rest.split_once("; expected ")
        && let Some((expected, line)) = rest.split_once(" at line ")
    {
        return tr("Unexpected CUE TRACK {number}; expected {expected} at line {line}")
            .replace("{number}", number)
            .replace("{expected}", expected)
            .replace("{line}", line);
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
        "game ID must contain nine ASCII alphanumeric characters" => {
            "Game ID must contain nine ASCII alphanumeric characters"
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
        "failed to capture command error output" => "failed to capture command error output",
        "failed to capture atracdenc error output" => "failed to capture atracdenc error output",
        "failed to capture yt-dlp error output" => "failed to capture yt-dlp error output",
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
        "PSISO audio offset overflow" => "PSISO audio offset overflow",
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
        "CUE supports at most 99 tracks" => "CUE supports at most 99 tracks",
        "CUE file exceeds the 4 MiB size limit" => "CUE file exceeds the 4 MiB size limit",
        "CCD file exceeds the 4 MiB size limit" => "CCD file exceeds the 4 MiB size limit",
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
        "image is empty or exceeds the 32 MB limit" => "Image is empty or exceeds the 32 MB limit",
        "could not resize image for the PBP" => "Could not resize the image for the PBP",
        "could not add alpha channel to PBP image" => {
            "Could not add an alpha channel to the PBP image"
        }
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
        "PSAR contains no PSISO disc images" => "PSAR contains no PSISO disc images",
        "compressed sector is out of bounds" => "Compressed sector is out of bounds",
        "audio track is out of bounds" => "Audio track is out of bounds",
        "staged filename is missing" => "Staged filename is missing",
        "sector offset overflow" => "Sector offset overflow",
        "empty disc image" => "Disc image is empty",
        "disc has no data track for ISO export" => "Disc has no data track for ISO export",
        "multiple companion CUE files have different contents" => {
            "Multiple companion CUE files have different contents"
        }
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

fn localize_external_command_failure(message: &str) -> Option<String> {
    let (operation, status_detail) = message.split_once(" failed with ")?;
    let (status, detail) = if let Some(value) = status_detail.strip_prefix("exit status: ") {
        match value.split_once(": ") {
            Some((code, detail)) => (tr("Exit code {code}").replace("{code}", code), Some(detail)),
            None => (tr("Exit code {code}").replace("{code}", value), None),
        }
    } else if let Some(value) = status_detail.strip_prefix("signal: ") {
        match value.split_once(": ") {
            Some((signal, detail)) => (
                tr("Signal {signal}").replace("{signal}", signal),
                Some(detail),
            ),
            None => (tr("Signal {signal}").replace("{signal}", value), None),
        }
    } else {
        (status_detail.to_owned(), None)
    };
    let operation = tr(operation);
    Some(match detail {
        Some(detail) => tr("External command {operation} failed with {status}: {detail}")
            .replace("{operation}", &operation)
            .replace("{status}", &status)
            .replace("{detail}", &localize_error(detail)),
        None => tr("External command {operation} failed with {status}")
            .replace("{operation}", &operation)
            .replace("{status}", &status),
    })
}
