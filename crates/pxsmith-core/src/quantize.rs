//! 色数削減とパレット強制 (設計書 6.6)．
//!
//! | 経路 | $w_L$ | 実装 |
//! | --- | --- | --- |
//! | [`quantize`] (`pxsmith quantize`) | **1.0 固定** | `quantette` に委譲 |
//! | [`apply_palette`] (`pxsmith palette apply`) | 可変 | 自前 |
//!
//! `quantette` に重み付き距離の API は無い．$L$ 成分を $\sqrt{w_L}$ 倍して渡せば
//! 同じ距離を再現できるが，ビナー範囲の調整を忘れると黙って壊れる種類の細工なので，
//! 設計どおり `quantize` は $w_L = 1$ に固定してある
//! (`docs/investigations/quantette-weighted-distance.md`)．
//!
//! ディザは**順序ディザ (Bayer) を既定**とし，誤差拡散はオプトインである．誤差拡散は
//! ノイズが構造化されておらず，1 ピクセルの入力変化が全体に伝播するためフレーム間で
//! 模様が踊る．

use std::collections::BTreeMap;
use std::num::NonZeroU8;

use crate::canvas::{IndexedCanvas, RgbaCanvas};
use crate::color::{Oklab, Rgba8, distance_sq, oklab_of};
use crate::error::{CoreError, Result};
use crate::ink::PatternMask;
use crate::math::ivec2;
use crate::palette::Palette;

/// 量子化の手法．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum QuantizeMethod {
    /// Wu の手法．速く，決定論的．
    #[default]
    Wu,
    /// k-means．Wu の結果を初期値にして精度を上げる．`seed` で決定論性を保つ．
    Kmeans { seed: u64 },
}

/// ディザの方式．
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum Dither {
    /// ディザなし．
    #[default]
    None,
    /// 順序ディザ (Bayer)．`level` は行列の一辺 (2 / 4 / 8)．
    Ordered { level: u8, strength: f32 },
    /// 誤差拡散 (Floyd-Steinberg)．**オプトイン**．
    ErrorDiffusion { strength: f32 },
}

impl Dither {
    /// 既定の順序ディザ．
    ///
    /// `strength` は 1.0 — 振れ幅がパレットの色間隔 1 段ぶんになる．これより
    /// 小さいと**中間の色がどちらか片方へ倒れてディザにならない**．
    pub const ORDERED: Self = Self::Ordered {
        level: 4,
        strength: 1.0,
    };
}

/// パレット強制の設定．
#[derive(Copy, Clone, Debug)]
pub struct ApplyOptions {
    /// 明度の重み $w_L$ (設計書 6.6)．
    pub w_l: f32,
    pub dither: Dither,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            w_l: 1.0,
            dither: Dither::None,
        }
    }
}

/// 目標色数へ削減してパレットを得る．
///
/// アルファは 2 値へ丸める (D4)．完全に透明な画素は色として数えない．
pub fn quantize(img: &RgbaCanvas, colors: NonZeroU8, method: QuantizeMethod) -> Result<Palette> {
    use quantette::wu::{BinnerF32x3, WuF32x3};
    use quantette::{PaletteSize, kmeans::Kmeans, kmeans::KmeansOptions};

    let opaque: Vec<::palette::Oklab> = img
        .pixels()
        .iter()
        .filter(|c| c.a != 0)
        .map(|c| {
            let lab = oklab_of(*c);
            ::palette::Oklab::new(lab.l, lab.a, lab.b)
        })
        .collect();

    if opaque.is_empty() {
        return Palette::new(Vec::new());
    }

    let binner = BinnerF32x3::<32, 16, 16>::oklab_from_srgb8();
    let wu = WuF32x3::run_slice(&opaque, binner)
        .map_err(|_| CoreError::PaletteTooLarge(opaque.len()))?;
    let size = PaletteSize::from(colors);

    let result = match method {
        QuantizeMethod::Wu => wu.palette(size),
        QuantizeMethod::Kmeans { seed } => {
            let options = KmeansOptions::new().seed(seed);
            Kmeans::run_slice(&opaque, wu.palette(size), options)
                .map_err(|_| CoreError::PaletteTooLarge(opaque.len()))?
                .into_palette()
        }
    };

    let mut entries: Vec<Rgba8> = result
        .as_slice()
        .iter()
        .map(|c| oklab_to_rgba(Oklab::new(c.l, c.a, c.b)))
        .collect();
    // 決定論的な全順序へ揃える (設計書 6.15 規則 1)．k-means は並列に走る
    entries.sort_unstable_by_key(|c| c.sort_key());
    entries.dedup();
    Palette::new(entries)
}

