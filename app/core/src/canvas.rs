//! A tiny drawing surface with the handful of operations the sheet needs. Coordinates are plain pixels of the
//! (supersampled) surface; colours are grey levels 0..=255.
use std::sync::OnceLock;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};
use ttf_parser::{Face, OutlineBuilder};

static BOLD: &[u8] = include_bytes!("../assets/DejaVuSans-Bold.ttf");
static REGULAR: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Regular,
    Bold,
}

fn face(f: Font) -> &'static Face<'static> {
    static R: OnceLock<Face<'static>> = OnceLock::new();
    static B: OnceLock<Face<'static>> = OnceLock::new();
    match f {
        Font::Regular => R.get_or_init(|| Face::parse(REGULAR, 0).expect("font")),
        Font::Bold => B.get_or_init(|| Face::parse(BOLD, 0).expect("font")),
    }
}

/// Width of `s` in pixels at font size `size` (pixels per em).
pub fn text_len(s: &str, f: Font, size: f64) -> f64 {
    let fc = face(f);
    let k = size / fc.units_per_em() as f64;
    s.chars().map(|c| fc.glyph_index(c).and_then(|g| fc.glyph_hor_advance(g)).unwrap_or(0) as f64 * k).sum()
}

pub fn ascent(f: Font, size: f64) -> f64 {
    let fc = face(f);
    (fc.ascender() as f64 * size / fc.units_per_em() as f64).ceil()
}

pub fn descent(f: Font, size: f64) -> f64 {
    let fc = face(f);
    (-(fc.descender() as f64) * size / fc.units_per_em() as f64).ceil()
}

struct Outline<'a> {
    pb: &'a mut PathBuilder,
    k: f32,
    x: f32,
    y: f32,
}
impl OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) { self.pb.move_to(self.x + x * self.k, self.y - y * self.k) }
    fn line_to(&mut self, x: f32, y: f32) { self.pb.line_to(self.x + x * self.k, self.y - y * self.k) }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb.quad_to(self.x + x1 * self.k, self.y - y1 * self.k, self.x + x * self.k, self.y - y * self.k)
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb.cubic_to(self.x + x1 * self.k, self.y - y1 * self.k, self.x + x2 * self.k, self.y - y2 * self.k, self.x + x * self.k, self.y - y * self.k)
    }
    fn close(&mut self) { self.pb.close() }
}

pub struct Canvas {
    pm: Pixmap,
    pub w: u32,
    pub h: u32,
}

