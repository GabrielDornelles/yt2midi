//! Whole-song exports built from rendered sheet pages: a print-ready PDF and an EPUB for Kindle.
use flate2::{write::ZlibEncoder, Compression};
use std::io::Write;

/// Grey 8-bit PNG tagged with the given dpi (an A4 page prints at its true size).
pub fn png_bytes(gray: &[u8], w: usize, h: usize, dpi: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Default);
        let ppm = (dpi as f64 / 0.0254).round() as u32;
        enc.set_pixel_dims(Some(png::PixelDimensions { xppu: ppm, yppu: ppm, unit: png::Unit::Meter }));
        let mut wr = enc.write_header().expect("png header");
        wr.write_image_data(gray).expect("png data");
    }
    out
}

/// File-name friendly title: letters, digits and single hyphens, cut at a word boundary.
pub fn safe_name(title: &str, limit: usize) -> String {
    let cleaned: String = title.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == ' ' { c } else { ' ' }).collect();
    let mut out = String::new();
    for w in cleaned.split_whitespace() {
        if out.chars().count() + w.chars().count() + 1 > limit && !out.is_empty() {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(w);
    }
    if out.is_empty() { "song".into() } else { out }
}

fn pdf_text(s: &str) -> String {
    let mut h = String::from("<FEFF");
    for u in s.encode_utf16() {
        h.push_str(&format!("{u:04X}"));
    }
    h.push('>');
    h
}

/// Lossless PDF: every page is the grey image at 300 dpi, so an A4 page prints at true A4 size.
pub fn pdf_from_pages(pages: &[(Vec<u8>, usize, usize)], title: &str, dpi: f64) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets: Vec<usize> = vec![];
    let n = pages.len();
    // object numbers: 1 catalog, 2 pages, 3 info, then (page, content, image) per page
    let mut add = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        let id = offsets.len();
        out.extend(format!("{id} 0 obj\n").as_bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    };
    add(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: String = (0..n).map(|i| format!("{} 0 R ", 4 + i * 3)).collect();
    add(&mut out, format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>").as_bytes());
    add(&mut out, format!("<< /Title {} /Author (yt2midi) /Producer (yt2midi) >>", pdf_text(title)).as_bytes());
    for (i, (gray, w, h)) in pages.iter().enumerate() {
        let (pw, ph) = (*w as f64 * 72.0 / dpi, *h as f64 * 72.0 / dpi);
        let base = 4 + i * 3;
        add(&mut out, format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Contents {} 0 R /Resources << /XObject << /Im0 {} 0 R >> >> >>", base + 1, base + 2).as_bytes());
        let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q");
        let mut c = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        c.extend(content.as_bytes());
        c.extend(b"\nendstream");
        add(&mut out, &c);
        let mut z = ZlibEncoder::new(Vec::new(), Compression::new(6));
        z.write_all(gray).unwrap();
        let data = z.finish().unwrap();
        let mut im = format!("<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n", data.len()).into_bytes();
        im.extend(data);
        im.extend(b"\nendstream");
        add(&mut out, &im);
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// One full-screen image per page. Pages keep their exact pixels (no resizing, sharpening or trimming).
pub fn epub_from_pages(pngs: &[Vec<u8>], w: usize, h: usize, title: &str) -> Result<Vec<u8>, String> {
    let uid = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    let t = xml_escape(title);
    let items: String = (1..=pngs.len())
        .map(|i| {
            format!(
                "<item id=\"p{i}\" href=\"p{i:04}.xhtml\" media-type=\"application/xhtml+xml\"/><item id=\"i{i}\" href=\"images/p{i:04}.png\" media-type=\"image/png\"{}/>",
                if i == 1 { " properties=\"cover-image\"" } else { "" }
            )
        })
        .collect();
    let spine: String = (1..=pngs.len()).map(|i| format!("<itemref idref=\"p{i}\"/>")).collect();
    let opf = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"uid\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:opf=\"http://www.idpf.org/2007/opf\">\n<dc:identifier id=\"uid\">{uid}</dc:identifier><dc:title>{t}</dc:title><dc:creator>yt2midi</dc:creator>\n<dc:language>en</dc:language><meta name=\"cover\" content=\"i1\"/><meta property=\"dcterms:modified\">2024-01-01T00:00:00Z</meta>\n<meta name=\"fixed-layout\" content=\"true\"/>\n</metadata><manifest>{items}<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/></manifest>\n<spine toc=\"ncx\">{spine}</spine></package>"
    );
    let ncx = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"><head><meta name=\"dtb:uid\" content=\"{uid}\"/></head>\n<docTitle><text>{t}</text></docTitle><navMap>\n<navPoint id=\"n1\" playOrder=\"1\"><navLabel><text>Page 1</text></navLabel><content src=\"p0001.xhtml\"/></navPoint></navMap></ncx>"
    );
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflate = zip::write::SimpleFileOptions::default();
        let mut put = |name: &str, data: &[u8], o: zip::write::SimpleFileOptions| -> Result<(), String> {
            z.start_file(name, o).map_err(|e| e.to_string())?;
            z.write_all(data).map_err(|e| e.to_string())
        };
        put("mimetype", b"application/epub+zip", stored)?;
        put("META-INF/container.xml", b"<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>", deflate)?;
        put("content.opf", opf.as_bytes(), deflate)?;
        put("toc.ncx", ncx.as_bytes(), deflate)?;
        for (i, data) in pngs.iter().enumerate() {
            let n = i + 1;
            let page = format!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>{n}</title>\n<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"/>\n<style type=\"text/css\">@page{{margin:0}} body{{margin:0;padding:0}}</style></head>\n<body><div><svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" version=\"1.1\"\n width=\"100%\" height=\"100%\" viewBox=\"0 0 {w} {h}\" preserveAspectRatio=\"xMidYMid meet\">\n<image width=\"{w}\" height=\"{h}\" xlink:href=\"images/p{n:04}.png\"/></svg></div></body></html>\n"
            );
            put(&format!("p{n:04}.xhtml"), page.as_bytes(), deflate)?;
            put(&format!("images/p{n:04}.png"), data, stored)?;
        }
        z.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}
