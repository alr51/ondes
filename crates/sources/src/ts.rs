//! Démultiplexeur MPEG-TS minimal pour les segments HLS.
//!
//! Extrait la première piste audio lisible (AAC en ADTS ou MPEG audio) d'une
//! suite de paquets de 188 octets. L'état (PID de la piste, paquet incomplet)
//! est conservé d'un segment à l'autre.

const PACKET: usize = 188;
const SYNC: u8 = 0x47;

/// Codec de la piste audio extraite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TsAudio {
    /// AAC en trames ADTS (`stream_type` 0x0F).
    Aac,
    /// MPEG-1/2 audio, en pratique du MP3 (`stream_type` 0x03 / 0x04).
    Mpeg,
}

#[derive(Debug, Default)]
pub(crate) struct TsDemuxer {
    pmt_pid: Option<u16>,
    audio: Option<(u16, TsAudio)>,
    /// Fin de paquet incomplète, complétée par le segment suivant.
    carry: Vec<u8>,
    /// `true` une fois un début de PES audio vu : la suite peut être transmise.
    in_pes: bool,
}

impl TsDemuxer {
    pub(crate) fn audio(&self) -> Option<TsAudio> {
        self.audio.map(|(_, kind)| kind)
    }

    /// Démultiplexe `data` et renvoie les octets audio bruts.
    pub(crate) fn feed(&mut self, data: &[u8]) -> Vec<u8> {
        let mut buf = std::mem::take(&mut self.carry);
        buf.extend_from_slice(data);
        let mut out = Vec::with_capacity(buf.len());
        let mut i = 0;
        while i + PACKET <= buf.len() {
            if buf[i] != SYNC {
                // Perte de synchronisation : on cherche le prochain octet 0x47.
                i += 1;
                continue;
            }
            self.packet(&buf[i..i + PACKET], &mut out);
            i += PACKET;
        }
        self.carry = buf[i..].to_vec();
        out
    }

    fn packet(&mut self, p: &[u8], out: &mut Vec<u8>) {
        let start_indicator = p[1] & 0x40 != 0;
        let pid = (u16::from(p[1] & 0x1f) << 8) | u16::from(p[2]);
        let adaptation = (p[3] >> 4) & 0b11;
        if adaptation & 0b01 == 0 {
            return; // pas de charge utile
        }
        let mut start = 4;
        if adaptation & 0b10 != 0 {
            start += 1 + usize::from(p[4]);
        }
        let Some(payload) = p.get(start..).filter(|s| !s.is_empty()) else {
            return;
        };

        if pid == 0 {
            if start_indicator && let Some(pmt) = parse_pat(payload) {
                self.pmt_pid = Some(pmt);
            }
        } else if Some(pid) == self.pmt_pid {
            if start_indicator
                && let Some(audio) = parse_pmt(payload)
                && self.audio != Some(audio)
            {
                tracing::debug!(pid = audio.0, kind = ?audio.1, "piste audio MPEG-TS");
                self.audio = Some(audio);
                self.in_pes = false;
            }
        } else if self.audio.is_some_and(|(a, _)| a == pid) {
            if start_indicator {
                match pes_payload(payload) {
                    Some(data) => {
                        self.in_pes = true;
                        out.extend_from_slice(data);
                    }
                    None => self.in_pes = false,
                }
            } else if self.in_pes {
                out.extend_from_slice(payload);
            }
        }
    }
}

/// Section PSI (PAT, PMT) au début de la charge utile, CRC compris.
fn section(payload: &[u8]) -> Option<&[u8]> {
    let pointer = usize::from(*payload.first()?);
    let s = payload.get(1 + pointer..)?;
    let len = (usize::from(s.get(1)? & 0x0f) << 8) | usize::from(*s.get(2)?);
    Some(&s[..(3 + len).min(s.len())])
}

/// PID de la PMT du premier programme.
fn parse_pat(payload: &[u8]) -> Option<u16> {
    let s = section(payload)?;
    if s[0] != 0x00 {
        return None;
    }
    let programs = s.get(8..s.len().saturating_sub(4))?;
    programs
        .as_chunks::<4>()
        .0
        .iter()
        .find(|c| u16::from_be_bytes([c[0], c[1]]) != 0) // 0 : table réseau
        .map(|c| (u16::from(c[2] & 0x1f) << 8) | u16::from(c[3]))
}

