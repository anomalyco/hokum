//! `$XDG_CONFIG_HOME/hokum/script.rhai`: a [Rhai](https://rhai.rs) function that sees every
//! notification before it shows and can rewrite it, pick its template, change how long it stays,
//! or drop it.
//!
//! ```rhai
//! fn notify(n) {
//!     if n.app == "Spotify" { return false; }          // never show
//!     if n.body.contains("deploy failed") { n.urgency = 2; }
//!     n.summary.replace("[anoma-ly] ", "");
//!     n                                                  // show it, changed
//! }
//! ```
//!
//! Scripts run with an operation budget, so a runaway loop fails that notification instead of
//! hanging Hokum; a failing script shows the notification unchanged.

use std::time::Duration;

use rhai::{AST, Dynamic, Engine, Map, Scope};

use crate::card::{Note, Timeout};

pub fn path() -> std::path::PathBuf {
    crate::style::path().with_file_name("script.rhai")
}

/// A starting point for a new script.
pub const STARTER: &str = r##"// Hokum runs notify(n) for every notification before it shows.
// Return n (changed or not) to show it, or false to drop it.
//
// n.app, n.summary, n.body, n.context: text
// n.urgency: 0 low, 1 normal, 2 critical
// n.value: 0.0 to 1.0 for a progress bar, or () for none
// n.timeout: seconds to stay, 0 to stay until dismissed, () for the default
// n.template: a template from your theme to draw it with, or () for the theme's choice
// n.actions: [#{ key: "reply", label: "Reply" }, ...]
// n.desktop: the app's desktop entry ("org.gnome.Nautilus"); set it, or n.app, to take that app's icon
// n.icon: the icon's file, or set an icon name or a path

fn notify(n) {
    // Examples:
    // if n.app == "Spotify" { return false; }
    // if n.body.contains("failed") { n.urgency = 2; }
    // if n.app == "Slack" && n.context == "#random" { n.timeout = 3; }
    n
}
"##;

pub struct Script {
    engine: Engine,
    ast: AST,
}

impl Script {
    pub fn compile(src: &str) -> Result<Script, String> {
        let mut engine = Engine::new();
        engine.set_max_operations(200_000);
        engine.set_max_call_levels(32);
        engine.set_max_string_size(64 * 1024);
        engine.set_max_array_size(1024);
        engine.set_max_map_size(256);
        let ast = engine.compile(src).map_err(|e| e.to_string())?;
        if !ast
            .iter_functions()
            .any(|f| f.name == "notify" && f.params.len() == 1)
        {
            return Err("the script needs a function `notify(n)`".into());
        }
        Ok(Script { engine, ast })
    }

    /// Runs `notify(n)`. False when the script dropped it; errors leave `n` as it was.
    pub fn apply(&self, n: &mut Note) -> Result<bool, String> {
        let result: Dynamic = self
            .engine
            .call_fn(&mut Scope::new(), &self.ast, "notify", (to_map(n),))
            .map_err(|e| e.to_string())?;
        if let Some(keep) = result.clone().try_cast::<bool>() {
            return Ok(keep);
        }
        if result.is_unit() {
            return Ok(true);
        }
        let Some(map) = result.try_cast::<Map>() else {
            return Err("notify(n) must return n, a changed copy of it, or false".into());
        };
        let before = (
            n.app.clone(),
            n.entry
                .clone()
                .or_else(|| n.desktop.clone())
                .unwrap_or_default(),
            icon_text(n),
        );
        from_map(n, &map);
        reicon(n, &map, (&before.0, &before.1, &before.2));
        Ok(true)
    }
}

fn to_map(n: &Note) -> Map {
    let mut m = Map::new();
    m.insert("app".into(), n.app.clone().into());
    m.insert("summary".into(), n.summary.clone().into());
    m.insert("body".into(), n.body.clone().into());
    m.insert(
        "context".into(),
        n.context.clone().unwrap_or_default().into(),
    );
    m.insert(
        "desktop".into(),
        n.entry
            .clone()
            .or_else(|| n.desktop.clone())
            .unwrap_or_default()
            .into(),
    );
    m.insert("icon".into(), icon_text(n).into());
    m.insert("urgency".into(), (n.urgency as i64).into());
    m.insert(
        "value".into(),
        n.value
            .map_or(Dynamic::UNIT, |v| Dynamic::from_float(v as f64)),
    );
    m.insert(
        "timeout".into(),
        match n.timeout {
            Timeout::Default => Dynamic::UNIT,
            Timeout::Never => 0_i64.into(),
            Timeout::After(d) => Dynamic::from_float(d.as_secs_f64()),
        },
    );
    m.insert(
        "template".into(),
        n.template.clone().map_or(Dynamic::UNIT, Into::into),
    );
    let actions: rhai::Array = n
        .actions
        .iter()
        .map(|(k, l)| {
            let mut a = Map::new();
            a.insert("key".into(), k.clone().into());
            a.insert("label".into(), l.clone().into());
            Dynamic::from_map(a)
        })
        .collect();
    m.insert("actions".into(), actions.into());
    m.insert("id".into(), (n.id as i64).into());
    m
}

