//! `config.kdl`: the one file that controls Hokum. It starts from a theme (`theme "capsule"`) and
//! overrides anything: `behaviour`, `tokens`, `template`, `rule`. Themes are the same format, so
//! a config is a theme of your own on top of another.
//!
//! The settings window edits this file through [`Edit`], which changes only the nodes it means
//! to and keeps everything else (comments, order, spacing) as you wrote it.

use std::path::PathBuf;

use kdl::{KdlDocument, KdlEntry, KdlEntryFormat, KdlNode, KdlNodeFormat, KdlValue};

use crate::style::{Behaviour, Depth, Legacy, LegacyStyle};
use crate::theme::{Lift, Problem, Rule, Theme, Tokens};

pub fn path() -> PathBuf {
    crate::style::path()
}

pub const DEFAULT: &str = r#"// Hokum's configuration. The settings window edits this file too, keeping your comments.
// Reference: https://github.com/anomalyco/hokum/blob/dev/docs/theming.md
theme "capsule"
"#;

/// The config, the theme it starts from, and the two merged.
#[derive(Clone)]
pub struct Loaded {
    /// The theme reference (`theme "…"`), "capsule" when unset.
    pub base_id: String,
    /// That theme on its own: what the config's overrides sit on.
    pub base: Theme,
    /// The config's own overrides.
    pub own: Theme,
    /// What Hokum draws with.
    pub theme: Theme,
    pub behaviour: Behaviour,
    /// Sound and Bluetooth, on or off.
    pub modules: crate::theme::Modules,
    /// What's wrong with the config or its theme; Hokum keeps drawing with the last good one.
    pub problems: Vec<Problem>,
}

