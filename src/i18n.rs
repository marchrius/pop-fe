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
