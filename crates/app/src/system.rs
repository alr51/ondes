//! Intégration au système : touches multimédia et panneau « média » du
//! bureau (MPRIS sous Linux, via souvlaki), notifications au changement de
//! morceau quand la fenêtre n'est pas au premier plan.

use std::time::Duration;

use gpui_kit::*;
use ondes_audio::{PlaybackState, SourceSpec};
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};

use crate::player::Player;
use crate::settings::Settings;

const SEEK_STEP: Duration = Duration::from_secs(10);

/// Ce qui est affiché à l'extérieur (MPRIS, notification).
#[derive(Debug, Clone, PartialEq, Default)]
struct NowPlaying {
    title: String,
    artist: Option<String>,
    album: Option<String>,
    cover: Option<String>,
    duration: Option<Duration>,
}

impl NowPlaying {
    fn from_player(p: &Player) -> Option<Self> {
        if let Some(t) = p.current_track() {
            return Some(Self {
                title: t.title.clone(),
                artist: t.artist.clone(),
                album: t.album.clone(),
                cover: t.cover.as_ref().map(|c| c.display().to_string()),
                duration: Some(t.duration).filter(|d| !d.is_zero()),
            });
        }
        let name = p.title()?;
        let live = p.track.as_ref().is_some_and(|t| t.live);
        Some(match (&p.now_playing, live) {
            // Radio : le titre en cours, avec la station comme « artiste ».
            (Some(song), true) => Self {
                title: song.clone(),
                artist: Some(name),
                ..Default::default()
            },
            _ => Self {
                title: name,
                duration: p.track.as_ref().and_then(|t| t.duration),
                ..Default::default()
            },
        })
    }
}

pub struct System {
    player: Entity<Player>,
    controls: Option<MediaControls>,
    shown: Option<NowPlaying>,
    playback: Option<MediaPlayback>,
    notified: Option<NowPlaying>,
    _subscriptions: Vec<Subscription>,
    _events: Option<Task<()>>,
}

