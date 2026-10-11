//! The single application-interface localization catalog and locale registry.
//!
//! Locale is presentation state. It is deliberately absent from RenderConfig,
//! projects, LevelData, manifests, cache keys, and creative documents.

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/sono_renderer_locales.rs"));

const MAX_REGISTRY_BYTES: usize = 64 * 1024;
const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
const MAX_MESSAGE_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct LocaleInfo {
    pub id: String,
    pub native_name: String,
    pub english_name: String,
    pub fallback: String,
    pub direction: String,
    pub audience: String,
    pub catalog_status: String,
    pub review_status: String,
}

#[derive(Debug, Deserialize)]
struct RegistryFile {
    schema_version: u32,
    locales: Vec<LocaleInfo>,
}

struct UniqueStringMap(BTreeMap<String, String>);

impl<'de> Deserialize<'de> for UniqueStringMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UniqueStringMapVisitor;

        impl<'de> Visitor<'de> for UniqueStringMapVisitor {
            type Value = UniqueStringMap;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object with unique string keys and string values")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = BTreeMap::new();
                while let Some((key, value)) = access.next_entry::<String, String>()? {
                    if values.insert(key.clone(), value).is_some() {
                        return Err(de::Error::custom(format!("duplicate message key {key:?}")));
                    }
                }
                Ok(UniqueStringMap(values))
            }
        }

        deserializer.deserialize_map(UniqueStringMapVisitor)
    }
}

#[derive(Debug)]
pub struct Catalogs {
    locales: Vec<LocaleInfo>,
    messages: BTreeMap<String, BTreeMap<String, String>>,
}

static CATALOGS: OnceLock<Catalogs> = OnceLock::new();

impl Catalogs {
    fn load_for_runtime() -> Self {
        let registry = if LOCALE_REGISTRY_JSON.len() <= MAX_REGISTRY_BYTES {
            serde_json::from_str::<RegistryFile>(LOCALE_REGISTRY_JSON)
                .ok()
                .filter(|registry| registry.schema_version == 1)
        } else {
            None
        };
        let Some(registry) = registry else {
            eprintln!(
                "Sono-Renderer localization fallback: invalid locale registry; using English."
            );
            return Self::empty_fallback();
        };

        let mut locales = Vec::new();
        let mut seen = BTreeSet::new();
        for locale in registry.locales {
            if validate_locale_metadata(&locale).is_ok() && seen.insert(locale.id.clone()) {
                locales.push(locale);
            } else {
                eprintln!("Sono-Renderer localization fallback: ignoring invalid locale metadata.");
            }
        }

        let mut messages = BTreeMap::new();
        for (id, contents) in BUILTIN_LOCALE_CATALOGS {
            if contents.len() > MAX_CATALOG_BYTES {
                eprintln!("Sono-Renderer localization fallback: catalog {id} exceeds size limit.");
                continue;
            }
            match serde_json::from_str::<UniqueStringMap>(contents) {
                Ok(catalog)
                    if catalog.0.iter().all(|(key, value)| {
                        !key.is_empty()
                            && key.len() <= 256
                            && value.len() <= MAX_MESSAGE_BYTES
                            && !value.contains('\0')
                    }) =>
                {
                    messages.insert((*id).to_owned(), catalog.0);
                }
                Ok(_) => eprintln!(
                    "Sono-Renderer localization fallback: catalog {id} has invalid messages."
                ),
                Err(error) => eprintln!(
                    "Sono-Renderer localization fallback: ignoring invalid catalog {id}: {error}"
                ),
            }
        }
        if !messages.contains_key("en") {
            eprintln!("Sono-Renderer localization fallback: English catalog unavailable.");
            return Self::empty_fallback();
        }
        Self { locales, messages }
    }

