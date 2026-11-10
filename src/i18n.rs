//! The window's own copy, in the language config.kdl asks for (`language "fr"`), or the
//! locale's when it doesn't say (`$LANGUAGE`, `$LC_ALL`, `$LC_MESSAGES`, `$LANG`).
//!
//! English is the source: every string the window draws lives here as its English text, and
//! [`t`] hands back the language's. Small sentences that put a name, a count or a path inside
//! them aren't in the table, because word order and plurals differ; they're the `fn`s below,
//! one per sentence, matching on [`lang`].
//!
//! What Hokum draws from somewhere else isn't here: apps' notifications, device and profile
//! names from PipeWire and BlueZ, property names, and the diagnostics for config.kdl and theme
//! files, which read like a compiler's.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    #[default]
    En,
    Fr,
}

impl Lang {
    /// `language "fr"` from config.kdl. The locale's `fr_FR.UTF-8` reads as French too: hand
    /// written configs pick up the habit, `hokum check` still knows it.
    pub fn of(word: &str) -> Option<Lang> {
        let primary = word
            .trim()
            .split(['_', '-', '.', '@'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match primary.as_str() {
            "en" => Some(Lang::En),
            "fr" => Some(Lang::Fr),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
        }
    }

    /// The words `language` takes, for an error that lists them.
    pub const WORDS: &'static [&'static str] = &["en", "fr"];

    /// The session's language: `$LANGUAGE` (a colon-separated list, read in order), then
    /// `$LC_ALL`, `$LC_MESSAGES`, `$LANG`; English when none of them names one.
    pub fn from_locale() -> Lang {
        let named = |var: &str| -> Option<Lang> {
            let value = std::env::var(var).ok()?;
            value
                .split(':')
                .find_map(|tag| Lang::of(tag.split(['.', '@']).next().unwrap_or("")))
        };
        ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(named)
            .unwrap_or(Lang::En)
    }
}

/// The language everything is drawn in. One process, one language: it's read by the window,
/// the panel, the cards and the tray, from places that have no state to carry it in.
static LANG: AtomicU8 = AtomicU8::new(Lang::En as u8);

/// Sets the language (at startup, and whenever config.kdl changes it).
pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Fr,
        _ => Lang::En,
    }
}