/// Première piste audio lisible déclarée par la PMT.
fn parse_pmt(payload: &[u8]) -> Option<(u16, TsAudio)> {
    let s = section(payload)?;
    if s[0] != 0x02 {
        return None;
    }
    let info_len = (usize::from(s.get(10)? & 0x0f) << 8) | usize::from(*s.get(11)?);
    let end = s.len().saturating_sub(4);
    let mut i = 12 + info_len;
    while i + 5 <= end {
        let stream_type = s[i];
        let pid = (u16::from(s[i + 1] & 0x1f) << 8) | u16::from(s[i + 2]);
        let es_len = (usize::from(s[i + 3] & 0x0f) << 8) | usize::from(s[i + 4]);
        let kind = match stream_type {
            0x0f => Some(TsAudio::Aac),
            0x03 | 0x04 => Some(TsAudio::Mpeg),
            _ => None,
        };
        if let Some(kind) = kind {
            return Some((pid, kind));
        }
        i += 5 + es_len;
    }
    None
}

/// Données d'un début de paquet PES, en-tête retiré.
fn pes_payload(p: &[u8]) -> Option<&[u8]> {
    if p.get(..3)? != [0, 0, 1] {
        return None;
    }
    let header_len = usize::from(*p.get(8)?);
    p.get(9 + header_len..)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn packet(pid: u16, start: bool, payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() <= PACKET - 4);
        let mut p = vec![
            SYNC,
            (u8::from(start) << 6) | (pid >> 8) as u8,
            pid as u8,
            0x10,
        ];
        let stuffing = PACKET - 4 - payload.len();
        if stuffing > 0 {
            // Champ d'adaptation pour remplir le paquet.
            p[3] = 0x30;
            p.push((stuffing - 1) as u8);
            if stuffing > 1 {
                p.push(0);
                p.extend(std::iter::repeat_n(0xff, stuffing - 2));
            }
        }
        p.extend_from_slice(payload);
        assert_eq!(p.len(), PACKET);
        p
    }

    /// Segment TS : PAT, PMT (une piste vidéo puis une audio), puis `audio`
    /// découpé en paquets PES.
    pub(crate) fn segment(stream_type: u8, audio: &[u8]) -> Vec<u8> {
        let pat = [
            0x00, // pointeur
            0x00, 0xb0, 13, 0x00, 0x01, 0xc1, 0x00, 0x00, // en-tête
            0x00, 0x01, 0xf0, 0x00, // programme 1 → PMT 0x1000
            0, 0, 0, 0, // CRC (non vérifié)
        ];
        let pmt = [
            0x00,
            0x02,
            0xb0,
            23,
            0x00,
            0x01,
            0xc1,
            0x00,
            0x00,
            0xe1,
            0x00,
            0xf0,
            0x00, //
            0x1b,
            0xe1,
            0x00,
            0xf0,
            0x00, // vidéo H.264, PID 0x100
            stream_type,
            0xe1,
            0x01,
            0xf0,
            0x00, // audio, PID 0x101
            0,
            0,
            0,
            0,
        ];
        let mut out = packet(0, true, &pat);
        out.extend(packet(0x1000, true, &pmt));
        out.extend(packet(
            0x100,
            true,
            &[0, 0, 1, 0xe0, 0, 0, 0x80, 0, 0, 9, 9],
        ));
        let mut pes = vec![0, 0, 1, 0xc0, 0, 0, 0x80, 0x80, 5, 1, 2, 3, 4, 5];
        pes.extend_from_slice(audio);
        for (i, chunk) in pes.chunks(PACKET - 4).enumerate() {
            out.extend(packet(0x101, i == 0, chunk));
        }
        out
    }

    #[test]
    fn extracts_audio_track() {
        let audio: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        let seg = segment(0x0f, &audio);
        let mut demux = TsDemuxer::default();
        // Découpage arbitraire : les paquets incomplets sont recollés.
        let mut got = demux.feed(&seg[..500]);
        got.extend(demux.feed(&seg[500..]));
        assert_eq!(demux.audio(), Some(TsAudio::Aac));
        assert_eq!(got, audio);
        // Segment suivant : même état, mêmes données.
        assert_eq!(demux.feed(&seg), audio);
    }

    #[test]
    fn ignores_unknown_audio_codec() {
        let mut demux = TsDemuxer::default();
        assert!(demux.feed(&segment(0x81, b"ac3")).is_empty()); // AC-3
        assert_eq!(demux.audio(), None);
        let mut demux = TsDemuxer::default();
        assert_eq!(demux.feed(&segment(0x03, b"mp3")), b"mp3");
        assert_eq!(demux.audio(), Some(TsAudio::Mpeg));
    }
}
