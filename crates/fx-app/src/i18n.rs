//! UI translations: one TOML catalog per language in `locales/`, embedded at compile time.
//!
//! English is the source of truth and the fallback. Keys are dotted (`cmd.open`); `{name}` marks a placeholder.
//! Plural texts are groups of `key.one`, `key.few`, `key.many`, `key.other`; [`tn`] picks the form and fills `{n}`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// The UI language. `System` follows the OS locale and falls back to English.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    En,
    #[serde(rename = "de")]
    De,
    #[serde(rename = "uk")]
    Uk,
    #[serde(rename = "es")]
    Es,
    #[serde(rename = "fr")]
    Fr,
    #[serde(rename = "zh-CN")]
    Zh,
}

impl Language {
    pub const ALL: [Language; 7] = [
        Self::System,
        Self::En,
        Self::De,
        Self::Uk,
        Self::Es,
        Self::Fr,
        Self::Zh,
    ];

    /// The languages that have a catalog.
    pub const CATALOGS: [Language; 6] =
        [Self::En, Self::De, Self::Uk, Self::Es, Self::Fr, Self::Zh];

    /// Name of the language in that language. `System` has none; use the `language.system` text.
    pub fn native_name(self) -> &'static str {
        match self {
            Self::System => "",
            Self::En => "English",
            Self::De => "Deutsch",
            Self::Uk => "Українська",
            Self::Es => "Español",
            Self::Fr => "Français",
            Self::Zh => "简体中文",
        }
    }

    /// `System` replaced by the OS language (English when unknown or unsupported).
    pub fn resolved(self) -> Self {
        match self {
            Self::System => sys_locale::get_locale().map_or(Self::En, |tag| from_tag(&tag)),
            other => other,
        }
    }

    /// Whether the language needs a font with CJK glyphs.
    pub fn needs_cjk(self) -> bool {
        self.resolved() == Self::Zh
    }

    fn source(self) -> &'static str {
        match self {
            Self::En | Self::System => include_str!("../locales/en.toml"),
            Self::De => include_str!("../locales/de.toml"),
            Self::Uk => include_str!("../locales/uk.toml"),
            Self::Es => include_str!("../locales/es.toml"),
            Self::Fr => include_str!("../locales/fr.toml"),
            Self::Zh => include_str!("../locales/zh-CN.toml"),
        }
    }

    /// CLDR plural category of the integer `n`.
    pub fn plural(self, n: u64) -> &'static str {
        match self {
            Self::Uk => match (n % 10, n % 100) {
                (1, r) if r != 11 => "one",
                (2..=4, r) if !(12..=14).contains(&r) => "few",
                _ => "many",
            },
            Self::Fr if n <= 1 => "one",
            Self::En | Self::De | Self::Es if n == 1 => "one",
            _ => "other",
        }
    }

    /// Plural forms a catalog must provide.
    pub fn plural_forms(self) -> &'static [&'static str] {
        match self {
            Self::Uk => &["one", "few", "many", "other"],
            Self::Zh => &["other"],
            _ => &["one", "other"],
        }
    }
}

/// Maps an OS locale tag such as `de-DE`, `uk_UA.UTF-8` or `zh-Hans-CN` to a language.
fn from_tag(tag: &str) -> Language {
    let lang = tag.split(['-', '_', '.', '@']).next().unwrap_or("");
    match lang.to_ascii_lowercase().as_str() {
        "de" => Language::De,
        "uk" => Language::Uk,
        "es" => Language::Es,
        "fr" => Language::Fr,
        "zh" => Language::Zh,
        _ => Language::En,
    }
}

type Catalog = HashMap<&'static str, &'static str>;

fn flatten(prefix: &str, table: &toml::Table, out: &mut Catalog) {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::Table(inner) => flatten(&path, inner, out),
            toml::Value::String(text) => {
                let (path, text) = (path.leak(), text.clone().leak());
                out.insert(path, text);
            }
            other => panic!("locale key {path}: expected a string, found {other}"),
        }
    }
}

