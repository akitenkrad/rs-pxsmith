//! 実データ枠のための自前レンダラ (区分 `render`)．
//!
//! 合成スプライト ([`crate::sprite`]) は**平坦な 13 色**でできている．実運用で持ち込まれる
//! 画像はそうではない — 3D レンダを縮小したものは陰影の階調とアンチエイリアスを持ち，
//! 数百色になる．格子推定にとってこの差は小さくない．
//!
//! | | 合成スプライト | レンダ |
//! | --- | --- | --- |
//! | 色数 | 13 | 数百 |
//! | 隣り合うセルの差 | 大きい (別の色) | 小さい (階調が連続する) |
//! | 縁 | 硬い | アンチエイリアスが乗る |
//!
//! 隣接セルの差が小さいと $\bar{V}_{\mathrm{image}}$ が下がり，信頼度の分母が変わる．
//! **合成データだけでは見えない場面**であり，実データを別枠で持つ理由そのものである．
//!
//! レンダは球と床のレイトレーシングで，影とスペキュラを持つ．3x3 の超標本から
//! 箱フィルタで縮小してアンチエイリアスを作る．自作なので CC0 である．

use px_core::{Rgba8, RgbaCanvas};

use crate::rng::Rng;

/// 元絵の一辺の候補 (縮小後)．
pub const SIZES: [u32; 4] = [32, 40, 48, 56];

/// 1 画素あたりの超標本 (縦横)．
const SUPERSAMPLE: u32 = 3;

