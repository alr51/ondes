//! Parcours réel sur les API (réseau) : `cargo test -p ondes-info -- --ignored`.

use ondes_info::{AlbumKind, Info};

#[tokio::test]
#[ignore = "réseau"]
async fn massive_attack() {
    let cache = std::env::temp_dir().join("ondes-info-test");
    let info = Info::new(cache).unwrap();

    let id = info.find_artist("Massive Attack").await.unwrap();
    let artist = info.artist(&id).await.unwrap();
    println!("{artist:#?}");
    assert_eq!(artist.name, "Massive Attack");
    let bio = info.bio(artist.wikidata.as_deref().unwrap()).await;
    assert!(bio.is_some());
    let albums: Vec<_> = artist
        .discography
        .iter()
        .filter(|g| g.kind == AlbumKind::Album)
        .map(|g| g.title.as_str())
        .collect();
    assert!(albums.contains(&"Mezzanine"), "{albums:?}");

    let album = info
        .find_album("Massive Attack", "Mezzanine")
        .await
        .unwrap();
    let album = info.album(&album).await.unwrap();
    println!("{album:#?}");
    assert!(album.tracks.iter().any(|t| t.title == "Teardrop"));

    let track = info
        .find_track("Massive Attack", "Teardrop", Some("Mezzanine"))
        .await
        .unwrap();
    let track = info.track(&track).await.unwrap();
    println!("{track:#?}");
    assert_eq!(track.albums.first().map(|a| a.kind), Some(AlbumKind::Album));
}

/// Titres tels que les radios les annoncent : apostrophes, tirets et
/// précisions diffèrent de MusicBrainz.
#[tokio::test]
#[ignore = "réseau"]
async fn radio_titles() {
    let cache = std::env::temp_dir().join(format!("ondes-info-test-{}", std::process::id()));
    let info = Info::new(cache.clone()).unwrap();
    for (artist, title) in [
        ("3rd Force", "Love`s The Reason"),
        ("Ilements", "Sweet Rub-A-Dub"),
        ("Black Roots", "What Them A Do [ft. Dub Judah]"),
        ("Plustwo", "Melody (1983 club vinyl mix)"),
        ("Isabella Lundgren", "Singin in the Rain"),
    ] {
        let id = info.find_track(artist, title, None).await;
        println!("{artist} — {title} : {id:?}");
        let track = info.track(&id.unwrap()).await.unwrap();
        assert!(
            track.artist.to_lowercase().contains(&artist.to_lowercase()),
            "{track:?}"
        );
    }
    let _ = std::fs::remove_dir_all(cache);
}

/// Une recherche sans résultat ne reste qu'un jour en cache.
#[tokio::test]
#[ignore = "réseau"]
async fn not_found_is_short_lived() {
    let cache = std::env::temp_dir().join(format!("ondes-info-nf-{}", std::process::id()));
    let info = Info::new(cache.clone()).unwrap();
    let found = info
        .find_track("Celso Fonseca", "Zzyzx Qwfp Inexistant", None)
        .await;
    assert!(
        matches!(found, Err(ondes_info::InfoError::NotFound)),
        "{found:?}"
    );
    let ages: Vec<u64> = std::fs::read_dir(&cache)
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().modified().unwrap())
        .map(|m| m.elapsed().unwrap().as_secs() / 86_400)
        .collect();
    assert_eq!(ages, [13], "vieillie pour expirer dans un jour");
    let _ = std::fs::remove_dir_all(cache);
}
