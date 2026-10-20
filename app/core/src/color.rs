//! Colour helpers. Lab values follow OpenCV's 8-bit convention (L*255/100, a and b offset by 128), which is
//! what the thresholds and hue maths in the analysis were tuned on.
use std::sync::OnceLock;

pub type Lab = [f32; 3];

fn gamma_lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = [0f32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *v = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        }
        t
    })
}

pub fn rgb_to_lab(r: u8, g: u8, b: u8) -> Lab {
    let lut = gamma_lut();
    let (r, g, b) = (lut[r as usize], lut[g as usize], lut[b as usize]);
    let x = (0.412453 * r + 0.357580 * g + 0.180423 * b) / 0.950456;
    let y = 0.212671 * r + 0.715160 * g + 0.072169 * b;
    let z = (0.019334 * r + 0.119193 * g + 0.950227 * b) / 1.088754;
    let f = |t: f32| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    let l = if y > 0.008856 { 116.0 * fy - 16.0 } else { 903.3 * y };
    let q = |v: f32| v.round().clamp(0.0, 255.0);
    [q(l * 255.0 / 100.0), q(500.0 * (fx - fy) + 128.0), q(200.0 * (fy - fz) + 128.0)]
}

/// Lab (OpenCV 8-bit convention) back to sRGB, for showing a hand colour.
pub fn lab_to_rgb(lab: [f32; 3]) -> [u8; 3] {
    let l = lab[0].clamp(0.0, 255.0) * 100.0 / 255.0;
    let a = lab[1].clamp(0.0, 255.0) - 128.0;
    let b = lab[2].clamp(0.0, 255.0) - 128.0;
    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;
    let inv = |t: f32| if t > 0.206893 { t * t * t } else { (t - 16.0 / 116.0) / 7.787 };
    let (x, y, z) = (inv(fx) * 0.950456, inv(fy), inv(fz) * 1.088754);
    let r = 3.240479 * x - 1.537150 * y - 0.498535 * z;
    let g = -0.969256 * x + 1.875991 * y + 0.041556 * z;
    let bl = 0.055648 * x - 0.204043 * y + 1.057311 * z;
    let enc = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        let v = if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
        (v * 255.0).round() as u8
    };
    [enc(r), enc(g), enc(bl)]
}

/// Angle (degrees, 0..360) of the a/b chroma vector.
pub fn hue_angle(lab: &Lab) -> f64 {
    let a = (lab[2] - 128.0) as f64;
    let b = (lab[1] - 128.0) as f64;
    a.atan2(b).to_degrees().rem_euclid(360.0)
}

pub fn chroma(lab: &Lab) -> f64 {
    (((lab[1] - 128.0) as f64).powi(2) + ((lab[2] - 128.0) as f64).powi(2)).sqrt()
}

pub fn dist(a: &Lab, b: &Lab) -> f64 {
    let (x, y, z) = ((a[0] - b[0]) as f64, (a[1] - b[1]) as f64, (a[2] - b[2]) as f64);
    (x * x + y * y + z * z).sqrt()
}

pub fn circ_dist(a: f64, b: f64) -> f64 {
    let d = (a - b).abs() % 360.0;
    d.min(360.0 - d)
}

/// Circular mean of angles in degrees.
pub fn circ_mean(a: &[f64]) -> f64 {
    let (mut s, mut c) = (0.0, 0.0);
    for v in a {
        let r = v.to_radians();
        s += r.sin();
        c += r.cos();
    }
    let n = a.len() as f64;
    (s / n).atan2(c / n).to_degrees().rem_euclid(360.0)
}