/// The text for `en`, in the language in use; English itself when it has no translation.
pub fn t(en: &'static str) -> &'static str {
    text(lang(), en)
}

/// The same, for a language named rather than the one in use.
pub fn text(lang: Lang, en: &'static str) -> &'static str {
    match lang {
        Lang::En => en,
        Lang::Fr => FR
            .iter()
            .find(|(e, _)| *e == en)
            .map(|(_, fr)| *fr)
            .unwrap_or(en),
    }
}

/// English is the key; French is what [`t`] answers.
const FR: &[(&str, &str)] = &[
    // The window's sidebar and Notifications' tabs.
    ("Notifications", "Notifications"),
    ("Sound", "Son"),
    ("Bluetooth", "Bluetooth"),
    ("Appearance", "Apparence"),
    ("Behaviour", "Comportement"),
    ("Applications", "Applications"),
    ("Preview", "Aperçu"),
    // Notifications: themes.
    ("Theme", "Thème"),
    ("Make your own theme", "Créez votre thème"),
    ("Copy prompt", "Copier le prompt"),
    ("Copied", "Copié"),
    ("Open the guide", "Ouvrir le guide"),
    (
        "A theme can change every part of a card, more than this window can show. Ask your coding agent to write one: paste this prompt and describe the look you want.",
        "Un thème peut changer chaque partie d'une carte, bien plus que cette fenêtre n'en montre. Demandez-le à votre agent de code : collez ce prompt et décrivez le rendu voulu.",
    ),
    (
        "Read https://hokum.app and create a Hokum theme for me, then switch my config to it. I want it to look like: ",
        "Lis https://hokum.app et crée-moi un thème Hokum, puis passe ma configuration dessus. Je veux que ça ressemble à : ",
    ),
    // Files.
    ("Files", "Fichiers"),
    ("Script", "Script"),
    ("Themes folder", "Dossier des thèmes"),
    ("Language", "Langue"),
    (
        "Auto follows your session's language",
        "Auto suit la langue de votre session",
    ),
    ("Custom", "Personnalisé"),
    ("Edit", "Modifier"),
    ("Open", "Ouvrir"),
    ("Create", "Créer"),
    (
        "Can rewrite, route or drop notifications",
        "Peut réécrire, router ou supprimer des notifications",
    ),
    // Behaviour.
    ("Position", "Position"),
    ("On the focused screen", "Sur l'écran au premier plan"),
    ("Stay for", "Durée d'affichage"),
    (
        "When the app doesn't say. Critical ones stay until dismissed",
        "Quand l'app ne précise rien. Les critiques restent jusqu'à fermeture",
    ),
    ("Show at most", "Afficher au plus"),
    (
        "The rest wait behind a “more” pill",
        "Les autres attendent derrière une pastille « plus »",
    ),
    ("Pause while hovered", "Pause au survol"),
    (
        "Timers wait while the pointer is over a card",
        "Les minuteurs s'arrêtent quand le pointeur est sur une carte",
    ),
    ("Do not disturb", "Ne pas déranger"),
    (
        "Only critical notifications and apps allowed to interrupt show",
        "Seules les critiques et les apps autorisées s'affichent",
    ),
    ("Top left", "En haut à gauche"),
    ("Top center", "En haut au centre"),
    ("Top right", "En haut à droite"),
    ("Bottom left", "En bas à gauche"),
    ("Bottom center", "En bas au centre"),
    ("Bottom right", "En bas à droite"),
    // Applications, and one app's page.
    ("Default", "Défaut"),
    ("Pinned", "Épinglé"),
    ("Interrupts", "Interrompt"),
    (
        "Apps show up here once they've sent a notification.",
        "Les apps apparaissent ici dès qu'elles envoient une notification.",
    ),
    ("Show notifications", "Afficher les notifications"),
    (
        "Pinned stays until dismissed",
        "Épinglé reste jusqu'à fermeture",
    ),
    (
        "Show during Do not disturb",
        "Afficher pendant « Ne pas déranger »",
    ),
    ("Edit config.kdl", "Modifier config.kdl"),
    // The notification daemon, and the script.
    (
        "Checking the notification daemon…",
        "Vérification du démon de notifications…",
    ),
    (
        "Hokum is showing your notifications",
        "Hokum affiche vos notifications",
    ),
    ("Use Hokum", "Utiliser Hokum"),
    ("Send test", "Envoyer un test"),
    // The tray panel and its menu.
    ("Open Hokum", "Ouvrir Hokum"),
    ("Turn off Do not disturb", "Désactiver « Ne pas déranger »"),
    ("Turn on Do not disturb", "Activer « Ne pas déranger »"),
    ("Quit", "Quitter"),
    // The welcome card.
    ("Welcome to Hokum", "Bienvenue dans Hokum"),
    (
        "This is how your notifications look now. Make them yours in Settings.",
        "Voici à quoi ressemblent vos notifications. Personnalisez-les dans les réglages.",
    ),
    ("Open Settings", "Ouvrir les réglages"),
    // Cards' chrome, and the cards Hokum draws itself.
    ("Clear all", "Tout effacer"),
    ("Dismiss", "Fermer"),
    ("Volume", "Volume"),
    ("Muted", "Muet"),
    ("Brightness", "Luminosité"),
    ("Try again", "Réessayer"),
    ("Copy command", "Copier la commande"),
    ("Open hokum.app", "Ouvrir hokum.app"),
    ("Hokum couldn't update", "Hokum n'a pas pu se mettre à jour"),
    ("Hokum updated", "Hokum mis à jour"),
    (
        "Downloading and checking it.",
        "Téléchargement et vérification.",
    ),
    (
        "The download was the wrong size.",
        "Le téléchargement n'a pas la bonne taille.",
    ),
    (
        "The download didn't match its checksum.",
        "Le téléchargement ne correspond pas à son empreinte.",
    ),
    // Sound.
    ("Balance", "Balance"),
    ("Devices", "Périphériques"),
    ("Routing", "Routage"),
    ("Clock", "Horloge"),
    ("New virtual output", "Nouvelle sortie virtuelle"),
    ("Name", "Nom"),
    ("Remove", "Retirer"),
    ("Virtual", "Virtuel"),
    ("Sample rate", "Fréquence d'échantillonnage"),
    ("Buffer size", "Taille du tampon"),
    ("Automatic", "Automatique"),
    (
        "Sound stopped working; restart Hokum",
        "Le son a cessé de fonctionner ; redémarrez Hokum",
    ),
    ("Not connected to PipeWire", "Pas connecté à PipeWire"),
    ("Couldn't switch the port", "Impossible de changer de port"),
    (
        "Couldn't switch the profile",
        "Impossible de changer de profil",
    ),
    ("PipeWire isn't running", "PipeWire ne tourne pas"),
    ("bad pod", "données PipeWire illisibles"),
    (
        "No session manager is running (WirePlumber)",
        "Aucun gestionnaire de session (WirePlumber) ne tourne",
    ),
    (
        "Only devices made here can be removed",
        "Seuls les périphériques créés ici peuvent être retirés",
    ),
    ("PipeWire went away", "PipeWire s'est arrêté"),
    // Failures that name the device or the app they were about.
    (
        "Couldn't change the default device",
        "Impossible de changer de périphérique par défaut",
    ),
    ("Couldn't move the app", "Impossible de déplacer l'app"),
    (
        "Couldn't change the sample rate",
        "Impossible de changer la fréquence d'échantillonnage",
    ),
    (
        "Couldn't change the buffer size",
        "Impossible de changer la taille du tampon",
    ),
    ("Couldn't connect those", "Impossible de les relier"),
    (
        "Couldn't change the volume",
        "Impossible de changer le volume",
    ),
    ("Couldn't mute", "Impossible de couper le son"),
    (
        "Couldn't change the app's volume",
        "Impossible de changer le volume de l'app",
    ),
    (
        "Couldn't mute the app",
        "Impossible de couper le son de l'app",
    ),
    ("That device is gone", "Ce périphérique n'existe plus"),
    ("That app is gone", "Cette app n'existe plus"),
    ("That card is gone", "Cette carte n'existe plus"),
    ("That port is gone", "Ce port n'existe plus"),
    ("That device has no ports", "Ce périphérique n'a aucun port"),
    ("That port isn't available", "Ce port n'est pas disponible"),
    (
        "That profile isn't available",
        "Ce profil n'est pas disponible",
    ),
    (
        "A monitor can't be the default",
        "Un moniteur ne peut pas être le périphérique par défaut",
    ),
    ("the device", "l'appareil"),
    // Bluetooth.
    ("Trusted", "Confiance"),
    ("Sound settings", "Réglages audio"),
    ("Forget", "Oublier"),
    ("Add device", "Ajouter un appareil"),
    ("Stop", "Arrêter"),
    ("Blocked", "Bloqué"),
    ("Adapter", "Adaptateur"),
    ("Properties", "Propriétés"),
    ("Services", "Services"),
    ("Powered", "Allumé"),
    ("Visible to others", "Visible par les autres"),
    ("Visible for", "Visible pendant"),
    ("Pairable", "Appairable"),
    ("Pairable for", "Appairable pendant"),
    ("1 minute", "1 minute"),
    ("3 minutes", "3 minutes"),
    ("5 minutes", "5 minutes"),
    ("15 minutes", "15 minutes"),
    ("Never", "Jamais"),
    ("Rename", "Renommer"),
    ("Advertising", "Diffusion"),
    ("Mute", "Couper le son"),
    ("Update", "Mettre à jour"),
    ("Cancel", "Annuler"),
    ("Pair", "Associer"),
    ("Deny", "Refuser"),
    ("Allow", "Autoriser"),
    ("Bluetooth isn't running", "Bluetooth ne tourne pas"),
    (
        "Bluetooth stopped working; restart Hokum",
        "Bluetooth a cessé de fonctionner ; redémarrez Hokum",
    ),
    ("No Bluetooth adapter", "Aucun adaptateur Bluetooth"),
    (
        "Couldn't turn Bluetooth on",
        "Impossible d'allumer Bluetooth",
    ),
    (
        "Couldn't turn Bluetooth off",
        "Impossible d'éteindre Bluetooth",
    ),
    (
        "Couldn't change the timeout",
        "Impossible de changer le délai",
    ),
    (
        "Couldn't change pairable",
        "Impossible de changer l'appairable",
    ),
    (
        "Couldn't rename the adapter",
        "Impossible de renommer l'adaptateur",
    ),
    (
        "Couldn't scan for devices",
        "Impossible de chercher des appareils",
    ),
    // Why a BlueZ call failed, after the sentence's colon.
    ("Bluetooth didn't answer", "Bluetooth n'a pas répondu"),
    ("not allowed", "non autorisé"),
    ("it's gone", "il n'existe plus"),
    (
        "wrong code or the device refused",
        "code erroné ou appareil qui refuse",
    ),
    ("cancelled", "annulé"),
    ("rejected", "refusé"),
    ("timed out", "délai dépassé"),
    ("it's off or out of range", "éteint ou hors de portée"),
    ("already connected", "déjà connecté"),
    ("already paired", "déjà associé"),
    ("already in progress", "déjà en cours"),
    ("Bluetooth isn't ready", "Bluetooth n'est pas prêt"),
    ("Bluetooth is off", "Bluetooth est éteint"),
    ("not connected", "pas connecté"),
    ("not supported", "non pris en charge"),
    ("not available", "indisponible"),
    ("not accepted", "non accepté"),
    (
        "blocked by airplane mode or a switch",
        "bloqué par le mode avion ou un interrupteur",
    ),
    (
        "no audio service is running",
        "aucun service audio ne tourne",
    ),
    ("the device refused", "l'appareil refuse"),
];

/// Sound's advanced page calls its tab Devices, and so does Bluetooth's, in its own words.
pub fn sound_devices() -> &'static str {
    match lang() {
        Lang::En => "Devices",
        Lang::Fr => "Périphériques",
    }
}

/// An app whose notifications are hidden: the list says `Muted`, the volume card says `Muted`
/// too, and French doesn't use the same word for both.
pub fn app_hidden() -> &'static str {
    match lang() {
        Lang::En => "Muted",
        Lang::Fr => "Masquée",
    }
}