/// All texts of `lang`, parsed once. Catalogs are embedded, so a parse failure is a build bug.
pub fn catalog(lang: Language) -> &'static HashMap<&'static str, &'static str> {
    static CELLS: [OnceLock<Catalog>; 7] = [const { OnceLock::new() }; 7];
    let lang = if lang == Language::System {
        Language::En
    } else {
        lang
    };
    CELLS[lang as usize].get_or_init(|| {
        let table: toml::Table = lang.source().parse().expect("locale catalog is valid TOML");
        let mut out = Catalog::new();
        flatten("", &table, &mut out);
        out
    })
}

static CURRENT: AtomicU8 = AtomicU8::new(Language::En as u8);

/// Sets the language used by [`t`], [`tr`] and [`tn`]. Takes effect on the next call.
pub fn set_language(lang: Language) {
    CURRENT.store(lang.resolved() as u8, Ordering::Relaxed);
}

pub fn current() -> Language {
    Language::ALL
        .into_iter()
        .find(|l| *l as u8 == CURRENT.load(Ordering::Relaxed))
        .unwrap_or(Language::En)
}

fn lookup(lang: Language, key: &str) -> Option<&'static str> {
    catalog(lang).get(key).copied()
}

/// The text for `key` in `lang`, else in English, else `None`.
pub fn find_in(lang: Language, key: &str) -> Option<&'static str> {
    lookup(lang, key).or_else(|| lookup(Language::En, key))
}

