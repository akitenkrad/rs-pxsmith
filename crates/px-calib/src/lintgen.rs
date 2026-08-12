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

    /// **敷ける面が要る欠陥か．** ディザ系は透明の穴があると検出に届かない．
    pub fn needs_solid_area(self) -> bool {
        matches!(
            self,
            Self::DitherClump | Self::DitherFlood | Self::HarshDither
        )
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
        // 市松のディザを敷き，その中で片方の色を横に長く続ける．
        //
        // **欠陥は検出窓より十分大きくする．** ディザの検出は 8 画素角の窓で
        // «2 色が 92% 以上» を求めるので，16x16 の絵に中央 8x8 だけ敷くと窓が
        // 領域の外へはみ出して 1 枚も見つからない — 実際 16x16 の 3 枚が
        // «ルールの見逃し» ではなく **生成の弱さ**で鳴っていなかった
        Defect::DitherClump => {
            let other = shifted(base, 0.18);
            let (w, h) = (img.width() as i32, img.height() as i32);
            // **窓の格子に揃える．** 検出は 8 画素角の窓を 8 画素刻みで置くので，
            // 領域が格子からずれていると窓が領域の外へはみ出し，16x16 では
            // 1 つも収まらない — «ルールの見逃し» に見えていた 2 枚はこれだった
            let margin = |n: i32| if n >= 32 { 8 } else { 0 };
            let (x0, y0) = (margin(w), margin(h));
            let (x1, y1) = (w - margin(w), h - margin(h));
            for y in y0..y1 {
                for x in x0..x1 {
                    if img.get(x, y).is_some_and(|c| c.a == 0) {
                        continue;
                    }
                    // 塊: 一定の行では片方の色だけを並べる
                    let clumped = (y - y0) % 5 == 0;
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
///
/// **明るい方へ寄せられないときは暗い方へ寄せる．** 明度を上げるだけだと，元が
/// すでに明るい絵で丸めた後に**同じ色**になり，ディザではなく塗り潰しになる —
/// 実際にそれで 2 枚がルール 10 に鳴っていなかった (鳴っていたのは «大面積の
/// 高彩度色» の方) ．
fn shifted(base: Rgba8, dl: f32) -> Rgba8 {
    let lab = oklab_of(base);
    for delta in [dl, -dl, dl * 2.0, -dl * 2.0] {
        let mut moved = lab;
        moved.l = (lab.l + delta).clamp(0.0, 1.0);
        let mut c = px_core::quantize::oklab_to_rgba(moved);
        c.a = base.a;
        if (c.r, c.g, c.b) != (base.r, base.g, base.b) {
            return c;
        }
    }
    // どうしても動かない (完全な白か黒) ときは反対側の端へ振る
    let mut c = if lab.l > 0.5 {
        Rgba8::rgb(0x20, 0x20, 0x28)
    } else {
        Rgba8::rgb(0xe0, 0xe0, 0xd8)
    };
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

    // 透けている割合を先に測る．**ディザ系の欠陥は «敷ける面» が要る** — 検出は
    // 8 画素角の窓で «2 色が 92% 以上» を求めるので，透明で穴が空くと見つからない．
    // 16x16 の絵で 2 枚が «ルールの見逃し» ではなくこれで鳴っていなかった
    let mut art: Vec<(std::path::PathBuf, RgbaCanvas, f32)> = Vec::new();
    for path in &paths {
        let img = px_io::png::read_rgba(path)
            .with_context(|| format!("{} を読めない", path.display()))?;
        let opaque = img.pixels().iter().filter(|c| c.a != 0).count() as f32;
        let ratio = opaque / (img.width() * img.height()).max(1) as f32;
        art.push((path.clone(), img, ratio));
    }
    let solid: Vec<usize> = (0..art.len()).filter(|&i| art[i].2 >= 0.98).collect();
    anyhow::ensure!(!solid.is_empty(), "ディザを敷ける絵が 1 枚も無い");

    let mut written = 0;
    for defect in Defect::ALL {
        for i in 0..per_defect {
            // 種ごとに違う絵を使う．**添字で選ぶので毎回同じ絵になる**
            let pick = defect.rule() as usize * 13 + i * 7;
            let index = if defect.needs_solid_area() {
                solid[pick % solid.len()]
            } else {
                pick % art.len()
            };
            let (path, src, _) = &art[index];
            let _ = path;
            let img = apply(src, defect, seed ^ (defect.rule() as u64) << 8 ^ i as u64);
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
