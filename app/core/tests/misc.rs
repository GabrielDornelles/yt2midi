use yt2midi_core::{downloader, export, keyboard};

#[test]
fn video_ids_from_the_usual_links() {
    for (url, id) in [
        ("https://youtu.be/8XFScqk42yI", "8XFScqk42yI"),
        ("https://www.youtube.com/watch?v=8XFScqk42yI&t=30s", "8XFScqk42yI"),
        ("https://youtu.be/8XFScqk42yI?si=abc", "8XFScqk42yI"),
        ("  https://www.youtube.com/watch?feature=share&v=EIs9pi_b6J4 ", "EIs9pi_b6J4"),
        ("https://www.youtube.com/embed/fwJMTJShg5E", "fwJMTJShg5E"),
    ] {
        assert_eq!(downloader::extract_video_id(url).unwrap(), id, "{url}");
    }
    assert!(downloader::extract_video_id("   ").is_err());
}

#[test]
fn keyboard_geometry() {
    assert_eq!(keyboard::midi_number("C4"), Some(60));
    assert_eq!(keyboard::midi_number("A0"), Some(21));
    assert_eq!(keyboard::midi_number("C#4"), Some(61));
    let keys = keyboard::build_keys(0.0, 1920.0, 844.0, 1077.0, "A0", 52).unwrap();
    assert_eq!(keys.len(), 88);
    assert_eq!(keys.first().unwrap().midi, 21);
    assert_eq!(keys.last().unwrap().midi, 108);
    assert!(keyboard::build_keys(0.0, 100.0, 0.0, 50.0, "H1", 7).is_none());
}

#[test]
fn export_file_names_and_pdf() {
    assert_eq!(export::safe_name("La La Land - Mia & Sebastian's Theme", 60), "La-La-Land-Mia-Sebastian-s-Theme");
    assert_eq!(export::safe_name("???", 60), "song");
    assert_eq!(export::safe_name("one two three", 8), "one-two");
    let pdf = export::pdf_from_pages(&[(vec![255u8; 30 * 20], 30, 20)], "Título", 300.0);
    assert!(pdf.starts_with(b"%PDF-1.4") && pdf.ends_with(b"%%EOF\n"));
    let epub = export::epub_from_pages(&[export::png_bytes(&[255u8; 30 * 20], 30, 20, 300)], 30, 20, "T & Co").unwrap();
    assert_eq!(&epub[..2], b"PK");
}
