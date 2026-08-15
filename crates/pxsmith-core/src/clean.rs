//! 整形 (`pxsmith clean`，設計書 6.3 / 6.14)．
//!
//! | 機能 | 内容 | 設計書 |
//! | --- | --- | --- |
//! | [`pixel_perfect_stroke`] | 連続する 3 点が L 字を成すとき中間点を落とす | 6.3 |
//! | [`remove_isolated`] | 孤立した小さい連結成分を周囲へ溶かす | 5 章 |
//! | [`remove_antialiasing`] | 低可視の中間色を落とす | 5 章 |
//! | [`denoise_dither`] | 規則性のないディザ状ノイズを規則パターンへ正規化 | 6.14 |
//!
//! 脱ディザノイズは**拡散モデル出力を減色したときに必ず起こる現象**への対処であり，
//! `pxsmith conform` の後段に置く (D46)．

use std::collections::BTreeMap;

use crate::canvas::IndexedCanvas;
use crate::color::distance_sq;
use crate::geom::regions::label_regions;
use crate::ink::PatternMask;
use crate::math::{IRect, IVec2, ivec2};
use crate::palette::Palette;

/// pixel-perfect 補正 (設計書 6.3)．
///
/// **判定は元の点列ではなく直前に採用した点に対して行う．** 元の添字で近傍を見ると
/// 隣接する角を連続で落とし，8 連結でなくなる．
pub fn pixel_perfect_stroke(points: &[IVec2]) -> Vec<IVec2> {
    let mut out: Vec<IVec2> = Vec::with_capacity(points.len());
    for (i, p) in points.iter().enumerate() {
        let is_corner = match (out.last(), points.get(i + 1)) {
            (Some(prev), Some(next)) => is_corner(*prev, *p, *next),
            _ => false,
        };
        if is_corner {
            continue;
        }
        out.push(*p);
    }
    out
}

fn is_corner(a: IVec2, b: IVec2, c: IVec2) -> bool {
    a.manhattan(b) == 1 && b.manhattan(c) == 1 && a.x != c.x && a.y != c.y
}

/// 面積が `min_area` 未満の連結成分を，最も長く接している隣の色へ溶かす．
///
/// 返り値は書き換えた画素の数．透明な領域は残す — 意図的な穴を埋めてしまう．
pub fn remove_isolated(canvas: &mut IndexedCanvas, min_area: u32) -> usize {
    let map = label_regions(canvas);
    let transparent = canvas.transparent();
    let mut changed = 0usize;

    for region in map.regions() {
        if region.area >= min_area || Some(region.index) == transparent {
            continue;
        }
        // 接している長さで隣を選ぶ．同点は添字の小さい方 (決定論性)
        let mut contact: BTreeMap<u8, usize> = BTreeMap::new();
        for p in region.bbox.iter() {
            if map.at(p).map(|r| r.id) != Some(region.id) {
                continue;
            }
            for d in [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)] {
                if let Some(n) = map.at(p + d)
                    && n.id != region.id
                {
                    *contact.entry(n.index).or_default() += 1;
                }
            }
        }
        let Some((&winner, _)) = contact
            .iter()
            .max_by_key(|(index, count)| (**count, std::cmp::Reverse(**index)))
        else {
            continue;
        };
        for p in region.bbox.iter() {
            if map.at(p).map(|r| r.id) == Some(region.id) {
                canvas.set_at(p, winner);
                changed += 1;
            }
        }
    }
    changed
}

/// AA 除去の設定．
#[derive(Copy, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AaOptions {
    /// 中間色とみなす色距離の許容．両隣の色を結ぶ線からこの距離以内なら中間色．
    pub tolerance: f32,
    /// この画素数以下しか使われていない色だけを対象にする．
    pub max_area: u32,
}

impl Default for AaOptions {
    fn default() -> Self {
        Self {
            tolerance: 0.05,
            max_area: 16,
        }
    }
}