/// `{name} is showing your notifications instead`.
pub fn other_notifier(name: &str) -> String {
    match lang() {
        Lang::En => format!("{name} is showing your notifications instead"),
        // Shorter than the English: the buttons beside it leave the line about 40 characters.
        Lang::Fr => format!("{name} affiche vos notifications"),
    }
}

/// The gallery's line under a custom theme: `Custom theme “name” from ~/…`.
pub fn custom_theme(name: &str, path: &str) -> String {
    match lang() {
        Lang::En => format!("Custom theme \u{201c}{name}\u{201d} from {path}"),
        Lang::Fr => format!("Thème personnalisé \u{ab} {name} \u{bb} depuis {path}"),
    }
}

/// `~/…/x.kdl has problems; showing Capsule`.
pub fn broken_theme(path: &str) -> String {
    match lang() {
        Lang::En => format!("{path} has problems; showing Capsule"),
        Lang::Fr => format!("{path} a des erreurs ; affichage de Capsule"),
    }
}

/// `No theme called “id”; showing Capsule`.
pub fn missing_theme(id: &str) -> String {
    match lang() {
        Lang::En => format!("No theme called \u{201c}{id}\u{201d}; showing Capsule"),
        Lang::Fr => format!("Aucun thème nommé \u{ab} {id} \u{bb} ; affichage de Capsule"),
    }
}

