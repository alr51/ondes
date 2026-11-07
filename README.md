# Ondes

Écouter des webradios et explorer les sons de l'ordinateur local.
Rust + [GPUI](https://www.gpui.rs/) + [GPUI Kit](https://gpui-kit.com/).

Voir [ROADMAP.md](ROADMAP.md).

## Prérequis (Linux, Debian/Ubuntu)

```sh
sudo apt install libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libx11-xcb-dev \
  libwayland-dev libvulkan-dev libfontconfig-dev libfreetype-dev libasound2-dev pkg-config
```

La toolchain Rust (1.95) est fixée par `rust-toolchain.toml`.

## Lancer

```sh
cargo run                                   # RUST_LOG=ondes_audio=debug pour suivre le moteur
cargo run -- https://icecast.radiofrance.fr/fip-midfi.mp3
cargo run -- ~/Musique/morceau.flac
```

Dans l'application : coller une URL (flux mp3/aac/ogg, `.pls`, `.m3u`) et
appuyer sur Entrée, « Ouvrir un fichier… », ou glisser un fichier sur la fenêtre.
Les touches multimédia du clavier et le panneau média du bureau (MPRIS)
pilotent la lecture.

### Raccourcis

| Raccourci | Action |
|---|---|
| `Ctrl+K` ou `Ctrl+Maj+P` | Palette de commandes |
| `Ctrl+Espace` | Lecture / pause |
| `Alt+→` / `Alt+←` | Morceau suivant / précédent |
| `Alt+Maj+→` / `Alt+Maj+←` | Avancer / reculer de 10 s |
| `Ctrl+↑` / `Ctrl+↓` | Volume |
| `Ctrl+U` / `Ctrl+R` | Aléatoire / mode de répétition |
| `Ctrl+D` | Radio en cours : ajouter ou retirer des favoris |
| `Ctrl+L` / `Ctrl+O` | Écouter une URL / ouvrir un fichier |
| `Ctrl+1` … `Ctrl+6` | Pages (Explorer, Favoris, Morceaux, Albums, Artistes, Dossiers) |
| `Ctrl+Q` | Quitter |

## Structure

- `crates/app` : binaire GPUI (fenêtre, vues)
- `crates/core` : types partagés et pont tokio ↔ GPUI
- `crates/audio` : moteur audio (thread dédié, commandes / événements)
- `crates/sources` : fichiers locaux, flux HTTP avec reconnexion, ICY, playlists
- `crates/radio` : client Radio Browser, favoris SQLite, import de playlists
- `crates/library` : bibliothèque locale (scan incrémental, tags, pochettes, recherche)

## Données

- Favoris et index de la bibliothèque : `~/.local/share/ondes/ondes.db`
- État (volume, dernière écoute, fenêtre…) : `~/.local/share/ondes/state.toml`
- Cache des logos et pochettes : `~/.cache/ondes/{logos,covers}` (peut être supprimé sans risque)

## Développement

En build debug, `ONDES_PAGE=tracks|albums|artists|folders|favorites` ouvre
directement une page et `ONDES_PALETTE=1` ouvre la palette (captures d'écran
automatisées). Avec
`XDG_DATA_HOME` / `XDG_CACHE_HOME`, l'application utilise des données isolées.
