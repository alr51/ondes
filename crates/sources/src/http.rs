//! Flux audio HTTP (webradios).
//!
//! La connexion se fait sur le runtime tokio ; les octets sont transmis par
//! un canal borné à un lecteur bloquant ([`HttpReader`]) que le décodeur
//! consomme depuis son propre thread. Un flux en direct (sans
//! `Content-Length`) est reconnecté automatiquement s'il se coupe.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

use bytes::{Buf, Bytes};
use futures_util::StreamExt;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap};
use tokio::runtime::Handle;

use crate::hls;
use crate::icy::IcyDemuxer;
use crate::playlist::{PlaylistKind, first_stream_url};
use crate::{Opened, SourceError};

/// Réglages de connexion.
#[derive(Debug, Clone)]
pub struct HttpOptions {
    /// Nombre de reconnexions successives sans recevoir de données avant d'abandonner.
    pub max_retries: u32,
    /// Capacité du tampon réseau, en blocs reçus.
    pub buffer_chunks: usize,
    pub connect_timeout: Duration,
    /// Délai sans données au-delà duquel on considère le flux coupé.
    pub read_timeout: Duration,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            max_retries: 5,
            buffer_chunks: 256,
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(15),
        }
    }
}

/// État du flux, signalé pendant la lecture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamStatus {
    /// Reconnecté après une coupure.
    Reconnected,
    /// Coupure : tentative de reconnexion n° `attempt`.
    Reconnecting { attempt: u32 },
    /// Abandon : le lecteur va atteindre la fin du flux.
    Failed(String),
    /// Nouveau titre annoncé par le flux (métadonnées ICY).
    Title(String),
}

pub(crate) type StatusFn = Arc<dyn Fn(StreamStatus) + Send + Sync>;

const MAX_PLAYLIST_DEPTH: usize = 3;
const MAX_PLAYLIST_BYTES: usize = 256 * 1024;

/// Ouvre un flux HTTP(S). Fonction **bloquante** : à appeler depuis un
/// thread ordinaire, jamais depuis un thread du runtime tokio.
///
/// Les playlists `.m3u` / `.pls` sont résolues vers leur premier flux ; les
/// playlists HLS (`.m3u8`) sont lues segment par segment.
pub fn open_stream(
    handle: &Handle,
    url: &str,
    opts: HttpOptions,
    on_status: impl Fn(StreamStatus) + Send + Sync + 'static,
) -> Result<Opened, SourceError> {
    url::Url::parse(url).map_err(|e| SourceError::InvalidUrl(format!("{url} ({e})")))?;

    let client = reqwest::Client::builder()
        .user_agent(concat!("Ondes/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(opts.connect_timeout)
        .read_timeout(opts.read_timeout)
        .build()
        .map_err(|e| SourceError::Network(e.to_string()))?;

    let status: StatusFn = Arc::new(on_status);
    let (resp, stream_url) = match handle.block_on(connect(&client, url))? {
        Target::Direct(resp, stream_url) => (resp, stream_url),
        Target::Hls { url, body } => return hls::open(handle, client, url, &body, status),
    };

    let headers = resp.headers();
    let mime = mime_of(headers);
    let byte_len = headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let live = byte_len.is_none();
    let title = header_text(headers, "icy-name")
        .or_else(|| host_of(&stream_url))
        .unwrap_or_else(|| stream_url.clone());
    let hint = mime
        .as_deref()
        .and_then(hint_from_mime)
        .map(str::to_string)
        .or_else(|| hint_from_url(&stream_url));

    tracing::info!(%stream_url, ?mime, live, "flux ouvert");

    let (tx, rx) = flume::bounded(opts.buffer_chunks.max(1));
    let task = handle.spawn(pump(client, stream_url, resp, tx, live, opts, status));

    Ok(Opened {
        reader: Box::new(HttpReader::new(rx, task.abort_handle())),
        title,
        hint,
        mime,
        byte_len,
        live,
    })
}

/// Résultat de la connexion.
enum Target {
    /// Flux audio continu, et son URL finale.
    Direct(reqwest::Response, String),
    /// Playlist HLS, déjà téléchargée.
    Hls { url: String, body: String },
}

/// Requête GET, en suivant les playlists jusqu'au vrai flux audio.
async fn connect(client: &reqwest::Client, url: &str) -> Result<Target, SourceError> {
    let mut url = url.to_string();
    for _ in 0..MAX_PLAYLIST_DEPTH {
        let resp = client
            .get(&url)
            // Demande les titres en cours ; retirés du flux par `IcyDemuxer`.
            .header("Icy-MetaData", "1")
            .send()
            .await
            .map_err(|e| network_error(&e))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(SourceError::HttpStatus(status.as_u16()));
        }
        // URL finale après redirections HTTP.
        let final_url = resp.url().to_string();
        let mime = mime_of(resp.headers());

        if let Some(kind) = PlaylistKind::detect(mime.as_deref(), &final_url) {
            let body = read_limited(resp, MAX_PLAYLIST_BYTES).await?;
            let body = String::from_utf8_lossy(&body);
            if kind == PlaylistKind::M3u && hls::is_hls(&body) {
                return Ok(Target::Hls {
                    url: final_url,
                    body: body.into_owned(),
                });
            }
            url = first_stream_url(kind, &body)?;
            tracing::debug!(%url, "playlist résolue");
            continue;
        }

        return match mime.as_deref() {
            None => Ok(Target::Direct(resp, final_url)),
            Some(m) if is_audio_mime(m) => Ok(Target::Direct(resp, final_url)),
            Some(m) => Err(SourceError::NotAudio(m.to_string())),
        };
    }
    Err(SourceError::PlaylistLoop)
}

async fn read_limited(resp: reqwest::Response, limit: usize) -> Result<Vec<u8>, SourceError> {
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| network_error(&e))?;
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(SourceError::EmptyPlaylist);
        }
    }
    Ok(body)
}

