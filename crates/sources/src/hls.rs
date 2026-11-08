//! Flux HLS (HTTP Live Streaming, playlists `.m3u8`).
//!
//! Une playlist maître est résolue vers la variante audio la plus adaptée.
//! La playlist media est relue au rythme des segments tant que le direct
//! dure. Chaque segment est téléchargé en entier puis transmis au lecteur :
//! - MPEG-TS : démultiplexé ([`TsDemuxer`]) en AAC (ADTS) ou MP3 ;
//! - audio brut (`.aac`, `.mp3`) : balises ID3 de tête retirées ;
//! - fMP4 : segment d'initialisation puis fragments, transmis tels quels.

use std::time::Duration;

use bytes::Bytes;
use tokio::runtime::Handle;
use url::Url;

use crate::http::{HttpOptions, HttpReader, StatusFn, StreamStatus, host_of, network_error};
use crate::record::Recorder;
use crate::ts::{TsAudio, TsDemuxer};
use crate::{Opened, SourceError};

const MAX_PLAYLIST_BYTES: usize = 1024 * 1024;
const MAX_SEGMENT_BYTES: usize = 32 * 1024 * 1024;
/// Segments joués en arrière du bord du direct au démarrage (RFC 8216, § 6.3.3).
const LIVE_EDGE_SEGMENTS: usize = 3;
/// Segments téléchargés d'avance (en plus du tampon décodé).
const SEGMENTS_AHEAD: usize = 3;
const MIN_REFRESH: Duration = Duration::from_millis(500);

/// Le corps d'une playlist M3U est-il du HLS ?
pub(crate) fn is_hls(body: &str) -> bool {
    body.contains("#EXT-X-")
}

