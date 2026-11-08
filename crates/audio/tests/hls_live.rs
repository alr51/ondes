//! Lecture de vrais flux HLS (réseau requis) :
//! `cargo test -p ondes-audio --test hls_live -- --ignored --nocapture`
//! Une URL précise : `ONDES_HLS_URL=https://… cargo test …`, une durée :
//! `ONDES_HLS_SECS=40`.

use std::time::Instant;

use ondes_sources::{HttpOptions, open_stream};
use rodio::{Decoder, Source};

const STREAMS: &[&str] = &[
    // MPEG-TS, AAC-LC, playlist maître.
    "https://stream.radiofrance.fr/fip/fip.m3u8?id=radiofrance",
    // fMP4, piste audio AAC séparée de la vidéo (EXT-X-MEDIA).
    "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_fmp4/master.m3u8",
];

#[test]
#[ignore = "réseau"]
fn decodes_real_hls_streams() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let urls: Vec<String> = match std::env::var("ONDES_HLS_URL") {
        Ok(url) => vec![url],
        Err(_) => STREAMS.iter().map(|s| s.to_string()).collect(),
    };
    for url in urls {
        let t = Instant::now();
        let opened = open_stream(rt.handle(), &url, HttpOptions::default(), |s| {
            eprintln!("  statut : {s:?}");
        })
        .unwrap_or_else(|e| panic!("{url} : {e}"));
        let hint = opened.hint.clone();
        let mut builder = Decoder::builder()
            .with_data(opened.reader)
            .with_seekable(false);
        if let Some(h) = &hint {
            builder = builder.with_hint(h);
        }
        let decoder = builder.build().unwrap_or_else(|e| panic!("{url} : {e}"));
        let (rate, channels) = (decoder.sample_rate().get(), decoder.channels().get());
        // 12 s d'audio par défaut : plusieurs segments, donc au moins un
        // recollage. Plus de 12 s sur un direct : la playlist est relue.
        let secs: usize = std::env::var("ONDES_HLS_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(12);
        let wanted = rate as usize * channels as usize * secs;
        let samples: Vec<f32> = decoder.take(wanted).collect();
        let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
        eprintln!(
            "{url}\n  {hint:?}, {rate} Hz, {channels} canaux, {} échantillons, crête {peak:.2}, {:.1} s",
            samples.len(),
            t.elapsed().as_secs_f32()
        );
        assert_eq!(samples.len(), wanted, "{url} : flux interrompu");
        assert!(peak > 0.01, "{url} : silence");
    }
}