/// 低可視の中間色を落とす．
///
/// 「両隣の色のちょうど中間にあり，使用面積が小さい色」を AA とみなし，近い方の
/// 隣の色へ寄せる．**生成 AI 出力や縮小で混入した中間色**を掃除するのが目的で，
/// 意図して置いた AA まで消さないよう面積で絞る．
pub fn remove_antialiasing(
    canvas: &mut IndexedCanvas,
    palette: &Palette,
    opts: &AaOptions,
) -> usize {
    let mut areas: BTreeMap<u8, u32> = BTreeMap::new();
    for &p in canvas.pixels() {
        *areas.entry(p).or_default() += 1;
    }

    // 各色について「自分を挟む 2 色」があるかを調べる
    let mut replacement: BTreeMap<u8, u8> = BTreeMap::new();
    for (&index, &area) in &areas {
        if area > opts.max_area || Some(index) == canvas.transparent() {
            continue;
        }
        if palette.is_locked(index) {
            continue;
        }
        let Some(mid) = palette.lab_of(index) else {
            continue;
        };

        let mut best: Option<(f32, u8)> = None;
        for &a in areas.keys() {
            for &b in areas.keys() {
                if a == b || a == index || b == index || a >= b {
                    continue;
                }
                // 端の色は候補より広く使われていること．AA どうしを端に選ぶと
                // 中間色の連鎖がまとめて消えてしまう
                if areas[&a] <= area || areas[&b] <= area {
                    continue;
                }
                let (Some(la), Some(lb)) = (palette.lab_of(a), palette.lab_of(b)) else {
                    continue;
                };
                // a と b の中点からの距離
                let midpoint = crate::color::Oklab::new(
                    (la.l + lb.l) * 0.5,
                    (la.a + lb.a) * 0.5,
                    (la.b + lb.b) * 0.5,
                );
                let d = distance_sq(mid, midpoint, 1.0).sqrt();
                if d > opts.tolerance {
                    continue;
                }
                // 近い方の端へ寄せる．同点は添字の小さい方
                let (da, db) = (distance_sq(mid, la, 1.0), distance_sq(mid, lb, 1.0));
                let target = if da <= db { a } else { b };
                match best {
                    Some((bd, _)) if bd <= d => {}
                    _ => best = Some((d, target)),
                }
            }
        }
        if let Some((_, target)) = best {
            replacement.insert(index, target);
        }
    }

    let mut changed = 0usize;
    for p in canvas.bounds().iter() {
        let Some(v) = canvas.get_at(p) else { continue };
        if let Some(&target) = replacement.get(&v) {
            canvas.set_at(p, target);
            changed += 1;
        }
    }
    changed
}

/// 見つかったディザ状ノイズの領域．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DitherNoise {
    pub area: IRect,
    /// 交互に現れる 2 色．
    pub colors: (u8, u8),
    /// 1 色目の割合．
    pub ratio_numerator: u32,
    pub ratio_denominator: u32,
}

impl DitherNoise {
    pub fn ratio(&self) -> f32 {
        if self.ratio_denominator == 0 {
            0.0
        } else {
            self.ratio_numerator as f32 / self.ratio_denominator as f32
        }
    }
}

/// 脱ディザノイズの設定．
#[derive(Copy, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DenoiseOptions {
    /// 窓の一辺．
    pub window: u32,
    /// 2 色が窓を占める割合の下限．
    pub dominance: f32,
    /// 4 近傍で色が入れ替わる割合の下限．
    ///
    /// 密度 $p$ の無相関な 2 色ノイズでは期待値が $2p(1-p) \le 0.5$ なので，
    /// **0.5 より上に置くとランダムディザを 1 件も拾えない**．平坦な面は 0 付近，
    /// 通常の絵は面が続くぶん低くなるので，0.4 付近が分かれ目になる．
    pub alternation: f32,
    /// 規則パターンとの一致率がこれを下回ったら「規則性がない」とみなす．
    pub regularity: f32,
}

impl Default for DenoiseOptions {
    fn default() -> Self {
        Self {
            window: 8,
            dominance: 0.92,
            alternation: 0.40,
            regularity: 0.85,
        }
    }
}