/// `~/…/script.rhai is running`, and a script that doesn't compile.
pub fn is_running(path: &str) -> String {
    match lang() {
        Lang::En => format!("{path} is running"),
        Lang::Fr => format!("{path} est actif"),
    }
}

pub fn error(e: &str) -> String {
    match lang() {
        Lang::En => format!("Error: {e}"),
        Lang::Fr => format!("Erreur : {e}"),
    }
}

/// `Couldn't save config.kdl: …`, and a file the window had to write.
pub fn couldnt_save(e: &str) -> String {
    match lang() {
        Lang::En => format!("Couldn't save config.kdl: {e}"),
        Lang::Fr => format!("Impossible d'enregistrer config.kdl : {e}"),
    }
}

pub fn couldnt_write(path: &str, e: &str) -> String {
    match lang() {
        Lang::En => format!("Couldn't write {path}: {e}"),
        Lang::Fr => format!("Impossible d'écrire {path} : {e}"),
    }
}

/// `1 more rule in config.kdl matches by text or urgency` (rules the window can't show).
pub fn more_rules_matching(n: usize) -> String {
    match (lang(), n) {
        (Lang::En, 1) => "1 more rule in config.kdl matches by text or urgency".into(),
        (Lang::En, n) => format!("{n} more rules in config.kdl match by text or urgency"),
        (Lang::Fr, 0) => "D'autres règles de config.kdl correspondent par texte ou urgence".into(),
        (Lang::Fr, 1) => "1 autre règle de config.kdl correspond par texte ou urgence".into(),
        (Lang::Fr, n) => {
            format!("{n} autres règles de config.kdl correspondent par texte ou urgence")
        }
    }
}

