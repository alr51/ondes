mod actions;
mod library;
mod library_view;
mod logos;
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
mod station_row;
mod system;

use gpui_kit::assets::AllAssets;
use gpui_kit::*;

use settings::{AppState, Settings};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ondes=info,ondes_core=info".into()),
        )
        .init();

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
            radio::FavoritesModel::init(cx);
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
                            size(px(w.width.max(720.)), px(w.height.max(480.))),
                        );
                        if w.maximized {
                            WindowBounds::Maximized(bounds)
                        } else {
                            WindowBounds::Windowed(bounds)
                        }
                    }
                    None => WindowBounds::centered(size(px(1100.), px(720.)), cx),
                }),
                window_min_size: Some(size(px(720.), px(480.))),
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
