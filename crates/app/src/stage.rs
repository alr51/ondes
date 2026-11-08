//! Scène de lecture : ce qui est joué, en grand, selon un mode d'affichage
//! (pochette ou logo, spectre, visualiseur, titre, file d'attente).
//!
//! Partagée par la vue compacte et la page « En cours » de la fenêtre
//! principale, chacune avec sa taille et son mode mémorisé.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::logos::Logos;
use crate::player::Player;
use crate::player_bar::clock;
use crate::settings::Settings;
use crate::spectrum::Spectrum;
use crate::visualizer::{ShaderView, effect_name};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Cover,
    Spectrum,
    Shader,
    Title,
    Queue,
}

impl Mode {
    pub const ALL: [Mode; 5] = [
        Mode::Cover,
        Mode::Spectrum,
        Mode::Shader,
        Mode::Title,
        Mode::Queue,
    ];

    fn key(self) -> &'static str {
        match self {
            Mode::Cover => "cover",
            Mode::Spectrum => "spectrum",
            Mode::Shader => "shader",
            Mode::Title => "title",
            Mode::Queue => "queue",
        }
    }

    fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|m| m.key() == key)
            .unwrap_or(Mode::Cover)
    }

    fn icon(self) -> IconName {
        match self {
            Mode::Cover => IconName::Image,
            Mode::Spectrum => IconName::AudioLines,
            Mode::Shader => IconName::Sparkles,
            Mode::Title => IconName::Type,
            Mode::Queue => IconName::ListMusic,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Mode::Cover => "Pochette",
            Mode::Spectrum => "Spectre",
            Mode::Shader => "Visualiseur",
            Mode::Title => "Titre",
            Mode::Queue => "File d'attente",
        }
    }
}

/// Où la scène est affichée : tailles et mode mémorisé différents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Compact,
    Page,
}

impl Place {
    fn saved_mode(self, cx: &App) -> Mode {
        let state = Settings::get(cx);
        Mode::from_key(match self {
            Place::Compact => &state.compact.mode,
            Place::Page => &state.now_playing_mode,
        })
    }

    fn save_mode(self, mode: Mode, cx: &mut App) {
        Settings::update(cx, |s| match self {
            Place::Compact => s.compact.mode = mode.key().into(),
            Place::Page => s.now_playing_mode = mode.key().into(),
        });
    }
}

pub struct Stage {
    player: Entity<Player>,
    place: Place,
    mode: Mode,
    spectrum: Entity<Spectrum>,
    /// Visualiseur, créé à l'affichage de son mode (rendu GPU).
    shader: Option<Entity<ShaderView>>,
    _subscriptions: Vec<Subscription>,
}

