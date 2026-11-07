//! Métadonnées ICY (Shoutcast / Icecast).
//!
//! Quand le client envoie `Icy-MetaData: 1`, le serveur répond avec
//! `icy-metaint: N` et insère, toutes les N octets d'audio, un bloc :
//! 1 octet de longueur L (en multiples de 16), puis L × 16 octets de texte
//! du genre `StreamTitle='Artiste - Titre';StreamUrl='';`, complété par des
//! zéros. [`IcyDemuxer`] retire ces blocs du flux et extrait le titre.

use bytes::{Bytes, BytesMut};

/// Sépare l'audio des métadonnées ICY.
#[derive(Debug)]
pub struct IcyDemuxer {
    metaint: usize,
    state: State,
    meta: BytesMut,
    last_title: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum State {
    /// Encore `n` octets d'audio avant le prochain bloc.
    Audio(usize),
    /// Prochain octet : longueur du bloc.
    Length,
    /// Encore `n` octets de métadonnées.
    Meta(usize),
}

/// Résultat d'un passage de [`IcyDemuxer::feed`].
#[derive(Debug, Default, PartialEq)]
pub struct Demuxed {
    /// Audio débarrassé des métadonnées, en morceaux contigus.
    pub audio: Vec<Bytes>,
    /// Nouveau titre, s'il a changé.
    pub title: Option<String>,
}

impl IcyDemuxer {
    pub fn new(metaint: usize) -> Self {
        Self {
            metaint,
            state: State::Audio(metaint),
            meta: BytesMut::new(),
            last_title: None,
        }
    }

    pub fn feed(&mut self, mut chunk: Bytes) -> Demuxed {
        let mut out = Demuxed::default();
        while !chunk.is_empty() {
            match self.state {
                State::Audio(left) => {
                    let n = left.min(chunk.len());
                    out.audio.push(chunk.split_to(n));
                    self.state = if n == left {
                        State::Length
                    } else {
                        State::Audio(left - n)
                    };
                }
                State::Length => {
                    let len = chunk[0] as usize * 16;
                    chunk = chunk.slice(1..);
                    self.state = if len == 0 {
                        State::Audio(self.metaint)
                    } else {
                        self.meta.clear();
                        State::Meta(len)
                    };
                }
                State::Meta(left) => {
                    let n = left.min(chunk.len());
                    self.meta.extend_from_slice(&chunk.split_to(n));
                    if n == left {
                        self.state = State::Audio(self.metaint);
                        let title = parse_stream_title(&self.meta);
                        if title.is_some() && title != self.last_title {
                            self.last_title = title.clone();
                            out.title = title;
                        }
                    } else {
                        self.state = State::Meta(left - n);
                    }
                }
            }
        }
        out
    }
}

/// Extrait `StreamTitle` d'un bloc de métadonnées. `None` si absent ou vide.
pub fn parse_stream_title(block: &[u8]) -> Option<String> {
    let end = block.iter().position(|&b| b == 0).unwrap_or(block.len());
    let text = decode_text(&block[..end]);
    let start = text.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &text[start..];
    // Le titre peut contenir des apostrophes : on s'arrête à `';`.
    let value = match rest.find("';") {
        Some(i) => &rest[..i],
        None => rest.strip_suffix('\'').unwrap_or(rest),
    };
    let value = value.trim();
    (!value.is_empty() && value != "-").then(|| value.to_string())
}

/// UTF-8 si valide, sinon Latin-1 (fréquent sur les vieux serveurs).
fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(text: &str) -> Vec<u8> {
        let mut b = text.as_bytes().to_vec();
        let len = b.len().div_ceil(16);
        b.resize(len * 16, 0);
        let mut out = vec![len as u8];
        out.extend(b);
        out
    }

    fn joined(d: &Demuxed) -> Vec<u8> {
        d.audio.iter().flat_map(|b| b.iter().copied()).collect()
    }

    #[test]
    fn parses_titles() {
        assert_eq!(
            parse_stream_title(b"StreamTitle='Nina Simone - Feeling Good';StreamUrl='';\0\0"),
            Some("Nina Simone - Feeling Good".into())
        );
        assert_eq!(
            parse_stream_title(b"StreamTitle='Rock'n'Roll';"),
            Some("Rock'n'Roll".into())
        );
        assert_eq!(parse_stream_title(b"StreamTitle='';"), None);
        assert_eq!(parse_stream_title(b"StreamUrl='x';"), None);
        // Latin-1 : « é » = 0xE9.
        assert_eq!(
            parse_stream_title(b"StreamTitle='Caf\xe9';"),
            Some("Café".into())
        );
    }

    #[test]
    fn strips_metadata_across_chunk_boundaries() {
        let mut stream = b"AAAA".to_vec();
        stream.extend(block("StreamTitle='Un';"));
        stream.extend(b"BBBB");
        stream.push(0); // bloc vide
        stream.extend(b"CCCC");
        stream.extend(block("StreamTitle='Un';")); // titre inchangé
        stream.extend(b"DD");

        // Découpage arbitraire, y compris au milieu des blocs.
        for size in [1, 3, 5, 7, 64] {
            let mut demux = IcyDemuxer::new(4);
            let mut audio = Vec::new();
            let mut titles = Vec::new();
            for piece in stream.chunks(size) {
                let d = demux.feed(Bytes::copy_from_slice(piece));
                audio.extend(joined(&d));
                titles.extend(d.title);
            }
            assert_eq!(audio, b"AAAABBBBCCCCDD", "taille {size}");
            assert_eq!(titles, vec!["Un".to_string()], "taille {size}");
        }
    }
}
