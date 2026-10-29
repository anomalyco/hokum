//! The Hokum window, hosted by the running service beside the overlay: one sidebar item per
//! module (Notifications, Sound, Bluetooth), each module's levels inside its own section.
//!
//! Notifications picks a theme and sets behaviour and per-app routing in config.kdl (keeping its
//! comments); the look itself is left to theme files, which coding agents write from
//! docs/themes.md. Editing the file by hand updates the window. The preview draws the same cards
//! with the same code. Sound and Bluetooth are `crate::sound::ui` and `crate::bluetooth::ui`;
//! the same state also draws the tray panel (`crate::panel`), with [`Surface::Panel`].

use std::collections::HashMap;
use std::sync::Arc;

use rune::{AppSpec, Color, Cursor, Cx, El, Image, KeyEvent, OverlayHandle, ScrollSpec, Tx, WindowOptions, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl, Sidebar, SidebarRow, Switch};
use zbus::blocking::Connection;

use crate::card::{Cards, Note, card, samples, tint};
use crate::cards::stack;
use crate::config::{AppRule, Edit, Loaded, Value};
use crate::style::{Appearance, Behaviour, Position};
use crate::theme::Theme;
use crate::watch::Config;

const NAV: f32 = 200.0;
const CONTENT: f32 = 560.0;
/// Sound's and Bluetooth's column.
const MODULE: f32 = 520.0;
pub const SIZE: (f32, f32) = (1260.0, 780.0);

/// Whether this process answers `org.freedesktop.Notifications`.
#[derive(Clone, PartialEq, Debug)]
pub enum Daemon {
    Checking,
    Hokum,
    Other(String),
}

#[derive(Clone, PartialEq, Debug)]
enum ScriptStatus {
    None,
    Loaded,
    Broken(String),
}

/// The window's sections, one per module.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Notifications,
    Sound,
    Bluetooth,
}

impl Section {
    pub fn word(self) -> &'static str {
        match self {
            Section::Notifications => "notifications",
            Section::Sound => "sound",
            Section::Bluetooth => "bluetooth",
        }
    }

    pub fn from_word(w: &str) -> Option<Section> {
        match w {
            "notifications" => Some(Section::Notifications),
            "sound" => Some(Section::Sound),
            "bluetooth" | "bt" => Some(Section::Bluetooth),
            _ => None,
        }
    }
}

/// What this state draws: the window, or the tray's panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Surface {
    Window,
    Panel,
}

pub struct Ui {
    pub loaded: Loaded,
    pub daemon: Daemon,
    pub overlay: Option<OverlayHandle<crate::cards::Ui>>,
    pub bus: Option<Connection>,
    pub handle: Option<rune::AppHandle<Ui>>,
    sent: u32,
    error: Option<String>,
    script: ScriptStatus,
    copied: bool,
    page: Page,
    /// Every theme the gallery offers: (id, name, the theme resolved).
    gallery: Vec<(String, String, Theme)>,
    /// Apps that have sent notifications, and apps the config has rules for.
    apps: Vec<String>,
    icons: HashMap<String, Option<Image>>,
    /// The preview's cards and the gallery's card, with icons decoded by `cache`.
    samples: Vec<Note>,
    tile_note: Note,
    cache: Option<rune::ImageCache>,
    pub section: Section,
    pub surface: Surface,
    pub sound: crate::sound::ui::Ui,
    pub bluetooth: crate::bluetooth::ui::Ui,
    /// Starts and stops the modules' backends (the service's window only).
    pub wiring: Option<Arc<crate::modules::Wiring>>,
    /// Why a module couldn't start.
    pub sound_error: Option<String>,
    pub bluetooth_error: Option<String>,
    /// Hides the tray panel (the panel only).
    pub dismiss: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Cards for Ui {
    fn dismiss(&mut self, _: u32) {}
    fn invoke(&mut self, _: u32, _: &str) {}
    fn activate(&mut self, _: u32) {}
    fn hover(&mut self, _: bool) {}
}

impl Ui {
    /// Changes config.kdl, then reloads everything from it.
    fn edit(&mut self, f: impl FnOnce(&mut Edit, &Loaded)) {
        let mut e = match Edit::open() {
            Ok(e) => e,
            Err(msg) => {
                self.error = Some(msg);
                return;
            }
        };
        f(&mut e, &self.loaded);
        self.error = e.save().err().map(|err| format!("Couldn't save config.kdl: {err}"));
        self.reload();
    }

    fn reload(&mut self) {
        let config = crate::watch::load();
        if let Some(o) = &self.overlay {
            let c = config.clone();
            o.update(move |ui, _| ui.configure(c));
        }
        self.configure(config);
    }

    /// The config, its theme and the script from files (at startup, and whenever they change).
    pub fn configure(&mut self, config: Config) {
        if let Some(w) = self.wiring.clone() {
            w.configure(self, &config);
        }
        self.loaded = config.loaded;
        self.bluetooth.sound = self.loaded.modules.sound;
        self.copied = false;
        self.script = match config.script.as_deref().map(crate::script::Script::compile) {
            None => ScriptStatus::None,
            Some(Ok(_)) => ScriptStatus::Loaded,
            Some(Err(e)) => ScriptStatus::Broken(e),
        };
        self.gallery = crate::theme::available()
            .into_iter()
            .filter_map(|(id, name)| crate::theme::resolve(&id).ok().map(|t| (id, name, t)))
            .collect();
        let (ruled, _) = crate::config::app_rules(&self.loaded.own);
        let mut apps = crate::apps::seen();
        for (app, _) in ruled {
            if !apps.iter().any(|a| a.eq_ignore_ascii_case(&app)) {
                apps.push(app);
            }
        }
        self.apps = apps;
        self.decode_icons();
    }

    fn decode_icons(&mut self) {
        let Some(cache) = &self.cache else { return };
        for app in &self.apps {
            self.icons.entry(app.clone()).or_insert_with(|| crate::icons::for_app("", None, app).map(|p| cache.file(p, 64.0, 2.0)));
        }
    }

    fn pick_theme(&mut self, id: String) {
        self.edit(|e, _| e.set_theme(&id));
    }

