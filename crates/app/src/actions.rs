//! Actions de l'application, raccourcis clavier et entrées de la palette.
//!
//! Les raccourcis évitent les touches seules (espace, flèches) qui
//! entreraient en conflit avec la saisie dans les champs de texte ; ils
//! reprennent ceux de Rhythmbox quand c'est possible. Les touches
//! multimédia du clavier passent par MPRIS (voir `system.rs`).

use gpui_kit::assets::IconName;
use gpui_kit::*;

gpui_kit::actions!(
    ondes,
    [
        TogglePlay,
        Stop,
        Next,
        Previous,
        SeekForward,
        SeekBackward,
        VolumeUp,
        VolumeDown,
        ToggleShuffle,
        CycleRepeat,
        ToggleRecording,
        OpenAudioSettings,
        ToggleCompact,
        ToggleVisualizer,
        NextEffect,
        PreviousEffect,
        ToggleFullscreen,
        CloseVisualizer,
        ToggleFavorite,
        OpenPalette,
        FocusLocation,
        OpenFile,
        ToggleTheme,
        ToggleSidebar,
        ToggleNotifications,
        CheckUpdates,
        ToggleUpdateCheck,
        About,
        GoNowPlaying,
        GoExplore,
        GoFavorites,
        GoTracks,
        GoAlbums,
        GoArtists,
        GoFolders,
        Quit,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-space", TogglePlay, None),
        KeyBinding::new("alt-right", Next, None),
        KeyBinding::new("alt-left", Previous, None),
        KeyBinding::new("alt-shift-right", SeekForward, None),
        KeyBinding::new("alt-shift-left", SeekBackward, None),
        KeyBinding::new("ctrl-up", VolumeUp, None),
        KeyBinding::new("ctrl-down", VolumeDown, None),
        KeyBinding::new("ctrl-u", ToggleShuffle, None),
        KeyBinding::new("ctrl-r", CycleRepeat, None),
        KeyBinding::new("ctrl-d", ToggleFavorite, None),
        KeyBinding::new("ctrl-shift-r", ToggleRecording, None),
        KeyBinding::new("ctrl-shift-m", ToggleCompact, None),
        KeyBinding::new("ctrl-shift-v", ToggleVisualizer, None),
        // Fenêtre du visualiseur.
        KeyBinding::new("right", NextEffect, Some("Visualizer")),
        KeyBinding::new("left", PreviousEffect, Some("Visualizer")),
        KeyBinding::new("f11", ToggleFullscreen, Some("Visualizer")),
        KeyBinding::new("escape", CloseVisualizer, Some("Visualizer")),
        KeyBinding::new("ctrl-,", OpenAudioSettings, None),
        KeyBinding::new("ctrl-k", OpenPalette, None),
        KeyBinding::new("ctrl-shift-p", OpenPalette, None),
        KeyBinding::new("ctrl-l", FocusLocation, None),
        KeyBinding::new("ctrl-o", OpenFile, None),
        KeyBinding::new("ctrl-b", ToggleSidebar, None),
        KeyBinding::new("ctrl-0", GoNowPlaying, None),
        KeyBinding::new("ctrl-1", GoExplore, None),
        KeyBinding::new("ctrl-2", GoFavorites, None),
        KeyBinding::new("ctrl-3", GoTracks, None),
        KeyBinding::new("ctrl-4", GoAlbums, None),
        KeyBinding::new("ctrl-5", GoArtists, None),
        KeyBinding::new("ctrl-6", GoFolders, None),
        KeyBinding::new("ctrl-q", Quit, None),
    ]);
}

/// Une entrée de la palette de commandes.
pub struct PaletteEntry {
    pub label: &'static str,
    pub icon: IconName,
    pub action: fn() -> Box<dyn Action>,
    pub keywords: &'static [&'static str],
}

macro_rules! entry {
    ($label:expr, $icon:ident, $action:ident, [$($kw:expr),*]) => {
        PaletteEntry {
            label: $label,
            icon: IconName::$icon,
            action: || Box::new($action),
            keywords: &[$($kw),*],
        }
    };
}