/// `2 more rules in config.kdl may apply to Slack` (rules that match more than one app).
pub fn more_rules_for(n: usize, app: &str) -> String {
    match (lang(), n) {
        (Lang::En, 1) => format!("1 more rule in config.kdl may apply to {app}"),
        (Lang::En, n) => format!("{n} more rules in config.kdl may apply to {app}"),
        (Lang::Fr, 0) => format!("D'autres règles de config.kdl peuvent s'appliquer à {app}"),
        (Lang::Fr, 1) => format!("1 autre règle de config.kdl peut s'appliquer à {app}"),
        (Lang::Fr, n) => format!("{n} autres règles de config.kdl peuvent s'appliquer à {app}"),
    }
}

/// `Turn on Sound` / `Turn off Sound`, at the end of a module's advanced page.
pub fn turn(on: bool, title: &str) -> String {
    match (lang(), on) {
        (Lang::En, true) => format!("Turn on {title}"),
        (Lang::En, false) => format!("Turn off {title}"),
        (Lang::Fr, true) => format!("Activer {title}"),
        (Lang::Fr, false) => format!("Désactiver {title}"),
    }
}

/// `Slack options`, the label of a row's chevron.
pub fn options(name: &str) -> String {
    match lang() {
        Lang::En => format!("{name} options"),
        Lang::Fr => format!("Options de {name}"),
    }
}

/// `Connect AirPods Pro`, `Disconnect …`, `Pair …`.
pub fn connect(name: &str, disconnect: bool) -> String {
    match (lang(), disconnect) {
        (Lang::En, false) => format!("Connect {name}"),
        (Lang::En, true) => format!("Disconnect {name}"),
        (Lang::Fr, false) => format!("Connecter {name}"),
        (Lang::Fr, true) => format!("Déconnecter {name}"),
    }
}

pub fn pair(name: &str) -> String {
    match lang() {
        Lang::En => format!("Pair {name}"),
        Lang::Fr => format!("Associer {name}"),
    }
}

/// `AirPods Pro connected · 82%`, and the same device at no battery reading.
pub fn connected(name: &str, battery: Option<u8>) -> String {
    match (lang(), battery) {
        (Lang::En, Some(b)) => format!("{name} connected · {b}%"),
        (Lang::En, None) => format!("{name} connected"),
        (Lang::Fr, Some(b)) => format!("{name} connecté · {b}%"),
        (Lang::Fr, None) => format!("{name} connecté"),
    }
}

/// `AirPods Pro battery low`, under it `12% left`.
pub fn battery_low(name: &str) -> String {
    match lang() {
        Lang::En => format!("{name} battery low"),
        Lang::Fr => format!("Batterie faible : {name}"),
    }
}

pub fn left(level: u8) -> String {
    match lang() {
        Lang::En => format!("{level}% left"),
        Lang::Fr => format!("{level} % restants"),
    }
}

/// A device's tooltip in the tray: `Built-in Audio muted`.
pub fn muted(device: &str) -> String {
    match lang() {
        Lang::En => format!("{device} muted"),
        Lang::Fr => format!("{device} : muet"),
    }
}

