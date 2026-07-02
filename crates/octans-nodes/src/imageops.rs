//! Compositor-style image operations (grayscale `Image`), modeled on Blender's compositor:
//! invert, brightness/contrast, gamma, box blur, mix, and per-pixel image math.
//!
//! Two-image ops run over the overlap (`min` dims), sized like `a` with `b` clamped at its
//! edges — a size mismatch degrades gracefully instead of panicking.

use crate::Image;
use octans_core::{Catalog, NodeRegistry};
use octans_macros::{node, NodeParams};
use serde::{Deserialize, Serialize};

fn px_of(img: &Image, x: usize, y: usize) -> u8 {
    let cx = x.min(img.w.saturating_sub(1));
    let cy = y.min(img.h.saturating_sub(1));
    img.px.get(cy * img.w + cx).copied().unwrap_or(0)
}

/// Invert: `255 − p`.
#[derive(Serialize, Deserialize)]
pub struct Invert;

#[node(id = "octans.image.invert", out = "image", serde)]
impl Invert {
    fn process(&self, image: &Image) -> Image {
        Image {
            w: image.w,
            h: image.h,
            px: image.px.iter().map(|&p| 255 - p).collect(),
        }
    }
}

/// Brightness/contrast: `(p − 127.5)·(1 + contrast/100) + 127.5 + brightness`, clamped.
#[derive(Serialize, Deserialize, NodeParams)]
pub struct BrightContrast {
    /// Added to every pixel.
    #[param(min = -255.0, max = 255.0)]
    pub brightness: f64,
    /// Percent contrast around mid-gray.
    #[param(min = -100.0, max = 100.0)]
    pub contrast: f64,
}

#[node(id = "octans.image.bright_contrast", out = "image", serde, params)]
impl BrightContrast {
    fn process(&self, image: &Image) -> Image {
        let gain = 1.0 + self.contrast / 100.0;
        let px = image
            .px
            .iter()
            .map(|&p| {
                ((p as f64 - 127.5) * gain + 127.5 + self.brightness)
                    .round()
                    .clamp(0.0, 255.0) as u8
            })
            .collect();
        Image {
            w: image.w,
            h: image.h,
            px,
        }
    }
}

/// Gamma: `255·(p/255)^gamma`.
#[derive(Serialize, Deserialize, NodeParams)]
pub struct Gamma {
    /// The exponent (1 = identity, <1 brightens, >1 darkens).
    #[param(min = 0.05, max = 10.0)]
    pub gamma: f64,
}

#[node(id = "octans.image.gamma", out = "image", serde, params)]
impl Gamma {
    fn process(&self, image: &Image) -> Image {
        let px = image
            .px
            .iter()
            .map(|&p| (255.0 * (p as f64 / 255.0).powf(self.gamma)).round() as u8)
            .collect();
        Image {
            w: image.w,
            h: image.h,
            px,
        }
    }
}

/// Separable box blur with the given radius (0 = pass-through).
#[derive(Serialize, Deserialize, NodeParams)]
pub struct BoxBlur {
    /// Blur radius in pixels.
    #[param(min = 0, max = 64)]
    pub radius: u32,
}

#[node(id = "octans.image.box_blur", out = "image", serde, params)]
impl BoxBlur {
    fn process(&self, image: &Image) -> Image {
        let r = self.radius as usize;
        if r == 0 || image.w == 0 || image.h == 0 {
            return image.clone();
        }
        let (w, h) = (image.w, image.h);
        let win = (2 * r + 1) as u32;
        // horizontal pass
        let mut tmp = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                for dx in 0..(2 * r + 1) {
                    let sx = (x + dx).saturating_sub(r).min(w - 1);
                    sum += image.px[y * w + sx] as u32;
                }
                tmp[y * w + x] = (sum / win) as u8;
            }
        }
        // vertical pass
        let mut out = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                for dy in 0..(2 * r + 1) {
                    let sy = (y + dy).saturating_sub(r).min(h - 1);
                    sum += tmp[sy * w + x] as u32;
                }
                out[y * w + x] = (sum / win) as u8;
            }
        }
        Image { w, h, px: out }
    }
}

/// Blend two images: `a·(1−fac) + b·fac`.
#[derive(Serialize, Deserialize)]
pub struct MixImages;

#[node(id = "octans.image.mix", out = "image", serde)]
impl MixImages {
    fn process(&self, a: &Image, b: &Image, #[param(default = 0.5f64)] fac: &f64) -> Image {
        let fac = fac.clamp(0.0, 1.0);
        let mut px = Vec::with_capacity(a.w * a.h);
        for y in 0..a.h {
            for x in 0..a.w {
                let pa = px_of(a, x, y) as f64;
                let pb = px_of(b, x, y) as f64;
                px.push((pa * (1.0 - fac) + pb * fac).round() as u8);
            }
        }
        Image { w: a.w, h: a.h, px }
    }
}

/// Per-pixel math on two images.
#[derive(Serialize, Deserialize, NodeParams)]
pub struct ImageMath {
    /// The blend operation.
    #[param(options = "add,subtract,multiply,screen,difference,lighten,darken")]
    pub op: String,
}

#[node(id = "octans.image.math", out = "image", serde, params)]
impl ImageMath {
    fn process(&self, a: &Image, b: &Image) -> Image {
        let f: fn(f64, f64) -> f64 = match self.op.as_str() {
            "add" => |x, y| x + y,
            "subtract" => |x, y| x - y,
            "multiply" => |x, y| x * y / 255.0,
            "screen" => |x, y| 255.0 - (255.0 - x) * (255.0 - y) / 255.0,
            "difference" => |x, y| (x - y).abs(),
            "lighten" => f64::max,
            "darken" => f64::min,
            _ => |x, _| x,
        };
        let mut px = Vec::with_capacity(a.w * a.h);
        for y in 0..a.h {
            for x in 0..a.w {
                let v = f(px_of(a, x, y) as f64, px_of(b, x, y) as f64);
                px.push(v.round().clamp(0.0, 255.0) as u8);
            }
        }
        Image { w: a.w, h: a.h, px }
    }
}

pub fn register_image_factories(reg: &mut NodeRegistry) {
    reg.register_serde::<Invert>("octans.image.invert");
    reg.register_serde::<BrightContrast>("octans.image.bright_contrast");
    reg.register_serde::<Gamma>("octans.image.gamma");
    reg.register_serde::<BoxBlur>("octans.image.box_blur");
    reg.register_serde::<MixImages>("octans.image.mix");
    reg.register_serde::<ImageMath>("octans.image.math");
}

pub fn register_image_catalog(cat: &mut Catalog) {
    cat.add(|| Invert);
    cat.add(|| BrightContrast {
        brightness: 0.0,
        contrast: 0.0,
    });
    cat.add(|| Gamma { gamma: 1.0 });
    cat.add(|| BoxBlur { radius: 2 });
    cat.add(|| MixImages);
    cat.add(|| ImageMath { op: "add".into() });
}