/// Groupes de la palette, dans l'ordre d'affichage.
pub fn palette() -> Vec<(&'static str, Vec<PaletteEntry>)> {
    vec![
        (
            "Lecture",
            vec![
                entry!(
                    "Lecture / pause",
                    Play,
                    TogglePlay,
                    ["play", "pause", "lire"]
                ),
                entry!("Arrêter", Square, Stop, ["stop"]),
                entry!("Morceau suivant", SkipForward, Next, ["next", "suivant"]),
                entry!(
                    "Morceau précédent",
                    SkipBack,
                    Previous,
                    ["previous", "précédent"]
                ),
                entry!(
                    "Avancer de 10 s",
                    FastForward,
                    SeekForward,
                    ["seek", "avance"]
                ),
                entry!("Reculer de 10 s", Rewind, SeekBackward, ["seek", "recul"]),
                entry!("Monter le volume", Volume2, VolumeUp, ["volume", "son"]),
                entry!("Baisser le volume", Volume1, VolumeDown, ["volume", "son"]),
                entry!("Aléatoire", Shuffle, ToggleShuffle, ["shuffle", "mélanger"]),
                entry!(
                    "Mode de répétition",
                    Repeat,
                    CycleRepeat,
                    ["repeat", "boucle"]
                ),
                entry!(
                    "Enregistrer le direct / arrêter l'enregistrement",
                    CircleDot,
                    ToggleRecording,
                    ["enregistrer", "record", "capture"]
                ),
                entry!(
                    "Vue compacte / fenêtre principale",
                    PictureInPicture2,
                    ToggleCompact,
                    ["compact", "mini", "réduire", "petit"]
                ),
                entry!(
                    "Visualiseur",
                    Sparkles,
                    ToggleVisualizer,
                    ["visualiseur", "shader", "effets", "plein écran"]
                ),
                entry!(
                    "Réglages audio…",
                    SlidersHorizontal,
                    OpenAudioSettings,
                    [
                        "égaliseur",
                        "equalizer",
                        "replaygain",
                        "normalisation",
                        "fondu",
                        "crossfade",
                        "spectre"
                    ]
                ),
                entry!(
                    "Ajouter / retirer la radio des favoris",
                    Heart,
                    ToggleFavorite,
                    ["favori", "favorite"]
                ),
            ],
        ),
        (
            "Aller à",
            vec![
                entry!(
                    "En cours",
                    Disc3,
                    GoNowPlaying,
                    ["lecture", "pochette", "visualiseur", "spectre", "file"]
                ),
                entry!(
                    "Explorer les radios",
                    Radio,
                    GoExplore,
                    ["radio", "recherche"]
                ),
                entry!("Radios favorites", Heart, GoFavorites, ["favoris"]),
                entry!("Morceaux", ListMusic, GoTracks, ["bibliothèque", "titres"]),
                entry!("Albums", DiscAlbum, GoAlbums, ["bibliothèque"]),
                entry!("Artistes", MicVocal, GoArtists, ["bibliothèque"]),
                entry!("Dossiers", FolderOpen, GoFolders, ["bibliothèque", "scan"]),
            ],
        ),
        (
            "Application",
            vec![
                entry!(
                    "Écouter une URL…",
                    Link,
                    FocusLocation,
                    ["url", "flux", "ouvrir"]
                ),
                entry!(
                    "Ouvrir un fichier…",
                    FileMusic,
                    OpenFile,
                    ["fichier", "ouvrir"]
                ),
                entry!(
                    "Basculer thème clair / sombre",
                    SunMoon,
                    ToggleTheme,
                    ["thème", "dark", "light"]
                ),
                entry!(
                    "Plier / déplier la barre latérale",
                    PanelLeft,
                    ToggleSidebar,
                    ["barre latérale", "sidebar", "menu", "plier", "masquer"]
                ),
                entry!(
                    "Activer / désactiver les notifications",
                    Bell,
                    ToggleNotifications,
                    ["notification"]
                ),
                entry!(
                    "Rechercher les mises à jour",
                    Download,
                    CheckUpdates,
                    ["mise à jour", "update", "version"]
                ),
                entry!(
                    "Activer / désactiver la vérification des mises à jour",
                    RefreshCw,
                    ToggleUpdateCheck,
                    ["mise à jour", "update"]
                ),
                entry!(
                    "À propos d'Ondes",
                    Info,
                    About,
                    ["version", "licences", "journal"]
                ),
                entry!("Quitter", LogOut, Quit, ["quit", "fermer"]),
            ],
        ),
    ]
}