#[derive(Copy, Clone, Debug)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    fn scale(self, k: f32) -> Self {
        Self::new(self.x * k, self.y * k, self.z * k)
    }
    fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    fn normalized(self) -> Self {
        let n = self.length();
        if n <= f32::EPSILON {
            self
        } else {
            self.scale(1.0 / n)
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct Sphere {
    center: Vec3,
    radius: f32,
    color: Vec3,
}

impl Sphere {
    /// 交差する最小の $t$．無ければ `None`．
    fn hit(&self, origin: Vec3, dir: Vec3) -> Option<f32> {
        let oc = origin.sub(self.center);
        let b = oc.dot(dir);
        let c = oc.dot(oc) - self.radius * self.radius;
        let disc = b * b - c;
        if disc < 0.0 {
            return None;
        }
        let t = -b - disc.sqrt();
        (t > 1.0e-3).then_some(t)
    }
}

struct Scene {
    spheres: Vec<Sphere>,
    light: Vec3,
    floor_y: f32,
    sky: (Vec3, Vec3),
}

fn scene(rng: &mut Rng) -> Scene {
    let unit = |r: &mut Rng| r.below(1000) as f32 / 1000.0;
    let spheres = (0..rng.range(2, 4))
        .map(|i| {
            let r = 0.5 + unit(rng) * 0.6;
            Sphere {
                center: Vec3::new(
                    -1.6 + unit(rng) * 3.2,
                    r - 1.0 + unit(rng) * 0.4,
                    -3.0 - i as f32 * 0.8 - unit(rng) * 1.5,
                ),
                radius: r,
                color: Vec3::new(
                    0.25 + unit(rng) * 0.75,
                    0.25 + unit(rng) * 0.75,
                    0.25 + unit(rng) * 0.75,
                ),
            }
        })
        .collect();
    Scene {
        spheres,
        light: Vec3::new(-0.6 + unit(rng) * 1.2, 0.7 + unit(rng) * 0.5, 0.4).normalized(),
        floor_y: -1.0,
        sky: (
            Vec3::new(
                0.10 + unit(rng) * 0.2,
                0.12 + unit(rng) * 0.2,
                0.25 + unit(rng) * 0.3,
            ),
            Vec3::new(0.45 + unit(rng) * 0.3, 0.5 + unit(rng) * 0.3, 0.7),
        ),
    }
}

/// 影に入っているか (点光源ではなく平行光)．
fn shadowed(scene: &Scene, point: Vec3) -> bool {
    scene
        .spheres
        .iter()
        .any(|s| s.hit(point, scene.light).is_some())
}

fn trace(scene: &Scene, origin: Vec3, dir: Vec3) -> Vec3 {
    let mut best: Option<(f32, Vec3, Vec3)> = None; // (t, 法線, 色)

    for s in &scene.spheres {
        if let Some(t) = s.hit(origin, dir)
            && best.is_none_or(|(bt, _, _)| t < bt)
        {
            let p = origin.add(dir.scale(t));
            best = Some((t, p.sub(s.center).normalized(), s.color));
        }
    }

    // 床 (市松模様)
    if dir.y < -1.0e-4 {
        let t = (scene.floor_y - origin.y) / dir.y;
        if t > 1.0e-3 && best.is_none_or(|(bt, _, _)| t < bt) {
            let p = origin.add(dir.scale(t));
            let check = ((p.x * 0.7).floor() as i32 + (p.z * 0.7).floor() as i32).rem_euclid(2);
            let base = if check == 0 { 0.72 } else { 0.30 };
            best = Some((
                t,
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(base, base * 0.95, base * 0.85),
            ));
        }
    }

    let Some((t, normal, albedo)) = best else {
        // 空はゆるい縦のグラデーション
        let k = (dir.y * 0.5 + 0.5).clamp(0.0, 1.0);
        let (a, b) = scene.sky;
        return a.scale(1.0 - k).add(b.scale(k));
    };

    let point = origin.add(dir.scale(t));
    let diffuse = normal.dot(scene.light).max(0.0);
    let lit = if shadowed(scene, point) { 0.0 } else { diffuse };
    // 環境光 + 拡散 + ゆるいスペキュラ
    let half = scene.light.sub(dir).normalized();
    let spec = if lit > 0.0 {
        normal.dot(half).max(0.0).powf(24.0) * 0.35
    } else {
        0.0
    };
    albedo
        .scale(0.22 + 0.78 * lit)
        .add(Vec3::new(spec, spec, spec))
}

fn to_u8(v: f32) -> u8 {
    // ガンマ 2.2 で表示側へ
    (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8
}

/// 種から 1 枚レンダする．同じ種からは必ず同じ絵が出る．
pub fn render(seed: u64) -> RgbaCanvas {
    let mut rng = Rng::new(seed);
    let w = *rng.pick(&SIZES);
    let h = *rng.pick(&SIZES);
    let sc = scene(&mut rng);

    let (sw, sh) = (w * SUPERSAMPLE, h * SUPERSAMPLE);
    let origin = Vec3::new(0.0, 0.0, 0.0);
    let aspect = sw as f32 / sh as f32;

    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            // 超標本を箱フィルタで畳む — これがアンチエイリアスになる
            let mut acc = Vec3::new(0.0, 0.0, 0.0);
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let px = (x * SUPERSAMPLE + sx) as f32 + 0.5;
                    let py = (y * SUPERSAMPLE + sy) as f32 + 0.5;
                    let u = (px / sw as f32 * 2.0 - 1.0) * aspect;
                    let v = 1.0 - py / sh as f32 * 2.0;
                    let dir = Vec3::new(u, v, -1.6).normalized();
                    acc = acc.add(trace(&sc, origin, dir));
                }
            }
            let n = (SUPERSAMPLE * SUPERSAMPLE) as f32;
            let c = acc.scale(1.0 / n);
            pixels.push(Rgba8::new(to_u8(c.x), to_u8(c.y), to_u8(c.z), 255));
        }
    }
    RgbaCanvas::from_pixels(w, h, pixels).expect("画素数は w*h で作っている")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn colors(c: &RgbaCanvas) -> BTreeSet<(u8, u8, u8)> {
        c.pixels().iter().map(|p| (p.r, p.g, p.b)).collect()
    }

    #[test]
    fn the_same_seed_gives_the_same_image() {
        assert_eq!(render(3).pixels(), render(3).pixels());
    }

    #[test]
    fn a_render_has_far_more_colours_than_a_synthetic_sprite() {
        // これが実データ枠を持つ理由である — 合成スプライトは 13 色しかない
        for seed in 0..6 {
            let n = colors(&render(seed)).len();
            assert!(n > 100, "seed {seed} が {n} 色しかない — 階調が出ていない");
        }
    }

    #[test]
    fn every_pixel_is_opaque() {
        assert!(render(1).pixels().iter().all(|p| p.a == 255));
    }

    #[test]
    fn sizes_stay_in_the_expected_range() {
        for seed in 0..6 {
            let c = render(seed);
            assert!(SIZES.contains(&c.width()) && SIZES.contains(&c.height()));
        }
    }

    #[test]
    fn neighbouring_pixels_differ_less_than_in_a_sprite() {
        // 階調が連続するぶん，隣り合う画素の差は合成スプライトより小さいはず．
        // 信頼度の分母 (画像全体の分散) が変わるので，分布のずれとして効いてくる
        let mean_step = |c: &px_core::RgbaCanvas| {
            let mut acc = 0u64;
            let mut n = 0u64;
            for y in 0..c.height() as i32 {
                for x in 1..c.width() as i32 {
                    let (a, b) = (c.get(x - 1, y).unwrap(), c.get(x, y).unwrap());
                    acc += u64::from(a.r.abs_diff(b.r))
                        + u64::from(a.g.abs_diff(b.g))
                        + u64::from(a.b.abs_diff(b.b));
                    n += 1;
                }
            }
            acc as f64 / n as f64
        };
        let rendered: f64 = (0..6).map(|s| mean_step(&render(s))).sum::<f64>() / 6.0;
        let sprites: f64 = (0..6)
            .map(|s| mean_step(&crate::sprite::synthesize(s)))
            .sum::<f64>()
            / 6.0;
        assert!(
            rendered < sprites,
            "レンダ {rendered:.1} が合成スプライト {sprites:.1} より粗い"
        );
    }
}
