//! Page « En cours » de la fenêtre principale : pochette, spectre,
//! visualiseur, titre ou file d'attente, en grand.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::*;

use crate::player::Player;
use crate::stage::{Mode, Place, Stage, mode_bar};

pub struct NowPlaying {
    player: Entity<Player>,
    stage: Entity<Stage>,
    _subscriptions: Vec<Subscription>,
}

impl NowPlaying {
    pub fn new(player: Entity<Player>, cx: &mut Context<Self>) -> Self {
        let stage = cx.new(|cx| Stage::new(player.clone(), Place::Page, cx));
        let subscriptions = vec![
            cx.observe(&player, |_, _, cx| cx.notify()),
            cx.observe(&stage, |_, _, cx| cx.notify()),
        ];
        Self {
            player,
            stage,
            _subscriptions: subscriptions,
        }
    }

    /// Page quittée : le visualiseur s'arrête.
    pub fn suspend(&mut self, cx: &mut Context<Self>) {
        self.stage.update(cx, |s, _| s.suspend());
    }
}

impl Render for NowPlaying {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let p = self.player.read(cx);
        let title = p.title();
        let subtitle = if p.track.as_ref().is_some_and(|t| t.live) {
            p.now_playing.clone().map(|t| format!("♪ {t}"))
        } else {
            p.track_subtitle()
        };
        // Le mode « Titre » affiche déjà tout en grand.
        let caption = (self.stage.read(cx).mode() != Mode::Title)
            .then(|| title.map(|t| (t, subtitle)))
            .flatten();

        let toolbar = h_flex()
            .gap_2()
            .items_center()
            .child(mode_bar(&self.stage, cx))
            .child(div().flex_1())
            .child(
                Button::new("open-visualizer")
                    .icon(IconName::Fullscreen)
                    .label("Visualiseur plein écran")
                    .ghost()
                    .small()
                    .tooltip("Ctrl+Maj+V")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::ToggleVisualizer), cx)
                    }),
            )
            .child(
                Button::new("open-compact")
                    .icon(IconName::PictureInPicture2)
                    .label("Vue compacte")
                    .ghost()
                    .small()
                    .tooltip("Ctrl+Maj+M")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::ToggleCompact), cx)
                    }),
            );

        v_flex()
            .size_full()
            .min_h_0()
            .gap_4()
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(self.stage.clone()))
            .children(caption.map(|(title, subtitle)| {
                v_flex()
                    .items_center()
                    .gap_1()
                    .text_center()
                    .child(div().text_xl().font_semibold().child(title))
                    .children(subtitle.map(|s| {
                        div()
                            .text_base()
                            .text_color(theme.muted_foreground)
                            .child(s)
                    }))
            }))
    }
}
