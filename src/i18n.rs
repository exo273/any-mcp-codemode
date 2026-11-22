//! User-facing text through Project Fluent. Locales are embedded; `en` is source and fallback.

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use std::sync::OnceLock;
use unic_langid::LanguageIdentifier;

/// Embedded locales: `(language, .ftl source)`. Adding a language = adding a line + a file.
const LOCALES: &[(&str, &str)] = &[
    ("en", include_str!("../data/locales/en.ftl")),
    ("es", include_str!("../data/locales/es.ftl")),
];

struct I18n {
    primary: FluentBundle<FluentResource>,
    fallback: FluentBundle<FluentResource>,
}

static LANG: OnceLock<String> = OnceLock::new();

fn bundle(lang: &str) -> FluentBundle<FluentResource> {
    let src = LOCALES
        .iter()
        .find(|(l, _)| *l == lang)
        .map_or(LOCALES[0].1, |(_, s)| s);
    let id: LanguageIdentifier = lang.parse().unwrap_or_default();
    let mut b = FluentBundle::new(vec![id]);
    b.set_use_isolating(false);
    let res = FluentResource::try_new(src.to_string()).unwrap_or_else(|(r, _)| r);
    let _ = b.add_resource(res);
    b
}

/// Pick the language: explicit (flag/env/config) → `LC_ALL`/`LC_MESSAGES`/`LANG` → OS → `en`.
/// Region and script are dropped (`es-CL` → `es`); unknown languages fall back to `en`.
#[must_use]
pub fn resolve(explicit: Option<&str>) -> String {
    let posix = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| {
        std::env::var(k)
            .ok()
            .filter(|v| !v.is_empty() && v != "C" && v != "POSIX")
    });
    let raw = explicit
        .map(str::to_string)
        .or(posix)
        .or_else(sys_locale::get_locale)
        .unwrap_or_default();
    let lang = raw
        .split(['-', '_', '.', '@'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if LOCALES.iter().any(|(l, _)| *l == lang) {
        lang
    } else {
        "en".into()
    }
}

/// Set the process language once (first call wins).
pub fn init(lang: &str) {
    let _ = LANG.set(lang.to_string());
}

fn with<R>(f: impl FnOnce(&I18n) -> R) -> R {
    thread_local! {
        static I: I18n = {
            let lang = LANG.get().map_or("en", String::as_str);
            I18n { primary: bundle(lang), fallback: bundle("en") }
        };
    }
    I.with(f)
}

/// Translate `id` (or `id.attr`) with named arguments. Missing keys fall back to `en`, then to the id.
#[must_use]
pub fn tr(id: &str, args: &[(&str, FluentValue<'_>)]) -> String {
    let (msg, attr) = id.split_once('.').map_or((id, None), |(m, a)| (m, Some(a)));
    let mut fa = FluentArgs::new();
    for (k, v) in args {
        fa.set(*k, v.clone());
    }
    with(|i| {
        for b in [&i.primary, &i.fallback] {
            let pattern = b.get_message(msg).and_then(|m| match attr {
                Some(a) => m.get_attribute(a).map(|x| x.value()),
                None => m.value(),
            });
            if let Some(p) = pattern {
                let mut errs = vec![];
                return b.format_pattern(p, Some(&fa), &mut errs).into_owned();
            }
        }
        id.to_string()
    })
}

/// `t!("id")` or `t!("id", name = value, ...)`.
#[macro_export]
macro_rules! t {
    ($id:expr) => { $crate::i18n::tr($id, &[]) };
    ($id:expr, $($k:ident = $v:expr),+ $(,)?) => {
        $crate::i18n::tr($id, &[$((stringify!($k), ::fluent_bundle::FluentValue::from($v))),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(src: &str) -> Vec<String> {
        src.lines()
            .filter(|l| !l.starts_with([' ', '#', '\t']) && l.contains(" = "))
            .map(|l| l.split(" = ").next().unwrap_or_default().to_string())
            .collect()
    }

    fn vars(src: &str, id: &str) -> Vec<String> {
        let start = src.find(&format!("\n{id} = ")).map_or(0, |i| i + 1);
        let block: String = src[start..]
            .lines()
            .take_while(|l| l.starts_with(id) || l.starts_with([' ', '\t']))
            .collect();
        let mut v: Vec<String> = block
            .split("{ $")
            .skip(1)
            .filter_map(|s| s.split([' ', '}']).next())
            .map(String::from)
            .collect();
        v.sort();
        v.dedup();
        v
    }

    #[test]
    fn all_locales_parse_and_share_keys_and_variables() {
        let en = LOCALES[0].1;
        for (lang, src) in LOCALES {
            assert!(
                FluentResource::try_new((*src).to_string()).is_ok(),
                "{lang}.ftl has syntax errors"
            );
            assert_eq!(ids(src), ids(en), "{lang}.ftl keys differ from en.ftl");
            for id in ids(en) {
                assert_eq!(
                    vars(src, &id),
                    vars(en, &id),
                    "{lang}.ftl `{id}` variables differ"
                );
            }
        }
    }

    #[test]
    fn formats_plurals_attributes_and_fallbacks() {
        let es = bundle("es");
        let p = es
            .get_message("doctor-ok")
            .and_then(|m| m.value())
            .expect("key");
        let mut a = FluentArgs::new();
        a.set("server", "gh");
        a.set("count", 1);
        assert_eq!(
            es.format_pattern(p, Some(&a), &mut vec![]),
            "✓ gh: 1 herramienta"
        );
        a.set("count", 3);
        assert_eq!(
            es.format_pattern(p, Some(&a), &mut vec![]),
            "✓ gh: 3 herramientas"
        );
        assert!(tr("CM0002.hint", &[("path", "x".into())]).contains('x'));
        assert_eq!(tr("no-such-key", &[]), "no-such-key");
    }

    #[test]
    fn resolve_normalizes_and_falls_back() {
        assert_eq!(resolve(Some("es-CL")), "es");
        assert_eq!(resolve(Some("es_ES.UTF-8")), "es");
        assert_eq!(resolve(Some("fr")), "en");
    }
}
