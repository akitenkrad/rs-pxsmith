//! 合成ドット絵の生成 (評価データセットの元絵)．
//!
//! 格子推定の正解を持つ評価データは「きれいなドット絵を劣化させたもの」なので，
//! まず劣化前の絵が要る．実素材 (`testdata/aseprite/`) は 3x3 など極端に小さいものを
//! 含み，6 倍に拡大しても格子推定が退化する大きさにしかならないため，元絵は
//! ここで合成する．**実データ 20〜30 件は別枠**であって，これの代わりではない．
//!
//! 満たすべき性質は次の 3 つである．
//!
//! | 性質 | 理由 |
//! | --- | --- |
//! | 画素はすべてパレットの色そのもの (中間色を作らない) | 劣化前の格子が厳密であること |
//! | 不透明 | JPEG がアルファを持てない．合成の段階で背景を敷いておく |
//! | 平坦すぎない | 完全に平坦な画像は信頼度 0 の退化ケース (設計書 6.1) |

use anyhow::{Context, Result};
use px_core::{Rgba8, RgbaCanvas};

use crate::rng::Rng;

/// 種にする実物のドット絵 1 枚．
#[derive(Clone, Debug)]
pub struct Seed {
    /// ファイル名 (目録に残して再現できるようにする)．
    pub name: String,
    /// **不透明化する前**の絵．背景は使う側が敷く．
    pub art: RgbaCanvas,
}

/// 種を敷く背景色．
///
/// 透明を黒で潰すと輪郭の外側が真っ黒な平面になり，実物より易しくなる (セル境界が
/// くっきり出る) ．**中間的な明るさを何通りか用意して散らす**．
const BACKGROUNDS: [Rgba8; 4] = [
    Rgba8::rgb(38, 42, 52),
    Rgba8::rgb(96, 92, 84),
    Rgba8::rgb(150, 148, 140),
    Rgba8::rgb(64, 78, 62),
];

/// 種を読む．**名前順で固定する** — 並びが変わると同じ目録から別の絵が出る．
///
/// 整数倍で拡大された絵は種にしない．そのまま $s$ 倍すると $s$ と $ks$ の格子が
/// 両方成立し，**正解が一意に決まらない** (`ingest --native` と同じ理由)．
pub fn load_seeds(dir: &std::path::Path) -> Result<Vec<Seed>> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("{} を読めない", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    paths.sort();

    let mut out = Vec::new();
    for path in paths {
        let art = px_io::png::read_rgba(&path)
            .with_context(|| format!("{} を読めない", path.display()))?;
        if let Some(k) = crate::ingest::integer_block_size(&art) {
            anyhow::bail!(
                "{} は {k} 倍に拡大された絵である．種は元絵の解像度でなければならない",
                path.display()
            );
        }
        out.push(Seed {
            name: path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            art,
        });
    }
    anyhow::ensure!(!out.is_empty(), "{} に PNG が無い", dir.display());
    Ok(out)
}

/// 背景を敷いて不透明にする．**JPEG がアルファを持てない**ので劣化の前に要る．
pub fn flatten(art: &RgbaCanvas, background: Rgba8) -> RgbaCanvas {
    let mut out = RgbaCanvas::filled(art.width(), art.height(), background);
    for y in 0..art.height() as i32 {
        for x in 0..art.width() as i32 {
            if let Some(c) = art.get(x, y)
                && c.a > 0
            {
                // 半透明は劣化前に潰す — 中間色を作らないという性質を保つため，
                // アルファは 2 値として扱う (作業層の不変条件 D4 と揃える)
                out.set(x, y, Rgba8::new(c.r, c.g, c.b, 255));
            }
        }
    }
    out
}

/// 種から元絵を 1 枚取り出す (背景を敷いて不透明にしたもの)．
pub fn from_seed(seeds: &[Seed], seed: u64) -> (String, RgbaCanvas) {
    let mut rng = Rng::new(seed);
    let pick = &seeds[rng.below(seeds.len() as u32) as usize];
    let bg = BACKGROUNDS[rng.below(BACKGROUNDS.len() as u32) as usize];
    (pick.name.clone(), flatten(&pick.art, bg))
}

/// 元絵の一辺の候補．L0 の助言上限 48 に合わせてある (設計書 4.1)．
pub const SIZES: [u32; 5] = [16, 24, 32, 40, 48];