    fn edit_config(&mut self) {
        let p = crate::config::path();
        if !p.exists() {
            let written = p.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&p, crate::config::DEFAULT));
            if let Err(e) = written {
                self.error = Some(format!("Couldn't write {}: {e}", p.display()));
                return;
            }
        }
        open(&p);
    }

    fn edit_script(&mut self) {
        let path = crate::script::path();
        if !path.exists() {
            let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, crate::script::STARTER));
            if let Err(e) = written {
                self.error = Some(format!("Couldn't write {}: {e}", path.display()));
                return;
            }
        }
        open(&path);
    }

    /// Writes one behaviour value, or removes it when it's what the theme already says.
    fn behave(&mut self, key: &'static str, value: Value, same: bool) {
        self.edit(|e, _| if same { e.remove_behaviour(key) } else { e.set_behaviour(key, value) });
    }

    /// Turns a module on or off in config.kdl; the service starts or stops its backend.
    fn set_module(&mut self, section: Section, on: bool) {
        self.edit(|e, _| e.set_module(section.word(), on));
    }

    pub fn toggle_dnd(&mut self) {
        let v = !self.loaded.behaviour.do_not_disturb;
        self.behave("do-not-disturb", Value::Flag(v), v == Behaviour::default().do_not_disturb);
    }

    /// Goes to a section: in the window at once; from the panel, by opening the window there.
    pub fn show_section(&mut self, section: Section) {
        match self.surface {
            Surface::Window => self.section = section,
            Surface::Panel => {
                if let Some(w) = &self.handle {
                    crate::modules::open(w, Some(section), None);
                }
                if let Some(d) = &self.dismiss {
                    d();
                }
            }
        }
    }

    /// The window was hidden: meters and discovery stop.
    pub fn hidden(&mut self) {
        self.sound.hidden();
        self.bluetooth.hidden();
    }

    /// The window is about to show.
    pub fn shown(&mut self) {
        self.sound.visible = true;
        self.bluetooth.visible = true;
    }

    fn set_app(&mut self, app: String, action: &'static str, value: Option<Vec<Value>>) {
        self.edit(|e, _| e.set_app(&app, action, value));
    }

    /// Take `org.freedesktop.Notifications` from whichever daemon has it.
    fn take_over(&mut self) {
        let (Some(bus), Some(handle)) = (self.bus.clone(), self.handle.clone()) else { return };
        self.daemon = Daemon::Checking;
        std::thread::spawn(move || {
            let daemon = crate::bus::claim(&bus, true);
            handle.update(move |s, _| s.daemon = daemon);
        });
    }

    fn test(&mut self) {
        let Some(o) = &self.overlay else { return };
        let mut all = samples();
        let mut n = all.remove(self.sent as usize % all.len());
        // Far from the ids the bus hands out.
        n.id = u32::MAX - self.sent;
        self.sent += 1;
        o.update(move |c, _| c.upsert(n));
    }
}

fn kit(s: &Ui, cx: &Cx<Ui>) -> (&'static Kit, &'static Kit) {
    let window = rune_theme_opencode::kit(cx.system_dark());
    let preview = match s.loaded.behaviour.appearance {
        Appearance::System => window,
        Appearance::Light => rune_theme_opencode::kit(false),
        Appearance::Dark => rune_theme_opencode::kit(true),
    };
    (window, preview)
}

fn section(k: &Kit, key: &str, title: Option<El>, rows: Vec<El>) -> El {
    let c = &k.c;
    let n = rows.len();
    let mut card = col().key("card").padding(4.0).radius(12.0).fill(c.bg_base).border(1.0, c.border_muted);
    for (i, r) in rows.into_iter().enumerate() {
        card = card.child(r);
        if i + 1 < n {
            card = card.child(el().key(format!("sep{i}")).h(1.0).ml(12.0).mr(12.0).fill(c.border_muted));
        }
    }
    col().key(key).w_full().gap(8.0).children(title).child(card)
}

fn heading(k: &Kit, title: impl Into<String>) -> El {
    text(title).key("header").font(k.ty.ui_medium.lh(16.0)).color(k.c.text_muted).ml(12.0)
}

fn setting(k: &Kit, key: &str, title: &str, description: Option<&str>, control: El) -> El {
    let c = &k.c;
    let mut copy = col().key("copy").grow(1.0).shrink().gap(2.0).child(text(title).key("title").font(k.ty.ui).color(c.text_base));
    if let Some(d) = description {
        copy = copy.child(text(d).key("description").font(k.ty.small).color(c.text_muted).wrap().w_full());
    }
    row()
        .key(key.to_owned())
        .label(title)
        .w_full()
        .min_h(if description.is_some() { 56.0 } else { 44.0 })
        .px(12.0)
        .py(8.0)
        .gap(16.0)
        .center_y()
        .child(copy)
        .child(control)
}

fn toggle(k: &Kit, key: &str, title: &str, description: Option<&str>, on: bool, f: impl Fn(&mut Ui) + Clone + 'static) -> El {
    let c = &k.c;
    let g = f.clone();
    setting(k, key, title, description, Switch::new(on).hide_label().on_toggle(move |s: &mut Ui| g(s)).build(k))
        .radius(8.0)
        .hover(c.hover)
        .cursor(Cursor::Pointer)
        .on_press(move |s: &mut Ui| f(s))
}

/// A segmented control over `options`, nothing selected when `current` isn't one of them.
fn choice<T: Copy + PartialEq + 'static>(
    k: &Kit,
    key: &str,
    options: &'static [(T, &'static str)],
    current: Option<T>,
    width: f32,
    pick: impl Fn(&mut Ui, T) + Clone + 'static,
) -> El {
    let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
    let i = current.and_then(|c| options.iter().position(|o| o.0 == c));
    SegmentedControl::new(&labels, i)
        .width(Some(width))
        .on_pick(move |s: &mut Ui, i: usize, _: &mut Tx| pick(s, options[i].0))
        .build(k)
        .key(key.to_owned())
}

fn button(k: &Kit, key: &str, label: &str, f: fn(&mut Ui)) -> El {
    Button::new(label).variant(ButtonVariant::Ghost).control_size(ControlSize::Regular).on_press(f).build(k).key(key.to_owned())
}