impl Loaded {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// The config's text: the file, a migrated settings.json, or the default.
fn text() -> String {
    if let Ok(s) = std::fs::read_to_string(path()) {
        return s;
    }
    migrate().unwrap_or_else(|| DEFAULT.to_owned())
}

pub fn load() -> Loaded {
    load_src(&text())
}

fn load_src(src: &str) -> Loaded {
    let mut problems = Vec::new();
    let own = match crate::theme::parse_config(src) {
        Ok(t) => t,
        Err(p) => {
            problems.extend(p);
            Theme::empty()
        }
    };
    let base_id = own.base.clone().unwrap_or_else(|| "capsule".into());
    let base = match crate::theme::resolve(&base_id) {
        Ok(t) => t,
        Err(p) => {
            problems.extend(p.into_iter().map(|p| Problem {
                line: p.line,
                message: format!("theme `{base_id}`: {}", p.message),
            }));
            crate::theme::resolve("capsule").unwrap_or_else(|_| Theme::empty())
        }
    };
    let mut theme = base.merge(&own);
    // The theme's templates were laid out at its own font-size, not the config's.
    theme.design_size = Some(
        base.design_size
            .unwrap_or_else(|| base.base(false).font_size),
    );
    theme.id = base.id.clone();
    theme.name = base.name.clone();
    problems.extend(crate::theme::missing_templates(&own.rules, &theme));
    let behaviour = own.behaviour();
    let modules = own.modules;
    Loaded {
        base_id,
        base,
        own,
        theme,
        behaviour,
        modules,
        problems,
    }
}

/// `hokum check [path]`: loads the config (or `path`, a config or theme file) the way the
/// service does, prints its problems to stderr and says whether there were none.
pub fn check(file: Option<&str>) -> bool {
    let (shown, problems, what) = match file {
        None => {
            let l = load();
            let sounds = crate::chime::problems(&l.behaviour, &l.theme.rules);
            let mut problems: Vec<(String, Problem)> =
                l.problems
                    .into_iter()
                    .map(|p| (path().display().to_string(), p))
                    .chain(sounds.into_iter().map(|message| {
                        (path().display().to_string(), Problem { line: 0, message })
                    }))
                    .collect();
            let script = crate::script::path();
            if let Ok(src) = std::fs::read_to_string(&script)
                && let Err(e) = crate::script::Script::compile(&src)
            {
                problems.push((
                    script.display().to_string(),
                    Problem {
                        line: 0,
                        message: e,
                    },
                ));
            }
            (
                path().display().to_string(),
                problems,
                format!("theme \"{}\"", l.base_id),
            )
        }
        Some(f)
            if std::path::Path::new(f)
                .file_name()
                .is_some_and(|n| n == "config.kdl") =>
        {
            match std::fs::read_to_string(f) {
                Ok(src) => {
                    let l = load_src(&src);
                    (
                        f.to_owned(),
                        l.problems.into_iter().map(|p| (f.to_owned(), p)).collect(),
                        format!("theme \"{}\"", l.base_id),
                    )
                }
                Err(e) => (
                    f.to_owned(),
                    vec![(
                        f.to_owned(),
                        Problem {
                            line: 0,
                            message: e.to_string(),
                        },
                    )],
                    String::new(),
                ),
            }
        }
        Some(f) => {
            let abs = std::path::absolute(f).unwrap_or_else(|_| f.into());
            match crate::theme::resolve(&abs.to_string_lossy()) {
                Ok(t) => (f.to_owned(), Vec::new(), format!("theme \"{}\"", t.name)),
                Err(p) => (
                    f.to_owned(),
                    p.into_iter().map(|p| (f.to_owned(), p)).collect(),
                    String::new(),
                ),
            }
        }
    };
    for (file, p) in &problems {
        if p.line == 0 {
            eprintln!("{file}: {}", p.message)
        } else {
            eprintln!("{file}: {p}")
        }
    }
    if problems.is_empty() {
        println!("{shown}: OK ({what})");
    }
    problems.is_empty()
}

/// settings.json, from before config.kdl, written out as a config once (and set aside).
fn migrate() -> Option<String> {
    let old = crate::style::dir().join("settings.json");
    let legacy: Legacy = serde_json::from_str(&std::fs::read_to_string(&old).ok()?).ok()?;
    let mut e = Edit {
        doc: DEFAULT.parse().ok()?,
    };
    let base = match legacy.theme.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => std::path::Path::new(t)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("soft")
            .to_owned(),
        None => "soft".into(),
    };
    e.set_theme(&base);
    if let Some(p) = legacy.position {
        e.set_behaviour("position", Value::Text(p.word().into()));
    }
    if let Some(l) = legacy.linger.as_deref() {
        e.set_behaviour(
            "stay",
            Value::Number(match l {
                "short" => 4.0,
                "long" => 10.0,
                _ => 6.0,
            }),
        );
    }
    if let Some(n) = legacy.max_shown {
        e.set_behaviour("show-at-most", Value::Number(n as f64));
    }
    if let Some(b) = legacy.pause_on_hover {
        e.set_behaviour("pause-on-hover", Value::Flag(b));
    }
    if let Some(b) = legacy.do_not_disturb {
        e.set_behaviour("do-not-disturb", Value::Flag(b));
    }
    if let Some(a) = legacy.appearance {
        e.set_behaviour("appearance", Value::Text(a.word().into()));
    }
    // The old style sat under no theme; as overrides it sits on Soft.
    if let (Some(s), true) = (legacy.style, base == "soft") {
        legacy_style(&mut e, s, &crate::theme::resolve("soft").ok()?.base(true));
    }
    let text = e.doc.to_string();
    if std::fs::write(path(), &text).is_ok() {
        let _ = std::fs::rename(&old, old.with_extension("json.migrated"));
    }
    Some(text)
}

/// The old look controls as tokens, written only where they differ from `soft`.
fn legacy_style(e: &mut Edit, s: LegacyStyle, soft: &Tokens) {
    let n = |v: f32| Value::Number(v as f64);
    let (radius, badge) = s.corners.radii();
    if (soft.radius, soft.badge_radius) != (radius, badge) {
        e.set_token("radius", &[n(radius)]);
        e.set_token("badge-radius", &[n(badge)]);
    }
    let (padding, gap, spacing) = s.density.spacing();
    if (soft.padding, soft.gap, soft.spacing) != ([padding; 4], gap, spacing) {
        e.set_token("padding", &[n(padding)]);
        e.set_token("gap", &[n(gap)]);
        e.set_token("spacing", &[n(spacing)]);
    }
    if soft.mark != s.mark {
        e.set_token("mark", &[Value::Text(s.mark.word().into())]);
    }
    let same_depth = matches!(
        (soft.shadow, s.depth),
        (Lift::Flat, Depth::Flat) | (Lift::Soft, Depth::Soft) | (Lift::Deep, Depth::Deep)
    );
    if !same_depth {
        e.set_token("shadow", &[Value::Text(s.depth.word().into())]);
    }
    if soft.width != s.width.points() {
        e.set_token("width", &[n(s.width.points())]);
    }
    match (soft.border.is_some(), s.border) {
        (a, b) if a == b => {}
        (_, true) => e.set_token("border", &[n(1.0), Value::Text("border".into())]),
        (_, false) => e.set_token("border", &[Value::Text("none".into())]),
    }
}