fn paint(g: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(g, g, g, 255);
    p.anti_alias = true;
    p
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Self {
        let mut pm = Pixmap::new(w, h).expect("page too large");
        pm.fill(tiny_skia::Color::WHITE);
        Canvas { pm, w, h }
    }

    fn stroke_path(&mut self, path: &Path, g: u8, width: f64, join: LineJoin) {
        let st = Stroke { width: width as f32, line_cap: LineCap::Butt, line_join: join, ..Default::default() };
        self.pm.stroke_path(path, &paint(g), &st, Transform::identity(), None);
    }

    pub fn line(&mut self, pts: &[(f64, f64)], g: u8, width: f64, round_join: bool) {
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0 as f32, pts[0].1 as f32);
        for p in &pts[1..] {
            pb.line_to(p.0 as f32, p.1 as f32);
        }
        if let Some(path) = pb.finish() {
            self.stroke_path(&path, g, width, if round_join { LineJoin::Round } else { LineJoin::Miter });
        }
    }

    pub fn polygon(&mut self, pts: &[(f64, f64)], g: u8) {
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0 as f32, pts[0].1 as f32);
        for p in &pts[1..] {
            pb.line_to(p.0 as f32, p.1 as f32);
        }
        pb.close();
        if let Some(path) = pb.finish() {
            self.pm.fill_path(&path, &paint(g), FillRule::Winding, Transform::identity(), None);
        }
    }

    /// Ellipse inside box `[x0,y0,x1,y1]`; the outline is drawn inwards from the box edge, like PIL does.
    pub fn ellipse(&mut self, b: [f64; 4], fill: Option<u8>, outline: Option<u8>, width: f64) {
        let oval = |b: [f64; 4]| tiny_skia::Rect::from_ltrb(b[0] as f32, b[1] as f32, b[2] as f32, b[3] as f32).and_then(PathBuilder::from_oval);
        if let (Some(g), Some(path)) = (fill, oval(b)) {
            self.pm.fill_path(&path, &paint(g), FillRule::Winding, Transform::identity(), None);
        }
        if let Some(g) = outline {
            let h = width / 2.0;
            if let Some(path) = oval([b[0] + h, b[1] + h, b[2] - h, b[3] - h]) {
                self.stroke_path(&path, g, width, LineJoin::Miter);
            }
        }
    }

    fn rrect_path(b: [f64; 4], r: f64) -> Option<Path> {
        let (x0, y0, x1, y1) = (b[0] as f32, b[1] as f32, b[2] as f32, b[3] as f32);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let r = (r as f32).min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
        let c = r * 0.5522847;
        let mut pb = PathBuilder::new();
        pb.move_to(x0 + r, y0);
        pb.line_to(x1 - r, y0);
        pb.cubic_to(x1 - r + c, y0, x1, y0 + r - c, x1, y0 + r);
        pb.line_to(x1, y1 - r);
        pb.cubic_to(x1, y1 - r + c, x1 - r + c, y1, x1 - r, y1);
        pb.line_to(x0 + r, y1);
        pb.cubic_to(x0 + r - c, y1, x0, y1 - r + c, x0, y1 - r);
        pb.line_to(x0, y0 + r);
        pb.cubic_to(x0, y0 + r - c, x0 + r - c, y0, x0 + r, y0);
        pb.close();
        pb.finish()
    }

    pub fn rounded_rect(&mut self, b: [f64; 4], radius: f64, fill: Option<u8>, outline: Option<u8>, width: f64) {
        if let (Some(g), Some(p)) = (fill, Self::rrect_path(b, radius)) {
            self.pm.fill_path(&p, &paint(g), FillRule::Winding, Transform::identity(), None);
        }
        if let Some(g) = outline {
            let h = width / 2.0;
            if let Some(p) = Self::rrect_path([b[0] + h, b[1] + h, b[2] - h, b[3] - h], radius - h) {
                self.stroke_path(&p, g, width, LineJoin::Miter);
            }
        }
    }

    /// Text with its top-left at the ascender line (PIL's default anchor "la").
    pub fn text(&mut self, x: f64, y_top: f64, s: &str, f: Font, size: f64, g: u8, stroke: f64) {
        let baseline = y_top + ascent(f, size);
        self.text_baseline(x, baseline, s, f, size, g, stroke);
    }

    /// Text centred on `x` and vertically between ascender and descender at `y` (PIL's anchor "mm").
    pub fn text_mm(&mut self, x: f64, y: f64, s: &str, f: Font, size: f64, g: u8) {
        let w = text_len(s, f, size);
        let baseline = y + (ascent(f, size) - descent(f, size)) / 2.0;
        self.text_baseline(x - w / 2.0, baseline, s, f, size, g, 0.0);
    }

    fn text_baseline(&mut self, x: f64, baseline: f64, s: &str, f: Font, size: f64, g: u8, stroke: f64) {
        let fc = face(f);
        let k = (size / fc.units_per_em() as f64) as f32;
        let mut pen = x as f32;
        let mut pb = PathBuilder::new();
        for c in s.chars() {
            let Some(gid) = fc.glyph_index(c) else { continue };
            let mut ob = Outline { pb: &mut pb, k, x: pen, y: baseline as f32 };
            fc.outline_glyph(gid, &mut ob);
            pen += fc.glyph_hor_advance(gid).unwrap_or(0) as f32 * k;
        }
        if let Some(path) = pb.finish() {
            self.pm.fill_path(&path, &paint(g), FillRule::Winding, Transform::identity(), None);
            if stroke > 0.0 {
                self.stroke_path(&path, g, stroke * 2.0, LineJoin::Round);
            }
        }
    }

    /// Grey plane of the surface (the red channel; everything drawn is grey).
    pub fn into_gray(self) -> (Vec<u8>, usize, usize) {
        let (w, h) = (self.w as usize, self.h as usize);
        let data = self.pm.data();
        let mut out = vec![0u8; w * h];
        for (o, px) in out.iter_mut().zip(data.chunks_exact(4)) {
            *o = px[0];
        }
        (out, w, h)
    }
}

/// Lanczos-3 downscale by an integer factor (what PIL's LANCZOS resize does), for a grey image.
pub fn downscale_lanczos(src: &[u8], w: usize, h: usize, factor: usize) -> (Vec<u8>, usize, usize) {
    use rayon::prelude::*;
    let (ow, oh) = (w / factor, h / factor);
    let weights = |n_in: usize, n_out: usize| -> Vec<(usize, Vec<f32>)> {
        let scale = n_in as f64 / n_out as f64;
        let support = 3.0 * scale.max(1.0);
        (0..n_out)
            .map(|i| {
                let center = (i as f64 + 0.5) * scale;
                let lo = ((center - support + 0.5) as i64).max(0) as usize;
                let hi = ((center + support + 0.5) as i64).min(n_in as i64) as usize;
                let mut ws: Vec<f64> = (lo..hi)
                    .map(|x| {
                        let t = (x as f64 - center + 0.5) / scale.max(1.0);
                        if t == 0.0 { 1.0 } else if t.abs() >= 3.0 { 0.0 } else { let pt = std::f64::consts::PI * t; 3.0 * pt.sin() * (pt / 3.0).sin() / (pt * pt) }
                    })
                    .collect();
                let sum: f64 = ws.iter().sum();
                ws.iter_mut().for_each(|v| *v /= sum);
                (lo, ws.into_iter().map(|v| v as f32).collect())
            })
            .collect()
    };
    let wx = weights(w, ow);
    let wy = weights(h, oh);
    // horizontal pass (all rows), then vertical
    let mut tmp = vec![0f32; ow * h];
    tmp.par_chunks_mut(ow).enumerate().for_each(|(y, row)| {
        let line = &src[y * w..(y + 1) * w];
        for (x, (lo, ws)) in wx.iter().enumerate() {
            row[x] = ws.iter().enumerate().map(|(i, wv)| line[lo + i] as f32 * wv).sum();
        }
    });
    let mut out = vec![0u8; ow * oh];
    out.par_chunks_mut(ow).enumerate().for_each(|(y, row)| {
        let (lo, ws) = &wy[y];
        for x in 0..ow {
            let v: f32 = ws.iter().enumerate().map(|(i, wv)| tmp[(lo + i) * ow + x] * wv).sum();
            row[x] = v.round().clamp(0.0, 255.0) as u8;
        }
    });
    (out, ow, oh)
}