/// 規則性のないディザ状ノイズを探す (設計書 6.14)．
///
/// 「局所窓内で 2 色が交互に現れるが**周期性がない**」領域を返す．周期性の有無を
/// 見るのが要点である — 意図して置いた市松やベイヤーは周期的なので残す．
pub fn detect_dither_noise(canvas: &IndexedCanvas, opts: &DenoiseOptions) -> Vec<DitherNoise> {
    let step = opts.window.max(2);
    let mut out = Vec::new();

    for wy in (0..canvas.height()).step_by(step as usize) {
        for wx in (0..canvas.width()).step_by(step as usize) {
            let w = step.min(canvas.width() - wx);
            let h = step.min(canvas.height() - wy);
            if w < 4 || h < 4 {
                continue;
            }
            let area = IRect::new(wx as i32, wy as i32, w, h);

            let mut counts: BTreeMap<u8, u32> = BTreeMap::new();
            for p in area.iter() {
                if let Some(v) = canvas.get_at(p) {
                    *counts.entry(v).or_default() += 1;
                }
            }
            let total: u32 = counts.values().sum();
            if total == 0 || counts.len() < 2 {
                continue;
            }
            let mut top: Vec<(u8, u32)> = counts.into_iter().collect();
            // 同数のときは添字の小さい方を優先 (決定論性)
            top.sort_by_key(|(index, count)| (std::cmp::Reverse(*count), *index));
            let (a, ca) = top[0];
            let (b, cb) = top[1];
            if ((ca + cb) as f32 / total as f32) < opts.dominance {
                continue;
            }

            // 隣り合う画素が入れ替わる割合
            let mut pairs = 0u32;
            let mut swaps = 0u32;
            for p in area.iter() {
                for d in [ivec2(1, 0), ivec2(0, 1)] {
                    let q = p + d;
                    if !area.contains(q) {
                        continue;
                    }
                    let (Some(u), Some(v)) = (canvas.get_at(p), canvas.get_at(q)) else {
                        continue;
                    };
                    pairs += 1;
                    if u != v {
                        swaps += 1;
                    }
                }
            }
            if pairs == 0 || (swaps as f32 / pairs as f32) < opts.alternation {
                continue;
            }

            // 規則パターンとどれだけ合うか．合うなら意図的なディザなので触らない
            if best_pattern_match(canvas, area, a, b) >= opts.regularity {
                continue;
            }

            out.push(DitherNoise {
                area,
                colors: (a, b),
                ratio_numerator: ca,
                ratio_denominator: ca + cb,
            });
        }
    }
    out
}

/// 市松・Bayer と最もよく一致する割合．
fn best_pattern_match(canvas: &IndexedCanvas, area: IRect, a: u8, b: u8) -> f32 {
    let mut best = 0.0f32;
    let mut patterns: Vec<PatternMask> = vec![
        PatternMask::Checker { size: 1 },
        PatternMask::Checker { size: 2 },
    ];
    for level in [4u8, 8, 12] {
        patterns.push(PatternMask::Bayer { order: 4, level });
    }

    for pattern in patterns {
        // 位相を 1 つずらした形も試す
        for (ox, oy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let mut hits = 0u32;
            let mut total = 0u32;
            for p in area.iter() {
                let Some(v) = canvas.get_at(p) else { continue };
                if v != a && v != b {
                    continue;
                }
                total += 1;
                let expect = if pattern.is_a(ivec2(p.x + ox, p.y + oy)) {
                    a
                } else {
                    b
                };
                if v == expect {
                    hits += 1;
                }
            }
            if total > 0 {
                best = best.max(hits as f32 / total as f32);
            }
        }
    }
    best
}