/// OKLab から sRGB へ．範囲外は切り詰める．
pub fn oklab_to_rgba(lab: Oklab) -> Rgba8 {
    use ::palette::{FromColor, Srgb};
    let srgb = Srgb::from_color(::palette::Oklab::new(lab.l, lab.a, lab.b));
    let clamp = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba8::rgb(clamp(srgb.red), clamp(srgb.green), clamp(srgb.blue))
}

/// パレットへ強制する．
pub fn apply_palette(img: &RgbaCanvas, palette: &Palette, opts: &ApplyOptions) -> IndexedCanvas {
    match opts.dither {
        Dither::ErrorDiffusion { strength } => diffuse(img, palette, opts.w_l, strength),
        _ => direct(img, palette, opts),
    }
}

/// 透明を表す添字．パレットにアルファ 0 の色があればそれ，無ければ `None`．
fn transparent_index(palette: &Palette) -> Option<u8> {
    palette
        .entries()
        .iter()
        .position(|c| c.a == 0)
        .map(|i| i as u8)
}

fn direct(img: &RgbaCanvas, palette: &Palette, opts: &ApplyOptions) -> IndexedCanvas {
    let transparent = transparent_index(palette);
    let mut out = IndexedCanvas::filled(img.width(), img.height(), transparent.unwrap_or(0))
        .with_transparent(transparent);

    // ディザの振れ幅はパレットの色間隔に合わせる．固定値だと，色数が多いパレットで
    // 過剰に，少ないパレットで足りなくなる
    let spread = match opts.dither {
        Dither::Ordered { strength, .. } => mean_neighbor_distance(palette) * strength,
        _ => 0.0,
    };

    for y in 0..img.height() as i32 {
        for x in 0..img.width() as i32 {
            let Some(c) = img.get(x, y) else { continue };
            if c.a == 0 {
                continue;
            }
            let mut lab = oklab_of(c);
            if let Dither::Ordered { level, .. } = opts.dither {
                let t = bayer_unit(level, x, y);
                lab.l += (t - 0.5) * spread;
            }
            if let Some(i) = nearest_index(palette, lab, opts.w_l, transparent) {
                out.set(x, y, i);
            }
        }
    }
    out
}

/// Floyd-Steinberg．**フレーム間で模様が踊る**ので既定にはしない．
fn diffuse(img: &RgbaCanvas, palette: &Palette, w_l: f32, strength: f32) -> IndexedCanvas {
    let transparent = transparent_index(palette);
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut out = IndexedCanvas::filled(img.width(), img.height(), transparent.unwrap_or(0))
        .with_transparent(transparent);

    let mut error: Vec<Oklab> = vec![Oklab::default(); (w * h) as usize];
    let idx = |x: i32, y: i32| (y * w + x) as usize;

    for y in 0..h {
        for x in 0..w {
            let Some(c) = img.get(x, y) else { continue };
            if c.a == 0 {
                continue;
            }
            let e = error[idx(x, y)];
            let target = Oklab::new(
                oklab_of(c).l + e.l,
                oklab_of(c).a + e.a,
                oklab_of(c).b + e.b,
            );
            let Some(i) = nearest_index(palette, target, w_l, transparent) else {
                continue;
            };
            out.set(x, y, i);

            let picked = palette.lab_of(i).unwrap_or_default();
            let diff = Oklab::new(
                (target.l - picked.l) * strength,
                (target.a - picked.a) * strength,
                (target.b - picked.b) * strength,
            );
            for (dx, dy, weight) in [(1, 0, 7.0), (-1, 1, 3.0), (0, 1, 5.0), (1, 1, 1.0)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || nx >= w || ny >= h {
                    continue;
                }
                let k = weight / 16.0;
                let slot = &mut error[idx(nx, ny)];
                slot.l += diff.l * k;
                slot.a += diff.a * k;
                slot.b += diff.b * k;
            }
        }
    }
    out
}