#[derive(Debug, Clone, PartialEq)]
struct Variant {
    uri: String,
    bandwidth: u64,
    codecs: Option<String>,
    audio_group: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct Rendition {
    group: String,
    uri: String,
    default: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Segment {
    seq: u64,
    uri: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct MediaPlaylist {
    target: Duration,
    segments: Vec<Segment>,
    ended: bool,
    /// Segment d'initialisation (fMP4).
    map: Option<String>,
    /// Méthode de chiffrement autre que `NONE`.
    encryption: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Playlist {
    Master {
        variants: Vec<Variant>,
        renditions: Vec<Rendition>,
    },
    Media(MediaPlaylist),
}

fn parse(body: &str, base: &Url) -> Playlist {
    let resolve = |uri: &str| {
        base.join(uri)
            .map(String::from)
            .unwrap_or_else(|_| uri.to_string())
    };
    let mut variants = Vec::new();
    let mut renditions = Vec::new();
    let mut pending: Option<Variant> = None;
    let mut media = MediaPlaylist {
        target: Duration::from_secs(6),
        ..Default::default()
    };
    let mut sequence = 0u64;

    for line in body.trim_start_matches('\u{feff}').lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        let Some(tag) = line.strip_prefix('#') else {
            if let Some(mut v) = pending.take() {
                v.uri = resolve(line);
                variants.push(v);
            } else {
                media.segments.push(Segment {
                    seq: sequence + media.segments.len() as u64,
                    uri: resolve(line),
                });
            }
            continue;
        };
        let (name, value) = tag.split_once(':').unwrap_or((tag, ""));
        match name {
            "EXT-X-STREAM-INF" => {
                let attrs = attributes(value);
                pending = Some(Variant {
                    uri: String::new(),
                    bandwidth: attr(&attrs, "BANDWIDTH")
                        .and_then(|b| b.parse().ok())
                        .unwrap_or(0),
                    codecs: attr(&attrs, "CODECS").map(str::to_ascii_lowercase),
                    audio_group: attr(&attrs, "AUDIO").map(str::to_string),
                });
            }
            "EXT-X-MEDIA" => {
                let attrs = attributes(value);
                if attr(&attrs, "TYPE") == Some("AUDIO")
                    && let (Some(group), Some(uri)) =
                        (attr(&attrs, "GROUP-ID"), attr(&attrs, "URI"))
                {
                    renditions.push(Rendition {
                        group: group.to_string(),
                        uri: resolve(uri),
                        default: attr(&attrs, "DEFAULT") == Some("YES"),
                    });
                }
            }
            "EXT-X-TARGETDURATION" => {
                if let Ok(secs) = value.trim().parse::<f64>()
                    && secs > 0.0
                {
                    media.target = Duration::from_secs_f64(secs.min(60.0));
                }
            }
            "EXT-X-MEDIA-SEQUENCE" => {
                if media.segments.is_empty() {
                    sequence = value.trim().parse().unwrap_or(0);
                }
            }
            "EXT-X-ENDLIST" => media.ended = true,
            "EXT-X-MAP" => {
                if media.map.is_none() {
                    media.map = attr(&attributes(value), "URI").map(resolve);
                }
            }
            "EXT-X-KEY" => {
                let attrs = attributes(value);
                match attr(&attrs, "METHOD") {
                    Some("NONE") | None => {}
                    Some(method) => media.encryption = Some(method.to_string()),
                }
            }
            _ => {}
        }
    }

    if variants.is_empty() {
        Playlist::Media(media)
    } else {
        Playlist::Master {
            variants,
            renditions,
        }
    }
}

/// Liste d'attributs `CLE=valeur,CLE="valeur, avec virgule"`.
fn attributes(s: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while let Some((key, after)) = rest.split_once('=') {
        let key = key.trim().trim_start_matches(',').trim();
        let (value, next) = if let Some(quoted) = after.strip_prefix('"') {
            match quoted.split_once('"') {
                Some((v, n)) => (v, n),
                None => (quoted, ""),
            }
        } else {
            after.split_once(',').unwrap_or((after, ""))
        };
        out.push((key, value.trim()));
        rest = next.trim_start_matches(',').trim_start();
    }
    out
}

fn attr<'a>(attrs: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// URL de la playlist media à suivre : la meilleure variante lisible, ou
/// la piste audio séparée qu'elle référence.
fn choose(variants: &[Variant], renditions: &[Rendition]) -> Option<String> {
    let rendition = |v: &Variant| {
        let group = v.audio_group.as_deref()?;
        let mut in_group = renditions.iter().filter(|r| r.group == group);
        let first = in_group.clone().next()?;
        Some(in_group.find(|r| r.default).unwrap_or(first).uri.clone())
    };
    let best = variants.iter().max_by_key(|v| {
        let codecs: Vec<&str> = v
            .codecs
            .as_deref()
            .map(|c| c.split(',').map(str::trim).collect())
            .unwrap_or_default();
        let has = |prefixes: &[&str]| {
            codecs
                .iter()
                .any(|c| prefixes.iter().any(|p| c.starts_with(p)))
        };
        let video = has(&["avc", "hvc", "hev", "vp0", "av01", "mp4v"]);
        // AC-3, E-AC-3, Opus, FLAC… : symphonia ne lit que l'AAC et le MP3.
        let unsupported = has(&["ac-3", "ec-3", "mp4a.a5", "mp4a.a6", "opus", "flac", "alac"]);
        // HE-AAC : lu sans SBR, donc plus terne qu'un AAC-LC.
        let he_aac = has(&["mp4a.40.5", "mp4a.40.29"]);
        let audio_only = !video || rendition(v).is_some();
        (!unsupported, audio_only, !he_aac, v.bandwidth)
    })?;
    Some(rendition(best).unwrap_or_else(|| best.uri.clone()))
}

/// Mise en forme des segments pour le décodeur.
enum Unpacker {
    Ts(TsDemuxer),
    /// AAC ou MP3 brut, éventuellement précédé de balises ID3.
    Packed,
    Fmp4,
}

impl Unpacker {
    fn unpack(&mut self, data: Bytes) -> Bytes {
        match self {
            Self::Ts(demux) => demux.feed(&data).into(),
            Self::Packed => {
                let skip = id3_len(&data);
                data.slice(skip..)
            }
            Self::Fmp4 => data,
        }
    }
}

/// Taille des balises ID3v2 en tête de `data`.
fn id3_len(data: &[u8]) -> usize {
    let mut pos = 0;
    while let Some(h) = data.get(pos..pos + 10)
        && &h[..3] == b"ID3"
    {
        let size = h[6..10]
            .iter()
            .fold(0usize, |acc, &b| (acc << 7) | usize::from(b & 0x7f));
        let footer = if h[5] & 0x10 != 0 { 10 } else { 0 };
        pos = (pos + 10 + size + footer).min(data.len());
    }
    pos
}

/// Format d'après le premier segment : de quoi le dépaqueter, et l'extension
/// à donner au décodeur.
fn detect(first: &[u8], has_map: bool) -> Result<(Unpacker, &'static str), SourceError> {
    if has_map {
        return Ok((Unpacker::Fmp4, "m4a"));
    }
    if first.first() == Some(&0x47) && first.get(188).is_none_or(|&b| b == 0x47) {
        let mut demux = TsDemuxer::default();
        demux.feed(first);
        return match demux.audio() {
            Some(TsAudio::Aac) => Ok((Unpacker::Ts(TsDemuxer::default()), "aac")),
            Some(TsAudio::Mpeg) => Ok((Unpacker::Ts(TsDemuxer::default()), "mp3")),
            None => Err(SourceError::Hls(
                "aucune piste audio lisible (AAC ou MP3) dans les segments".into(),
            )),
        };
    }
    let audio = &first[id3_len(first)..];
    match audio {
        // Synchronisation ADTS (couche 0) : AAC.
        [0xff, b, ..] if b & 0xf6 == 0xf0 => Ok((Unpacker::Packed, "aac")),
        // Synchronisation MPEG audio : MP3.
        [0xff, b, ..] if b & 0xe0 == 0xe0 => Ok((Unpacker::Packed, "mp3")),
        _ => Err(SourceError::Hls("format des segments non reconnu".into())),
    }
}

async fn fetch(client: &reqwest::Client, url: &str, limit: usize) -> Result<Bytes, SourceError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| network_error(&e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(SourceError::HttpStatus(status.as_u16()));
    }
    if resp.content_length().is_some_and(|n| n > limit as u64) {
        return Err(SourceError::Hls("réponse trop volumineuse".into()));
    }
    let body = resp.bytes().await.map_err(|e| network_error(&e))?;
    if body.len() > limit {
        return Err(SourceError::Hls("réponse trop volumineuse".into()));
    }
    Ok(body)
}

async fn fetch_media(client: &reqwest::Client, url: &str) -> Result<MediaPlaylist, SourceError> {
    let body = fetch(client, url, MAX_PLAYLIST_BYTES).await?;
    let base = Url::parse(url).map_err(|e| SourceError::InvalidUrl(e.to_string()))?;
    match parse(&String::from_utf8_lossy(&body), &base) {
        Playlist::Media(media) => Ok(media),
        Playlist::Master { .. } => Err(SourceError::Hls("playlists maîtres imbriquées".into())),
    }
}

/// Ouvre un flux HLS dont la playlist (`body`, servie par `url`) est déjà
/// téléchargée. Fonction **bloquante**, comme [`crate::open_stream`].
pub(crate) fn open(
    handle: &Handle,
    client: reqwest::Client,
    url: String,
    body: &str,
    status: StatusFn,
) -> Result<Opened, SourceError> {
    let base = Url::parse(&url).map_err(|e| SourceError::InvalidUrl(e.to_string()))?;
    let start = handle.block_on(async {
        let (media_url, media) = match parse(body, &base) {
            Playlist::Media(media) => (url.clone(), media),
            Playlist::Master {
                variants,
                renditions,
            } => {
                let media_url = choose(&variants, &renditions).ok_or(SourceError::EmptyPlaylist)?;
                tracing::debug!(%media_url, "variante HLS choisie");
                let media = fetch_media(&client, &media_url).await?;
                (media_url, media)
            }
        };
        if let Some(method) = &media.encryption {
            return Err(SourceError::Hls(format!(
                "flux chiffré ({method}) non pris en charge"
            )));
        }
        let first = if media.ended {
            0
        } else {
            media.segments.len().saturating_sub(LIVE_EDGE_SEGMENTS)
        };
        let segment = media
            .segments
            .get(first)
            .ok_or(SourceError::EmptyPlaylist)?;
        let init = match &media.map {
            Some(map) => Some(fetch(&client, map, MAX_SEGMENT_BYTES).await?),
            None => None,
        };
        let data = fetch(&client, &segment.uri, MAX_SEGMENT_BYTES).await?;
        let (mut unpacker, hint) = detect(&data, init.is_some())?;
        let init = init.map(|i| i.to_vec()).unwrap_or_default();
        let mut head = init.clone();
        head.extend_from_slice(&unpacker.unpack(data));
        let next_seq = segment.seq + 1;
        Ok::<_, SourceError>((
            media_url,
            media,
            next_seq,
            unpacker,
            hint,
            init,
            Bytes::from(head),
        ))
    })?;
    let (media_url, media, next_seq, unpacker, hint, init, head) = start;

    tracing::info!(%media_url, hint, ended = media.ended, "flux HLS ouvert");
    let title = host_of(&url).unwrap_or_else(|| url.clone());
    let (tx, rx) = flume::bounded(SEGMENTS_AHEAD);
    let pump = Pump {
        client,
        url: media_url,
        unpacker,
        tx,
        status,
        max_retries: HttpOptions::default().max_retries,
    };
    let task = handle.spawn(pump.run(media, next_seq, head));

    // fMP4 : un enregistrement commencé en cours de route a besoin de l'initialisation.
    let recorder = Recorder::default();
    recorder.set_header(init);
    let mut reader = HttpReader::new(rx, task.abort_handle());
    reader.recorder = Some(recorder.clone());
    Ok(Opened {
        reader: Box::new(reader),
        title,
        hint: Some(hint.to_string()),
        mime: None,
        byte_len: None,
        // Même un HLS fini (ENDLIST) est lu sans déplacement possible.
        live: true,
        replay_gain: None,
        recorder: Some(recorder),
    })
}

struct Pump {
    client: reqwest::Client,
    url: String,
    unpacker: Unpacker,
    tx: flume::Sender<Bytes>,
    status: StatusFn,
    max_retries: u32,
}

impl Pump {
    /// Télécharge les segments au fil de la playlist, relue tant que le direct dure.
    async fn run(mut self, mut playlist: MediaPlaylist, mut next_seq: u64, head: Bytes) {
        if self.tx.send_async(head).await.is_err() {
            return;
        }
        let mut failures = 0u32;
        loop {
            let mut progressed = false;
            let start = next_seq;
            for segment in playlist.segments.iter().filter(|s| s.seq >= start) {
                next_seq = segment.seq + 1;
                progressed = true;
                match fetch(&self.client, &segment.uri, MAX_SEGMENT_BYTES).await {
                    Ok(data) => {
                        if failures > 0 {
                            (self.status)(StreamStatus::Reconnected);
                            failures = 0;
                        }
                        let audio = self.unpacker.unpack(data);
                        if self.tx.send_async(audio).await.is_err() {
                            return;
                        }
                    }
                    // Segment perdu : on passe au suivant.
                    Err(e) => {
                        if !self.failed(&mut failures, &e).await {
                            return;
                        }
                    }
                }
            }
            if playlist.ended {
                return;
            }

            // Nouveau segment attendu dans une durée cible ; moitié moins si
            // la playlist n'avait rien de neuf (RFC 8216, § 6.3.4).
            let wait = if progressed {
                playlist.target
            } else {
                playlist.target / 2
            };
            tokio::time::sleep(wait.max(MIN_REFRESH)).await;
            if self.tx.is_disconnected() {
                return;
            }
            match fetch_media(&self.client, &self.url).await {
                Ok(fresh) => {
                    if let Some(last) = fresh.segments.last()
                        && last.seq + 1 + (LIVE_EDGE_SEGMENTS as u64) < next_seq
                    {
                        // Numérotation repartie de zéro (redémarrage du serveur).
                        tracing::info!(url = %self.url, "numérotation HLS réinitialisée");
                        next_seq = fresh.segments
                            [fresh.segments.len().saturating_sub(LIVE_EDGE_SEGMENTS)]
                        .seq;
                    } else if let Some(first) = fresh.segments.first()
                        && first.seq > next_seq
                    {
                        tracing::warn!(lost = first.seq - next_seq, "segments HLS manqués");
                    }
                    playlist = fresh;
                }
                Err(e) => {
                    if !self.failed(&mut failures, &e).await {
                        return;
                    }
                }
            }
        }
    }

    /// Compte un échec ; `false` s'il faut abandonner.
    async fn failed(&self, failures: &mut u32, error: &SourceError) -> bool {
        *failures += 1;
        if *failures > self.max_retries {
            tracing::warn!(url = %self.url, %error, "abandon du flux HLS");
            (self.status)(StreamStatus::Failed(error.to_string()));
            return false;
        }
        tracing::info!(url = %self.url, attempt = *failures, %error, "erreur HLS");
        (self.status)(StreamStatus::Reconnecting { attempt: *failures });
        tokio::time::sleep(crate::http::backoff(*failures)).await;
        !self.tx.is_disconnected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://radio.example/live/master.m3u8").unwrap()
    }

    #[test]
    fn parses_attributes() {
        let a = attributes(r#"BANDWIDTH=128000,CODECS="mp4a.40.2,avc1.4d401f",AUDIO="aac""#);
        assert_eq!(attr(&a, "BANDWIDTH"), Some("128000"));
        assert_eq!(attr(&a, "CODECS"), Some("mp4a.40.2,avc1.4d401f"));
        assert_eq!(attr(&a, "AUDIO"), Some("aac"));
        assert_eq!(attr(&a, "URI"), None);
    }

    #[test]
    fn chooses_best_audio_variant() {
        let body = "#EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS=\"mp4a.40.5\"\nhe.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=2000000,CODECS=\"avc1.4d401f,mp4a.40.2\"\nvideo.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\n/hifi/lc.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=640000,CODECS=\"ec-3\"\nec3.m3u8\n";
        let Playlist::Master {
            variants,
            renditions,
        } = parse(body, &base())
        else {
            panic!("playlist maître attendue");
        };
        assert_eq!(variants.len(), 4);
        assert_eq!(
            choose(&variants, &renditions).as_deref(),
            Some("https://radio.example/hifi/lc.m3u8")
        );
    }

    #[test]
    fn prefers_separate_audio_rendition() {
        let body = "#EXTM3U\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"fr\",DEFAULT=NO,URI=\"fr.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"main\",DEFAULT=YES,URI=\"main.m3u8\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=900000,CODECS=\"avc1.64001f,mp4a.40.2\",AUDIO=\"a\"\nv.m3u8\n";
        let Playlist::Master {
            variants,
            renditions,
        } = parse(body, &base())
        else {
            panic!("playlist maître attendue");
        };
        assert_eq!(
            choose(&variants, &renditions).as_deref(),
            Some("https://radio.example/live/main.m3u8")
        );
    }

    #[test]
    fn parses_media_playlist() {
        let body = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:4\n\
            #EXT-X-MEDIA-SEQUENCE:1200\n#EXT-X-MAP:URI=\"init.mp4\"\n\
            #EXT-X-KEY:METHOD=NONE\n\
            #EXTINF:4.0,\nseg1200.m4s\n#EXTINF:4.0,\nhttps://cdn.example/seg1201.m4s\n";
        let Playlist::Media(media) = parse(body, &base()) else {
            panic!("playlist media attendue");
        };
        assert_eq!(media.target, Duration::from_secs(4));
        assert!(!media.ended);
        assert_eq!(media.encryption, None);
        assert_eq!(
            media.map.as_deref(),
            Some("https://radio.example/live/init.mp4")
        );
        assert_eq!(
            media.segments,
            vec![
                Segment {
                    seq: 1200,
                    uri: "https://radio.example/live/seg1200.m4s".into()
                },
                Segment {
                    seq: 1201,
                    uri: "https://cdn.example/seg1201.m4s".into()
                },
            ]
        );
        let Playlist::Media(media) = parse(
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:6,\na.ts\n#EXT-X-ENDLIST\n",
            &base(),
        ) else {
            panic!();
        };
        assert!(media.ended);
        assert_eq!(media.encryption.as_deref(), Some("AES-128"));
    }

    #[test]
    fn detects_segment_format() {
        let ts = crate::ts::tests::segment(0x0f, &[0xff, 0xf1, 1, 2]);
        assert_eq!(detect(&ts, false).unwrap().1, "aac");
        assert!(detect(&crate::ts::tests::segment(0x81, b"x"), false).is_err());

        let mut packed = b"ID3\x04\x00\x00\x00\x00\x00\x02xx".to_vec();
        packed.extend([0xff, 0xf1, 0x50, 0x80]);
        let (mut unpacker, hint) = detect(&packed, false).unwrap();
        assert_eq!(hint, "aac");
        assert_eq!(
            &unpacker.unpack(Bytes::from(packed))[..],
            &[0xff, 0xf1, 0x50, 0x80]
        );
        assert_eq!(detect(&[0xff, 0xfb, 0x90, 0], false).unwrap().1, "mp3");
        assert_eq!(detect(b"styp", true).unwrap().1, "m4a");
        assert!(detect(b"<html>", false).is_err());
    }
}