impl System {
    pub fn new(player: Entity<Player>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = flume::unbounded::<MediaControlEvent>();
        let hwnd = hwnd(window);
        // Sous Windows, souvlaki panique (`expect`) sans HWND : on s'abstient.
        let controls = if cfg!(target_os = "windows") && hwnd.is_none() {
            None
        } else {
            Some(
                MediaControls::new(PlatformConfig {
                    display_name: "Ondes",
                    dbus_name: "ondes",
                    hwnd,
                })
                .and_then(|mut c| {
                    c.attach(move |event| {
                        let _ = tx.send(event);
                    })?;
                    Ok(c)
                }),
            )
        };
        let (controls, events) = match controls {
            None => {
                tracing::warn!("touches multimédia indisponibles : pas de HWND");
                (None, None)
            }
            Some(Ok(c)) => {
                tracing::info!("touches multimédia actives");
                let task = cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = rx.recv_async().await {
                        if this
                            .update_in(cx, |this, window, cx| this.handle(event, window, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                (Some(c), Some(task))
            }
            Some(Err(e)) => {
                tracing::warn!("touches multimédia / MPRIS indisponibles : {e:?}");
                (None, None)
            }
        };

        let subscriptions =
            vec![cx.observe_in(&player, window, |this, _, window, cx| this.sync(window, cx))];
        Self {
            player,
            controls,
            shown: None,
            playback: None,
            notified: None,
            _subscriptions: subscriptions,
            _events: events,
        }
    }

    fn handle(&mut self, event: MediaControlEvent, window: &mut Window, cx: &mut Context<Self>) {
        tracing::debug!(?event, "commande multimédia");
        match event {
            MediaControlEvent::Raise => return window.activate_window(),
            MediaControlEvent::Quit => return cx.quit(),
            _ => {}
        }
        let active = self.player.read(cx).state.is_active();
        self.player.update(cx, |p, cx| match event {
            MediaControlEvent::Play if !active => p.toggle_play(cx),
            MediaControlEvent::Pause if active => p.toggle_play(cx),
            MediaControlEvent::Toggle => p.toggle_play(cx),
            MediaControlEvent::Next => p.next(cx),
            MediaControlEvent::Previous => p.prev(cx),
            MediaControlEvent::Stop => p.stop(cx),
            MediaControlEvent::Seek(dir) => p.seek_by(signed(dir, SEEK_STEP), cx),
            MediaControlEvent::SeekBy(dir, d) => p.seek_by(signed(dir, d), cx),
            MediaControlEvent::SetPosition(MediaPosition(pos)) => p.seek(pos, cx),
            MediaControlEvent::SetVolume(v) => p.set_volume(v.clamp(0., 1.) as f32, cx),
            MediaControlEvent::OpenUri(uri) => {
                if let Some(spec) = SourceSpec::parse(&uri) {
                    p.load(spec, cx);
                }
            }
            _ => {}
        });
    }

    /// Reporte l'état du lecteur vers MPRIS et les notifications.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.player.read(cx);
        let now = NowPlaying::from_player(p);
        let position = Some(MediaPosition(p.position));
        let playback = match p.state {
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Loading => {
                MediaPlayback::Playing { progress: position }
            }
            PlaybackState::Paused => MediaPlayback::Paused { progress: position },
            PlaybackState::Idle | PlaybackState::Ended => MediaPlayback::Stopped,
        };
        let playing = p.state == PlaybackState::Playing;

        if let Some(controls) = &mut self.controls {
            if now != self.shown {
                let meta = now.clone().unwrap_or_default();
                // `file:///C:/…` sous Windows, échappement des espaces, etc.
                let cover_url = meta
                    .cover
                    .as_ref()
                    .and_then(|c| reqwest::Url::from_file_path(c).ok())
                    .map(String::from);
                let result = controls.set_metadata(MediaMetadata {
                    title: Some(meta.title.as_str()).filter(|t| !t.is_empty()),
                    artist: meta.artist.as_deref(),
                    album: meta.album.as_deref(),
                    cover_url: cover_url.as_deref(),
                    duration: meta.duration,
                });
                if let Err(e) = result {
                    tracing::debug!("MPRIS (métadonnées) : {e:?}");
                }
                self.shown = now.clone();
            }
            // La position n'est renvoyée qu'aux changements d'état : les
            // clients MPRIS l'extrapolent d'eux-mêmes.
            let changed = match (&self.playback, &playback) {
                (Some(a), b) => std::mem::discriminant(a) != std::mem::discriminant(b),
                (None, _) => true,
            };
            if changed {
                if let Err(e) = controls.set_playback(playback.clone()) {
                    tracing::debug!("MPRIS (état) : {e:?}");
                }
                self.playback = Some(playback);
            }
        }

        // Notification : nouveau titre, en lecture, fenêtre en arrière-plan.
        if playing && now.is_some() && now != self.notified {
            self.notified = now.clone();
            let enabled = Settings::get(cx).notifications;
            if enabled
                && !window.is_window_active()
                && let Some(now) = now
            {
                notify(now);
            }
        }
    }
}

/// HWND de la fenêtre principale : SMTC (Windows) s'attache à une fenêtre,
/// et souvlaki panique sans elle. Inutile ailleurs.
#[cfg(target_os = "windows")]
fn hwnd(window: &Window) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn hwnd(_: &Window) -> Option<*mut std::ffi::c_void> {
    None
}

fn signed(dir: SeekDirection, d: Duration) -> i64 {
    let ms = d.as_millis() as i64;
    match dir {
        SeekDirection::Forward => ms,
        SeekDirection::Backward => -ms,
    }
}

/// Notification de bureau (envoyée hors du thread UI : appel D-Bus).
fn notify(now: NowPlaying) {
    std::thread::spawn(move || {
        let body = [now.artist.as_deref(), now.album.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" — ");
        let mut n = notify_rust::Notification::new();
        n.appname("Ondes")
            .summary(&now.title)
            .body(&body)
            .timeout(notify_rust::Timeout::Milliseconds(4_000));
        match &now.cover {
            Some(cover) => n.icon(cover),
            None => n.icon("audio-x-generic"),
        };
        if let Err(e) = n.show() {
            tracing::debug!("notification impossible : {e}");
        }
    });
}