    fn load_embedded() -> Result<Self, String> {
        if LOCALE_REGISTRY_JSON.len() > MAX_REGISTRY_BYTES {
            return Err("locale registry exceeds the 64 KiB limit".into());
        }
        let registry: RegistryFile = serde_json::from_str(LOCALE_REGISTRY_JSON)
            .map_err(|error| format!("invalid embedded locale registry: {error}"))?;
        if registry.schema_version != 1 {
            return Err(format!(
                "unsupported locale registry schema {}",
                registry.schema_version
            ));
        }
        let mut seen = BTreeSet::new();
        for locale in &registry.locales {
            validate_locale_metadata(locale)?;
            if !seen.insert(locale.id.as_str()) {
                return Err(format!("duplicate locale ID {:?}", locale.id));
            }
        }
        let mut messages = BTreeMap::new();
        for (id, contents) in BUILTIN_LOCALE_CATALOGS {
            if contents.len() > MAX_CATALOG_BYTES {
                return Err(format!("locale catalog {id} exceeds the 2 MiB limit"));
            }
            let catalog: UniqueStringMap = serde_json::from_str(contents)
                .map_err(|error| format!("invalid embedded catalog {id}: {error}"))?;
            let catalog = catalog.0;
            if catalog.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 256
                    || value.len() > MAX_MESSAGE_BYTES
                    || value.contains('\0')
            }) {
                return Err(format!("locale catalog {id} contains an invalid message"));
            }
            messages.insert((*id).to_owned(), catalog);
        }
        if !seen.contains("en") || !messages.contains_key("en") {
            return Err("canonical English locale/catalog is missing".into());
        }
        Ok(Self {
            locales: registry.locales,
            messages,
        })
    }

    fn empty_fallback() -> Self {
        let en = LocaleInfo {
            id: "en".into(),
            native_name: "English".into(),
            english_name: "English".into(),
            fallback: "en".into(),
            direction: "ltr".into(),
            audience: "release".into(),
            catalog_status: "canonical-source".into(),
            review_status: "source-copy".into(),
        };
        let english = BUILTIN_LOCALE_CATALOGS
            .iter()
            .find(|(id, contents)| *id == "en" && contents.len() <= MAX_CATALOG_BYTES)
            .and_then(|(_, contents)| serde_json::from_str::<UniqueStringMap>(contents).ok())
            .map(|catalog| catalog.0)
            .unwrap_or_else(|| BTreeMap::from([("app.name".into(), "Sono-Renderer".into())]));
        Self {
            locales: vec![en],
            messages: BTreeMap::from([("en".into(), english)]),
        }
    }
}

fn catalogs() -> &'static Catalogs {
    CATALOGS.get_or_init(Catalogs::load_for_runtime)
}

pub fn locale_registry() -> &'static [LocaleInfo] {
    &catalogs().locales
}

pub fn locale_info(id: &str) -> Option<&'static LocaleInfo> {
    locale_registry().iter().find(|locale| locale.id == id)
}

pub fn release_locales() -> impl Iterator<Item = &'static LocaleInfo> {
    locale_registry()
        .iter()
        .filter(|locale| locale.audience == "release")
}

pub fn selectable_locales(include_developer: bool) -> impl Iterator<Item = &'static LocaleInfo> {
    locale_registry().iter().filter(move |locale| {
        locale.audience == "release" || (include_developer && locale.audience == "developer")
    })
}

pub fn is_valid_preference(id: &str, allow_developer: bool) -> bool {
    locale_info(id).is_some_and(|locale| {
        locale.audience == "release" || (allow_developer && locale.audience == "developer")
    })
}

pub fn direction_is_rtl(id: &str) -> bool {
    locale_info(id).is_some_and(|locale| locale.direction == "rtl")
}

pub fn contains_message(locale: &str, key: &str) -> bool {
    let data = catalogs();
    data.messages
        .get(locale)
        .is_some_and(|catalog| catalog.contains_key(key))
        || data
            .messages
            .get("en")
            .is_some_and(|catalog| catalog.contains_key(key))
}

pub fn tr(locale: &str, key: &str) -> String {
    tr_args(locale, key, &[])
}

