//! Visualiseurs par shaders : vue réutilisable ([`ShaderView`]) et mode
//! immersif de la fenêtre principale ([`VisualizerView`]), en plein écran
//! au choix.
//!
//! Le rendu se fait sur le thread d'`ondes-visual` ; chaque nouvelle image
//! remplace la précédente (libérée de l'atlas de GPUI).

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ondes_audio::{AnalyzerHold, PlaybackState};
use ondes_visual::{EFFECTS, Visualizer};

use crate::player::Player;
use crate::settings::Settings;

/// Délai avant que les commandes de la fenêtre s'effacent.
const OVERLAY_TIMEOUT: Duration = Duration::from_secs(3);

pub fn effect_name(effect: usize) -> &'static str {
    EFFECTS[effect % EFFECTS.len()].0
}

/// Affiche le rendu d'un effet ; s'adapte à la taille qu'on lui donne.
pub struct ShaderView {
    player: Entity<Player>,
    visualizer: Option<Rc<Visualizer>>,
    image: Option<Arc<RenderImage>>,
    seq: u64,
    effect: usize,
    _hold: Option<AnalyzerHold>,
    /// Redessine au rythme du rendu (30 images/s), pas à chaque image de l'écran.
    _tick: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl ShaderView {
    pub fn new(player: Entity<Player>, cx: &mut Context<Self>) -> Self {
        let effect = Settings::get(cx).visual.effect % EFFECTS.len();
        let analyzer = player.read(cx).analyzer().cloned();
        let (visualizer, hold) = match analyzer {
            Some(analyzer) => {
                let hold = analyzer.hold();
                let visualizer = Visualizer::start(
                    effect,
                    Box::new(move |bands: &mut [f32]| analyzer.spectrum(bands)),
                );
                (Some(Rc::new(visualizer)), Some(hold))
            }
            None => (None, None),
        };
        let subscriptions = vec![
            cx.observe(&player, |_, _, cx| cx.notify()),
            // Image courante rendue à l'atlas quand la vue disparaît.
            cx.on_release(|this: &mut Self, cx| {
                if let Some(image) = this.image.take() {
                    cx.drop_image(image, None);
                }
            }),
        ];
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self {
            player,
            visualizer,
            image: None,
            seq: 0,
            effect,
            _hold: hold,
            _tick: tick,
            _subscriptions: subscriptions,
        }
    }

    pub fn effect(&self) -> usize {
        self.effect
    }

    /// Effet suivant (`step` = 1) ou précédent (`step` = -1), mémorisé.
    pub fn cycle(&mut self, step: isize, cx: &mut Context<Self>) {
        let n = EFFECTS.len() as isize;
        self.effect = (self.effect as isize + step).rem_euclid(n) as usize;
        if let Some(v) = &self.visualizer {
            v.set_effect(self.effect);
        }
        let effect = self.effect;
        Settings::update(cx, |s| s.visual.effect = effect);
        cx.notify();
    }
}

impl Render for ShaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let Some(visualizer) = self.visualizer.clone() else {
            return message("Moteur audio indisponible", theme.muted_foreground);
        };
        if let Some(error) = visualizer.error() {
            return message(&format!("Visualiseur indisponible : {error}"), theme.danger);
        }
        let playing = self.player.read(cx).state == PlaybackState::Playing;
        visualizer.set_playing(playing);

        if let Some(frame) = visualizer.take_frame(self.seq) {
            self.seq = frame.seq;
            // Les pixels sont déjà en BGRA, l'ordre attendu par GPUI.
            if let Some(buffer) =
                image::RgbaImage::from_raw(frame.width, frame.height, frame.pixels)
            {
                let new = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
                if let Some(old) = self.image.replace(new) {
                    window.drop_image(old).ok();
                }
            }
        }

        // La taille réelle n'est connue qu'à la mise en page : un canvas la relève.
        let sizer = canvas(
            move |bounds, window, _| {
                let scale = window.scale_factor();
                visualizer.set_size(
                    (f32::from(bounds.size.width) * scale) as u32,
                    (f32::from(bounds.size.height) * scale) as u32,
                );
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(gpui_kit::black())
            .child(sizer)
            .children(self.image.clone().map(|image| {
                img(ImageSource::Render(image))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
            }))
            .into_any_element()
    }
}

fn message(text: &str, color: Hsla) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .text_sm()
        .text_center()
        .text_color(color)
        .child(text.to_string())
        .into_any_element()
}

