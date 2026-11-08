//! Fenêtre « Réglages audio » : égaliseur, normalisation ReplayGain,
//! enchaînement des morceaux, spectre, dossier des enregistrements.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Selectable, Sizable, StyledExt, WindowExt as _,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    v_flex,
};
use gpui_kit::*;
use ondes_audio::{EQ_FREQS, EQ_MAX_DB};

use crate::player::Player;
use crate::settings::{AudioSettings, Settings};

/// Préréglages de l'égaliseur (dB par bande, de 31 Hz à 16 kHz).
const PRESETS: &[(&str, [f32; 10])] = &[
    ("Neutre", [0.; 10]),
    ("Graves", [6., 5., 4., 2., 0., 0., 0., 0., 0., 0.]),
    ("Aigus", [0., 0., 0., 0., 0., 1., 2., 4., 5., 6.]),
    ("Voix", [-3., -2., -1., 1., 3., 4., 3., 1., 0., -1.]),
    ("Loudness", [5., 4., 2., 0., -1., -1., 0., 2., 4., 5.]),
];

pub fn open(player: Entity<Player>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| AudioSettingsView::new(player, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        let view = view.clone();
        dialog
            .title("Réglages audio")
            .w(px(620.))
            .content(move |content, _, _| content.child(view.clone()))
    });
}

pub struct AudioSettingsView {
    player: Entity<Player>,
    bands: Vec<Entity<SliderState>>,
    crossfade: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
}

impl AudioSettingsView {
    fn new(player: Entity<Player>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let audio = Settings::get(cx).audio;
        let mut subscriptions = Vec::new();
        let bands: Vec<_> = (0..EQ_FREQS.len())
            .map(|i| {
                let state = cx.new(|_| {
                    SliderState::new()
                        .min(-EQ_MAX_DB)
                        .max(EQ_MAX_DB)
                        .step(0.5)
                        .default_value(audio.eq_gains[i])
                });
                subscriptions.push(cx.subscribe(
                    &state,
                    move |this, _, event: &SliderEvent, cx| {
                        let (SliderEvent::Change(v) | SliderEvent::Release(v)) = event;
                        let gain = v.start();
                        this.update(cx, |a| {
                            a.eq_gains[i] = gain;
                            // Toucher une bande active l'égaliseur.
                            a.eq_enabled = true;
                        });
                    },
                ));
                state
            })
            .collect();
        let crossfade = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(10.)
                .step(0.5)
                .default_value(audio.crossfade_secs)
        });
        subscriptions.push(
            cx.subscribe(&crossfade, |this, _, event: &SliderEvent, cx| {
                let (SliderEvent::Change(v) | SliderEvent::Release(v)) = event;
                let secs = v.start();
                this.update(cx, |a| a.crossfade_secs = secs);
            }),
        );
        subscriptions.push(cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()));
        Self {
            player,
            bands,
            crossfade,
            _subscriptions: subscriptions,
        }
    }

    /// Modifie les réglages, les enregistre et les applique au moteur.
    fn update(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut AudioSettings)) {
        Settings::update(cx, |s| f(&mut s.audio));
        let audio = Settings::get(cx).audio;
        self.player
            .update(cx, |p, _| p.apply_audio_settings(&audio));
        cx.notify();
    }

    fn apply_preset(&mut self, gains: [f32; 10], window: &mut Window, cx: &mut Context<Self>) {
        for (state, &g) in self.bands.iter().zip(&gains) {
            state.update(cx, |s, cx| s.set_value(g, window, cx));
        }
        self.update(cx, |a| {
            a.eq_gains = gains;
            a.eq_enabled = gains.iter().any(|g| *g != 0.);
        });
    }

    fn choose_record_dir(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choisir".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(dir) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| this.update(cx, |a| a.record_dir = Some(dir)))
                    .ok();
            }
        })
        .detach();
    }
}

fn section(title: &'static str, hint: &'static str, cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .gap_0p5()
        .child(div().text_sm().font_semibold().child(title))
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(hint),
        )
}

fn freq_label(f: f32) -> String {
    if f >= 1_000. {
        format!("{}k", f / 1_000.)
    } else {
        format!("{f}")
    }
}

