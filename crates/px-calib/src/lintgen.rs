//! **lint の負例を作る．**
//!
//! 「良い絵に現行の閾値を掛ける」で分かるのは**誤爆**だけである．ルールが本来の相手を
//! 捕まえるかは，**わざと壊した絵**でしか測れない．手描きの失敗例は用意できないので，
//! CC0 の実物のドット絵に**狙った欠陥を 1 つだけ**入れて作る (`degrade` と同じ考え方) ．
//!
//! - 元絵は良い絵なので，鳴ったルールは**入れた欠陥のせい**だと言える
//! - 生成は種で決まる (決定論性の規則 1)
//! - **1 枚に 1 種類の欠陥**しか入れない．混ぜると «どのルールが何を見たか» が分からない

use std::path::Path;

use anyhow::{Context, Result};
use px_core::canvas::RgbaCanvas;
use px_core::color::{Rgba8, oklab_of};

use crate::rng::Rng;

/// 入れる欠陥の種類．**ルール 1 つに 1 種類**を対応させる．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Defect {
    /// ルール 3 — どこにも無い色の 1 画素を撒く．
    StrayPixels,
    /// ルール 10 — ディザの中で同じ色を長く続ける (塊化)．
    DitherClump,
    /// ルール 15 — 画面の大半をディザで埋める．
    DitherFlood,
    /// ルール 11 — 隣り合う面の明度差を潰す．
    FlatLightness,
    /// ルール 16 — 大面積を高彩度で塗る．
    LoudFill,
    /// ルール 17 — 明度が大きく離れた 2 色でディザを敷く．
    HarshDither,
    /// ルール 18 — 純黒を混ぜる．
    PureBlack,
}

impl Defect {
    pub const ALL: [Defect; 7] = [
        Defect::StrayPixels,
        Defect::DitherClump,
        Defect::DitherFlood,
        Defect::FlatLightness,
        Defect::LoudFill,
        Defect::HarshDither,
        Defect::PureBlack,
    ];

    /// 狙っているルール番号．
    pub fn rule(self) -> u8 {
        match self {
            Self::StrayPixels => 3,
            Self::DitherClump => 10,
            Self::DitherFlood => 15,
            Self::FlatLightness => 11,
            Self::LoudFill => 16,
            Self::HarshDither => 17,
            Self::PureBlack => 18,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::StrayPixels => "stray",
            Self::DitherClump => "clump",
            Self::DitherFlood => "flood",
            Self::FlatLightness => "flat",
            Self::LoudFill => "loud",
            Self::HarshDither => "harsh",
            Self::PureBlack => "black",
        }
    }
}

/// 絵の中で最も広い不透明な色 (置き換えの相手にする)．
fn dominant(img: &RgbaCanvas) -> Option<Rgba8> {
    let mut counts: std::collections::BTreeMap<[u8; 4], usize> = std::collections::BTreeMap::new();
    for p in img.pixels() {
        if p.a == 0 {
            continue;
        }
        *counts.entry([p.r, p.g, p.b, p.a]).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(c, n)| (*n, std::cmp::Reverse(*c)))
        .map(|(c, _)| Rgba8 {
            r: c[0],
            g: c[1],
            b: c[2],
            a: c[3],
        })
}

/// 不透明な画素の座標を集める．
fn opaque_pixels(img: &RgbaCanvas) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for y in 0..img.height() as i32 {
        for x in 0..img.width() as i32 {
            if img.get(x, y).is_some_and(|c| c.a != 0) {
                out.push((x, y));
            }
        }
    }
    out
}

