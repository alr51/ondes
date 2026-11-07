//! Modèle du lecteur côté UI : relaie les commandes au moteur audio et
//! reflète ses événements dans une entité GPUI observable.

use std::time::Duration;

use gpui_kit::*;
use ondes_audio::{AudioEngine, Command, Event, PlaybackState, SourceSpec, TrackInfo};

use crate::runtime::Rt;

pub struct Player {
    engine: Option<AudioEngine>,
    pub state: PlaybackState,
    pub track: Option<TrackInfo>,
    /// Libellé affiché pendant l'ouverture d'une nouvelle source.
    pub pending: Option<String>,
    pub position: Duration,
    pub error: Option<String>,
    pub reconnecting: Option<u32>,
    volume: f32,
    _events: Task<()>,
}

impl Player {
    pub fn new(volume: f32, cx: &mut Context<Self>) -> Self {
        let (engine, error) = match AudioEngine::start(Rt::handle(cx)) {
            Ok(engine) => (Some(engine), None),
            Err(e) => {
                tracing::error!("moteur audio indisponible : {e}");
                (None, Some(e.to_string()))
            }
        };

        let events_task = match &engine {
            Some(engine) => {
                engine.send(Command::SetVolume(volume));
                let events = engine.events();
                cx.spawn(async move |this, cx| {
                    while let Ok(event) = events.recv_async().await {
                        if this
                            .update(cx, |this, cx| {
                                this.apply(event);
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
            }
            None => Task::ready(()),
        };

        Self {
            engine,
            state: PlaybackState::Idle,
            track: None,
            pending: None,
            position: Duration::ZERO,
            error,
            reconnecting: None,
            volume,
            _events: events_task,
        }
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::State(state) => {
                self.state = state;
                match state {
                    PlaybackState::Loading => self.error = None,
                    PlaybackState::Playing | PlaybackState::Idle => self.reconnecting = None,
                    _ => {}
                }
            }
            Event::Track(track) => {
                self.track = Some(track);
                self.pending = None;
                self.position = Duration::ZERO;
            }
            Event::Position(pos) => self.position = pos,
            Event::Reconnecting { attempt } => self.reconnecting = Some(attempt),
            Event::Error(message) => {
                self.error = Some(message);
                self.pending = None;
            }
        }
    }

    fn send(&self, cmd: Command) {
        if let Some(engine) = &self.engine {
            engine.send(cmd);
        }
    }

    pub fn load(&mut self, spec: SourceSpec, cx: &mut Context<Self>) {
        self.pending = Some(spec.label());
        self.track = None;
        self.error = None;
        self.position = Duration::ZERO;
        self.send(Command::Load(spec));
        cx.notify();
    }

    pub fn toggle_play(&mut self, _: &mut Context<Self>) {
        self.send(Command::TogglePlay);
    }

    pub fn stop(&mut self, _: &mut Context<Self>) {
        self.send(Command::Stop);
    }

    pub fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        self.volume = volume;
        self.send(Command::SetVolume(volume));
        cx.notify();
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Y a-t-il quelque chose à lire ou à arrêter ?
    pub fn has_source(&self) -> bool {
        self.engine.is_some() && (self.track.is_some() || self.pending.is_some())
    }
}