/// 種から元絵を 1 枚作る．同じ種からは必ず同じ絵が出る．
pub fn synthesize(seed: u64) -> RgbaCanvas {
    let mut rng = Rng::new(seed);
    let w = *rng.pick(&SIZES);
    let h = *rng.pick(&SIZES);
    let palette = palette(&mut rng);

    let mut bmp = Bitmap::new(w, h);
    let ramps = (palette.len() as u8 - 1) / 4;

    // 1. 大きな矩形をいくつか置く — 平坦な面と直線の境界を作る
    for _ in 0..rng.range(2, 4) {
        let ramp = rng.below(u32::from(ramps)) as u8;
        let step = rng.below(4) as u8;
        let (x0, y0) = (rng.below(w), rng.below(h));
        let (rw, rh) = (rng.range(3, w / 2), rng.range(3, h / 2));
        bmp.fill_rect(x0, y0, rw, rh, 1 + ramp * 4 + step);
    }

    // 2. 円板を 1〜2 個 — 階段状の縁が輪郭追跡とジャギーの材料になる
    for _ in 0..rng.range(1, 2) {
        let ramp = rng.below(u32::from(ramps)) as u8;
        let r = rng.range(3, (w.min(h) / 3).max(3));
        let cx = rng.range(r, w.saturating_sub(r).max(r));
        let cy = rng.range(r, h.saturating_sub(r).max(r));
        bmp.fill_disc(cx, cy, r, 1 + ramp * 4 + 2);
        // 縁を 1 段暗い色で囲む
        bmp.outline(1 + ramp * 4 + 2, 1 + ramp * 4);
    }

    // 3. 斜線 — 格子の位相がずれたときに一番効く材料
    let ramp = rng.below(u32::from(ramps)) as u8;
    bmp.diagonal(1 + ramp * 4 + 3);

    // 4. Bayer 2x2 のディザ帯 — 市松模様が格子推定を惑わせないかを見る
    let ramp = rng.below(u32::from(ramps)) as u8;
    let band = rng.below(h.saturating_sub(4).max(1));
    bmp.dither_band(band, 4.min(h), 1 + ramp * 4 + 1, 1 + ramp * 4 + 3);

    let pixels: Vec<Rgba8> = bmp.idx.iter().map(|&i| palette[i as usize]).collect();
    RgbaCanvas::from_pixels(w, h, pixels).expect("画素数は w*h で作っている")
}

/// 背景 1 色 + 4 段のランプ 3 本．すべて不透明．
fn palette(rng: &mut Rng) -> Vec<Rgba8> {
    let mut out = Vec::with_capacity(13);
    let bg_hue = rng.below(360) as f32;
    out.push(hsv(bg_hue, 0.15, 0.20));
    let base = rng.below(360) as f32;
    for r in 0..3 {
        // 3 本のランプは色相環を 3 等分した位置に置く
        let hue = (base + 120.0 * r as f32) % 360.0;
        for step in 0..4 {
            let t = step as f32 / 3.0;
            out.push(hsv(hue, 0.80 - 0.25 * t, 0.30 + 0.55 * t));
        }
    }
    out
}

