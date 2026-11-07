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
`Ctrl+Q` pour quitter.

## Structure

- `crates/app` : binaire GPUI (fenêtre, vues)
- `crates/core` : types partagés et pont tokio ↔ GPUI
- `crates/audio` : moteur audio (thread dédié, commandes / événements)
- `crates/sources` : fichiers locaux, flux HTTP avec reconnexion, ICY, playlists
- `crates/radio` : client Radio Browser, favoris SQLite, import de playlists

## Données

- Favoris : `~/.local/share/ondes/ondes.db`
- Cache des logos : `~/.cache/ondes/logos` (peut être supprimé sans risque)