fn text(m: &Map, key: &str) -> Option<String> {
    m.get(key).and_then(|v| v.clone().into_string().ok())
}

fn number(m: &Map, key: &str) -> Option<f64> {
    let v = m.get(key)?;
    v.as_float()
        .ok()
        .or_else(|| v.as_int().ok().map(|i| i as f64))
}

fn icon_text(n: &Note) -> String {
    match &n.icon_src {
        Some(crate::icons::Source::File(p)) => p.to_string_lossy().into_owned(),
        _ => String::new(),
    }
}

/// After the script: an icon it named, else the icon of an entry or app it switched to.
fn reicon(n: &mut Note, m: &Map, before: (&str, &str, &str)) {
    let (app, desktop, icon) = before;
    if let Some(v) = text(m, "icon").filter(|v| v != icon && !v.is_empty())
        && let Some(p) = crate::icons::icon(&v)
    {
        n.icon_src = Some(crate::icons::Source::File(p));
        return;
    }
    let entry = match text(m, "desktop").filter(|v| v != desktop && !v.is_empty()) {
        Some(d) => crate::entries::by_id(&d),
        None if n.app != app => crate::entries::by_name(&n.app),
        None => return,
    };
    if let Some(e) = entry {
        if let Some(p) = e.icon() {
            n.icon_src = Some(crate::icons::Source::File(p));
        }
        n.desktop = n.desktop.take().or_else(|| Some(e.id.clone()));
        n.entry = Some(e.id);
    }
}

fn from_map(n: &mut Note, m: &Map) {
    if let Some(v) = text(m, "app") {
        n.app = v;
    }
    if let Some(v) = text(m, "summary") {
        n.summary = v;
    }
    if let Some(v) = text(m, "body") {
        n.body = v;
    }
    if let Some(v) = text(m, "context") {
        n.context = Some(v).filter(|c| !c.is_empty());
    }
    if let Some(u) = m.get("urgency").and_then(|v| v.as_int().ok()) {
        n.urgency = u.clamp(0, 2) as u8;
    }
    n.value = number(m, "value").map(|v| (v as f32).clamp(0.0, 1.0));
    if m.get("timeout").is_some_and(|v| v.is_unit()) {
        n.timeout = Timeout::Default;
    } else if let Some(s) = number(m, "timeout") {
        n.timeout = if s <= 0.0 {
            Timeout::Never
        } else {
            Timeout::After(Duration::from_secs_f64(s.min(86_400.0)))
        };
    }
    n.template = text(m, "template").filter(|t| !t.is_empty());
    if let Some(list) = m.get("actions").and_then(|v| v.clone().into_array().ok()) {
        n.actions = list
            .into_iter()
            .filter_map(|a| {
                let a = a.try_cast::<Map>()?;
                Some((text(&a, "key")?, text(&a, "label").unwrap_or_default()))
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_rewrite_drop_and_survive_their_own_mistakes() {
        let s = Script::compile(
            r#"
fn notify(n) {
    if n.app == "Spotify" { return false; }
    n.summary.replace("[anoma-ly] ", "");
    if n.body.contains("failed") { n.urgency = 2; n.timeout = 0; }
    n.template = "chat";
    n
}"#,
        )
        .unwrap();
        let mut n = Note::new(1, "Slack", "[anoma-ly] in #ops", "deploy failed");
        assert_eq!(s.apply(&mut n), Ok(true));
        assert_eq!(
            (
                n.summary.as_str(),
                n.urgency,
                n.timeout,
                n.template.as_deref()
            ),
            ("in #ops", 2, Timeout::Never, Some("chat"))
        );
        let mut m = Note::new(2, "Spotify", "song", "");
        assert_eq!(s.apply(&mut m), Ok(false));

        let looping = Script::compile("fn notify(n) { loop {} }").unwrap();
        let mut k = Note::new(3, "x", "y", "");
        assert!(
            looping.apply(&mut k).is_err(),
            "the operation budget stops a runaway script"
        );
        assert!(Script::compile("fn other(n) { n }").is_err());
        Script::compile(STARTER).unwrap();
    }
}