/// Sound's Clock: the quantum, and the rate it runs at.
pub fn samples(quantum: u32) -> String {
    match lang() {
        Lang::En => format!("{quantum} samples"),
        Lang::Fr => format!("{quantum} échantillons"),
    }
}

pub fn buffer(rate: &str, quantum: u32, ms: f32) -> String {
    match lang() {
        Lang::En => format!("{rate} · {quantum} samples · {ms:.1} ms"),
        Lang::Fr => format!("{rate} · {quantum} échantillons · {ms:.1} ms"),
    }
}

/// An adapter timeout the list doesn't offer: `240 seconds`.
pub fn seconds(n: u32) -> String {
    match lang() {
        Lang::En => format!("{n} seconds"),
        Lang::Fr => format!("{n} secondes"),
    }
}

/// `what: why`, with the space French puts before a colon: `PipeWire est injoignable : …`.
pub fn detail(what: &str, why: &str) -> String {
    detail_lang(lang(), what, why)
}

fn detail_lang(lang: Lang, what: &str, why: &str) -> String {
    match lang {
        Lang::En => format!("{what}: {why}"),
        Lang::Fr => format!("{what} : {why}"),
    }
}

/// A command that failed, as a module's section says it over the list: `Couldn't forget {}`,
/// with the names it mentions filled in. English is the key, so the call site reads what it
/// says; the French goes in [`FAILED`] with `{}` where the names go.
pub fn failed(en: &'static str, names: &[&str]) -> String {
    fill(lang(), en, names)
}

fn fill(lang: Lang, en: &'static str, names: &[&str]) -> String {
    let pattern = match lang {
        Lang::En => en,
        // A failure with no name in it is a fixed sentence from the table; one that names
        // something is a pattern, because word order and elision differ.
        Lang::Fr => FAILED
            .iter()
            .find(|(e, _)| *e == en)
            .map_or_else(|| text(Lang::Fr, en), |(_, fr)| *fr),
    };
    let mut out = pattern.to_owned();
    for name in names {
        out = out.replacen("{}", name, 1);
    }
    out
}

/// The failures BlueZ and PipeWire commands report, with the names they mention. Word order
/// and elision differ, so these are whole sentences rather than a verb and a name; a failure
/// with no name in it is simply a fixed sentence in the table above.
const FAILED: &[(&str, &str)] = &[
    (
        "Couldn't make {} visible",
        "Impossible de rendre {} visible",
    ),
    ("Couldn't make {} hidden", "Impossible de masquer {}"),
    ("Couldn't connect to {}", "Connexion à {} impossible"),
    ("Couldn't disconnect {}", "Déconnexion de {} impossible"),
    ("Couldn't pair with {}", "Association avec {} impossible"),
    (
        "Pairing with {} timed out",
        "L'association avec {} a expiré",
    ),
    ("Couldn't trust {}", "Impossible de faire confiance à {}"),
    (
        "Couldn't stop trusting {}",
        "Impossible de retirer la confiance à {}",
    ),
    ("Couldn't block {}", "Impossible de bloquer {}"),
    ("Couldn't unblock {}", "Impossible de débloquer {}"),
    ("Couldn't rename {}", "Impossible de renommer {}"),
    ("Couldn't forget {}", "Impossible d'oublier {}"),
    (
        "Couldn't connect {} on {}",
        "Impossible de connecter {} sur {}",
    ),
    (
        "Couldn't disconnect {} on {}",
        "Impossible de déconnecter {} sur {}",
    ),
    ("Couldn't switch to {}", "Impossible de passer en {}"),
    ("Couldn't create {}", "Impossible de créer {}"),
    ("Couldn't create {}: {}", "Impossible de créer {} : {}"),
    ("Couldn't remove {}: {}", "Impossible de retirer {} : {}"),
    (
        "Couldn't connect those: {}",
        "Impossible de les relier : {}",
    ),
    (
        "Couldn't disconnect those: {}",
        "Impossible de les délier : {}",
    ),
    ("can't reach PipeWire: {}", "PipeWire est injoignable : {}"),
    (
        "There's already a device called {}",
        "Un périphérique s'appelle déjà {}",
    ),
    ("PipeWire registry: {}", "Registre PipeWire : {}"),
];

