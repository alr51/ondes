//! Fenêtre principale : barre latérale, contenu, barre de lecture.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, StyledExt, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    sidebar::{Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    v_flex,
};
use gpui_kit::*;

use ondes_audio::SourceSpec;

use crate::player::Player;
use crate::player_bar::PlayerBar;
use crate::radio_explore::RadioExplore;
use crate::radio_favorites::RadioFavorites;
use crate::runtime::Rt;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    RadioExplore,
    RadioFavorites,
    LibraryTracks,
    LibraryAlbums,
    LibraryFolders,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::RadioExplore => "Explorer les radios",
            Page::RadioFavorites => "Radios favorites",
            Page::LibraryTracks => "Morceaux",
            Page::LibraryAlbums => "Albums",
            Page::LibraryFolders => "Dossiers",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Page::RadioExplore => IconName::Radio,
            Page::RadioFavorites => IconName::Heart,
            Page::LibraryTracks => IconName::ListMusic,
            Page::LibraryAlbums => IconName::DiscAlbum,
            Page::LibraryFolders => IconName::FolderOpen,
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Page::RadioExplore | Page::RadioFavorites => "",
            Page::LibraryTracks | Page::LibraryAlbums | Page::LibraryFolders => {
                "La bibliothèque locale arrive en phase 3."
            }
        }
    }
}

/// État du pont tokio, vérifié au démarrage.
enum RtStatus {
    Checking,
    Ready(Duration),
    Failed(String),
}