/// The text for `key` in the current language. A missing key yields the key itself.
pub fn t(key: &'static str) -> &'static str {
    find_in(current(), key).unwrap_or(key)
}

/// Like [`t`] with every `{name}` replaced by its value.
pub fn tr(key: &'static str, args: &[(&str, &str)]) -> String {
    render(current(), key, args)
}

/// Plural text for `n`; `{n}` is filled in addition to `args`.
pub fn tn(key: &'static str, n: u64, args: &[(&str, &str)]) -> String {
    render_plural(current(), key, n, args)
}

fn fill(text: &str, args: &[(&str, &str)]) -> String {
    args.iter().fold(text.to_owned(), |text, (name, value)| {
        text.replace(&format!("{{{name}}}"), value)
    })
}

pub(crate) fn render(lang: Language, key: &'static str, args: &[(&str, &str)]) -> String {
    fill(find_in(lang, key).unwrap_or(key), args)
}

pub(crate) fn render_plural(
    lang: Language,
    key: &'static str,
    n: u64,
    args: &[(&str, &str)],
) -> String {
    let text = [lang, Language::En]
        .into_iter()
        .find_map(|l| {
            lookup(l, &format!("{key}.{}", l.plural(n)))
                .or_else(|| lookup(l, &format!("{key}.other")))
        })
        .unwrap_or(key);
    let n = n.to_string();
    let mut all = vec![("n", n.as_str())];
    all.extend_from_slice(args);
    fill(text, &all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    const FORMS: [&str; 5] = ["zero", "one", "few", "many", "other"];

    /// Splits `key.form` into (`key`, `Some(form)`); other keys are (`key`, `None`).
    fn split(key: &str) -> (&str, Option<&str>) {
        match key.rsplit_once('.') {
            Some((base, form)) if FORMS.contains(&form) => (base, Some(form)),
            _ => (key, None),
        }
    }

    fn placeholders(text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            let Some(len) = rest[start..].find('}') else {
                panic!("unclosed placeholder in {text:?}");
            };
            out.insert(rest[start + 1..start + len].to_owned());
            rest = &rest[start + len + 1..];
        }
        out
    }

    /// base key -> (forms present, placeholders of all forms).
    fn groups(lang: Language) -> BTreeMap<String, (BTreeSet<Option<String>>, BTreeSet<String>)> {
        let mut out: BTreeMap<String, (BTreeSet<Option<String>>, BTreeSet<String>)> =
            BTreeMap::new();
        for (key, text) in catalog(lang) {
            let (base, form) = split(key);
            let entry = out.entry(base.to_owned()).or_default();
            entry.0.insert(form.map(str::to_owned));
            entry.1.extend(placeholders(text));
        }
        out
    }

    #[test]
    fn every_catalog_matches_english() {
        let en = groups(Language::En);
        for lang in Language::CATALOGS {
            let other = groups(lang);
            let missing: Vec<_> = en.keys().filter(|k| !other.contains_key(*k)).collect();
            let extra: Vec<_> = other.keys().filter(|k| !en.contains_key(*k)).collect();
            assert!(missing.is_empty(), "{lang:?} misses keys {missing:?}");
            assert!(extra.is_empty(), "{lang:?} has unknown keys {extra:?}");
            for (key, (forms, holders)) in &en {
                let (got_forms, got_holders) = &other[key];
                assert_eq!(holders, got_holders, "{lang:?} placeholders of {key}");
                if forms.contains(&None) {
                    assert_eq!(got_forms, forms, "{lang:?} must not pluralize {key}");
                } else {
                    let want: BTreeSet<_> = lang
                        .plural_forms()
                        .iter()
                        .map(|f| Some((*f).to_owned()))
                        .collect();
                    assert_eq!(got_forms, &want, "{lang:?} plural forms of {key}");
                }
            }
        }
    }

    #[test]
    fn ukrainian_plurals() {
        let forms = |ns: &[u64]| {
            ns.iter()
                .map(|&n| Language::Uk.plural(n))
                .collect::<Vec<_>>()
        };
        assert_eq!(forms(&[1, 21, 101]), ["one"; 3]);
        assert_eq!(forms(&[2, 3, 4, 22, 34]), ["few"; 5]);
        assert_eq!(forms(&[0, 5, 11, 12, 14, 20, 25, 111]), ["many"; 8]);
    }

    #[test]
    fn plural_text_is_chosen_and_filled() {
        let uk = |n| render_plural(Language::Uk, "notice.deleted", n, &[]);
        assert_eq!(uk(1), "Видалено 1 елемент");
        assert_eq!(uk(2), "Видалено 2 елементи");
        assert_eq!(uk(5), "Видалено 5 елементів");
        assert_eq!(uk(21), "Видалено 21 елемент");
        assert_eq!(
            render_plural(Language::En, "notice.deleted", 1, &[]),
            "Deleted 1 item"
        );
        assert_eq!(
            render_plural(Language::Fr, "notice.deleted", 0, &[]),
            "0 élément supprimé"
        );
        assert_eq!(
            render_plural(Language::Zh, "notice.deleted", 3, &[]),
            "已删除3项"
        );
    }

    #[test]
    fn missing_key_falls_back_to_english_then_key() {
        assert_eq!(render(Language::De, "no.such.key", &[]), "no.such.key");
        assert_eq!(
            render(Language::En, "notice.exists", &[("name", "a")]),
            "An item named a already exists"
        );
    }

    #[test]
    fn os_tags_map_to_languages() {
        for (tag, want) in [
            ("de-DE", Language::De),
            ("uk_UA.UTF-8", Language::Uk),
            ("zh-Hans-CN", Language::Zh),
            ("fr", Language::Fr),
            ("es_MX", Language::Es),
            ("C", Language::En),
            ("ja-JP", Language::En),
        ] {
            assert_eq!(from_tag(tag), want, "{tag}");
        }
    }

    #[test]
    fn language_round_trips_through_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W {
            l: Language,
        }
        for l in Language::ALL {
            let text = toml::to_string(&W { l }).unwrap();
            assert_eq!(toml::from_str::<W>(&text).unwrap(), W { l });
        }
        assert_eq!(
            toml::to_string(&W { l: Language::Zh }).unwrap(),
            "l = \"zh-CN\"\n"
        );
    }
}
