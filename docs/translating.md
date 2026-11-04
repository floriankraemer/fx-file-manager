# Translating

The GUI is available in English, German, Ukrainian, Spanish, French and Simplified Chinese.
English is the source of truth.
The other five catalogs are machine translated and may contain mistakes.
Corrections are welcome as pull requests.

## Where the texts live

Each language has one TOML catalog in `crates/fx-app/locales/`:

- `en.toml`
- `de.toml`
- `uk.toml`
- `es.toml`
- `fr.toml`
- `zh-CN.toml`

The catalogs are embedded at compile time.
Keys are dotted, for example `cmd.open` or `notice.renamed`.
A missing key falls back to English, and a key missing everywhere shows the key itself.

## Placeholders

`{name}` marks a value filled in at run time, for example `Could not rename {name}: {error}`.
Keep every placeholder exactly as it is, and translate only the text around it.
The word order may change.

## Plurals

Texts that depend on a count are groups with the suffixes `.one`, `.few`, `.many` and `.other`.
Every text of a group receives the count as `{n}`.
The suffixes a language needs follow CLDR:

| Language | Required suffixes |
| -------- | ----------------- |
| English, German, Spanish, French | `one`, `other` |
| Ukrainian | `one`, `few`, `many`, `other` |
| Simplified Chinese | `other` |

For Ukrainian, `one` is used for 1, 21, 101 and so on, but not 11.
`few` is used for 2 to 4, 22 to 24 and so on, but not 12 to 14.
`many` is used for 0, 5 to 20, 25 to 30 and so on.
French uses `one` for 0 and 1.

## Fixing a translation

1. Edit the text in the catalog of that language.
2. Keep the placeholders.
3. Run `cargo test --workspace`.

The test `every_catalog_matches_english` fails when a locale misses a key, has an unknown key, uses different placeholders, or lacks a plural form.
CI runs it on every pull request.

## Adding a language

1. Copy `en.toml` to `crates/fx-app/locales/<code>.toml` and translate it.
2. Add a variant to `Language` in `crates/fx-app/src/i18n.rs`.
   Give it a serde name, a `native_name`, an entry in `source`, its plural rule in `plural`, and its plural suffixes in `plural_forms`.
3. Add it to `Language::ALL` and `Language::CATALOGS`.
4. Run `cargo test --workspace`.

The language picker lists each language under its own name.
Right-to-left languages are not supported.

## Adding a text

1. Add the key to `en.toml` and to every other catalog.
2. Use `t("key")`, `tr("key", &[("name", value)])` or `tn("key", count, &[])` in the code.

Never put a user-facing string literal in `crates/fx-gui`.

## Chinese fonts

The built-in font has no Chinese glyphs.
When the language is Simplified Chinese, the GUI adds an installed system font as a fallback.
It looks for Noto Sans CJK SC, Noto Sans SC, Source Han Sans SC, WenQuanYi, Microsoft YaHei, SimSun, PingFang SC, Hiragino Sans GB and Heiti SC.
No font is bundled, to keep the binary small.
If none of them is installed, Chinese text shows as boxes.