pub fn tr_args(locale: &str, key: &str, arguments: &[(&str, &str)]) -> String {
    let data = catalogs();
    let developer = data
        .locales
        .iter()
        .find(|candidate| candidate.id == locale)
        .is_some_and(|candidate| candidate.audience == "developer");
    let source_locale = if developer { "en" } else { locale };
    let fallback_id = data
        .locales
        .iter()
        .find(|candidate| candidate.id == source_locale)
        .map(|candidate| candidate.fallback.as_str())
        .unwrap_or("en");
    let template = data
        .messages
        .get(source_locale)
        .and_then(|catalog| catalog.get(key))
        .or_else(|| {
            data.messages
                .get(fallback_id)
                .and_then(|catalog| catalog.get(key))
        })
        .or_else(|| data.messages.get("en").and_then(|catalog| catalog.get(key)))
        .map(String::as_str)
        .unwrap_or("Sono-Renderer");
    let decorated = match locale {
        "qps-ploc" => pseudo_localize(template),
        "qps-rtl" => rtl_stress(template),
        _ => template.to_owned(),
    };
    substitute(&decorated, arguments)
}

pub fn tr_plural(locale: &str, key: &str, count: usize) -> String {
    let count_string = count.to_string();
    let category = plural_category(locale, count);
    let data = catalogs();
    let source_locale = if locale == "qps-ploc" || locale == "qps-rtl" {
        "en"
    } else {
        locale
    };
    for form in [category, "other"] {
        let full_key = format!("{key}.{form}");
        if data
            .messages
            .get(source_locale)
            .is_some_and(|catalog| catalog.contains_key(&full_key))
            || data
                .messages
                .get("en")
                .is_some_and(|catalog| catalog.contains_key(&full_key))
        {
            return tr_args(locale, &full_key, &[("count", &count_string)]);
        }
    }
    tr_args(locale, key, &[("count", &count_string)])
}

fn plural_category(locale: &str, count: usize) -> &'static str {
    match locale {
        "ja" | "ko" | "zh-CN" | "zh-TW" => "other",
        "ru" => {
            let mod10 = count % 10;
            let mod100 = count % 100;
            if mod10 == 1 && mod100 != 11 {
                "one"
            } else if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
                "few"
            } else {
                "many"
            }
        }
        "fr" if count <= 1 => "one",
        _ if count == 1 => "one",
        _ => "other",
    }
}

fn substitute(template: &str, arguments: &[(&str, &str)]) -> String {
    let mut rendered = template.to_owned();
    for (name, value) in arguments {
        let marker = format!("{{{name}}}");
        rendered = rendered.replace(&marker, value);
    }
    rendered
}

fn pseudo_localize(source: &str) -> String {
    let accented: String = source
        .chars()
        .map(|character| match character {
            'a' => 'á',
            'e' => 'ë',
            'i' => 'ï',
            'o' => 'ô',
            'u' => 'ü',
            'A' => 'Á',
            'E' => 'Ë',
            'I' => 'Ï',
            'O' => 'Ô',
            'U' => 'Ü',
            _ => character,
        })
        .collect();
    format!("[!! {accented} ~~~~~ !!]")
}

fn rtl_stress(source: &str) -> String {
    format!("\u{2067}اختبار RTL ← {source}\u{2069}")
}

fn parse_placeholders(message: &str) -> Result<BTreeSet<String>, String> {
    let bytes = message.as_bytes();
    let mut index = 0;
    let mut result = BTreeSet::new();
    while index < bytes.len() {
        if bytes[index] != b'{' {
            if bytes[index] == b'}' {
                return Err("unmatched closing brace".into());
            }
            index += 1;
            continue;
        }
        let Some(relative_end) = bytes[index + 1..].iter().position(|byte| *byte == b'}') else {
            return Err("unclosed placeholder".into());
        };
        let end = index + 1 + relative_end;
        let name = &message[index + 1..end];
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.')
        {
            return Err(format!("invalid placeholder {{{name}}}"));
        }
        result.insert(name.to_owned());
        index = end + 1;
    }
    Ok(result)
}

fn validate_locale_metadata(locale: &LocaleInfo) -> Result<(), String> {
    let valid_id = !locale.id.is_empty()
        && locale.id.len() <= 35
        && locale
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !valid_id
        || locale.native_name.is_empty()
        || locale.native_name.len() > 128
        || locale.english_name.is_empty()
        || locale.fallback.is_empty()
        || !matches!(locale.direction.as_str(), "ltr" | "rtl")
        || !matches!(locale.audience.as_str(), "release" | "developer")
    {
        return Err(format!("invalid locale metadata for {:?}", locale.id));
    }
    Ok(())
}