/// Transfère le corps de la réponse vers le canal, et reconnecte les flux en direct.
async fn pump(
    client: reqwest::Client,
    url: String,
    first: reqwest::Response,
    tx: flume::Sender<Bytes>,
    live: bool,
    opts: HttpOptions,
    status: StatusFn,
) {
    let mut resp = Some(first);
    let mut attempt = 0u32;
    loop {
        let mut last_error = None;
        if let Some(r) = resp.take() {
            match forward(r, &tx, &status).await {
                Forward::ReceiverGone => return,
                Forward::Ended { received, error } => {
                    if received {
                        attempt = 0;
                    }
                    last_error = error;
                }
            }
            if !live {
                // Fichier distant : la fin du corps est la fin du morceau.
                if let Some(err) = last_error {
                    status(StreamStatus::Failed(err));
                }
                return;
            }
        }

        attempt += 1;
        if attempt > opts.max_retries {
            let reason = last_error.unwrap_or_else(|| "le flux ne répond plus".to_string());
            tracing::warn!(%url, %reason, "abandon du flux");
            status(StreamStatus::Failed(reason));
            return;
        }
        tracing::info!(%url, attempt, ?last_error, "reconnexion");
        status(StreamStatus::Reconnecting { attempt });
        tokio::time::sleep(backoff(attempt)).await;
        if tx.is_disconnected() {
            return;
        }
        match connect(&client, &url).await {
            Ok(Target::Direct(r, _)) => {
                status(StreamStatus::Reconnected);
                resp = Some(r);
            }
            Ok(Target::Hls { .. }) => {
                tracing::debug!(%url, "le flux est devenu une playlist HLS");
            }
            Err(e) => tracing::debug!(%url, error = %e, "échec de reconnexion"),
        }
    }
}

enum Forward {
    ReceiverGone,
    Ended {
        received: bool,
        error: Option<String>,
    },
}