/// A Bluetooth headset's profile: `Headset · LDAC` (the codec stays as PipeWire reports it).
pub fn headset(codec: &str) -> String {
    match lang() {
        Lang::En => format!("Headset · {codec}"),
        Lang::Fr => format!("Casque · {codec}"),
    }
}

pub fn pipewire(version: &str) -> String {
    format!("PipeWire {version}")
}

/// The update card, from the release service.
pub fn update_available(version: &str) -> String {
    match lang() {
        Lang::En => format!("Hokum {version} is available"),
        Lang::Fr => format!("Hokum {version} est disponible"),
    }
}

pub fn update_now() -> &'static str {
    match lang() {
        Lang::En => "Update now; Hokum restarts in a moment.",
        Lang::Fr => "Mettez à jour ; Hokum redémarre dans un instant.",
    }
}

pub fn update_with(cmd: &str) -> String {
    match lang() {
        Lang::En => format!("Update with your package manager: {cmd}"),
        Lang::Fr => format!("Mettez à jour avec votre gestionnaire de paquets : {cmd}"),
    }
}

pub fn update_package(package: &str) -> String {
    match lang() {
        Lang::En => format!("Update the {package} package with your package manager."),
        Lang::Fr => {
            format!("Mettez à jour le paquet {package} avec votre gestionnaire de paquets.")
        }
    }
}

pub fn update_elsewhere() -> &'static str {
    match lang() {
        Lang::En => "Update Hokum the way you installed it.",
        Lang::Fr => "Mettez à jour Hokum comme vous l'avez installé.",
    }
}

pub fn updating(version: &str) -> String {
    match lang() {
        Lang::En => format!("Updating to Hokum {version}…"),
        Lang::Fr => format!("Mise à jour vers Hokum {version}…"),
    }
}

pub fn download_failed(e: &str) -> String {
    match lang() {
        Lang::En => format!("The download failed ({e})."),
        Lang::Fr => format!("Le téléchargement a échoué ({e})."),
    }
}

pub fn not_executable(e: &str) -> String {
    match lang() {
        Lang::En => format!("Couldn't make it executable: {e}."),
        Lang::Fr => format!("Impossible de le rendre exécutable : {e}."),
    }
}

pub fn couldnt_replace(path: &str, e: &str) -> String {
    match lang() {
        Lang::En => format!("Couldn't replace {path}: {e}."),
        Lang::Fr => format!("Impossible de remplacer {path} : {e}."),
    }
}

pub fn now_running(version: &str) -> String {
    match lang() {
        Lang::En => format!("Now running {version}."),
        Lang::Fr => format!("Exécute maintenant {version}."),
    }
}

pub fn restart_failed(unit: &str) -> String {
    match lang() {
        Lang::En => format!("Installed, but restarting {unit} failed."),
        Lang::Fr => format!("Installé, mais le redémarrage de {unit} a échoué."),
    }
}

pub fn no_restart(e: &str) -> String {
    match lang() {
        Lang::En => format!("Installed, but couldn't restart: {e}."),
        Lang::Fr => format!("Installé, mais le redémarrage a échoué : {e}."),
    }
}