pub fn validate_embedded_catalogs() -> Result<(), Vec<String>> {
    let data = Catalogs::load_embedded().map_err(|error| vec![error])?;
    let Some(english) = data.messages.get("en") else {
        return Err(vec!["English catalog is missing".into()]);
    };
    let mut errors = Vec::new();
    let ids: BTreeSet<&str> = data
        .locales
        .iter()
        .map(|locale| locale.id.as_str())
        .collect();
    for locale in &data.locales {
        if !ids.contains(locale.fallback.as_str()) {
            errors.push(format!(
                "locale {} references missing fallback {}",
                locale.id, locale.fallback
            ));
        }
        let mut cursor = locale;
        let mut chain = BTreeSet::new();
        loop {
            if !chain.insert(cursor.id.as_str()) {
                errors.push(format!("fallback cycle contains locale {}", cursor.id));
                break;
            }
            if cursor.id == "en" {
                break;
            }
            let Some(next) = data
                .locales
                .iter()
                .find(|candidate| candidate.id == cursor.fallback)
            else {
                break;
            };
            cursor = next;
        }
        let Some(catalog) = data.messages.get(&locale.id) else {
            errors.push(format!("locale {} has no catalog", locale.id));
            continue;
        };
        for key in catalog.keys() {
            if !english.contains_key(key) {
                errors.push(format!("locale {} has unknown key {key}", locale.id));
            }
        }
        if locale.audience == "release" {
            for (key, source) in english {
                let Some(target) = catalog.get(key) else {
                    errors.push(format!("locale {} is missing {key}", locale.id));
                    continue;
                };
                match (parse_placeholders(source), parse_placeholders(target)) {
                    (Ok(source), Ok(target)) if source == target => {}
                    (Ok(source), Ok(target)) => errors.push(format!(
                        "locale {} placeholder mismatch in {key}: {source:?} vs {target:?}",
                        locale.id
                    )),
                    (_, Err(error)) => errors.push(format!(
                        "locale {} has malformed placeholders in {key}: {error}",
                        locale.id
                    )),
                    (Err(error), _) => errors.push(format!("English {key} is malformed: {error}")),
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Add host-provided CJK/Hangul fonts as UI-only fallback fonts when present.
/// Fonts are read from their platform installation locations and are never
/// bundled or copied into Presentation Studio projects.
pub fn install_system_ui_font_fallbacks(
    context: &eframe::egui::Context,
    locale: &str,
) -> Vec<String> {
    use eframe::egui::{FontData, FontDefinitions, FontFamily};
    use std::sync::Arc;
    let mut installed = Vec::new();
    let mut definitions = FontDefinitions::default();
    for (index, path) in candidate_ui_font_paths(locale).into_iter().enumerate() {
        if installed.len() >= 2 {
            break;
        }
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if !(16 * 1024..=40 * 1024 * 1024).contains(&metadata.len()) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let name = format!("sono-ui-system-fallback-{index}-{stem}");
        definitions
            .font_data
            .insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            definitions
                .families
                .entry(family)
                .or_default()
                .push(name.clone());
        }
        installed.push(name);
    }
    if !installed.is_empty() {
        context.set_fonts(definitions);
    }
    installed
}

fn candidate_ui_font_paths(locale: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(not(target_os = "windows"))]
    let _ = locale;
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("Fonts");
        let preferred = match locale {
            "ja" => ["NotoSansJP-VF.ttf", "meiryo.ttc", "msgothic.ttc"],
            "zh-CN" => ["msyh.ttc", "simsun.ttc", "NotoSansJP-VF.ttf"],
            "zh-TW" => ["msjh.ttc", "mingliub.ttc", "NotoSansJP-VF.ttf"],
            "ko" => ["malgun.ttf", "NotoSansJP-VF.ttf", "meiryo.ttc"],
            _ => ["NotoSansJP-VF.ttf", "meiryo.ttc", "malgun.ttf"],
        };
        for name in preferred.into_iter().chain([
            "simsun.ttc",
            "mingliub.ttc",
            "malgun.ttf",
            "meiryo.ttc",
            "msgothic.ttc",
        ]) {
            let path = root.join(name);
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    #[cfg(target_os = "macos")]
    for path in [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    ] {
        paths.push(PathBuf::from(path));
    }
    #[cfg(target_os = "linux")]
    for path in [
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-VF.otf",
        "/usr/share/fonts/truetype/noto/NotoSansJP-Regular.otf",
        "/usr/share/fonts/truetype/nanum/NanumGothic.ttf",
    ] {
        paths.push(PathBuf::from(path));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_locale_catalogs_are_valid_complete_and_placeholder_compatible() {
        validate_embedded_catalogs().unwrap_or_else(|errors| panic!("{errors:#?}"));
        assert_eq!(release_locales().count(), 10);
        assert!(locale_registry()
            .iter()
            .any(|locale| locale.id == "qps-ploc"));
        assert!(locale_registry()
            .iter()
            .any(|locale| locale.id == "qps-rtl"));
        assert!(!selectable_locales(false).any(|locale| locale.audience == "developer"));
    }

    #[test]
    fn fallback_and_structured_substitution_are_safe() {
        assert_eq!(
            tr("unknown-locale", "home.render.action"),
            tr("en", "home.render.action")
        );
        assert_eq!(tr("es", "not.a.real.key"), "Sono-Renderer");
        assert_eq!(
            substitute("Rendered {count} frames", &[("count", "12")]),
            "Rendered 12 frames"
        );
        assert!(parse_placeholders("bad { unclosed").is_err());
        assert!(parse_placeholders("bad }").is_err());
        assert!(serde_json::from_str::<UniqueStringMap>(r#"{"key":"one","key":"two"}"#).is_err());
        assert_eq!(plural_category("ru", 1), "one");
        assert_eq!(plural_category("ru", 4), "few");
        assert_eq!(plural_category("ru", 11), "many");
        assert_eq!(plural_category("ja", 1), "other");
        assert_eq!(plural_category("fr", 0), "one");
    }

    #[test]
    fn real_locale_catalogs_are_independent_unicode_resources() {
        assert_ne!(
            tr("zh-CN", "home.project.title"),
            tr("zh-TW", "home.project.title")
        );
        for (locale, expected) in [
            ("ja", "日本語"),
            ("zh-CN", "简体中文"),
            ("zh-TW", "繁體中文"),
            ("ko", "한국어"),
            ("es", "Español"),
            ("pt-BR", "Português (Brasil)"),
            ("fr", "Français"),
            ("de", "Deutsch"),
            ("ru", "Русский"),
        ] {
            assert!(locale_info(locale).unwrap().native_name.contains(expected));
            assert!(!tr(locale, "home.question").contains('\u{fffd}'));
            assert_ne!(tr(locale, "home.question"), tr("en", "home.question"));
        }
        for locale in ["ja", "zh-CN", "zh-TW", "ko", "es", "pt-BR", "ru"] {
            assert!(tr(locale, "home.question")
                .chars()
                .any(|character| !character.is_ascii()));
        }
        assert!(direction_is_rtl("qps-rtl"));
        assert!(tr("qps-ploc", "home.question").starts_with("[!! "));
        assert!(tr("qps-rtl", "home.question").contains("\u{2067}"));
    }

    #[test]
    fn locale_is_presentation_only_and_never_in_a_render_identity() {
        let config = crate::render::RenderConfig::default();
        let original = serde_json::to_value(&config).unwrap();
        let before = crate::content_hash::sha256_hex(&serde_json::to_vec(&original).unwrap());
        let _display = tr("ja", "home.render.action");
        let after = crate::content_hash::sha256_hex(&serde_json::to_vec(&original).unwrap());
        assert_eq!(before, after);
        assert!(serde_json::to_string(&original)
            .unwrap()
            .find("ja")
            .is_none());
    }
}