/// A value to write: Hokum spells it the way a person would.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    Number(f64),
    Flag(bool),
}

impl Value {
    fn repr(&self) -> String {
        match self {
            Value::Text(s) => format!("{s:?}"),
            Value::Number(n) if n.fract() == 0.0 && n.abs() < 1e12 => format!("{}", *n as i64),
            Value::Number(n) => format!("{n}"),
            Value::Flag(b) => if *b { "#true" } else { "#false" }.into(),
        }
    }

    fn kdl(&self) -> KdlValue {
        match self {
            Value::Text(s) => KdlValue::String(s.clone()),
            Value::Number(n) if n.fract() == 0.0 => KdlValue::Integer(*n as i128),
            Value::Number(n) => KdlValue::Float(*n),
            Value::Flag(b) => KdlValue::Bool(*b),
        }
    }

    fn entry(&self) -> KdlEntry {
        let mut e = KdlEntry::new(self.kdl());
        e.set_format(KdlEntryFormat {
            value_repr: self.repr(),
            leading: " ".into(),
            ..Default::default()
        });
        e
    }
}

/// Changes the config file without disturbing what it doesn't change.
pub struct Edit {
    doc: KdlDocument,
}

fn args(node: &KdlNode) -> impl Iterator<Item = &KdlValue> {
    node.entries()
        .iter()
        .filter(|e| e.name().is_none())
        .map(|e| e.value())
}

fn props(node: &KdlNode) -> Vec<(&str, &KdlValue)> {
    node.entries()
        .iter()
        .filter_map(|e| Some((e.name()?.value(), e.value())))
        .collect()
}

/// The indentation of a block's children, from the first one (four spaces when empty).
fn indent(block: &KdlDocument) -> String {
    block
        .nodes()
        .first()
        .and_then(|n| n.format())
        .map(|f| f.leading.rsplit('\n').next().unwrap_or("").to_owned())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c == ' ' || c == '\t'))
        .unwrap_or_else(|| "    ".into())
}

/// Appends `node` to `block` on its own line at the block's indentation.
fn push_child(block: &mut KdlDocument, mut node: KdlNode) {
    let leading = indent(block);
    if let Some(last) = block.nodes_mut().last_mut()
        && let Some(f) = last.format_mut()
        && !f.terminator.contains('\n')
        && !f.trailing.contains('\n')
    {
        f.terminator = "\n".into();
    }
    match node.format_mut() {
        // A snippet keeps its layout; it only takes the block's indentation.
        Some(f) => {
            f.leading = leading;
            if !f.terminator.contains('\n') && !f.trailing.contains('\n') {
                f.terminator = "\n".into();
            }
        }
        None => node.set_format(KdlNodeFormat {
            leading,
            terminator: "\n".into(),
            ..Default::default()
        }),
    }
    block.nodes_mut().push(node);
}

/// A node written out as KDL text, so it keeps exactly the layout written here.
fn snippet(text: &str) -> KdlNode {
    let doc: KdlDocument = text.parse().expect("snippets are valid KDL");
    doc.nodes()[0].clone()
}

fn new_block(name: &str) -> KdlNode {
    let mut n = KdlNode::new(name);
    n.set_children(KdlDocument::new());
    n
}