/// The pill over the cards that don't fit: `3 more`.
pub fn more_hidden(n: usize) -> String {
    match lang() {
        Lang::En => format!("{n} more"),
        Lang::Fr => format!("{n} de plus"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The source's `\u{201c}` and `\"` as the string they stand for.
    fn unescape(s: &str) -> String {
        let mut out = String::new();
        let mut it = s.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match it.next() {
                Some('u') => {
                    let mut hex = String::new();
                    for c in it.by_ref() {
                        match c {
                            '{' => {}
                            '}' => break,
                            c if c.is_ascii_hexdigit() => hex.push(c),
                            _ => break,
                        }
                    }
                    if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(c);
                    }
                }
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
        }
        out
    }

    #[test]
    fn every_string_has_one_translation() {
        let mut seen = HashSet::new();
        for (en, fr) in FR {
            assert!(seen.insert(*en), "{en:?} is translated twice");
            assert!(!en.is_empty() && !fr.is_empty(), "{en:?} is empty");
        }
    }

    /// Every `t("…")` and `failed("…")` in the code is in a table, and every entry is used: a
    /// rename or a dropped call site fails here rather than quietly showing English in French.
    #[test]
    fn the_table_and_the_code_agree() {
        let mut used: HashSet<String> = HashSet::new();
        let mut failures: HashSet<String> = HashSet::new();
        let mut files = Vec::new();
        let mut todo = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = todo.pop() {
            for entry in std::fs::read_dir(&dir).expect("src reads") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    todo.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && path.file_name().is_some_and(|n| n != "i18n.rs")
                {
                    files.push(path);
                }
            }
        }
        for path in files {
            let src = std::fs::read_to_string(&path).expect("a source file reads");
            // The string a call starts with: `t("…")`, `failed("…", &[…])`. A call that wraps
            // it onto the next line counts as well.
            let literal = |from: usize| {
                let rest = src[from..].trim_start().strip_prefix('"')?;
                let end = rest.find('"')?;
                Some(unescape(&rest[..end]))
            };
            // `t("…")` only: `insert("…")`, `format!` and the like end in `t(` too.
            for (i, _) in src.match_indices("t(") {
                let ours = src[..i]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
                if let Some(en) = ours.then(|| literal(i + 2)).flatten() {
                    used.insert(en);
                }
            }
            // `i18n::failed("…", &[…])`: map.rs has a `failed` of its own, and bus.rs binds a
            // `failed` to a D-Bus error.
            for (i, _) in src.match_indices("i18n::failed(") {
                if let Some(en) = literal(i + 13) {
                    failures.insert(en);
                }
            }
        }
        for en in &used {
            assert!(
                FR.iter().any(|(e, _)| e == en),
                "{en:?} is drawn but has no translation",
            );
        }
        for (en, _) in FR {
            assert!(
                used.contains(*en) || failures.contains(*en),
                "{en:?} is translated but never drawn",
            );
        }
        for en in &failures {
            assert!(
                FAILED.iter().any(|(e, _)| e == en) || FR.iter().any(|(e, _)| e == en),
                "{en:?} is a failure but has no translation",
            );
        }
        for (en, fr) in FAILED {
            assert!(
                failures.contains(*en),
                "{en:?} is a pattern that nothing draws",
            );
            assert_eq!(
                en.matches("{}").count(),
                fr.matches("{}").count(),
                "{en:?} and its French fill a different number of names",
            );
        }
    }

    #[test]
    fn a_colon_takes_the_space_french_gives_it() {
        assert_eq!(detail_lang(Lang::En, "PipeWire", "gone"), "PipeWire: gone");
        assert_eq!(
            detail_lang(Lang::Fr, "PipeWire", "parti"),
            "PipeWire : parti"
        );
    }

    #[test]
    fn the_locale_names_a_language() {
        assert_eq!(Lang::of("fr_FR"), Some(Lang::Fr));
        assert_eq!(Lang::of("en_US"), Some(Lang::En));
        assert_eq!(Lang::of("de_DE"), None);
        assert_eq!(Lang::of(""), None);
    }

    #[test]
    fn a_failure_fills_the_names_it_mentions() {
        assert_eq!(
            fill(Lang::En, "Couldn't forget {}", &["Buds"]),
            "Couldn't forget Buds"
        );
        assert_eq!(
            fill(Lang::Fr, "Couldn't forget {}", &["Buds"]),
            "Impossible d'oublier Buds"
        );
        assert_eq!(
            fill(Lang::Fr, "Couldn't connect {} on {}", &["Audio", "Buds"]),
            "Impossible de connecter Audio sur Buds"
        );
        // A fixed sentence, and one Hokum has no French for, keep their English.
        assert_eq!(
            fill(Lang::Fr, "Couldn't turn Bluetooth on", &[]),
            "Impossible d'allumer Bluetooth"
        );
        assert_eq!(fill(Lang::Fr, "Something new", &[]), "Something new");
    }

    #[test]
    fn the_table_answers_in_the_asked_language() {
        assert_eq!(text(Lang::Fr, "Pinned"), "Épinglé");
        assert_eq!(text(Lang::Fr, "Not in the table"), "Not in the table");
        assert_eq!(text(Lang::En, "Pinned"), "Pinned");
    }
}