/// A tiny screen with a target in each corner.
fn corner_picker(k: &Kit, current: Position, theme_value: Position) -> El {
    let c = &k.c;
    let corners = [
        (Position::TopLeft, Some(4.0), Some(4.0), None, None, "Top left"),
        (Position::TopRight, None, Some(4.0), Some(4.0), None, "Top right"),
        (Position::BottomLeft, Some(4.0), None, None, Some(4.0), "Bottom left"),
        (Position::BottomRight, None, None, Some(4.0), Some(4.0), "Bottom right"),
    ];
    let mut screen = el().key("screen").size(84.0, 52.0).radius(8.0).fill(c.bg_deep).border(1.0, c.border_base);
    for (i, (p, l, t, r, b, label)) in corners.into_iter().enumerate() {
        let on = p == current;
        screen = screen.child(
            el().key(format!("corner{i}"))
                .role("radio")
                .label(label)
                .pin(l, t, r, b)
                .size(30.0, 16.0)
                .radius(4.0)
                .fill(if on { c.text_base } else { c.layer_02 })
                .hover(c.hover)
                .cursor(Cursor::Pointer)
                .on_press(move |s: &mut Ui| s.behave("position", Value::Text(p.word().into()), p == theme_value)),
        );
    }
    screen
}

const STAY: &[(u32, &str)] = &[(4, "4 s"), (6, "6 s"), (10, "10 s")];
const SHOWN: &[(usize, &str)] = &[(3, "3"), (5, "5"), (8, "8")];
const APPEARANCE: &[(Appearance, &str)] = &[(Appearance::System, "System"), (Appearance::Light, "Light"), (Appearance::Dark, "Dark")];
/// Per-app stay: 0 is the theme's default, u32::MAX until dismissed.
const APP_STAY: &[(u32, &str)] = &[(0, "Default"), (3, "3 s"), (10, "10 s"), (u32::MAX, "Pinned")];

fn status(k: &Kit, s: &Ui) -> El {
    let c = &k.c;
    let (dot, line) = match &s.daemon {
        Daemon::Checking => (c.icon_faint, "Checking the notification daemon…".to_owned()),
        Daemon::Hokum => (c.success.icon, "Hokum is showing your notifications".to_owned()),
        Daemon::Other(name) => (c.warning.icon, format!("{name} is showing your notifications instead")),
    };
    let mut r = row()
        .key("status")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(el().key("dot").size(8.0, 8.0).radius(4.0).fill(dot))
        .child(text(line).key("line").font(k.ty.ui).color(c.text_muted).grow(1.0).shrink().truncate());
    if matches!(s.daemon, Daemon::Other(_)) {
        r = r.child(
            Button::new("Use Hokum")
                .variant(ButtonVariant::Neutral)
                .control_size(ControlSize::Regular)
                .on_press(|s: &mut Ui| s.take_over())
                .build(k)
                .key("take-over"),
        );
    }
    r.child(button(k, "test", "Send test", |s| s.test()))
}

const TILE: (f32, f32) = (108.0, 72.0);

/// One theme in the gallery: a real card drawn with it, scaled down.
fn tile(s: &Ui, k: &Kit, i: usize, id: &str, name: &str, theme: &Theme) -> El {
    let c = &k.c;
    let on = s.loaded.base_id == id;
    let w = theme.base(k.dark).width;
    let scale = (TILE.0 - 12.0) / w;
    let inner = TILE.1 / scale;
    let mini = col()
        .key("mini")
        .size(w, inner)
        .absolute((TILE.0 - w) / 2.0, (TILE.1 - inner) / 2.0)
        .justify_center()
        .scale(scale)
        .child(card::<Ui>(&s.tile_note, theme, k, true));
    let pick = id.to_owned();
    let stage = el()
        .key("stage")
        .size(TILE.0, TILE.1)
        .radius(10.0)
        .fill(c.bg_deep)
        .border(if on { 2.0 } else { 1.0 }, if on { c.text_base } else { c.border_muted })
        .clip_soft(0.0)
        .child(mini)
        // Over the card, so a click picks the theme rather than pressing the card.
        .child(
            el().key("pick")
                .role("radio")
                .label(name.to_owned())
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(10.0)
                .hover(c.hover)
                .press(c.pressed)
                .cursor(Cursor::Pointer)
                .on_press_tx(move |s: &mut Ui, tx: &mut Tx| {
                    // The preview swaps to the new theme at once rather than fading across.
                    tx.instant();
                    s.pick_theme(pick.clone())
                }),
        );
    col()
        .key(format!("theme-{i}"))
        .w(TILE.0)
        .gap(6.0)
        .child(stage)
        .child(text(name.to_owned()).key("name").font(k.ty.small_medium).color(if on { c.text_base } else { c.text_muted }).self_center().truncate())
}

fn themes(s: &Ui, k: &Kit) -> El {
    let c = &k.c;
    let mut tiles: Vec<El> = s.gallery.iter().enumerate().map(|(i, (id, name, t))| tile(s, k, i, id, name, t)).collect();
    let id = &s.loaded.base_id;
    // A theme by path (or one that doesn't load) isn't in the gallery; show it anyway.
    let found = s.loaded.base.id == *id;
    let name = if found { s.loaded.base.name.as_str() } else { "Custom" };
    if !s.gallery.iter().any(|g| g.0 == *id) {
        tiles.push(tile(s, k, tiles.len(), id, name, &s.loaded.base));
    }
    let mut top = col().key("theme-pick").w_full().padding(12.0).gap(12.0).child(row().key("tiles").w_full().gaps(12.0, 10.0).flex_wrap().children(tiles));
    if !crate::theme::is_bundled(id) {
        let line = match crate::theme::find(id) {
            Some(crate::theme::Found::File(p)) if found => format!("Custom theme \u{201c}{name}\u{201d} from {}", home(&p)),
            Some(crate::theme::Found::File(p)) => format!("{} has problems; showing Capsule", home(&p)),
            _ => format!("No theme called \u{201c}{id}\u{201d}; showing Capsule"),
        };
        top = top.child(text(line).key("custom").font(k.ty.small).color(c.text_muted).wrap().w_full().selectable(true));
    }
    let mode = setting(k, "appearance", "Appearance", None, {
        choice(k, "appearance-control", APPEARANCE, Some(s.loaded.behaviour.appearance), 240.0, move |s, v| {
            s.behave("appearance", Value::Text(v.word().into()), v == Behaviour::default().appearance)
        })
    });
    section(k, "themes", Some(heading(k, "Theme")), vec![top, mode])
}