impl Stage {
    pub fn new(player: Entity<Player>, place: Place, cx: &mut Context<Self>) -> Self {
        let mode = place.saved_mode(cx);
        let spectrum = cx.new(|cx| match place {
            Place::Compact => Spectrum::with_size(player.clone(), 28, 150., 6., true, cx),
            Place::Page => Spectrum::with_size(player.clone(), 48, 260., 10., true, cx),
        });
        let subscriptions = vec![
            cx.observe(&player, |_, _, cx| cx.notify()),
            cx.observe(&Logos::global(cx), |_, _, cx| cx.notify()),
        ];
        Self {
            player,
            place,
            mode,
            spectrum,
            shader: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        if mode != Mode::Shader {
            self.shader = None;
        }
        self.place.save_mode(mode, cx);
        cx.notify();
    }

    /// Scène masquée : le visualiseur est arrêté (recréé au retour).
    pub fn suspend(&mut self) {
        self.shader = None;
    }

    /// Image du mode « Pochette » : pochette du morceau, ou logo de la radio.
    fn artwork(&self, cx: &mut Context<Self>) -> Option<std::path::PathBuf> {
        let p = self.player.read(cx);
        if let Some(cover) = p.current_track().and_then(|t| t.cover.clone()) {
            return Some(cover);
        }
        let logo = p.station.as_ref()?.favicon.clone()?;
        Logos::global(cx).update(cx, |l, cx| l.get(&logo, cx))
    }
}

/// Boutons de choix du mode.
pub fn mode_bar(stage: &Entity<Stage>, cx: &App) -> Div {
    let current = stage.read(cx).mode();
    let large = stage.read(cx).place == Place::Page;
    h_flex()
        .gap_0p5()
        .items_center()
        .children(Mode::ALL.map(|mode| {
            let stage = stage.clone();
            let button = Button::new(mode.key())
                .icon(mode.icon())
                .ghost()
                .selected(current == mode)
                .tooltip(mode.label())
                .on_click(move |_, _, cx| stage.update(cx, |s, cx| s.set_mode(mode, cx)));
            if large {
                button.small().label(mode.label())
            } else {
                button.xsmall()
            }
        }))
}

impl Render for Stage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let large = self.place == Place::Page;
        match self.mode {
            Mode::Cover => {
                let live = self.player.read(cx).track.as_ref().is_some_and(|t| t.live);
                let muted = theme.muted_foreground;
                let icon_size = if large { 128. } else { 64. };
                let fallback = move || {
                    Icon::new(if live {
                        IconName::Radio
                    } else {
                        IconName::Music
                    })
                    .size(px(icon_size))
                    .text_color(muted)
                    .into_any_element()
                };
                let image = match self.artwork(cx) {
                    Some(path) => img(path)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .with_fallback(fallback)
                        .into_any_element(),
                    None => fallback(),
                };
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(theme.radius_lg)
                    .overflow_hidden()
                    .child(image)
                    .into_any_element()
            }
            Mode::Shader => {
                let player = self.player.clone();
                let shader = self
                    .shader
                    .get_or_insert_with(|| cx.new(|cx| ShaderView::new(player, cx)))
                    .clone();
                let effect = effect_name(shader.read(cx).effect());
                div()
                    .id("stage-shader")
                    .relative()
                    .size_full()
                    .rounded(theme.radius_lg)
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(shader) = &this.shader {
                            shader.update(cx, |s, cx| s.cycle(1, cx));
                        }
                        cx.notify();
                    }))
                    .child(shader)
                    .child(
                        div()
                            .absolute()
                            .top_2()
                            .right_2()
                            .px_1p5()
                            .rounded(theme.radius)
                            .bg(gpui_kit::black().opacity(0.4))
                            .when(large, |d| d.text_sm())
                            .when(!large, |d| d.text_xs())
                            .text_color(gpui_kit::white())
                            .child(effect),
                    )
                    .into_any_element()
            }
            Mode::Spectrum => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(self.spectrum.clone())
                .into_any_element(),
            Mode::Title => {
                let p = self.player.read(cx);
                let live = p.track.as_ref().is_some_and(|t| t.live);
                let main = p
                    .now_playing
                    .clone()
                    .filter(|_| live)
                    .or_else(|| p.title())
                    .unwrap_or_else(|| "Aucune lecture".into());
                let secondary = if live {
                    p.title().filter(|t| *t != main)
                } else {
                    p.track_subtitle()
                };
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .px_2()
                    .text_center()
                    .child(
                        div()
                            .font_semibold()
                            .when(large, |d| d.text_3xl())
                            .when(!large, |d| d.text_2xl())
                            .child(main),
                    )
                    .children(secondary.map(|s| {
                        div()
                            .when(large, |d| d.text_xl())
                            .when(!large, |d| d.text_base())
                            .text_color(theme.muted_foreground)
                            .child(s)
                    }))
                    .into_any_element()
            }
            Mode::Queue => {
                let p = self.player.read(cx);
                if p.queue.is_empty() {
                    return div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("File d'attente vide")
                        .into_any_element();
                }
                let row = |title: String, detail: String, current: bool| {
                    h_flex()
                        .w_full()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded(theme.radius)
                        .when(current, |r| r.bg(theme.accent))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .when(large, |d| d.text_base())
                                        .when(!large, |d| d.text_sm())
                                        .truncate()
                                        .when(current, |d| d.font_semibold())
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .when(large, |d| d.text_sm())
                                        .when(!large, |d| d.text_xs())
                                        .truncate()
                                        .text_color(theme.muted_foreground)
                                        .child(detail),
                                ),
                        )
                };
                let detail = |t: &ondes_library::Track| {
                    let artist = t.artist.clone().unwrap_or_default();
                    match (artist.is_empty(), t.duration.is_zero()) {
                        (false, false) => format!("{artist} · {}", clock(t.duration)),
                        (false, true) => artist,
                        (true, false) => clock(t.duration),
                        (true, true) => String::new(),
                    }
                };
                v_flex()
                    .id("stage-queue")
                    .size_full()
                    .overflow_y_scroll()
                    .gap_0p5()
                    .children(
                        p.current_track()
                            .map(|t| row(t.title.clone(), detail(t), true)),
                    )
                    .children(
                        p.queue
                            .upcoming(100)
                            .map(|t| row(t.title.clone(), detail(t), false)),
                    )
                    .into_any_element()
            }
        }
    }
}