pub struct Shell {
    page: Page,
    /// Tant que l'utilisateur n'a pas choisi, le thème suit le système.
    follow_system_theme: bool,
    rt_status: RtStatus,
    player: Entity<Player>,
    player_bar: Entity<PlayerBar>,
    explore: Entity<RadioExplore>,
    favorites: Entity<RadioFavorites>,
    /// Champ « URL ou chemin » de la barre d'ouverture rapide.
    location: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(initial: Option<SourceSpec>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Theme::sync_system_appearance(Some(window), cx);
        let appearance_sub = cx.observe_window_appearance(window, |this, window, cx| {
            if this.follow_system_theme {
                Theme::sync_system_appearance(Some(window), cx);
            }
        });

        let location = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("URL d'une radio (mp3, aac, ogg, .pls, .m3u) ou chemin d'un fichier")
        });
        let enter_sub = cx.subscribe_in(&location, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.listen_location(cx);
            }
        });

        let player = cx.new(|cx| Player::new(0.8, cx));
        let player_bar = cx.new(|cx| PlayerBar::new(player.clone(), cx));
        let explore = cx.new(|cx| RadioExplore::new(player.clone(), window, cx));
        let favorites = cx.new(|cx| RadioFavorites::new(player.clone(), window, cx));

        let shell = Self {
            page: Page::RadioExplore,
            follow_system_theme: true,
            rt_status: RtStatus::Checking,
            player,
            player_bar,
            explore,
            favorites,
            location,
            _subscriptions: vec![appearance_sub, enter_sub],
        };
        shell.check_runtime(cx);
        if let Some(spec) = initial {
            shell.play(spec, cx);
        }
        shell
    }

    /// Aller-retour GPUI → tokio → GPUI : prouve que le pont fonctionne.
    fn check_runtime(&self, cx: &mut Context<Self>) {
        let task = Rt::spawn(cx, ondes_core::rt::probe());
        cx.spawn(async move |this, cx| {
            let status = match task.await {
                Ok(elapsed) => RtStatus::Ready(elapsed),
                Err(err) => RtStatus::Failed(err.to_string()),
            };
            this.update(cx, |this, cx| {
                this.rt_status = status;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn play(&self, spec: SourceSpec, cx: &mut Context<Self>) {
        self.player.update(cx, |p, cx| p.load(spec, cx));
    }

    fn listen_location(&mut self, cx: &mut Context<Self>) {
        let text = self.location.read(cx).value();
        if let Some(spec) = SourceSpec::parse(&text) {
            self.play(spec, cx);
        }
    }

    fn open_file_dialog(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Écouter".into()),
        });
        cx.spawn(async move |this, cx| {
            let path = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Err(e)) => {
                    tracing::warn!("boîte de dialogue indisponible : {e:#}");
                    None
                }
                _ => None,
            };
            if let Some(path) = path {
                this.update(cx, |this, cx| this.play(SourceSpec::File(path), cx))
                    .ok();
            }
        })
        .detach();
    }

    fn render_quick_open(&self, cx: &mut Context<Self>) -> Div {
        h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .child(Input::new(&self.location).cleanable(true)),
            )
            .child(
                Button::new("listen")
                    .icon(IconName::Play)
                    .label("Écouter")
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.listen_location(cx))),
            )
            .child(
                Button::new("open-file")
                    .icon(IconName::FolderOpen)
                    .label("Ouvrir un fichier…")
                    .outline()
                    .on_click(cx.listener(|this, _, _, cx| this.open_file_dialog(cx))),
            )
    }

    fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_system_theme = false;
        let mode = if cx.theme().is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        Theme::change(mode, Some(window), cx);
    }

    fn nav_item(&self, page: Page, cx: &mut Context<Self>) -> SidebarMenuItem {
        let entity = cx.entity().downgrade();
        SidebarMenuItem::new(page.title())
            .icon(page.icon())
            .active(self.page == page)
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |this, cx| {
                        this.page = page;
                        cx.notify();
                    })
                    .ok();
            })
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let is_dark = theme.is_dark();

        Sidebar::new("main-sidebar")
            .w(px(232.))
            .header(
                SidebarHeader::new().child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size_8()
                                .rounded(theme.radius)
                                .bg(theme.sidebar_primary)
                                .text_color(theme.sidebar_primary_foreground)
                                .child(IconName::RadioTower),
                        )
                        .child(div().font_semibold().child("Ondes")),
                ),
            )
            .child(
                SidebarGroup::new("Radios").child(SidebarMenu::new().children([
                    self.nav_item(Page::RadioExplore, cx),
                    self.nav_item(Page::RadioFavorites, cx),
                ])),
            )
            .child(
                SidebarGroup::new("Bibliothèque").child(SidebarMenu::new().children([
                    self.nav_item(Page::LibraryTracks, cx),
                    self.nav_item(Page::LibraryAlbums, cx),
                    self.nav_item(Page::LibraryFolders, cx),
                ])),
            )
            .footer(
                SidebarFooter::new().child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .child(self.render_rt_status(cx))
                        .child(
                            Button::new("toggle-theme")
                                .icon(if is_dark {
                                    IconName::Sun
                                } else {
                                    IconName::Moon
                                })
                                .ghost()
                                .small()
                                .tooltip(if is_dark {
                                    "Thème clair"
                                } else {
                                    "Thème sombre"
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_theme(window, cx);
                                })),
                        ),
                ),
            )
    }

    fn render_rt_status(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let (color, label) = match &self.rt_status {
            RtStatus::Checking => (theme.muted_foreground, "Async : démarrage…".to_string()),
            RtStatus::Ready(d) => (
                theme.success,
                format!("Async : OK ({:.1} ms)", d.as_secs_f64() * 1000.),
            ),
            RtStatus::Failed(err) => (theme.danger, format!("Async : {err}")),
        };
        h_flex()
            .gap_1p5()
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(div().size_2().rounded_full().bg(color))
            .child(label)
    }

    fn render_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let quick_open = self.render_quick_open(cx);
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .h_full()
            .min_h_0()
            .p_6()
            .gap_4()
            .child(quick_open)
            .child(div().text_xl().font_semibold().child(self.page.title()))
            .child(match self.page {
                Page::RadioExplore => self.explore.clone().into_any_element(),
                Page::RadioFavorites => self.favorites.clone().into_any_element(),
                _ => v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_dashed()
                    .border_color(theme.border)
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(self.page.icon()).large())
                    .child(div().text_sm().child(self.page.placeholder()))
                    .into_any_element(),
            })
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            // Glisser-déposer un fichier audio n'importe où dans la fenêtre.
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if let Some(path) = paths.paths().first() {
                    this.play(SourceSpec::File(path.clone()), cx);
                }
            }))
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(self.render_content(cx)),
            )
            .child(self.player_bar.clone())
    }
}