const DOCS: &str = "https://hokum.app";
const PROMPT: &str = "Read https://hokum.app and create a Hokum theme for me, then switch my config to it. I want it to look like: ";

/// Custom themes are written by a coding agent from the docs, not in this window.
fn own_theme(s: &Ui, k: &Kit) -> El {
    let c = &k.c;
    let body = col()
        .key("own")
        .w_full()
        .padding(12.0)
        .gap(10.0)
        .child(
            text("A theme can change every part of a card, more than this window can show. Ask your coding agent to write one: paste this prompt and describe the look you want.")
                .key("about")
                .font(k.ty.small)
                .color(c.text_muted)
                .wrap()
                .w_full(),
        )
        .child(
            col()
                .key("prompt")
                .w_full()
                .padding(10.0)
                .radius(8.0)
                .fill(c.bg_deep)
                .border(1.0, c.border_muted)
                .child(text(PROMPT).key("text").font(k.ty.small.mono()).color(c.text_base).wrap().w_full().selectable(true)),
        )
        .child(
            row()
                .key("own-actions")
                .gap(4.0)
                .child(
                    Button::new(if s.copied { "Copied" } else { "Copy prompt" })
                        .icon(if s.copied { &rune_theme_opencode::icons::CHECK } else { &rune_theme_opencode::icons::COPY })
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Regular)
                        .on_press_tx(|s: &mut Ui, tx: &mut Tx| {
                            tx.copy(PROMPT);
                            s.copied = true;
                        })
                        .build(k)
                        .key("copy-prompt"),
                )
                .child(
                    Button::new("Open the guide")
                        .icon(&rune_theme_opencode::icons::GLOBE)
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Regular)
                        .on_press_tx(|_: &mut Ui, tx: &mut Tx| {
                            let _ = tx.open_url(DOCS);
                        })
                        .build(k)
                        .key("open-docs"),
                ),
        );
    section(k, "own-theme", Some(heading(k, "Make your own theme")), vec![body])
}

/// config.kdl, the themes folder and the script.
fn files(s: &Ui, k: &Kit) -> El {
    let c = &k.c;
    let config = setting(k, "config", "config.kdl", Some(&home(&crate::config::path())), button(k, "edit-config", "Edit", |s| s.edit_config()));
    let folder = setting(
        k,
        "folder",
        "Themes folder",
        Some(&home(&crate::theme::dir())),
        button(k, "open-folder", "Open", |_| {
            let dir = crate::theme::dir();
            let _ = std::fs::create_dir_all(&dir);
            open(&dir);
        }),
    );
    let (dot, line) = match &s.script {
        ScriptStatus::None => (c.icon_faint, "Can rewrite, route or drop notifications".to_owned()),
        ScriptStatus::Loaded => (c.success.icon, format!("{} is running", home(&crate::script::path()))),
        ScriptStatus::Broken(e) => (c.danger.icon, format!("Error: {e}")),
    };
    let script = row()
        .key("script")
        .w_full()
        .min_h(56.0)
        .px(12.0)
        .py(8.0)
        .gap(16.0)
        .center_y()
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .shrink()
                .w(0.0)
                .gap(2.0)
                .child(text("Script").key("title").font(k.ty.ui).color(c.text_base))
                .child(
                    row()
                        .key("state")
                        .w_full()
                        .gap(6.0)
                        .center_y()
                        .child(el().key("dot").size(6.0, 6.0).radius(3.0).fill(dot))
                        .child(text(line).key("line").font(k.ty.small).color(c.text_muted).wrap().grow(1.0).shrink().w(0.0)),
                ),
        )
        .child(button(k, "edit-script", if s.script == ScriptStatus::None { "Create" } else { "Edit" }, |s| s.edit_script()));
    section(k, "files", Some(heading(k, "Files")), vec![config, folder, script])
}

fn behaviour(s: &Ui, k: &Kit) -> El {
    let b = &s.loaded.behaviour;
    let d = Behaviour::default();
    let stay = Some(b.stay.round() as u32);
    section(
        k,
        "behaviour",
        None,
        vec![
            setting(k, "position", "Position", Some("The corner of the focused screen"), corner_picker(k, b.position, d.position)),
            setting(k, "stay", "Stay for", Some("When the app doesn't say. Critical ones stay until dismissed"), {
                choice(k, "stay-control", STAY, stay, 180.0, move |s, v| s.behave("stay", Value::Number(v as f64), v as f32 == d.stay))
            }),
            setting(k, "shown", "Show at most", Some("The rest wait behind a \u{201c}more\u{201d} pill"), {
                choice(k, "shown-control", SHOWN, Some(b.max_shown), 180.0, move |s, v| s.behave("show-at-most", Value::Number(v as f64), v == d.max_shown))
            }),
            toggle(k, "pause", "Pause while hovered", Some("Timers wait while the pointer is over a card"), b.pause_on_hover, move |s: &mut Ui| {
                let v = !s.loaded.behaviour.pause_on_hover;
                s.behave("pause-on-hover", Value::Flag(v), v == Behaviour::default().pause_on_hover)
            }),
            toggle(k, "dnd", "Do not disturb", Some("Only critical notifications and apps allowed to interrupt show"), b.do_not_disturb, move |s: &mut Ui| {
                let v = !s.loaded.behaviour.do_not_disturb;
                s.behave("do-not-disturb", Value::Flag(v), v == Behaviour::default().do_not_disturb)
            }),
        ],
    )
}

/// The app's icon, or its letter on its colour.
fn app_mark(s: &Ui, k: &Kit, app: &str, size: f32) -> El {
    match s.icons.get(app).cloned().flatten() {
        Some(img) => el().key("icon").size(size, size).radius(size * 0.27).pixels_fit(img.pixels(), rune::SurfaceFit::Fit),
        None => el()
            .key("icon")
            .size(size, size)
            .radius(size * 0.27)
            .center()
            .fill(tint(app))
            .child(
                text(app.chars().find(|c| c.is_alphanumeric()).unwrap_or('•').to_uppercase().to_string())
                    .font(rune::TextStyle { size: (size * 0.55).round(), ..k.ty.ui_medium }.lh(size))
                    .color(Color::WHITE),
            ),
    }
}

