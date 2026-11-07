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
mod shell;
mod station_row;

use gpui_kit::assets::AllAssets;
use gpui_kit::*;

actions!(ondes, [Quit]);

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ondes=info,ondes_core=info".into()),
        )
        .init();

    let rt = runtime::Rt::start()?;
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
            radio::Directory::init(cx);
            radio::FavoritesModel::init(cx);
            logos::Logos::init(cx);
            library::LibraryModel::init(cx);

            cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1100.), px(720.)), cx)),
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