async fn forward(resp: reqwest::Response, tx: &flume::Sender<Bytes>, status: &StatusFn) -> Forward {
    let mut icy = resp
        .headers()
        .get("icy-metaint")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .map(IcyDemuxer::new);
    let mut stream = resp.bytes_stream();
    let mut received = false;
    loop {
        match stream.next().await {
            Some(Ok(chunk)) => {
                if chunk.is_empty() {
                    continue;
                }
                received = true;
                let audio = match &mut icy {
                    Some(demux) => {
                        let out = demux.feed(chunk);
                        if let Some(title) = out.title {
                            status(StreamStatus::Title(title));
                        }
                        out.audio
                    }
                    None => vec![chunk],
                };
                for piece in audio {
                    if tx.send_async(piece).await.is_err() {
                        return Forward::ReceiverGone;
                    }
                }
            }
            Some(Err(e)) => {
                return Forward::Ended {
                    received,
                    error: Some(e.to_string()),
                };
            }
            None => {
                return Forward::Ended {
                    received,
                    error: None,
                };
            }
        }
    }
}

/// Erreur réseau en français, d'après sa cause (DNS, délai, TLS…).
pub(crate) fn network_error(e: &reqwest::Error) -> SourceError {
    // Le détail utile est souvent au fond de la chaîne des causes.
    let mut chain = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        chain.push_str(" : ");
        chain.push_str(&cause.to_string());
        source = cause.source();
    }
    SourceError::Network(describe_network_error(
        &chain,
        e.is_timeout(),
        e.is_connect(),
    ))
}

fn describe_network_error(chain: &str, timeout: bool, connect: bool) -> String {
    let lower = chain.to_lowercase();
    if timeout {
        "le serveur ne répond pas (délai dépassé)".into()
    } else if lower.contains("dns")
        || lower.contains("lookup address")
        || lower.contains("name or service")
    {
        "serveur introuvable : adresse inconnue ou pas de connexion Internet".into()
    } else if lower.contains("certificate") || lower.contains("tls") || lower.contains("handshake")
    {
        "connexion sécurisée impossible (certificat du serveur invalide)".into()
    } else if lower.contains("refused") {
        "connexion refusée : la station est sans doute hors ligne".into()
    } else if connect {
        "connexion impossible au serveur de la station".into()
    } else {
        chain.to_string()
    }
}

pub(crate) fn backoff(attempt: u32) -> Duration {
    let ms = 500u64.saturating_mul(1 << attempt.saturating_sub(1).min(4));
    Duration::from_millis(ms.min(8_000))
}

/// Lecteur bloquant alimenté par la tâche réseau.
pub(crate) struct HttpReader {
    rx: flume::Receiver<Bytes>,
    current: Bytes,
    pos: u64,
    task: tokio::task::AbortHandle,
}

impl HttpReader {
    pub(crate) fn new(rx: flume::Receiver<Bytes>, task: tokio::task::AbortHandle) -> Self {
        Self {
            rx,
            current: Bytes::new(),
            pos: 0,
            task,
        }
    }
}

impl Read for HttpReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        while self.current.is_empty() {
            match self.rx.recv() {
                Ok(chunk) => self.current = chunk,
                // Tâche terminée : fin du flux.
                Err(flume::RecvError::Disconnected) => return Ok(0),
            }
        }
        let n = buf.len().min(self.current.len());
        buf[..n].copy_from_slice(&self.current[..n]);
        self.current.advance(n);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match pos {
            SeekFrom::Current(0) => Ok(self.pos),
            SeekFrom::Start(p) if p == self.pos => Ok(self.pos),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "un flux réseau ne permet pas le déplacement",
            )),
        }
    }
}

impl Drop for HttpReader {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn mime_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or(v).trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
}

/// En-tête texte ; les serveurs Icecast envoient parfois du Latin-1.
fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?;
    let text = match value.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => value.as_bytes().iter().map(|&b| b as char).collect(),
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn is_audio_mime(mime: &str) -> bool {
    mime.starts_with("audio/")
        || matches!(
            mime,
            "application/ogg" | "application/octet-stream" | "video/mp2t"
        )
}

fn hint_from_mime(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "audio/mpeg" | "audio/mp3" | "audio/mpeg3" | "audio/x-mpeg" => "mp3",
        "audio/aac" | "audio/aacp" | "audio/x-aac" => "aac",
        "audio/ogg" | "application/ogg" | "audio/vorbis" | "audio/opus" => "ogg",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        _ => return None,
    })
}