fn app_rule(s: &Ui, app: &str) -> AppRule {
    let (rules, _) = crate::config::app_rules(&s.loaded.own);
    rules.into_iter().find(|(a, _)| a.eq_ignore_ascii_case(app)).map(|(_, r)| r).unwrap_or_default()
}

/// What an app's rule says, in a few words.
fn app_summary(rule: &AppRule) -> String {
    if rule.hide {
        return "Muted".into();
    }
    let mut parts = Vec::new();
    match rule.stay {
        Some(s) if s <= 0.0 => parts.push("Pinned".to_owned()),
        Some(s) => parts.push(format!("{} s", s.round())),
        None => {}
    }
    if rule.interrupt {
        parts.push("Interrupts".into());
    }
    if parts.is_empty() { "Default".into() } else { parts.join(" \u{b7} ") }
}

/// A note for rules the window can't show, with a way to them.
fn rules_note(k: &Kit, line: String) -> El {
    row()
        .key("custom")
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(10.0)
        .center_y()
        .child(text(line).key("line").font(k.ty.small).color(k.c.text_muted).wrap().grow(1.0).shrink().w(0.0))
        .child(button(k, "edit-rules", "Edit config.kdl", |s| s.edit_config()))
}

fn apps(s: &Ui, k: &Kit) -> El {
    let c = &k.c;
    let (_, custom) = crate::config::app_rules(&s.loaded.own);
    let mut rows: Vec<El> = s
        .apps
        .iter()
        .enumerate()
        .map(|(i, app)| {
            let rule = app_rule(s, app);
            let page = Page::App(app.clone());
            row()
                .key(format!("app-{i}"))
                .role("button")
                .label(app.clone())
                .w_full()
                .h(44.0)
                .px(12.0)
                .gap(10.0)
                .center_y()
                .radius(8.0)
                .hover(c.hover)
                .press(c.pressed)
                .cursor(Cursor::Pointer)
                .on_press_tx(move |s: &mut Ui, tx: &mut Tx| go(s, tx, page.clone()))
                .child(app_mark(s, k, app, 22.0))
                .child(text(app.clone()).key("name").font(k.ty.ui).color(if rule.hide { c.text_faint } else { c.text_base }).grow(1.0).shrink().truncate())
                .child(text(app_summary(&rule)).key("summary").font(k.ty.small).color(c.text_muted))
                .child(rune::icon(&CHEVRON, 16.0).key("chevron").color(c.icon_faint))
        })
        .collect();
    if rows.is_empty() {
        rows.push(text("Apps show up here once they've sent a notification.").key("empty").font(k.ty.small).color(c.text_muted).px(12.0).py(12.0));
    }
    if custom > 0 {
        rows.push(rules_note(k, if custom == 1 { "1 more rule in config.kdl matches by text or urgency".into() } else { format!("{custom} more rules in config.kdl match by text or urgency") }));
    }
    section(k, "apps", None, rows)
}

/// One app's choices, kept as `rule app="…"` in config.kdl.
fn app_page(s: &Ui, k: &Kit, app: &str) -> Vec<El> {
    let c = &k.c;
    let rule = app_rule(s, app);
    let hide = rule.hide;
    let name = app.to_owned();
    let mut rows = vec![toggle(k, "show", "Show notifications", None, !hide, move |s: &mut Ui| s.set_app(name.clone(), "hide", (!hide).then(Vec::new)))];
    if !hide {
        let stay = match rule.stay {
            None => 0,
            Some(s) if s <= 0.0 => u32::MAX,
            Some(s) => s.round() as u32,
        };
        let name = app.to_owned();
        rows.push(setting(k, "stay", "Stay for", Some("Pinned stays until dismissed"), {
            let current = Some(if APP_STAY.iter().any(|o| o.0 == stay) { stay } else { 0 });
            choice(k, "stay-control", APP_STAY, current, 280.0, move |s, v| {
                let value = match v {
                    0 => None,
                    u32::MAX => Some(vec![Value::Text("never".into())]),
                    n => Some(vec![Value::Number(n as f64)]),
                };
                s.set_app(name.clone(), "timeout", value)
            })
        }));
        let name = app.to_owned();
        let interrupt = rule.interrupt;
        rows.push(toggle(k, "interrupt", "Show during Do not disturb", None, interrupt, move |s: &mut Ui| {
            s.set_app(name.clone(), "interrupt", (!interrupt).then(Vec::new))
        }));
    }
    // Rules that match more than one exact app name may apply too.
    let others = s.loaded.own.rules.iter().filter(|r| crate::config::simple_app(r).is_none() && r.app.as_ref().is_none_or(|g| crate::theme::glob(g, app))).count();
    if others > 0 {
        rows.push(rules_note(k, format!("{others} more rule{} in config.kdl may apply to {app}", if others == 1 { "" } else { "s" })));
    }
    let head = row()
        .key("app-head")
        .w_full()
        .gap(12.0)
        .center_y()
        .child(app_mark(s, k, app, 32.0))
        .child(text(app.to_owned()).key("name").font(rune::TextStyle { size: 20.0, ..k.ty.ui_medium }.lh(24.0)).color(c.text_base).grow(1.0).shrink().truncate());
    vec![head, section(k, "app", None, rows)]
}

/// Notifications' pages: three tabs, and one app's page under Applications.
#[derive(Clone, PartialEq, Debug)]
enum Page {
    Appearance,
    Behaviour,
    Apps,
    App(String),
}