/// HSV から RGB (すべて不透明)．
fn hsv(h: f32, s: f32, v: f32) -> Rgba8 {
    let c = v * s;
    let hp = (h % 360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to8 = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgba8::new(to8(r), to8(g), to8(b), 255)
}

/// 添字の作業面．描く道具はここへ生やす．
struct Bitmap {
    w: u32,
    h: u32,
    idx: Vec<u8>,
}

impl Bitmap {
    fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            idx: vec![0u8; (w * h) as usize],
        }
    }

    fn put(&mut self, x: u32, y: u32, color: u8) {
        if x < self.w && y < self.h {
            self.idx[(y * self.w + x) as usize] = color;
        }
    }

    fn get(&self, x: i64, y: i64) -> Option<u8> {
        if x < 0 || y < 0 || x >= i64::from(self.w) || y >= i64::from(self.h) {
            None
        } else {
            Some(self.idx[(y * i64::from(self.w) + x) as usize])
        }
    }

    fn fill_rect(&mut self, x0: u32, y0: u32, rw: u32, rh: u32, color: u8) {
        for y in y0..(y0 + rh).min(self.h) {
            for x in x0..(x0 + rw).min(self.w) {
                self.put(x, y, color);
            }
        }
    }

    fn fill_disc(&mut self, cx: u32, cy: u32, r: u32, color: u8) {
        let r2 = (r * r) as i64;
        for y in 0..self.h {
            for x in 0..self.w {
                let dx = i64::from(x) - i64::from(cx);
                let dy = i64::from(y) - i64::from(cy);
                if dx * dx + dy * dy <= r2 {
                    self.put(x, y, color);
                }
            }
        }
    }

    /// `color` の領域のうち，違う色に接している画素を `edge` へ置き換える (4 近傍)．
    fn outline(&mut self, color: u8, edge: u8) {
        let src = Self {
            w: self.w,
            h: self.h,
            idx: self.idx.clone(),
        };
        for y in 0..self.h {
            for x in 0..self.w {
                if src.get(i64::from(x), i64::from(y)) != Some(color) {
                    continue;
                }
                let (xi, yi) = (i64::from(x), i64::from(y));
                let touches_other = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| src.get(xi + dx, yi + dy).is_none_or(|c| c != color));
                if touches_other {
                    self.put(x, y, edge);
                }
            }
        }
    }

    /// 左上から右下への 1 画素幅の階段．
    fn diagonal(&mut self, color: u8) {
        for y in 0..self.h {
            let x = y * self.w / self.h.max(1);
            self.put(x, y, color);
        }
    }

    /// Bayer 2x2 の市松で 2 色を混ぜた帯．
    fn dither_band(&mut self, y0: u32, height: u32, a: u8, b: u8) {
        for y in y0..(y0 + height).min(self.h) {
            for x in 0..self.w {
                let color = if (x + y) % 2 == 0 { a } else { b };
                self.put(x, y, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn colors(c: &RgbaCanvas) -> BTreeSet<(u8, u8, u8, u8)> {
        c.pixels().iter().map(|p| (p.r, p.g, p.b, p.a)).collect()
    }

    #[test]
    fn the_same_seed_gives_the_same_sprite() {
        let a = synthesize(123);
        let b = synthesize(123);
        assert_eq!((a.width(), a.height()), (b.width(), b.height()));
        assert_eq!(a.pixels(), b.pixels());
    }

    #[test]
    fn different_seeds_give_different_sprites() {
        let a = synthesize(1);
        let b = synthesize(2);
        assert!(
            (a.width(), a.height()) != (b.width(), b.height()) || a.pixels() != b.pixels(),
            "種を変えても同じ絵になっている"
        );
    }

    #[test]
    fn every_pixel_is_opaque() {
        // JPEG はアルファを持てないので，劣化前から不透明でなければならない
        for seed in 0..30 {
            let c = synthesize(seed);
            assert!(
                c.pixels().iter().all(|p| p.a == 255),
                "seed {seed} に半透明の画素がある"
            );
        }
    }

    #[test]
    fn sprites_are_not_flat() {
        // 完全に平坦な画像は信頼度 0 の退化ケース (設計書 6.1) なので評価に使えない
        for seed in 0..30 {
            let c = synthesize(seed);
            let used = colors(&c);
            assert!(used.len() >= 4, "seed {seed} は {} 色しかない", used.len());
            let n = c.pixels().len();
            let most = used
                .iter()
                .map(|u| {
                    c.pixels()
                        .iter()
                        .filter(|p| (p.r, p.g, p.b, p.a) == *u)
                        .count()
                })
                .max()
                .unwrap_or(n);
            assert!(
                most * 10 <= n * 9,
                "seed {seed} は 1 色が 90% を超えている ({most}/{n})"
            );
        }
    }

    #[test]
    fn sizes_stay_within_the_advised_bounds() {
        for seed in 0..30 {
            let c = synthesize(seed);
            assert!(SIZES.contains(&c.width()) && SIZES.contains(&c.height()));
        }
    }

    #[test]
    fn colors_stay_within_the_l0_limit() {
        // 62 色を超えると L0 で表せない (D8)．元絵は L0 で書ける範囲に収める
        for seed in 0..30 {
            assert!(colors(&synthesize(seed)).len() <= 62);
        }
    }
}