impl Render for AudioSettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let audio = Settings::get(cx).audio;
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let radius = theme.radius;

        // Égaliseur
        let bands = h_flex()
            .gap_1()
            .justify_between()
            .children(self.bands.iter().enumerate().map(|(i, state)| {
                let gain = audio.eq_gains[i];
                v_flex()
                    .w(px(44.))
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .child(
                        div()
                            .text_color(muted)
                            .child(format!("{gain:+.1}").replace("+0.0", "0")),
                    )
                    .child(
                        Slider::new(state)
                            .vertical()
                            .h(px(120.))
                            .accessibility_label(format!("{} Hz", EQ_FREQS[i])),
                    )
                    .child(div().child(freq_label(EQ_FREQS[i])))
            }));
        let presets =
            h_flex()
                .gap_1()
                .flex_wrap()
                .children(PRESETS.iter().map(|&(name, gains)| {
                    Button::new(SharedString::from(format!("preset-{name}")))
                        .label(name)
                        .xsmall()
                        .outline()
                        .selected(audio.eq_enabled && audio.eq_gains == gains)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.apply_preset(gains, window, cx)
                        }))
                }));
        let equalizer = v_flex()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .items_start()
                    .child(section(
                        "Égaliseur",
                        "10 bandes, ±12 dB. Le volume est réduit d'autant que la bande la plus amplifiée, pour ne pas saturer.",
                        cx,
                    ))
                    .child(
                        Switch::new("eq-enabled")
                            .checked(audio.eq_enabled)
                            .accessibility_label("Activer l'égaliseur")
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                let on = *on;
                                this.update(cx, |a| a.eq_enabled = on)
                            })),
                    ),
            )
            .child(
                div()
                    .p_3()
                    .rounded(radius)
                    .border_1()
                    .border_color(border)
                    .opacity(if audio.eq_enabled { 1. } else { 0.6 })
                    .child(bands),
            )
            .child(presets);

        // Normalisation
        let modes = [
            ("off", "Désactivée"),
            ("track", "Par morceau"),
            ("album", "Par album"),
        ];
        let replay_gain = v_flex()
            .gap_2()
            .child(section(
                "Normalisation du volume (ReplayGain)",
                "D'après les tags REPLAYGAIN des fichiers ; sans effet sur les radios et les fichiers non tagués.",
                cx,
            ))
            .child(
                ButtonGroup::new("replay-gain")
                    .outline()
                    .small()
                    .children(modes.iter().map(|&(key, label)| {
                        Button::new(key)
                            .label(label)
                            .selected(audio.replay_gain == key)
                    }))
                    .on_click(cx.listener(move |this, selected: &Vec<usize>, _, cx| {
                        if let Some(&i) = selected.first() {
                            this.update(cx, |a| a.replay_gain = modes[i].0.into());
                        }
                    })),
            );

        // Enchaînement
        let fade_label = if audio.crossfade_secs <= 0. {
            "Désactivé : enchaînement sans blanc".to_string()
        } else {
            format!("{:.1} s", audio.crossfade_secs).replace(".0 s", " s")
        };
        let crossfade = v_flex()
            .gap_2()
            .child(section(
                "Fondu enchaîné",
                "Entre deux morceaux de la bibliothèque. Le morceau suivant est toujours préparé à l'avance.",
                cx,
            ))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(div().w(px(260.)).child(Slider::new(&self.crossfade)))
                    .child(div().text_sm().child(fade_label)),
            )
            .child(
                Switch::new("crossfade-album")
                    .checked(audio.crossfade_same_album)
                    .label("Aussi entre deux morceaux d'un même album")
                    .small()
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.update(cx, |a| a.crossfade_same_album = on)
                    })),
            );

        // Spectre et enregistrements
        let record_dir = audio.record_dir();
        let open_dir = record_dir.clone();
        let other = v_flex()
            .gap_3()
            .child(
                Switch::new("visualizer")
                    .checked(audio.visualizer)
                    .label("Afficher le spectre dans la barre de lecture")
                    .small()
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.update(cx, |a| a.visualizer = on)
                    })),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(section(
                        "Enregistrements des directs",
                        "Le flux est copié tel quel, sans réencodage (Ctrl+Maj+R pendant l'écoute).",
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .child(record_dir.display().to_string()),
                            )
                            .child(
                                Button::new("record-dir")
                                    .label("Changer…")
                                    .outline()
                                    .small()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.choose_record_dir(cx)),
                                    ),
                            )
                            .child(
                                Button::new("open-record-dir")
                                    .icon(IconName::FolderOpen)
                                    .ghost()
                                    .small()
                                    .tooltip("Ouvrir le dossier")
                                    .on_click(move |_, _, cx| {
                                        std::fs::create_dir_all(&open_dir).ok();
                                        cx.open_url(&format!("file://{}", open_dir.display()));
                                    }),
                            ),
                    ),
            );

        v_flex()
            .id("audio-settings")
            .max_h(px(560.))
            .overflow_y_scroll()
            .gap_5()
            .child(equalizer)
            .child(replay_gain)
            .child(crossfade)
            .child(other)
    }
}
