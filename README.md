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
cargo run            # RUST_LOG=ondes=debug pour plus de logs
```

Raccourci : `Ctrl+Q` pour quitter.

## Structure

- `crates/app` : binaire GPUI (fenêtre, vues)
- `crates/core` : types partagés et pont tokio ↔ GPUI