/// 欠陥を 1 つ入れる．**元絵は壊さない** (複製に入れる)．
pub fn apply(src: &RgbaCanvas, defect: Defect, seed: u64) -> RgbaCanvas {
    let mut img = src.clone();
    let mut rng = Rng::new(seed);
    let pixels = opaque_pixels(src);
    if pixels.is_empty() {
        return img;
    }
    let base = dominant(src).unwrap_or(Rgba8::rgb(128, 128, 128));
    let lab = oklab_of(base);

    match defect {
        // どこにも無い色を 1 画素だけ置く．**色は 1 点ごとに変える** — 同じ色を
        // 2 度使うと «その色は他にもある» ことになりルール 3 の相手ではなくなる
        Defect::StrayPixels => {
            for i in 0..12u32 {
                let (x, y) = pixels[rng.below(pixels.len() as u32) as usize];
                let c = Rgba8::rgb(
                    (17 + i * 19) as u8,
                    (211 - i * 13) as u8,
                    (73 + i * 7) as u8,
                );
                img.set(x, y, c);
            }
        }
        // 市松のディザを敷き，その中で片方の色を横に長く続ける
        Defect::DitherClump => {
            let other = shifted(base, 0.18);
            let (w, h) = (img.width() as i32, img.height() as i32);
            let (x0, y0) = (w / 4, h / 4);
            for y in y0..(y0 + h / 2).min(h) {
                for x in x0..(x0 + w / 2).min(w) {
                    if img.get(x, y).is_some_and(|c| c.a == 0) {
                        continue;
                    }
                    // 塊: 一定の行では片方の色だけを並べる
                    let clumped = (y - y0) % 5 == 0 && (x - x0) < w / 2;
                    let use_other = if clumped { true } else { (x + y) % 2 == 0 };
                    img.set(x, y, if use_other { other } else { base });
                }
            }
        }
        // 画面のほとんどをディザで埋める
        Defect::DitherFlood => {
            let other = shifted(base, 0.12);
            for &(x, y) in &pixels {
                img.set(x, y, if (x + y) % 2 == 0 { base } else { other });
            }
        }
        // 隣り合う面の明度差を潰す (どの色も base とほぼ同じ明度にする)
        Defect::FlatLightness => {
            for &(x, y) in &pixels {
                let Some(c) = img.get(x, y) else { continue };
                let mut l = oklab_of(c);
                // 明度だけ base に寄せる — 色相 ・彩度は残す
                l.l = lab.l + (l.l - lab.l) * 0.05;
                let mut c2 = px_core::quantize::oklab_to_rgba(l);
                c2.a = c.a;
                img.set(x, y, c2);
            }
        }
        // 大面積を高彩度で塗る
        Defect::LoudFill => {
            let loud = Rgba8::rgb(0xff, 0x18, 0x08);
            let (w, h) = (img.width() as i32, img.height() as i32);
            for y in 0..h {
                for x in 0..w {
                    if y >= h / 3 && img.get(x, y).is_some_and(|c| c.a != 0) {
                        img.set(x, y, loud);
                    }
                }
            }
        }
        // 明度が大きく離れた 2 色でディザを敷く
        Defect::HarshDither => {
            let (dark, light) = (Rgba8::rgb(0x14, 0x12, 0x1a), Rgba8::rgb(0xf4, 0xf2, 0xe8));
            for &(x, y) in &pixels {
                img.set(x, y, if (x + y) % 2 == 0 { dark } else { light });
            }
        }
        // 純黒を混ぜる
        Defect::PureBlack => {
            let (w, h) = (img.width() as i32, img.height() as i32);
            for y in 0..h {
                for x in 0..w {
                    if img.get(x, y).is_some_and(|c| c.a != 0) && (x + y) % 7 == 0 {
                        img.set(x, y, Rgba8::rgb(0, 0, 0));
                    }
                }
            }
        }
    }
    img
}

/// 明度を少しずらした色 (ディザの相方に使う)．
fn shifted(base: Rgba8, dl: f32) -> Rgba8 {
    let mut lab = oklab_of(base);
    lab.l = (lab.l + dl).clamp(0.0, 1.0);
    let mut c = px_core::quantize::oklab_to_rgba(lab);
    c.a = base.a;
    c
}

/// 種のディレクトリから負例を書き出す．
pub fn generate(seeds: &Path, out: &Path, per_defect: usize, seed: u64) -> Result<usize> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(seeds)
        .with_context(|| format!("{} を読めない", seeds.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    paths.sort();
    anyhow::ensure!(!paths.is_empty(), "{} に PNG が無い", seeds.display());
    std::fs::create_dir_all(out)?;

    let mut written = 0;
    for defect in Defect::ALL {
        for i in 0..per_defect {
            // 種ごとに違う絵を使う．**添字で選ぶので毎回同じ絵になる**
            let path = &paths[(defect.rule() as usize * 13 + i * 7) % paths.len()];
            let src = px_io::png::read_rgba(path)
                .with_context(|| format!("{} を読めない", path.display()))?;
            let img = apply(&src, defect, seed ^ (defect.rule() as u64) << 8 ^ i as u64);
            let name = format!("{}-{i:02}.png", defect.as_str());
            px_io::png::write_rgba(out.join(&name), &img)
                .with_context(|| format!("{name} を書けない"))?;
            written += 1;
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art() -> RgbaCanvas {
        let mut img = RgbaCanvas::filled(16, 16, Rgba8::TRANSPARENT);
        for y in 0..16 {
            for x in 0..16 {
                let v = ((x * 31 + y * 17) % 5) as u8;
                img.set(x, y, Rgba8::rgb(40 + v * 40, 60 + v * 30, 120 - v * 20));
            }
        }
        img
    }

    #[test]
    fn a_defect_changes_the_art_and_keeps_the_size() {
        for defect in Defect::ALL {
            let src = art();
            let out = apply(&src, defect, 7);
            assert_eq!((out.width(), out.height()), (src.width(), src.height()));
            assert_ne!(
                out.pixels(),
                src.pixels(),
                "{defect:?} で何も変わっていない"
            );
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_art() {
        let src = art();
        let a = apply(&src, Defect::StrayPixels, 3);
        let b = apply(&src, Defect::StrayPixels, 3);
        assert_eq!(a.pixels(), b.pixels());
    }

    /// 迷子の画素は**それぞれ違う色**でなければならない — 同じ色を 2 度使うと
    /// «その色は他にもある» ことになり，ルール 3 の相手ではなくなる．
    #[test]
    fn stray_pixels_do_not_share_a_colour() {
        let src = art();
        let out = apply(&src, Defect::StrayPixels, 5);
        let mut added: Vec<[u8; 4]> = Vec::new();
        for (a, b) in out.pixels().iter().zip(src.pixels()) {
            if a != b {
                added.push([a.r, a.g, a.b, a.a]);
            }
        }
        let mut uniq = added.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(added.len(), uniq.len(), "同じ色の迷子が複数ある");
    }
}