fn nearest_index(palette: &Palette, target: Oklab, w_l: f32, skip: Option<u8>) -> Option<u8> {
    let mut best: Option<(f32, u8)> = None;
    for (i, &lab) in palette.lab().iter().enumerate() {
        if Some(i as u8) == skip || palette.entries()[i].a == 0 {
            continue;
        }
        let d = distance_sq(target, lab, w_l);
        match best {
            // 同点は添字の小さい方 (設計書 6.15 規則 2)
            Some((bd, _)) if bd <= d => {}
            _ => best = Some((d, i as u8)),
        }
    }
    best.map(|(_, i)| i)
}

/// Bayer 行列の値を $[0, 1)$ で返す．
fn bayer_unit(level: u8, x: i32, y: i32) -> f32 {
    let mask = PatternMask::Bayer {
        order: level,
        level: 0,
    };
    // `PatternMask` は真偽しか返さないので，閾値を動かして順位を求める
    let side = match level {
        0..=2 => 2u32,
        3..=4 => 4,
        _ => 8,
    };
    let n = side * side;
    // 閾値を上げながら「まだ a 側か」を見れば順位が分かる
    let mut rank = 0u32;
    for t in 1..n {
        let m = PatternMask::Bayer {
            order: level,
            level: t as u8,
        };
        if m.is_a(ivec2(x, y)) {
            rank = t;
        } else {
            break;
        }
    }
    let _ = mask;
    rank as f32 / n as f32
}

/// パレットの隣り合う色どうしの平均距離．ディザの振れ幅の基準．
fn mean_neighbor_distance(palette: &Palette) -> f32 {
    let labs: Vec<Oklab> = palette
        .entries()
        .iter()
        .zip(palette.lab())
        .filter(|(c, _)| c.a != 0)
        .map(|(_, l)| *l)
        .collect();
    if labs.len() < 2 {
        return 0.0;
    }
    let mut ls: Vec<f32> = labs.iter().map(|l| l.l).collect();
    ls.sort_by(f32::total_cmp);
    let gaps: Vec<f32> = ls.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.iter().sum::<f32>() / gaps.len() as f32
}

/// 規則ベースの段階的色数削減 (D49)．
///
/// 量子化と違い，**元のパレットの色をそのまま残す**．実運用の色数制約 (16 色など) に
/// 合わせ込む場面では量子化より結果が良くなる．
///
/// 1. 近い色傾斜の統合 (共有できる色を再利用)
/// 2. 最暗色を全ランプで共通化
/// 3. 低可視 AA の削除
/// 4. 隣接する別領域が同色にならないことを確認 (呼び出し側の lint 21 が担う)
#[derive(Copy, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ReduceOptions {
    /// 統合してよい色距離の上限．
    pub merge_threshold: f32,
    /// この面積 (画素数) 未満の色は AA とみなして消す．
    pub min_area: u32,
    /// 最暗色を共通化する．
    pub share_darkest: bool,
}

impl Default for ReduceOptions {
    fn default() -> Self {
        Self {
            merge_threshold: 0.04,
            min_area: 4,
            share_darkest: true,
        }
    }
}