fn hint_from_url(url: &str) -> Option<String> {
    let path = url::Url::parse(url).ok()?.path().to_ascii_lowercase();
    let ext = path.rsplit_once('.')?.1;
    matches!(ext, "mp3" | "aac" | "ogg" | "oga" | "flac" | "m4a" | "wav").then(|| ext.to_string())
}

pub(crate) fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(|h| h.trim_start_matches("www.").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn network_messages() {
        assert!(describe_network_error("x", true, true).contains("délai"));
        assert!(
            describe_network_error(
                "error sending request : dns error : failed to lookup address",
                false,
                true
            )
            .contains("introuvable")
        );
        assert!(
            describe_network_error("invalid peer certificate", false, true).contains("certificat")
        );
        assert!(
            describe_network_error("Connection refused (os error 111)", false, true)
                .contains("hors ligne")
        );
    }

    #[test]
    fn unreachable_server_has_clear_message() {
        // Port local fermé : connexion refusée.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let err =
            open_stream(rt.handle(), "http://127.0.0.1:9/flux", fast_opts(), |_| {}).unwrap_err();
        assert!(err.to_string().contains("hors ligne"), "{err}");
    }

    #[test]
    fn mime_hints() {
        assert_eq!(hint_from_mime("audio/aacp"), Some("aac"));
        assert_eq!(hint_from_mime("text/html"), None);
        assert_eq!(
            hint_from_url("http://x/live/stream.MP3?x=1").as_deref(),
            Some("mp3")
        );
        assert_eq!(hint_from_url("http://x/live"), None);
        assert_eq!(
            host_of("https://www.radio.example:8000/a").as_deref(),
            Some("radio.example")
        );
        assert_eq!(backoff(1), Duration::from_millis(500));
        assert_eq!(backoff(10), Duration::from_millis(8_000));
    }

    /// Mini serveur HTTP : la connexion n° i reçoit la réponse n° i.
    /// `build` reçoit l'adresse du serveur pour pouvoir y faire référence.
    fn serve(build: impl FnOnce(&str) -> Vec<Vec<u8>>) -> (tokio::runtime::Runtime, String) {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let responses = build(&base);
        rt.spawn(async move {
            for i in 0.. {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let resp = responses.get(i).cloned();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = sock.read(&mut buf).await;
                    if let Some(resp) = resp {
                        let _ = sock.write_all(&resp).await;
                    }
                    let _ = sock.shutdown().await;
                });
            }
        });
        (rt, base)
    }

    fn response(headers: &str, body: &[u8]) -> Vec<u8> {
        let mut r = format!("HTTP/1.1 200 OK\r\nConnection: close\r\n{headers}\r\n").into_bytes();
        r.extend_from_slice(body);
        r
    }

    fn fast_opts() -> HttpOptions {
        HttpOptions {
            max_retries: 2,
            ..Default::default()
        }
    }

    #[test]
    fn reads_finite_stream() {
        let body = vec![7u8; 10_000];
        let resp = response(
            "Content-Type: audio/mpeg\r\nContent-Length: 10000\r\nicy-name: Radio Test\r\n",
            &body,
        );
        let (rt, base) = serve(move |_| vec![resp]);
        let mut opened =
            open_stream(rt.handle(), &format!("{base}/s"), fast_opts(), |_| {}).unwrap();
        assert_eq!(opened.title, "Radio Test");
        assert_eq!(opened.hint.as_deref(), Some("mp3"));
        assert!(!opened.live);
        let mut got = Vec::new();
        opened.reader.read_to_end(&mut got).unwrap();
        assert_eq!(got, body);
        assert_eq!(opened.reader.stream_position().unwrap(), 10_000);
        assert!(opened.reader.seek(SeekFrom::Start(0)).is_err());
    }

    #[test]
    fn resolves_pls_then_reconnects_live_stream() {
        let (rt, base) = serve(|base| {
            vec![
                response(
                    "Content-Type: audio/x-scpls\r\n",
                    format!("[playlist]\nFile1={base}/live\n").as_bytes(),
                ),
                // Flux en direct (pas de Content-Length) qui se coupe…
                response("Content-Type: audio/aacp\r\n", b"part1-"),
                // …puis reprend après reconnexion.
                response("Content-Type: audio/aacp\r\n", b"part2"),
            ]
        });

        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let mut opened = open_stream(
            rt.handle(),
            &format!("{base}/radio.pls"),
            fast_opts(),
            move |s| ev.lock().unwrap().push(s),
        )
        .unwrap();
        assert!(opened.live);
        assert_eq!(opened.hint.as_deref(), Some("aac"));

        // Lit jusqu'à la fin : part1, reconnexion, part2, puis abandon.
        let mut got = Vec::new();
        opened.reader.read_to_end(&mut got).unwrap();
        assert_eq!(got, b"part1-part2");

        let events = events.lock().unwrap();
        assert_eq!(events[0], StreamStatus::Reconnecting { attempt: 1 });
        assert_eq!(events[1], StreamStatus::Reconnected);
        assert!(matches!(events.last(), Some(StreamStatus::Failed(_))));
    }

    #[test]
    fn strips_icy_metadata_and_reports_title() {
        let mut body = b"abcd".to_vec();
        let meta = b"StreamTitle='Artiste - Titre';";
        let blocks = meta.len().div_ceil(16);
        body.push(blocks as u8);
        body.extend(meta);
        body.resize(4 + 1 + blocks * 16, 0);
        body.extend(b"efgh");
        let resp = response("Content-Type: audio/mpeg\r\nicy-metaint: 4\r\n", &body);
        let (rt, base) = serve(move |_| vec![resp]);

        let titles = Arc::new(Mutex::new(Vec::new()));
        let t = titles.clone();
        let mut opened = open_stream(
            rt.handle(),
            &format!("{base}/live"),
            fast_opts(),
            move |s| {
                if let StreamStatus::Title(title) = s {
                    t.lock().unwrap().push(title);
                }
            },
        )
        .unwrap();
        let mut got = [0u8; 8];
        opened.reader.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"abcdefgh");
        assert_eq!(*titles.lock().unwrap(), vec!["Artiste - Titre".to_string()]);
    }

    #[test]
    fn plays_live_hls_with_playlist_refresh() {
        let seg1 = crate::ts::tests::segment(0x0f, b"segment-1;");
        let seg2 = crate::ts::tests::segment(0x0f, b"segment-2");
        let (rt, base) = serve(|base| {
            let hls = "Content-Type: application/vnd.apple.mpegurl\r\n";
            let ts = "Content-Type: video/mp2t\r\n";
            vec![
                response(
                    hls,
                    format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,CODECS=\"mp4a.40.2\"\n{base}/a.m3u8\n")
                        .as_bytes(),
                ),
                // Direct : un seul segment pour l'instant…
                response(
                    hls,
                    b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:7\n#EXTINF:1,\ns7.ts\n",
                ),
                response(ts, &seg1),
                // …puis la playlist relue en annonce un second, et se termine.
                response(
                    hls,
                    b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:7\n#EXTINF:1,\ns7.ts\n#EXTINF:1,\ns8.ts\n#EXT-X-ENDLIST\n",
                ),
                response(ts, &seg2),
            ]
        });
        let mut opened = open_stream(
            rt.handle(),
            &format!("{base}/live.m3u8"),
            fast_opts(),
            |_| {},
        )
        .unwrap();
        assert!(opened.live);
        assert_eq!(opened.hint.as_deref(), Some("aac"));
        let mut got = Vec::new();
        opened.reader.read_to_end(&mut got).unwrap();
        assert_eq!(got, b"segment-1;segment-2");
    }

    #[test]
    fn rejects_html() {
        let (rt, base) = serve(|_| vec![response("Content-Type: text/html\r\n", b"<html>")]);
        let err = open_stream(rt.handle(), &format!("{base}/"), fast_opts(), |_| {}).unwrap_err();
        assert!(matches!(err, SourceError::NotAudio(m) if m == "text/html"));
    }

    #[test]
    fn rejects_invalid_url() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert!(matches!(
            open_stream(rt.handle(), "pas une url", fast_opts(), |_| {}),
            Err(SourceError::InvalidUrl(_))
        ));
    }
}