/// 規則性のないディザを Bayer の対応レベルへ置き換える (設計書 6.14)．
///
/// 返り値は書き換えた画素の数．
pub fn denoise_dither(canvas: &mut IndexedCanvas, opts: &DenoiseOptions) -> usize {
    let found = detect_dither_noise(canvas, opts);
    let mut changed = 0usize;

    for noise in found {
        let (a, b) = noise.colors;
        // 元の濃さを保つように Bayer の閾値を選ぶ
        let level = (noise.ratio() * 16.0).round().clamp(0.0, 16.0) as u8;
        let pattern = PatternMask::Bayer { order: 4, level };
        for p in noise.area.iter() {
            let Some(v) = canvas.get_at(p) else { continue };
            if v != a && v != b {
                continue;
            }
            let next = if pattern.is_a(p) { a } else { b };
            if next != v {
                canvas.set_at(p, next);
                changed += 1;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;
    use crate::math::line;

    #[test]
    fn pixel_perfect_drops_l_corners() {
        // 右へ 1，下へ 1 の L 字 — 中間点が落ちる
        let pts = vec![ivec2(0, 0), ivec2(1, 0), ivec2(1, 1)];
        assert_eq!(pixel_perfect_stroke(&pts), vec![ivec2(0, 0), ivec2(1, 1)]);
    }

    #[test]
    fn pixel_perfect_keeps_straight_runs() {
        let pts: Vec<IVec2> = (0..5).map(|x| ivec2(x, 0)).collect();
        assert_eq!(pixel_perfect_stroke(&pts), pts);
    }

    #[test]
    fn pixel_perfect_keeps_the_endpoints() {
        let pts = vec![ivec2(0, 0), ivec2(1, 0), ivec2(1, 1)];
        let out = pixel_perfect_stroke(&pts);
        assert_eq!(out.first(), pts.first());
        assert_eq!(out.last(), pts.last());
    }

    /// 6.3 の要点 — 直前に採用した点で判定する．元の点列で見ると連続で落とす．
    #[test]
    fn pixel_perfect_keeps_the_result_eight_connected() {
        // 階段状の点列
        let pts = vec![
            ivec2(0, 0),
            ivec2(1, 0),
            ivec2(1, 1),
            ivec2(2, 1),
            ivec2(2, 2),
            ivec2(3, 2),
        ];
        let out = pixel_perfect_stroke(&pts);
        for w in out.windows(2) {
            assert_eq!(w[0].chebyshev(w[1]), 1, "8 連結が切れている: {:?}", out);
        }
    }

    #[test]
    fn pixel_perfect_on_a_bresenham_line_stays_connected() {
        let pts = line(ivec2(0, 0), ivec2(9, 4));
        let out = pixel_perfect_stroke(&pts);
        assert!(out.len() <= pts.len());
        for w in out.windows(2) {
            assert_eq!(w[0].chebyshev(w[1]), 1);
        }
    }

    #[test]
    fn isolated_pixels_are_dissolved_into_the_surrounding_colour() {
        let mut c = IndexedCanvas::filled(5, 5, 1);
        c.set(2, 2, 7);
        assert_eq!(remove_isolated(&mut c, 2), 1);
        assert_eq!(c.get(2, 2), Some(1));
    }

    #[test]
    fn large_regions_are_kept() {
        let mut c = IndexedCanvas::from_pixels(4, 2, vec![1, 1, 2, 2, 1, 1, 2, 2]).unwrap();
        let before = c.clone();
        assert_eq!(remove_isolated(&mut c, 3), 0);
        assert_eq!(c, before);
    }

    #[test]
    fn transparent_holes_are_not_filled() {
        let mut c = IndexedCanvas::filled(5, 5, 1).with_transparent(Some(0));
        c.set(2, 2, 0);
        remove_isolated(&mut c, 4);
        assert_eq!(c.get(2, 2), Some(0), "意図的な穴が埋まっている");
    }

    #[test]
    fn remove_isolated_is_deterministic() {
        let mut a = IndexedCanvas::from_pixels(3, 3, vec![1, 1, 2, 1, 9, 2, 1, 2, 2]).unwrap();
        let mut b = a.clone();
        remove_isolated(&mut a, 2);
        remove_isolated(&mut b, 2);
        assert_eq!(a, b);
    }

    fn aa_palette() -> Palette {
        // 中間色は **OKLab 上での**中点を採る．sRGB は非線形なので，
        // 0x20 と 0xf0 の算術平均 (0x88) は OKLab の中点にならない
        let dark = Rgba8::rgb(0x20, 0x20, 0x20);
        let light = Rgba8::rgb(0xf0, 0xf0, 0xf0);
        let (a, b) = (crate::color::oklab_of(dark), crate::color::oklab_of(light));
        let mid = crate::quantize::oklab_to_rgba(crate::color::Oklab::new(
            (a.l + b.l) * 0.5,
            (a.a + b.a) * 0.5,
            (a.b + b.b) * 0.5,
        ));
        Palette::new(vec![dark, mid, light]).unwrap()
    }

    #[test]
    fn low_area_intermediate_colours_are_removed() {
        // 添字 1 が中間色で，2 画素しか使われていない
        let mut c =
            IndexedCanvas::from_pixels(6, 2, vec![0, 0, 1, 2, 2, 2, 0, 0, 1, 2, 2, 2]).unwrap();
        let changed = remove_antialiasing(&mut c, &aa_palette(), &AaOptions::default());
        assert!(changed > 0, "中間色が消えていない");
        assert!(
            !c.pixels().contains(&1),
            "中間色が残っている: {:?}",
            c.pixels()
        );
    }

    #[test]
    fn widely_used_intermediate_colours_are_kept() {
        // 添字 1 が広く使われている — 意図した中間色なので残す
        let mut c = IndexedCanvas::filled(8, 8, 1);
        for x in 0..8 {
            c.set(x, 0, 0);
            c.set(x, 7, 2);
        }
        let before = c.clone();
        remove_antialiasing(&mut c, &aa_palette(), &AaOptions::default());
        assert_eq!(c, before);
    }

    #[test]
    fn locked_colours_are_never_removed() {
        let mut c =
            IndexedCanvas::from_pixels(6, 2, vec![0, 0, 1, 2, 2, 2, 0, 0, 1, 2, 2, 2]).unwrap();
        let mut palette = aa_palette();
        palette.set_locked(1, true);
        remove_antialiasing(&mut c, &palette, &AaOptions::default());
        assert!(c.pixels().contains(&1), "施錠した色が消えている");
    }

    /// 市松は周期的なので触らない．
    #[test]
    fn a_regular_checkerboard_is_not_noise() {
        let mut c = IndexedCanvas::filled(16, 16, 0);
        for p in c.bounds().iter() {
            c.set_at(p, if (p.x + p.y) % 2 == 0 { 0 } else { 1 });
        }
        assert!(
            detect_dither_noise(&c, &DenoiseOptions::default()).is_empty(),
            "規則的な市松をノイズと判定している"
        );
    }

    /// Bayer も周期的なので触らない．
    #[test]
    fn a_bayer_pattern_is_not_noise() {
        let mut c = IndexedCanvas::filled(16, 16, 0);
        let pattern = PatternMask::Bayer { order: 4, level: 8 };
        for p in c.bounds().iter() {
            c.set_at(p, if pattern.is_a(p) { 0 } else { 1 });
        }
        assert!(detect_dither_noise(&c, &DenoiseOptions::default()).is_empty());
    }

    /// 規則性のない 2 色ディザ．線形合同法の下位ビットは周期が短いので
    /// xorshift で混ぜる．
    fn random_dither(w: u32, h: u32, seed: u32) -> IndexedCanvas {
        let mut c = IndexedCanvas::filled(w, h, 0);
        let mut state = seed | 1;
        for p in c.bounds().iter() {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            c.set_at(p, if state & 1 == 0 { 0 } else { 1 });
        }
        c
    }

    #[test]
    fn random_dither_is_detected() {
        let c = random_dither(16, 16, 7);
        let found = detect_dither_noise(&c, &DenoiseOptions::default());
        assert!(!found.is_empty(), "ランダムディザを見つけられていない");
        assert_eq!(found[0].colors, (0, 1));
    }

    #[test]
    fn denoise_replaces_noise_with_a_regular_pattern() {
        let mut c = random_dither(16, 16, 7);
        let changed = denoise_dither(&mut c, &DenoiseOptions::default());
        assert!(changed > 0, "何も直していない");
        // 直した後はノイズとして検出されないこと
        assert!(
            detect_dither_noise(&c, &DenoiseOptions::default()).is_empty(),
            "正規化した後もノイズとして残っている"
        );
    }

    #[test]
    fn denoise_leaves_flat_areas_alone() {
        let mut c = IndexedCanvas::filled(16, 16, 3);
        assert_eq!(denoise_dither(&mut c, &DenoiseOptions::default()), 0);
    }

    #[test]
    fn denoise_is_deterministic() {
        let mut a = random_dither(16, 16, 11);
        let mut b = a.clone();
        denoise_dither(&mut a, &DenoiseOptions::default());
        denoise_dither(&mut b, &DenoiseOptions::default());
        assert_eq!(a, b);
    }

    #[test]
    fn denoise_roughly_preserves_the_density() {
        let mut c = random_dither(16, 16, 3);
        let before = c.pixels().iter().filter(|&&v| v == 0).count();
        denoise_dither(&mut c, &DenoiseOptions::default());
        let after = c.pixels().iter().filter(|&&v| v == 0).count();
        let diff = (before as i32 - after as i32).abs();
        assert!(diff <= 48, "濃さが大きく変わった ({before} -> {after})");
    }
}
