// Pas de fenêtre console sous Windows (sauf en debug, pour voir les journaux).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod about;
mod actions;
mod audio_settings;
mod compact;
mod history;
mod info;
mod library;
mod library_view;
mod logos;
mod now_playing;
mod paths;
mod player;
mod player_bar;
mod queue;
mod radio;
mod radio_explore;
mod radio_favorites;
mod runtime;
mod settings;
mod shell;
mod spectrum;
mod stage;
mod station_row;
mod system;
mod update;
mod visualizer;

use gpui_kit::assets::AllAssets;
use gpui_kit::*;

use settings::{AppState, Settings};

/// Journal sur la sortie d'erreur et dans un fichier (l'application
/// installée n'a pas de terminal). Les plantages y sont aussi consignés.
fn init_logging() {
    use tracing_subscriber::prelude::*;

    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            "ondes=info,ondes_core=info,ondes_audio=info,ondes_sources=info,ondes_radio=info,ondes_library=info".into()
        })
    };
    let path = paths::log_file();
    let file = path
        .parent()
        .and_then(|dir| std::fs::create_dir_all(dir).ok())
        .and_then(|_| {
            let _ = std::fs::rename(&path, path.with_extension("log.1"));
            std::fs::File::create(&path).ok()
        });
    let file_layer = file.map(|file| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .with_filter(filter())
    });
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter()))
        .with(file_layer)
        .init();

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("plantage : {info}\n{backtrace}");
        default_hook(info);
    }));
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        journal = %path.display(),
        "démarrage d'Ondes"
    );
}

fn main() -> anyhow::Result<()> {
    init_logging();

    let rt = runtime::Rt::start()?;
    let state = AppState::load();
    // `ondes <url|fichier>` : lecture immédiate (utile pour « Ouvrir avec »).
    let initial = std::env::args()
        .nth(1)
        .and_then(|arg| ondes_audio::SourceSpec::parse(&arg));

    // `AllAssets` : catalogue Lucide complet (radio, musique, volume…).
    gpui_kit::application()
        .with_assets(AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_global(rt);
            let saved_window = state.window;
            Settings::init(state, cx);
            radio::Directory::init(cx);
            radio::RadioFranceApi::init(cx);
            radio::FavoritesModel::init(cx);
            history::HistoryModel::init(cx);
            info::init(cx);
            logos::Logos::init(cx);
            library::LibraryModel::init(cx);

            actions::bind_keys(cx);
            cx.on_action(|_: &actions::Quit, cx| cx.quit());
            // Écrit l'état en attente avant de quitter.
            cx.on_app_quit(|cx| {
                Settings::global(cx).update(cx, |s, _| s.save_now());
                async {}
            })
            .detach();
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let options = WindowOptions {
                window_bounds: Some(match saved_window {
                    Some(w) => {
                        let bounds = Bounds::new(
                            point(px(w.x), px(w.y)),
                            size(px(w.width.max(260.)), px(w.height.max(240.))),
                        );
                        if w.maximized {
                            WindowBounds::Maximized(bounds)
                        } else {
                            WindowBounds::Windowed(bounds)
                        }
                    }
                    None => WindowBounds::centered(size(px(1100.), px(720.)), cx),
                }),
                // Petite fenêtre : la vue compacte prend le relais (voir `shell.rs`).
                window_min_size: Some(size(px(260.), px(240.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Ondes".into()),
                    ..Default::default()
                }),
                app_id: Some("ondes".into()),
                ..Default::default()
            };

            if let Err(err) = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| shell::Shell::new(initial, window, cx))
            }) {
                tracing::error!("impossible d'ouvrir la fenêtre : {err:#}");
                cx.quit();
            }
        });

    Ok(())
}