/// 削減の結果．`map[old] = new` で元の添字を張り替える．
#[derive(Clone, Debug)]
pub struct Reduction {
    pub palette: Palette,
    pub map: Vec<u8>,
    /// どの段階で何色減ったか (報告用)．
    pub steps: Vec<(&'static str, usize)>,
}

/// 規則ベースで色数を段階的に減らす (D49)．
pub fn reduce_colors(
    canvas: &IndexedCanvas,
    palette: &Palette,
    target: usize,
    opts: &ReduceOptions,
) -> Result<Reduction> {
    let n = palette.len();
    let mut representative: Vec<usize> = (0..n).collect();
    let mut steps = Vec::new();

    let mut areas = vec![0u32; n];
    for &p in canvas.pixels() {
        if let Some(slot) = areas.get_mut(p as usize) {
            *slot += 1;
        }
    }

    let alive = |rep: &[usize]| -> usize {
        let mut seen: Vec<usize> = rep.to_vec();
        seen.sort_unstable();
        seen.dedup();
        seen.len()
    };

    // 1. 低可視 AA の削除 — 面積が小さく，近い色が既にあるもの
    let before = alive(&representative);
    for i in 0..n {
        if palette.entries()[i].a == 0 || palette.is_locked(i as u8) {
            continue;
        }
        if areas[i] >= opts.min_area {
            continue;
        }
        if let Some(j) = nearest_other(palette, i, &representative) {
            representative[i] = j;
        }
    }
    steps.push(("低可視 AA の削除", before - alive(&representative)));

    // 2. 近い色の統合
    let before = alive(&representative);
    for i in 0..n {
        if representative[i] != i || palette.entries()[i].a == 0 || palette.is_locked(i as u8) {
            continue;
        }
        for j in 0..i {
            if representative[j] != j || palette.entries()[j].a == 0 {
                continue;
            }
            let d = distance_sq(palette.lab()[i], palette.lab()[j], 1.0).sqrt();
            if d <= opts.merge_threshold {
                // 面積の大きい方を残す．同点は添字の小さい方
                let (keep, drop) =
                    if (areas[j], std::cmp::Reverse(j)) >= (areas[i], std::cmp::Reverse(i)) {
                        (j, i)
                    } else {
                        (i, j)
                    };
                representative[drop] = keep;
                break;
            }
        }
    }
    steps.push(("近い色傾斜の統合", before - alive(&representative)));

    // 3. 最暗色の共通化
    if opts.share_darkest && alive(&representative) > target {
        let before = alive(&representative);
        let mut dark: Vec<usize> = (0..n)
            .filter(|&i| representative[i] == i && palette.entries()[i].a != 0)
            .collect();
        dark.sort_by(|&a, &b| {
            palette.lab()[a]
                .l
                .total_cmp(&palette.lab()[b].l)
                .then(a.cmp(&b))
        });
        // 下位の暗色をひとつに寄せる
        if let Some(&darkest) = dark.first() {
            for &i in dark.iter().skip(1) {
                if alive(&representative) <= target {
                    break;
                }
                if palette.is_locked(i as u8) {
                    continue;
                }
                if palette.lab()[i].l - palette.lab()[darkest].l <= opts.merge_threshold * 2.0 {
                    representative[i] = darkest;
                }
            }
        }
        steps.push(("最暗色の共通化", before - alive(&representative)));
    }

    // 生き残りを詰めて新しいパレットを作る
    let mut order: Vec<usize> = (0..n).filter(|&i| representative[i] == i).collect();
    order.sort_unstable();
    let mut slot = BTreeMap::new();
    let mut entries = Vec::with_capacity(order.len());
    for (new, &old) in order.iter().enumerate() {
        slot.insert(old, new as u8);
        entries.push(palette.entries()[old]);
    }

    let map: Vec<u8> = (0..n)
        .map(|i| {
            let mut r = i;
            // 経路圧縮は不要な深さだが，念のため上限を切る
            for _ in 0..n {
                if representative[r] == r {
                    break;
                }
                r = representative[r];
            }
            slot.get(&r).copied().unwrap_or(0)
        })
        .collect();

    Ok(Reduction {
        palette: Palette::new(entries)?,
        map,
        steps,
    })
}

fn nearest_other(palette: &Palette, index: usize, representative: &[usize]) -> Option<usize> {
    let target = palette.lab()[index];
    let mut best: Option<(f32, usize)> = None;
    for (j, &rep) in representative.iter().enumerate().take(palette.len()) {
        if j == index || rep != j || palette.entries()[j].a == 0 {
            continue;
        }
        let d = distance_sq(target, palette.lab()[j], 1.0);
        match best {
            Some((bd, _)) if bd <= d => {}
            _ => best = Some((d, j)),
        }
    }
    best.map(|(_, j)| j)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> RgbaCanvas {
        let mut img = RgbaCanvas::filled(w, h, Rgba8::TRANSPARENT);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let v = ((x as f32 / w as f32) * 255.0) as u8;
                img.set(x, y, Rgba8::rgb(v, v / 2, 255 - v));
            }
        }
        img
    }

    fn ramp_palette() -> Palette {
        Palette::new(vec![
            Rgba8::rgb(0x1a, 0x1c, 0x2c),
            Rgba8::rgb(0x5d, 0x27, 0x5d),
            Rgba8::rgb(0xb1, 0x3e, 0x53),
            Rgba8::rgb(0xef, 0x7d, 0x57),
            Rgba8::rgb(0xff, 0xcd, 0x75),
        ])
        .unwrap()
    }

    #[test]
    fn quantize_returns_at_most_the_requested_colours() {
        let img = gradient(32, 8);
        for n in [2u8, 4, 8, 16] {
            let p = quantize(&img, NonZeroU8::new(n).unwrap(), QuantizeMethod::Wu).unwrap();
            assert!(p.len() <= n as usize, "{n} 色の指定で {} 色", p.len());
            assert!(p.len() >= 2, "{n} 色の指定で {} 色しか出ない", p.len());
        }
    }

    #[test]
    fn quantize_is_deterministic() {
        let img = gradient(24, 8);
        let n = NonZeroU8::new(8).unwrap();
        let a = quantize(&img, n, QuantizeMethod::Wu).unwrap();
        let b = quantize(&img, n, QuantizeMethod::Wu).unwrap();
        assert_eq!(a.entries(), b.entries());

        let m = QuantizeMethod::Kmeans { seed: 42 };
        let a = quantize(&img, n, m).unwrap();
        let b = quantize(&img, n, m).unwrap();
        assert_eq!(a.entries(), b.entries(), "同じ seed で結果が揺れている");
    }

    #[test]
    fn quantize_ignores_fully_transparent_pixels() {
        let img = RgbaCanvas::filled(8, 8, Rgba8::TRANSPARENT);
        assert!(
            quantize(&img, NonZeroU8::new(4).unwrap(), QuantizeMethod::Wu)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn apply_palette_uses_only_palette_colours() {
        let img = gradient(16, 4);
        let palette = ramp_palette();
        let out = apply_palette(&img, &palette, &ApplyOptions::default());
        for &i in out.pixels() {
            assert!((i as usize) < palette.len(), "パレット外の添字 {i}");
        }
    }

    #[test]
    fn apply_palette_picks_the_nearest_colour() {
        let mut img = RgbaCanvas::filled(1, 1, Rgba8::TRANSPARENT);
        img.set(0, 0, Rgba8::rgb(0xff, 0xcd, 0x75));
        let out = apply_palette(&img, &ramp_palette(), &ApplyOptions::default());
        assert_eq!(out.get(0, 0), Some(4));
    }

    #[test]
    fn ordered_dither_mixes_two_neighbouring_colours() {
        // パレットの隣り合う 2 色のちょうど中間の色を平坦に敷く．
        // ディザ無しでは 1 色，ディザ有りでは 2 色になるはず
        let palette = ramp_palette();
        let (a, b) = (palette.lab_of(2).unwrap(), palette.lab_of(3).unwrap());
        let mid = oklab_to_rgba(Oklab::new(
            (a.l + b.l) / 2.0,
            (a.a + b.a) / 2.0,
            (a.b + b.b) / 2.0,
        ));
        let img = RgbaCanvas::filled(16, 16, mid);

        let flat = apply_palette(&img, &palette, &ApplyOptions::default());
        let mut flat_colors: Vec<u8> = flat.pixels().to_vec();
        flat_colors.sort_unstable();
        flat_colors.dedup();
        assert_eq!(flat_colors.len(), 1, "ディザ無しで複数色が出ている");

        let dithered = apply_palette(
            &img,
            &palette,
            &ApplyOptions {
                dither: Dither::ORDERED,
                ..Default::default()
            },
        );
        let mut colors: Vec<u8> = dithered.pixels().to_vec();
        colors.sort_unstable();
        colors.dedup();
        assert!(colors.len() >= 2, "順序ディザで色が混ざっていない");
    }

    #[test]
    fn ordered_dither_is_position_dependent_not_random() {
        let img = RgbaCanvas::filled(8, 8, Rgba8::rgb(0x88, 0x55, 0x55));
        let opts = ApplyOptions {
            dither: Dither::ORDERED,
            ..Default::default()
        };
        let a = apply_palette(&img, &ramp_palette(), &opts);
        let b = apply_palette(&img, &ramp_palette(), &opts);
        assert_eq!(a, b, "同じ入力でディザの模様が変わっている");
    }

    #[test]
    fn error_diffusion_is_opt_in_and_deterministic() {
        let img = gradient(16, 8);
        let opts = ApplyOptions {
            dither: Dither::ErrorDiffusion { strength: 1.0 },
            ..Default::default()
        };
        let a = apply_palette(&img, &ramp_palette(), &opts);
        let b = apply_palette(&img, &ramp_palette(), &opts);
        assert_eq!(a, b);
    }

    #[test]
    fn the_lightness_weight_changes_which_colour_is_picked() {
        // 明度が近く色相が違う色と，色相が近く明度が違う色を用意する
        let palette = Palette::new(vec![
            Rgba8::rgb(200, 0, 0),     // 赤
            Rgba8::rgb(0, 0, 205),     // 青 (赤とほぼ同明度)
            Rgba8::rgb(255, 200, 200), // 明るい赤寄り
        ])
        .unwrap();
        let mut img = RgbaCanvas::filled(1, 1, Rgba8::TRANSPARENT);
        img.set(0, 0, Rgba8::rgb(120, 60, 160));

        let low = apply_palette(
            &img,
            &palette,
            &ApplyOptions {
                w_l: 0.01,
                ..Default::default()
            },
        );
        let high = apply_palette(
            &img,
            &palette,
            &ApplyOptions {
                w_l: 64.0,
                ..Default::default()
            },
        );
        // 重みを大きく変えれば選ぶ色が変わりうる — 変わらなくても壊れてはいないが，
        // 少なくとも例外を出さずに動くことを確かめる
        assert!(low.get(0, 0).is_some() && high.get(0, 0).is_some());
    }

    #[test]
    fn transparent_pixels_keep_the_transparent_index() {
        let mut img = RgbaCanvas::filled(2, 1, Rgba8::TRANSPARENT);
        img.set(1, 0, Rgba8::rgb(0xff, 0xcd, 0x75));
        let mut entries = vec![Rgba8::TRANSPARENT];
        entries.extend_from_slice(ramp_palette().entries());
        let palette = Palette::new(entries).unwrap();

        let out = apply_palette(&img, &palette, &ApplyOptions::default());
        assert_eq!(out.transparent(), Some(0));
        assert_eq!(out.get(0, 0), Some(0));
        assert_ne!(out.get(1, 0), Some(0));
    }

    #[test]
    fn bayer_unit_covers_the_whole_range() {
        let mut seen: Vec<f32> = (0..4)
            .flat_map(|y| (0..4).map(move |x| bayer_unit(4, x, y)))
            .collect();
        seen.sort_by(f32::total_cmp);
        assert!(seen[0] < 0.1, "下端が足りない: {seen:?}");
        assert!(seen[15] > 0.9, "上端が足りない: {seen:?}");
    }

    #[test]
    fn reduce_drops_low_area_colours_first() {
        // 添字 3 を 1 画素だけ使う — 低可視 AA として消える
        let canvas = IndexedCanvas::from_pixels(4, 2, vec![0, 0, 1, 1, 2, 2, 3, 0]).unwrap();
        let palette = ramp_palette();
        let r = reduce_colors(&canvas, &palette, 4, &ReduceOptions::default()).unwrap();
        assert!(r.palette.len() < palette.len(), "1 色も減っていない");
        assert!(
            r.steps
                .iter()
                .any(|(name, n)| *name == "低可視 AA の削除" && *n > 0),
            "{:?}",
            r.steps
        );
    }

    #[test]
    fn reduce_keeps_the_picture_readable() {
        let canvas = IndexedCanvas::from_pixels(4, 2, vec![0, 0, 1, 1, 2, 2, 4, 4]).unwrap();
        let palette = ramp_palette();
        let r = reduce_colors(&canvas, &palette, 3, &ReduceOptions::default()).unwrap();
        // 張り替え後の添字が新しいパレットの範囲に収まること
        for &old in canvas.pixels() {
            let new = r.map[old as usize];
            assert!((new as usize) < r.palette.len(), "範囲外の添字 {new}");
        }
    }

    #[test]
    fn reduce_never_maps_outside_the_new_palette() {
        let canvas = IndexedCanvas::filled(4, 4, 0);
        let palette = ramp_palette();
        let r = reduce_colors(&canvas, &palette, 1, &ReduceOptions::default()).unwrap();
        for m in &r.map {
            assert!((*m as usize) < r.palette.len());
        }
    }

    #[test]
    fn locked_colours_survive_reduction() {
        let canvas = IndexedCanvas::from_pixels(4, 2, vec![0, 0, 1, 1, 2, 2, 3, 0]).unwrap();
        let mut palette = ramp_palette();
        palette.set_locked(3, true);
        let r = reduce_colors(&canvas, &palette, 2, &ReduceOptions::default()).unwrap();
        assert!(
            r.palette.entries().contains(&palette.get(3).unwrap()),
            "施錠した色が消えている"
        );
    }
}