impl Edit {
    /// The config as it is on disk (or would be). Refuses to edit a file that doesn't parse,
    /// rather than overwrite what someone is in the middle of writing.
    pub fn open() -> Result<Edit, String> {
        let src = text();
        src.parse::<KdlDocument>()
            .map(|doc| Edit { doc })
            .map_err(|e| format!("config.kdl doesn't parse yet ({e}); fix it and try again"))
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = p.with_extension("kdl.tmp");
        let mut text = self.doc.to_string();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, p)
    }

    #[cfg(test)]
    fn text(&self) -> String {
        self.doc.to_string()
    }

    /// The top-level `theme "…"`, set or added.
    pub fn set_theme(&mut self, id: &str) {
        let v = Value::Text(id.into());
        if let Some(node) = self
            .doc
            .nodes_mut()
            .iter_mut()
            .find(|n| n.name().value() == "theme")
        {
            node.entries_mut().clear();
            node.push(v.entry());
            return;
        }
        let mut node = KdlNode::new("theme");
        node.push(v.entry());
        node.set_format(KdlNodeFormat {
            terminator: "\n".into(),
            ..Default::default()
        });
        // After the leading comments, before everything else.
        let at = self.doc.nodes().iter().position(|_| true).unwrap_or(0);
        self.doc.nodes_mut().insert(at, node);
    }

    /// The top-level block `name` with no arguments (`tokens`, not `tokens "dark"`), added at
    /// the end when missing.
    fn block(&mut self, name: &str) -> &mut KdlDocument {
        let found = self
            .doc
            .nodes()
            .iter()
            .position(|n| n.name().value() == name && args(n).next().is_none());
        let i = match found {
            Some(i) => i,
            None => {
                if let Some(last) = self.doc.nodes_mut().last_mut()
                    && let Some(f) = last.format_mut()
                    && !f.terminator.contains('\n')
                {
                    f.terminator = "\n".into();
                }
                let mut node = new_block(name);
                node.autoformat();
                if let Some(f) = node.format_mut() {
                    f.leading = "\n".into();
                }
                self.doc.nodes_mut().push(node);
                self.doc.nodes().len() - 1
            }
        };
        self.doc.nodes_mut()[i].ensure_children()
    }

    fn set_in(block: &mut KdlDocument, key: &str, values: &[Value]) {
        if let Some(node) = block
            .nodes_mut()
            .iter_mut()
            .find(|n| n.name().value() == key)
        {
            node.entries_mut().clear();
            for v in values {
                node.push(v.entry());
            }
            return;
        }
        let mut node = KdlNode::new(key);
        for v in values {
            node.push(v.entry());
        }
        push_child(block, node);
    }

    /// Removes `key` from `block`. A node's leading text holds the line break before it, so the
    /// node after a removed one takes that break when it has none of its own.
    fn remove_in(block: &mut KdlDocument, key: &str) {
        while let Some(i) = block.nodes().iter().position(|n| n.name().value() == key) {
            let gone = block.nodes_mut().remove(i);
            let lead = gone.format().map(|f| f.leading.clone()).unwrap_or_default();
            if let Some(next) = block.nodes_mut().get_mut(i)
                && let Some(f) = next.format_mut()
                && lead.contains('\n')
                && !f.leading.contains('\n')
            {
                let breaks: String = lead
                    .chars()
                    .take_while(|c| c.is_whitespace() && *c != ' ' && *c != '\t')
                    .collect();
                f.leading = format!(
                    "{}{}",
                    if breaks.is_empty() {
                        "\n".into()
                    } else {
                        breaks
                    },
                    f.leading
                );
            }
        }
    }

    /// Drops a top-level block left empty.
    fn prune(&mut self, name: &str) {
        self.doc.nodes_mut().retain(|n| {
            !(n.name().value() == name
                && args(n).next().is_none()
                && n.children().is_some_and(|c| c.nodes().is_empty()))
        });
    }

    pub fn set_token(&mut self, key: &str, values: &[Value]) {
        Self::set_in(self.block("tokens"), key, values);
    }

    pub fn set_behaviour(&mut self, key: &str, value: Value) {
        Self::set_in(self.block("behaviour"), key, &[value]);
    }

    pub fn remove_behaviour(&mut self, key: &str) {
        if self
            .doc
            .nodes()
            .iter()
            .any(|n| n.name().value() == "behaviour")
        {
            Self::remove_in(self.block("behaviour"), key);
            self.prune("behaviour");
        }
    }

    /// `modules { <key> #false }`, or no line when it's on (the default); an empty block goes.
    pub fn set_module(&mut self, key: &str, on: bool) {
        if on {
            if self
                .doc
                .nodes()
                .iter()
                .any(|n| n.name().value() == "modules")
            {
                Self::remove_in(self.block("modules"), key);
                self.prune("modules");
            }
        } else {
            Self::set_in(self.block("modules"), key, &[Value::Flag(false)]);
        }
    }

    /// The `rule app="…"` the settings window manages for `app`: one matcher, `app`, exactly.
    fn app_rule(&mut self, app: &str, create: bool) -> Option<&mut KdlNode> {
        let i = self
            .doc
            .nodes()
            .iter()
            .position(|n| n.name().value() == "rule" && is_app_rule(n, app));
        let i = match (i, create) {
            (Some(i), _) => i,
            (None, false) => return None,
            (None, true) => {
                let mut node = snippet(&format!(
                    "rule app={} {{\n}}\n",
                    Value::Text(app.into()).repr()
                ));
                if let Some(f) = node.format_mut() {
                    f.leading = "\n".into();
                }
                if let Some(last) = self.doc.nodes_mut().last_mut()
                    && let Some(f) = last.format_mut()
                    && !f.terminator.contains('\n')
                {
                    f.terminator = "\n".into();
                }
                self.doc.nodes_mut().push(node);
                self.doc.nodes().len() - 1
            }
        };
        Some(&mut self.doc.nodes_mut()[i])
    }

    /// Sets (`Some`) or removes (`None`) one action in `app`'s rule; a rule left with nothing
    /// to do goes.
    pub fn set_app(&mut self, app: &str, action: &str, value: Option<Vec<Value>>) {
        let create = value.is_some();
        let Some(rule) = self.app_rule(app, create) else {
            return;
        };
        let block = rule.ensure_children();
        match value {
            Some(v) => Self::set_in(block, action, &v),
            None => Self::remove_in(block, action),
        }
        self.tidy(app);
    }

    /// Drops `app`'s rule when it has nothing left to do.
    fn tidy(&mut self, app: &str) {
        let app = app.to_owned();
        self.doc.nodes_mut().retain(|n| {
            !(n.name().value() == "rule"
                && is_app_rule(n, &app)
                && n.children().is_none_or(|c| c.nodes().is_empty()))
        });
    }
}