static BELL: rune::Icon = rune::Icon {
    name: "bell",
    view: 16.0,
    body: r#"<path d="M4.5 10.5V7a3.5 3.5 0 0 1 7 0v3.5l1 1.5h-9l1-1.5Z" stroke="currentColor" stroke-linejoin="round"/><path d="M6.5 14h3" stroke="currentColor"/>"#,
};
static CHEVRON: rune::Icon = rune::Icon { name: "chevron", view: 16.0, body: r#"<path d="M6.5 4L10.5 8L6.5 12" stroke="currentColor"/>"# };
static BACK: rune::Icon = rune::Icon { name: "back", view: 16.0, body: r#"<path d="M9.5 4L5.5 8L9.5 12" stroke="currentColor"/>"# };

/// Goes to `page`, swapping the content at once: screens don't fade into each other.
fn go(s: &mut Ui, tx: &mut Tx, page: Page) {
    tx.instant();
    s.page = page;
}

fn go_section(s: &mut Ui, tx: &mut Tx, section: Section) {
    tx.instant();
    if section != Section::Bluetooth {
        s.bluetooth.set_adding(false);
    }
    s.section = section;
}

fn nav(s: &Ui, k: &Kit) -> El {
    let c = &k.c;
    let item = |key: &str, label: &str, icon: &'static rune::Icon, section: Section| {
        SidebarRow::new(label)
            .icon(icon)
            .selected(s.section == section)
            .on_select(move |s: &mut Ui, tx: &mut Tx| go_section(s, tx, section))
            .build(k)
            .key(key.to_owned())
    };
    let title = text("Hokum").key("name").font(rune::TextStyle { size: 20.0, ..k.ty.ui_medium }.lh(24.0)).color(c.text_base).px(8.0).pt(20.0).pb(12.0);
    let list = Sidebar::new()
        .label("Hokum")
        .header(title)
        .child(item("nav-notifications", "Notifications", &BELL, Section::Notifications))
        .child(item("nav-sound", "Sound", &crate::sound::ui::glyphs::SPEAKER, Section::Sound))
        .child(item("nav-bluetooth", "Bluetooth", &crate::bluetooth::ui::glyphs::BLUETOOTH, Section::Bluetooth))
        .build(k);
    row()
        .key("nav")
        .w(NAV)
        .h_full()
        .fill(c.bg_base)
        .child(col().key("list").grow(1.0).w(0.0).h_full().px(4.0).child(list))
        .child(el().key("edge").w(1.0).h_full().fill(c.border_muted))
}

fn page_title(k: &Kit, t: &str) -> El {
    text(t.to_owned()).key("title").font(rune::TextStyle { size: 20.0, ..k.ty.ui_medium }.lh(24.0)).color(k.c.text_base)
}

/// config.kdl's problems and failed saves, under any section's title.
fn problems(s: &Ui, k: &Kit) -> Vec<El> {
    let c = &k.c;
    let mut out = Vec::new();
    if let Some(e) = &s.error {
        out.push(text(e.clone()).key("error").font(k.ty.small).color(c.danger.fg).wrap().w_full());
    }
    for (i, p) in s.loaded.problems.iter().take(6).enumerate() {
        out.push(text(p.to_string()).key(format!("problem-{i}")).font(k.ty.small.mono()).color(c.danger.fg).wrap().w_full().selectable(true));
    }
    out
}

const TABS: &[(&str, &str)] = &[("appearance", "Appearance"), ("behaviour", "Behaviour"), ("apps", "Applications")];

fn notifications(s: &Ui, k: &Kit) -> El {
    let c = &k.c;
    let tab = match s.page {
        Page::Appearance => 0,
        Page::Behaviour => 1,
        Page::Apps | Page::App(_) => 2,
    };
    let labels: Vec<&str> = TABS.iter().map(|t| t.1).collect();
    let tabs = SegmentedControl::new(&labels, Some(tab))
        .width(Some(300.0))
        .on_pick(|s: &mut Ui, i: usize, tx: &mut Tx| go(s, tx, [Page::Appearance, Page::Behaviour, Page::Apps][i].clone()))
        .build(k)
        .key("tabs");
    let title = row().key("title-row").w_full().gap(12.0).center_y().child(page_title(k, "Notifications")).child(el().key("fill").grow(1.0)).child(tabs);
    let header = col().key("header").w_full().px(24.0).pt(20.0).pb(14.0).gap(12.0).child(title).child(status(k, s)).children(problems(s, k));
    let (key, body): (&str, Vec<El>) = match &s.page {
        Page::Appearance => ("appearance", vec![themes(s, k), own_theme(s, k), files(s, k)]),
        Page::Behaviour => ("behaviour", vec![behaviour(s, k)]),
        Page::Apps => ("apps", vec![apps(s, k)]),
        Page::App(app) => {
            let back = Button::new("Applications")
                .icon(&BACK)
                .variant(ButtonVariant::GhostMuted)
                .control_size(ControlSize::Regular)
                .on_press_tx(|s: &mut Ui, tx: &mut Tx| go(s, tx, Page::Apps))
                .build(k)
                .key("back")
                .self_start()
                .ml(-8.0);
            ("app", std::iter::once(back).chain(app_page(s, k, app)).collect())
        }
    };
    let page = col().key(format!("page-{key}")).w_full().px(24.0).pt(20.0).pb(32.0).gap(20.0).children(body);
    col()
        .key("content")
        .w(CONTENT)
        .h_full()
        .fill(c.page)
        .child(header)
        .child(el().key("rule").w_full().h(1.0).fill(c.border_muted))
        .child(col().key(format!("scroll-{key}")).w_full().grow(1.0).h(0.0).scroll(ScrollSpec { stick_bottom: false }).clip_fade(0.0, 0.0, 0.0, 16.0).child(page))
}

/// A faint mark and at most one quiet line: a module that's off, or can't run.
fn quiet(k: &Kit, glyph: &'static rune::Icon, line: Option<String>) -> El {
    col()
        .key("quiet")
        .w_full()
        .py(48.0)
        .gap(12.0)
        .items_center()
        .child(rune::icon(glyph, 40.0).key("mark").color(k.c.border_strong))
        .children(line.map(|l| text(l).key("line").font(k.ty.small).color(k.c.text_faint).wrap()))
}

/// Sound or Bluetooth: the title and the module's switch, then the module's own screens,
/// centred in the space beside the sidebar.
fn module(s: &Ui, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let (section, title, on, error, glyph) = match s.section {
        Section::Bluetooth => (Section::Bluetooth, "Bluetooth", s.loaded.modules.bluetooth, &s.bluetooth_error, &crate::bluetooth::ui::glyphs::BLUETOOTH),
        _ => (Section::Sound, "Sound", s.loaded.modules.sound, &s.sound_error, &crate::sound::ui::glyphs::SPEAKER),
    };
    let header = col()
        .key("header")
        .w_full()
        .px(24.0)
        .pt(20.0)
        .pb(14.0)
        .gap(8.0)
        .child(row().key("title-row").w_full().h(28.0).center_y().child(page_title(k, title)))
        .children(problems(s, k));
    // Turning a module off is rare: it sits at the end of its advanced page.
    let toggle = |label: String, variant: ButtonVariant| {
        Button::new(label)
            .variant(variant)
            .control_size(ControlSize::Regular)
            .on_press(move |s: &mut Ui| s.set_module(section, !on))
            .build(k)
            .key("module-toggle")
    };
    let advanced = match section {
        Section::Bluetooth => s.bluetooth.page != crate::bluetooth::ui::Page::Main,
        _ => s.sound.page != crate::sound::ui::Page::Main,
    };
    let body = match (on, error) {
        (false, _) => quiet(k, glyph, None).child(toggle(format!("Turn on {title}"), ButtonVariant::Neutral)),
        (true, Some(e)) => quiet(k, glyph, Some(e.clone())),
        (true, None) => {
            let body = match section {
                Section::Bluetooth => crate::bluetooth::ui::section(&s.bluetooth, k, layer),
                _ => crate::sound::ui::section(&s.sound, k, layer),
            };
            let off = advanced.then(|| row().key("module-off").w_full().px(12.0).child(toggle(format!("Turn off {title}"), ButtonVariant::GhostMuted)));
            col().key("module-body").w_full().child(body).children(off)
        }
    };
    let column = col().key(format!("module-{}", section.word())).w(MODULE).pb(32.0).child(body);
    col()
        .key("content")
        .grow(1.0)
        .h_full()
        .fill(c.page)
        .child(row().key("header-row").w_full().justify_center().child(header.w(MODULE + 12.0)))
        .child(row().key("rule-row").w_full().justify_center().child(el().key("rule").w(MODULE + 12.0).h(1.0).fill(c.border_muted)))
        .child(
            col()
                .key(format!("scroll-{}", section.word()))
                .w_full()
                .grow(1.0)
                .h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .clip_fade(0.0, 0.0, 0.0, 16.0)
                .child(row().key("centre").w_full().pt(8.0).justify_center().child(column)),
        )
}

fn preview(s: &Ui, k: &Kit, window: &Kit) -> El {
    let backdrop = if k.dark {
        (rune::oklch(0.32, 0.06, 265.0), rune::oklch(0.22, 0.05, 300.0))
    } else {
        (rune::oklch(0.86, 0.05, 230.0), rune::oklch(0.78, 0.07, 290.0))
    };
    col()
        .key("preview")
        .grow(1.0)
        .h_full()
        .padding(16.0)
        .gap(10.0)
        .fill(window.c.bg_deep)
        .child(text("Preview").key("label").font(window.ty.small_medium).color(window.c.text_faint).ml(4.0))
        .child(
            el().key("screen")
                .w_full()
                .grow(1.0)
                .radius(14.0)
                .gradient(backdrop.0, backdrop.1)
                .clip_soft(0.0)
                .child(stack::<Ui>(&s.samples, &s.loaded.behaviour, &s.loaded.theme, k, None)),
        )
}

pub fn view(s: &Ui, cx: &mut Cx<Ui>) -> El {
    let (window, preview_kit) = kit(s, cx);
    if s.surface == Surface::Panel {
        return crate::panel::view(s, window);
    }
    let mut layer = Vec::new();
    if s.section != Section::Sound {
        s.sound.unmeter();
    }
    let main = match s.section {
        Section::Notifications => row().key("notifications").grow(1.0).h_full().child(notifications(s, window)).child(preview(s, preview_kit, window)),
        _ => module(s, window, &mut layer),
    };
    let body = row().key("window").w_full().h_full().fill(window.c.page).child(nav(s, window)).child(main);
    col().key("root").w_full().h_full().child(body).children(layer)
}

fn tile_note() -> Note {
    let mut n = samples().remove(0);
    n.actions.clear();
    n.body = "ship it before anyone sees the mako ones 👋".into();
    n
}

pub fn init() -> Ui {
    let mut ui = Ui {
        loaded: crate::config::load(),
        daemon: Daemon::Checking,
        overlay: None,
        bus: None,
        handle: None,
        sent: 0,
        error: None,
        script: ScriptStatus::None,
        copied: false,
        page: Page::Appearance,
        gallery: Vec::new(),
        apps: Vec::new(),
        icons: HashMap::new(),
        samples: samples(),
        tile_note: tile_note(),
        cache: None,
        section: Section::Notifications,
        surface: Surface::Window,
        sound: crate::sound::ui::Ui::new(None, crate::sound::ui::Mode::Window),
        bluetooth: crate::bluetooth::ui::Ui::new(None, crate::bluetooth::ui::Mode::Window),
        wiring: None,
        sound_error: None,
        bluetooth_error: None,
        dismiss: None,
    };
    ui.configure(crate::watch::load());
    ui
}

/// A path with the home folder as `~`.
fn home(p: &std::path::Path) -> String {
    match std::env::var_os("HOME").and_then(|h| p.strip_prefix(h).ok().map(|r| r.display().to_string())) {
        Some(rest) => format!("~/{rest}"),
        None => p.display().to_string(),
    }
}

/// Opens a file or folder in the desktop's handler for it.
fn open(path: &std::path::Path) {
    let _ = std::process::Command::new("xdg-open").arg(path).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

fn decode(s: &mut Ui, cache: rune::ImageCache, wait: bool) {
    for n in s.samples.iter_mut().chain(std::iter::once(&mut s.tile_note)) {
        n.decode(&cache);
        if wait {
            for img in n.icon.iter().chain(&n.image) {
                let _ = img.wait();
            }
        }
    }
    s.cache = Some(cache);
    s.decode_icons();
    if wait {
        crate::sound::ui::decode_now(&mut s.sound);
        for img in s.icons.values().flatten() {
            let _ = img.wait();
        }
    }
}

/// Decodes everything now (headless renders have no event loop to finish decodes).
fn decode_now(s: &mut Ui) {
    decode(s, rune::ImageCache::new(std::sync::Arc::new(|| {})), true);
}

fn scene_hokum(s: &mut Ui) {
    s.daemon = Daemon::Hokum;
    decode_now(s);
}

fn scene_other(s: &mut Ui) {
    s.daemon = Daemon::Other("mako".into());
    decode_now(s);
}

fn scene_behaviour(s: &mut Ui) {
    scene_hokum(s);
    s.page = Page::Behaviour;
}

fn scene_applications(s: &mut Ui) {
    scene_hokum(s);
    s.page = Page::Apps;
}

/// The first app's page (Slack when none has sent anything yet).
fn scene_app(s: &mut Ui) {
    scene_hokum(s);
    s.page = Page::App(s.apps.first().cloned().unwrap_or_else(|| "Slack".into()));
}

/// Sound and Bluetooth on their mock backends' samples, whatever config.kdl says.
fn mock_modules(s: &mut Ui) {
    scene_hokum(s);
    s.loaded.modules = crate::theme::Modules::default();
    crate::sound::ui::scenes::main(&mut s.sound);
    crate::bluetooth::ui::scenes::main(&mut s.bluetooth);
    s.bluetooth.sound = true;
}

fn scene_sound(s: &mut Ui) {
    mock_modules(s);
    s.section = Section::Sound;
}

fn scene_sound_expanded(s: &mut Ui) {
    scene_sound(s);
    crate::sound::ui::scenes::expanded(&mut s.sound);
}

fn scene_sound_advanced(s: &mut Ui) {
    scene_sound(s);
    crate::sound::ui::scenes::advanced(&mut s.sound);
}

fn scene_sound_lost(s: &mut Ui) {
    scene_sound(s);
    crate::sound::ui::scenes::lost(&mut s.sound);
}

fn scene_sound_off(s: &mut Ui) {
    scene_sound(s);
    s.loaded.modules.sound = false;
}

fn scene_bluetooth(s: &mut Ui) {
    mock_modules(s);
    s.section = Section::Bluetooth;
}

fn scene_bluetooth_expanded(s: &mut Ui) {
    scene_bluetooth(s);
    crate::bluetooth::ui::scenes::expanded(&mut s.bluetooth);
}

fn scene_bluetooth_pairing(s: &mut Ui) {
    scene_bluetooth(s);
    crate::bluetooth::ui::scenes::pairing(&mut s.bluetooth);
}

fn scene_bluetooth_advanced(s: &mut Ui) {
    scene_bluetooth(s);
    crate::bluetooth::ui::scenes::advanced(&mut s.bluetooth);
}

fn scene_bluetooth_off(s: &mut Ui) {
    scene_bluetooth(s);
    crate::bluetooth::ui::scenes::off(&mut s.bluetooth);
}

/// The tray panel, with the input showing (an app is recording) and Bluetooth devices.
fn scene_panel(s: &mut Ui) {
    mock_modules(s);
    s.surface = Surface::Panel;
    s.sound.mode = crate::sound::ui::Mode::Popover;
    s.bluetooth.mode = crate::bluetooth::ui::Mode::Popover;
}

pub fn setup(s: &mut Ui, waker: rune::Waker) {
    crate::sound::ui::setup(&mut s.sound, waker.clone());
    decode(s, rune::ImageCache::new(waker), false);
}

/// Keys go to the section on screen (in the panel: Esc closes what's open, then the panel).
pub fn on_key(s: &mut Ui, e: &KeyEvent, tx: &mut Tx) -> bool {
    if s.surface == Surface::Panel {
        if e.key != rune::Key::Escape {
            return false;
        }
        if s.sound.escape(tx) || s.bluetooth.escape(tx) {
            return true;
        }
        if let Some(d) = &s.dismiss {
            d();
        }
        return true;
    }
    match s.section {
        Section::Sound if s.loaded.modules.sound => crate::sound::ui::on_key(&mut s.sound, e, tx),
        Section::Bluetooth if s.loaded.modules.bluetooth => crate::bluetooth::ui::on_key(&mut s.bluetooth, e, tx),
        Section::Notifications if e.key == rune::Key::Escape && matches!(s.page, Page::App(_)) => {
            go(s, tx, Page::Apps);
            true
        }
        _ => false,
    }
}

pub fn spec() -> AppSpec<Ui> {
    AppSpec {
        scenes: &[
            ("hokum", scene_hokum),
            ("notifications", scene_hokum),
            ("other", scene_other),
            ("behaviour", scene_behaviour),
            ("applications", scene_applications),
            ("app", scene_app),
            ("sound", scene_sound),
            ("sound-expanded", scene_sound_expanded),
            ("sound-advanced", scene_sound_advanced),
            ("sound-lost", scene_sound_lost),
            ("sound-off", scene_sound_off),
            ("bluetooth", scene_bluetooth),
            ("bluetooth-expanded", scene_bluetooth_expanded),
            ("bluetooth-pairing", scene_bluetooth_pairing),
            ("bluetooth-advanced", scene_bluetooth_advanced),
            ("bluetooth-off", scene_bluetooth_off),
            ("panel", scene_panel),
        ],
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        setup: Some(setup),
        on_key: Some(on_key),
        ..AppSpec::new("Hokum", SIZE, init, view)
    }
}

/// Fixed-size, like Hex's window: tiling compositors open it floating.
pub fn window() -> WindowOptions {
    WindowOptions { min_size: Some(SIZE), max_size: Some(SIZE), resizable: false, ..WindowOptions::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(rt: &mut rune::Runtime<Ui>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
    }

    /// Present at full opacity, with nothing left fading out.
    fn shown_at_once(rt: &rune::Runtime<Ui>, page: &str) {
        let (_, opacity) = rt.presented(page).unwrap_or_else(|| panic!("no {page}"));
        assert_eq!(opacity, 1.0, "{page} fades in");
        let exiting: Vec<String> = rt.elements().into_iter().filter(|e| e.exiting).map(|e| e.path).collect();
        assert!(exiting.is_empty(), "still fading out: {exiting:?}");
    }

    #[test]
    fn screens_switch_without_fading() {
        let mut ui = init();
        ui.apps = vec!["Slack".into()];
        let mut rt = rune::Runtime::new(ui, view, SIZE, rune::Env::default());
        rt.frame(0.0);
        press(&mut rt, "tabs/segment1");
        shown_at_once(&rt, "page-behaviour");
        press(&mut rt, "tabs/segment2");
        shown_at_once(&rt, "page-apps");
        press(&mut rt, "app-0");
        shown_at_once(&rt, "page-app");
        press(&mut rt, "back");
        shown_at_once(&rt, "page-apps");
        press(&mut rt, "nav-sound");
        shown_at_once(&rt, "module-sound");
        press(&mut rt, "nav-bluetooth");
        shown_at_once(&rt, "module-bluetooth");
        press(&mut rt, "nav-notifications");
        shown_at_once(&rt, "page-apps");
    }
}