/// Mode immersif : l'effet seul, commandes en surimpression qui s'effacent.
pub struct VisualizerView {
    player: Entity<Player>,
    view: Entity<ShaderView>,
    focus: FocusHandle,
    /// Dernier mouvement de souris : les commandes s'affichent un moment.
    last_move: Instant,
    /// Redessine la fenêtre quand les commandes doivent s'effacer.
    _hide: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl VisualizerView {
    pub fn new(player: Entity<Player>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let view = cx.new(|cx| ShaderView::new(player.clone(), cx));
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let subscriptions = vec![cx.observe(&player, |_, _, cx| cx.notify())];
        Self {
            player,
            view,
            focus,
            last_move: Instant::now(),
            _hide: None,
            _subscriptions: subscriptions,
        }
    }

    /// Affiche les commandes, puis les efface après un moment sans mouvement.
    fn show_overlay(&mut self, cx: &mut Context<Self>) {
        self.last_move = Instant::now();
        self._hide = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(OVERLAY_TIMEOUT + Duration::from_millis(50))
                .await;
            this.update(cx, |_, cx| cx.notify()).ok();
        }));
        cx.notify();
    }

    fn cycle(&mut self, step: isize, cx: &mut Context<Self>) {
        self.view.update(cx, |v, cx| v.cycle(step, cx));
        self.show_overlay(cx);
    }
}

impl Render for VisualizerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let effect = self.view.read(cx).effect();
        let show_overlay = self.last_move.elapsed() < OVERLAY_TIMEOUT;
        let fullscreen = window.is_fullscreen();
        let p = self.player.read(cx);
        let title = p.title().unwrap_or_else(|| "Aucune lecture".into());
        let subtitle = if p.track.as_ref().is_some_and(|t| t.live) {
            p.now_playing.clone()
        } else {
            p.track_subtitle()
        };
        let theme = cx.theme();

        let overlay = h_flex()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .p_4()
            .gap_3()
            .items_end()
            .bg(linear_gradient(
                0.,
                linear_color_stop(gpui_kit::black().opacity(0.75), 0.),
                linear_color_stop(gpui_kit::black().opacity(0.), 1.),
            ))
            .text_color(gpui_kit::white())
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_lg().font_semibold().truncate().child(title))
                    .children(subtitle.map(|s| div().text_sm().opacity(0.8).truncate().child(s))),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("prev-effect")
                            .icon(IconName::ChevronLeft)
                            .ghost()
                            .small()
                            .tooltip("Effet précédent (←)")
                            .on_click(cx.listener(|this, _, _, cx| this.cycle(-1, cx))),
                    )
                    .child(
                        div()
                            .w(px(100.))
                            .text_center()
                            .text_sm()
                            .child(effect_name(effect)),
                    )
                    .child(
                        Button::new("next-effect")
                            .icon(IconName::ChevronRight)
                            .ghost()
                            .small()
                            .tooltip("Effet suivant (→)")
                            .on_click(cx.listener(|this, _, _, cx| this.cycle(1, cx))),
                    )
                    .child(
                        Button::new("fullscreen")
                            .icon(if fullscreen {
                                IconName::Minimize2
                            } else {
                                IconName::Fullscreen
                            })
                            .ghost()
                            .small()
                            .tooltip(if fullscreen {
                                "Quitter le plein écran (F11, Échap)"
                            } else {
                                "Plein écran (F11, double-clic)"
                            })
                            .on_click(|_, window, _| window.toggle_fullscreen()),
                    )
                    .child(
                        // Quitte aussi le plein écran.
                        Button::new("leave-immersive")
                            .icon(IconName::ArrowLeft)
                            .label("Vue principale")
                            .ghost()
                            .small()
                            .tooltip_with_action(
                                "Quitter le mode immersif",
                                &crate::actions::ToggleVisualizer,
                                None,
                            )
                            .on_click(|_, window, cx| {
                                window
                                    .dispatch_action(Box::new(crate::actions::ToggleVisualizer), cx)
                            }),
                    ),
            );

        div()
            .id("visualizer")
            .key_context("Visualizer")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .bg(theme.background)
            .on_mouse_move(cx.listener(|this, _, _, cx| this.show_overlay(cx)))
            .on_click(|event, window, _| {
                if event.click_count() >= 2 {
                    window.toggle_fullscreen();
                }
            })
            .on_action(cx.listener(|this, _: &crate::actions::NextEffect, _, cx| this.cycle(1, cx)))
            .on_action(
                cx.listener(|this, _: &crate::actions::PreviousEffect, _, cx| this.cycle(-1, cx)),
            )
            .on_action(|_: &crate::actions::ToggleFullscreen, window, _| window.toggle_fullscreen())
            .on_action(|_: &crate::actions::CloseVisualizer, window, cx| {
                // Échap : quitte d'abord le plein écran, puis le mode immersif.
                if window.is_fullscreen() {
                    window.toggle_fullscreen();
                } else {
                    window.dispatch_action(Box::new(crate::actions::ToggleVisualizer), cx);
                }
            })
            .child(self.view.clone())
            .when(show_overlay, |d| d.child(overlay))
    }
}