fn is_app_rule(n: &KdlNode, app: &str) -> bool {
    let p = props(n);
    args(n).next().is_none()
        && p.len() == 1
        && p[0].0 == "app"
        && p[0]
            .1
            .as_string()
            .is_some_and(|a| a.eq_ignore_ascii_case(app))
}

/// What the settings window shows for an app, from the config's rule for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppRule {
    pub hide: bool,
    pub interrupt: bool,
    /// Seconds, or 0 for until dismissed.
    pub stay: Option<f32>,
}

/// Whether a rule is one the settings window manages: it matches one app by its exact name.
pub fn simple_app(r: &Rule) -> Option<&str> {
    let app = r.app.as_deref()?;
    (r.summary.is_none() && r.body.is_none() && r.urgency.is_none() && !app.contains('*'))
        .then_some(app)
}

/// The settings window's view of `own`'s rules: per app, and how many it can't show.
pub fn app_rules(own: &Theme) -> (Vec<(String, AppRule)>, usize) {
    let mut apps: Vec<(String, AppRule)> = Vec::new();
    let mut custom = 0;
    for r in &own.rules {
        let Some(app) = simple_app(r) else {
            custom += 1;
            continue;
        };
        let rule = AppRule {
            hide: r.hide,
            interrupt: r.interrupt,
            stay: r.timeout.map(|t| match t {
                crate::card::Timeout::After(d) => d.as_secs_f32(),
                _ => 0.0,
            }),
        };
        match apps.iter_mut().find(|(a, _)| a.eq_ignore_ascii_case(app)) {
            Some((_, existing)) => *existing = rule,
            None => apps.push((app.to_owned(), rule)),
        }
    }
    (apps, custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_card_scales_from_font_size_and_the_desktop() {
        let n = crate::card::Note::new(1, "Slack", "Dax", "ship it");
        let close = |a: f32, b: f32| (a - b).abs() < 0.001;
        // Stock: the theme's own size, nothing scaled.
        let stock = load_src("theme \"soft\"\n")
            .theme
            .look_at(&n, false, 1.0)
            .tokens;
        assert_eq!((stock.font_size, stock.scale), (13.0, 1.0));
        // config.kdl's font-size over the theme's 13: the template's own sizes follow.
        let bigger = load_src("theme \"soft\"\ntokens { font-size 15; }\n")
            .theme
            .look_at(&n, false, 1.0)
            .tokens;
        assert!(close(bigger.font_size, 15.0) && close(bigger.scale, 15.0 / 13.0));
        // A theme laid out at another size keeps its proportions: Terminal is 12 on its own.
        let terminal = load_src("theme \"terminal\"\n")
            .theme
            .look_at(&n, false, 1.0)
            .tokens;
        assert_eq!((terminal.font_size, terminal.scale), (12.0, 1.0));
        // The desktop's text scale multiplies both.
        let large = load_src("theme \"soft\"\ntokens { font-size 15; }\n")
            .theme
            .look_at(&n, false, 1.25)
            .tokens;
        assert!(close(large.font_size, 18.75) && close(large.scale, 1.25 * 15.0 / 13.0));
        // Absurd combinations stay readable and inside the card.
        let tiny = load_src("theme \"soft\"\ntokens { font-size 6; }\n")
            .theme
            .look_at(&n, false, 0.5)
            .tokens;
        assert!(close(tiny.font_size, 6.0) && close(tiny.scale, 6.0 / 13.0));
        let huge = load_src("theme \"soft\"\ntokens { font-size 24; }\n")
            .theme
            .look_at(&n, false, 3.0)
            .tokens;
        assert!(close(huge.font_size, 48.0) && close(huge.scale, 48.0 / 13.0));
        // A rule's font-size for one app scales that card the same way.
        let rule = load_src("theme \"soft\"\nrule app=\"Slack\" { tokens { font-size 16; } }\n")
            .theme
            .look_at(&n, false, 1.0)
            .tokens;
        assert!(close(rule.font_size, 16.0) && close(rule.scale, 16.0 / 13.0));
    }

    fn edit(src: &str) -> Edit {
        Edit {
            doc: src.parse().unwrap(),
        }
    }

    #[test]
    fn the_default_is_capsule_with_app_icons() {
        let own = crate::theme::parse_config(DEFAULT).unwrap();
        assert_eq!(own.base.as_deref(), Some("capsule"));
        let t = crate::theme::resolve("").unwrap();
        assert_eq!(t.id, "capsule");
        assert_eq!(t.base(true).mark, crate::style::Mark::Icon);
    }

    #[test]
    fn edits_keep_comments_and_order() {
        let mut e = edit(
            "// mine\ntheme \"signal\" // base\n\ntokens {\n    radius 10 // rounder\n}\n\nbehaviour {\n    stay 10 // longer\n    position \"top-left\"\n}\n",
        );
        e.set_theme("soft");
        e.set_behaviour("stay", Value::Number(4.0));
        e.set_behaviour("show-at-most", Value::Number(3.0));
        assert_eq!(
            e.text(),
            "// mine\ntheme \"soft\" // base\n\ntokens {\n    radius 10 // rounder\n}\n\nbehaviour {\n    stay 4 // longer\n    position \"top-left\"\n    show-at-most 3\n}\n"
        );
        e.remove_behaviour("stay");
        assert!(
            e.text()
                .contains("behaviour {\n    position \"top-left\"\n"),
            "the next key keeps its own line: {}",
            e.text()
        );
        e.remove_behaviour("position");
        e.remove_behaviour("show-at-most");
        assert!(
            !e.text().contains("behaviour"),
            "an empty block goes: {}",
            e.text()
        );
        assert!(
            e.text().contains("radius 10 // rounder"),
            "tokens are the file's: {}",
            e.text()
        );
        crate::theme::parse_config(&e.text()).unwrap();
    }

    #[test]
    fn app_rules_come_and_go_and_leave_hand_written_ones_alone() {
        let mut e = edit("theme \"soft\"\nrule app=\"Slack*\" { hide }\n");
        e.set_app("Spotify", "hide", Some(vec![]));
        e.set_app("Slack", "timeout", Some(vec![Value::Number(3.0)]));
        e.set_app("Slack", "interrupt", Some(vec![]));
        assert!(
            e.text()
                .contains("rule app=\"Slack\" {\n    timeout 3\n    interrupt\n}"),
            "new rules read like hand-written ones:\n{}",
            e.text()
        );
        let t =
            crate::theme::parse_config(&e.text()).unwrap_or_else(|p| panic!("{p:?}\n{}", e.text()));
        let (apps, custom) = app_rules(&t);
        assert_eq!(custom, 1, "the glob rule isn't the window's");
        assert_eq!(
            apps.iter().map(|a| a.0.as_str()).collect::<Vec<_>>(),
            ["Spotify", "Slack"]
        );
        assert!(apps[0].1.hide && apps[1].1.stay == Some(3.0) && apps[1].1.interrupt);
        e.set_app("Spotify", "hide", None);
        e.set_app("Slack", "timeout", None);
        e.set_app("Slack", "interrupt", None);
        assert_eq!(e.text(), "theme \"soft\"\nrule app=\"Slack*\" { hide }\n");
    }

    #[test]
    fn modules_are_on_unless_the_config_says_otherwise() {
        assert_eq!(load_src(DEFAULT).modules, crate::theme::Modules::default());
        let m = load_src("modules { osd #false; bluetooth-notifications #false }\n").modules;
        assert!(m.sound && m.bluetooth && m.brightness && !m.osd && !m.bluetooth_notifications);
        let m = load_src("modules { brightness #false }\n").modules;
        assert!(!m.brightness && m.osd, "brightness is its own switch");
        let mut e = edit("theme \"soft\"\n");
        e.set_module("bluetooth", false);
        assert!(
            e.text().contains("modules {\n    bluetooth #false\n}"),
            "{}",
            e.text()
        );
        let l = load_src(&e.text());
        assert!(
            l.ok() && l.modules.sound && !l.modules.bluetooth,
            "{:?}",
            l.problems
        );
        e.set_module("bluetooth", true);
        assert_eq!(e.text(), "theme \"soft\"\n");
        assert!(!load_src("modules { radio #true }\n").ok());
        assert!(
            crate::theme::parse("modules { sound #false }\n", "t").is_err(),
            "themes don't set modules"
        );
    }

    #[test]
    fn sounds_are_off_unless_turned_on() {
        use crate::chime::Choice;
        let b = load_src("theme \"soft\"\n").behaviour;
        assert!(
            !b.sound
                && b.sound_name == "message-new-instant"
                && b.sound_critical == "dialog-warning"
        );
        assert!(load_src("behaviour { sound #true; }\n").behaviour.sound);
        let l = load_src("behaviour { sound \"bell\"; sound-critical \"~/alarm.oga\"; }\n");
        assert!(l.ok() && l.behaviour.sound && l.behaviour.sound_name == "bell");
        assert_eq!(l.behaviour.sound_critical, "~/alarm.oga");
        for bad in [
            "sound 3",
            "sound \"a/b\"",
            "sound \"rel/x.oga\"",
            "sound-critical #true",
        ] {
            assert!(
                !load_src(&format!("behaviour {{ {bad}; }}\n")).ok(),
                "{bad}"
            );
        }
        let l = load_src(
            "rule app=\"Slack\" { sound \"/s/ping.wav\"; }\nrule app=\"Spotify\" { sound #false; }\n",
        );
        assert!(l.ok());
        assert_eq!(
            l.theme
                .rules
                .iter()
                .rev()
                .take(2)
                .map(|r| r.sound.clone())
                .collect::<Vec<_>>(),
            [Some(Choice::Off), Some(Choice::Sound("/s/ping.wav".into()))]
        );
    }

    #[test]
    fn clock_is_auto_12h_or_24h() {
        use crate::clock::Clock;
        assert_eq!(load_src("theme \"soft\"\n").behaviour.clock, Clock::Auto);
        let l = load_src("behaviour { clock \"12h\"; }\n");
        assert!(l.ok() && l.behaviour.clock == Clock::H12);
        assert_eq!(
            load_src("behaviour { clock \"24h\"; }\n").behaviour.clock,
            Clock::H24
        );
        assert!(!load_src("behaviour { clock \"am/pm\"; }\n").ok());
    }

    #[test]
    fn app_rules_keep_styling_written_by_hand() {
        let src =
            "rule app=\"Slack\" {\n    tokens { accent \"#d38be0\" }\n    template \"card\"\n}\n";
        let mut e = edit(src);
        e.set_app("Slack", "timeout", Some(vec![Value::Number(3.0)]));
        e.set_app("Slack", "timeout", None);
        assert_eq!(e.text(), src);
        e.set_app("Slack", "hide", Some(vec![]));
        let t = crate::theme::parse_config(&e.text()).unwrap();
        assert!(
            t.rules[0].hide
                && t.rules[0].template.as_deref() == Some("card")
                && t.rules[0].tokens != Default::default()
        );
    }
}
