//! 色・格子・ディザ系の 11 ルール (設計書 7.3)．
//!
//! **閾値はすべて暫定である．** ルールは検査対象を作るフェーズで実装すると決めた
//! (実装計画書 M2 の注記) が，閾値の決定には `testdata/lint-cases/` の正例・負例が
//! 要る．ここに書いてある値は「合成した例で意図どおり動く」までしか根拠がない．

use std::collections::BTreeMap;

use px_core::canvas::{IndexedCanvas, RgbaCanvas};
use px_core::clean::{DenoiseOptions, detect_dither_noise};
use px_core::color::{Oklab, oklab_of};
use px_core::frame::{Frame, Surface};
use px_core::geom::regions::{RegionMap, label_regions};
use px_core::grid::{GridParams, local_grid, uniformity};
use px_core::math::{IRect, ivec2};
use px_core::palette::Palette;
use serde::{Deserialize, Serialize};

use crate::{Report, Violation, rule};

/// 閾値．**評価データセットと `testdata/lint-cases/` で校正するまで暫定値**．
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct LintConfig {
    /// ルール 3 — この画素数未満の連結成分を孤立とみなす．
    pub isolated_max_area: u32,
    /// ルール 2 ・9 — 格子推定の閾値．
    pub grid: GridParams,
    /// ルール 9 — 局所推定の一致率がこれを下回ったらミクセル．
    pub uniformity: f32,
    /// ルール 9 — 局所推定の窓の一辺．
    pub grid_window: u32,
    /// ルール 10 — ディザ領域内の同色連結成分がこの長さを超えたら塊化．
    pub dither_clump: u32,
    /// ルール 11 — 隣接領域の $\Delta L$ の下限．
    pub min_lightness_delta: f32,
    /// ルール 11 — この面積未満の領域は隣接判定から外す (AA を拾わないため)．
    pub min_region_area: u32,
    /// ルール 15 — 画面に占めるディザ領域の割合の上限．
    pub max_dither_ratio: f32,
    /// ルール 16 — 「大面積」とみなす画面比．
    pub large_area_ratio: f32,
    /// ルール 16 — 大面積で許す彩度と明度の上限．
    pub max_large_chroma: f32,
    pub max_large_lightness: f32,
    /// ルール 17 — 隣接 2 色の $\Delta L$ がこれを超えるディザは高コントラスト．
    pub high_contrast_delta: f32,
    /// ルール 18 — 純黒とみなす明度と彩度の上限．
    pub pure_black_lightness: f32,
    pub pure_black_chroma: f32,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            isolated_max_area: 2,
            grid: GridParams::default(),
            uniformity: 0.8,
            grid_window: 32,
            dither_clump: 6,
            min_lightness_delta: 0.06,
            min_region_area: 4,
            max_dither_ratio: 0.35,
            large_area_ratio: 0.25,
            max_large_chroma: 0.16,
            max_large_lightness: 0.85,
            high_contrast_delta: 0.35,
            pure_black_lightness: 0.06,
            pure_black_chroma: 0.01,
        }
    }
}

/// フレームを検査する．
///
/// レイヤごとにインデックスカラーの面を検査し，パレット側のルールは 1 度だけ見る．
pub fn lint_frame(frame: &Frame, cfg: &LintConfig) -> Report {
    let mut report = Report::default();
    report.extend(lint_palette(&frame.palette, cfg));

    for layer in &frame.layers {
        if let Surface::Indexed(canvas) = &layer.surface {
            report.extend(lint_canvas(canvas, &frame.palette, cfg));
        }
    }
    report.sorted()
}

/// 1 枚のキャンバスを検査する．
pub fn lint_canvas(canvas: &IndexedCanvas, palette: &Palette, cfg: &LintConfig) -> Report {
    let mut report = Report::default();
    let regions = label_regions(canvas);

    rule_1_palette_escape(canvas, palette, &mut report);
    rule_3_isolated(&regions, canvas, cfg, &mut report);
    rule_10_dither_clumping(canvas, cfg, &mut report);
    rule_11_lightness_delta(&regions, palette, canvas, cfg, &mut report);
    rule_15_dither_ratio(canvas, cfg, &mut report);
    rule_16_large_saturated(&regions, palette, canvas, cfg, &mut report);
    rule_17_high_contrast_dither(canvas, palette, cfg, &mut report);
    report.sorted()
}

/// パレットだけを見るルール (5 ・18)．
pub fn lint_palette(palette: &Palette, cfg: &LintConfig) -> Report {
    let mut report = Report::default();
    rule_5_chroma_curve(palette, &mut report);
    rule_18_pure_black(palette, cfg, &mut report);
    report
}

/// 格子を見るルール (2 ・9)．RGBA の入力にしか意味がないので入口を分ける．
pub fn lint_grid(img: &RgbaCanvas, cfg: &LintConfig) -> Report {
    let mut report = Report::default();
    rule_2_broken_grid(img, cfg, &mut report);
    rule_9_mixels(img, cfg, &mut report);
    report.sorted()
}

// --- ルール 1: パレット逸脱 ---

fn rule_1_palette_escape(canvas: &IndexedCanvas, palette: &Palette, report: &mut Report) {
    let r = rule(1).expect("ルール 1 は定義済み");
    let mut seen: Vec<u8> = canvas.pixels().to_vec();
    seen.sort_unstable();
    seen.dedup();

    for index in seen {
        if palette.get(index).is_none() {
            let at = canvas
                .bounds()
                .iter()
                .find(|p| canvas.get_at(*p) == Some(index));
            let mut v = Violation::new(
                r,
                format!("添字 {index} がパレット ({} 色) の範囲外", palette.len()),
            );
            if let Some(p) = at {
                v = v.at(p);
            }
            report.push(v);
        }
    }
}

// --- ルール 2: 格子崩れ ---

fn rule_2_broken_grid(img: &RgbaCanvas, cfg: &LintConfig, report: &mut Report) {
    let r = rule(2).expect("ルール 2 は定義済み");
    if px_core::grid::estimate_grid(img, &cfg.grid).is_err() {
        report.push(Violation::new(
            r,
            format!(
                "セル内の平均分散が ε = {:.1e} を満たす格子が見つからない",
                cfg.grid.epsilon
            ),
        ));
    }
}

// --- ルール 3: 孤立ピクセル ---

fn rule_3_isolated(
    regions: &RegionMap,
    canvas: &IndexedCanvas,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(3).expect("ルール 3 は定義済み");
    for region in regions.regions() {
        if region.area >= cfg.isolated_max_area || canvas.transparent() == Some(region.index) {
            continue;
        }
        report.push(
            Violation::new(
                r,
                format!("{} 画素の孤立した領域 (添字 {})", region.area, region.index),
            )
            .at(ivec2(region.bbox.x, region.bbox.y))
            .area(region.bbox),
        );
    }
}

// --- ルール 5: 彩度カーブ異常 ---

fn rule_5_chroma_curve(palette: &Palette, report: &mut Report) {
    let r = rule(5).expect("ルール 5 は定義済み");
    for (i, ramp) in palette.ramps().iter().enumerate() {
        let labs: Vec<Oklab> = ramp
            .entries()
            .iter()
            .filter_map(|&e| palette.lab_of(e))
            .collect();
        if labs.len() < 3 {
            continue;
        }
        let chromas: Vec<f32> = labs.iter().map(|l| l.chroma()).collect();
        let rising = chromas.windows(2).all(|w| w[0] <= w[1]);
        let falling = chromas.windows(2).all(|w| w[0] >= w[1]);
        if rising || falling {
            report.push(Violation::new(
                r,
                format!(
                    "ランプ {i} の彩度が明度に対し単調 ({})．中間で最大になる形が自然",
                    if rising { "増加" } else { "減少" }
                ),
            ));
        }
    }
}

// --- ルール 9: ミクセル ---

fn rule_9_mixels(img: &RgbaCanvas, cfg: &LintConfig, report: &mut Report) {
    let r = rule(9).expect("ルール 9 は定義済み");
    let local = local_grid(img, cfg.grid_window, &cfg.grid);
    if let Some((scale, ratio)) = uniformity(&local)
        && ratio < cfg.uniformity
    {
        report.push(Violation::new(
            r,
            format!(
                "局所格子が場所により異なる (最頻 {scale} の一致率 {:.1}% < {:.1}%)",
                ratio * 100.0,
                cfg.uniformity * 100.0
            ),
        ));
    }
}

// --- ルール 10: ディザの塊化 ---

fn rule_10_dither_clumping(canvas: &IndexedCanvas, cfg: &LintConfig, report: &mut Report) {
    let r = rule(10).expect("ルール 10 は定義済み");
    // ディザとみなせる領域を先に見つけ，その中で同色が続いていないかを見る
    let opts = DenoiseOptions::default();
    for area in dither_areas(canvas, &opts) {
        let sub = crop(canvas, area);
        let map = label_regions(&sub);
        for region in map.regions() {
            let longest = region.bbox.w.max(region.bbox.h);
            if region.area > 1 && longest > cfg.dither_clump {
                report.push(
                    Violation::new(
                        r,
                        format!(
                            "ディザ領域で添字 {} が {longest} 画素続いている",
                            region.index
                        ),
                    )
                    .area(area),
                );
                break;
            }
        }
    }
}

/// ディザとみなせる窓．規則的かどうかは問わない — 塊化はどちらでも問題になる．
fn dither_areas(canvas: &IndexedCanvas, opts: &DenoiseOptions) -> Vec<IRect> {
    let loose = DenoiseOptions {
        // 規則的なディザも対象に含めるため，規則性の判定を無効にする
        regularity: 2.0,
        ..*opts
    };
    detect_dither_noise(canvas, &loose)
        .into_iter()
        .map(|n| n.area)
        .collect()
}

fn crop(canvas: &IndexedCanvas, area: IRect) -> IndexedCanvas {
    let fill = canvas.transparent().unwrap_or(0);
    canvas.crop(area, fill)
}

// --- ルール 11: 明度差不足 ---

fn rule_11_lightness_delta(
    regions: &RegionMap,
    palette: &Palette,
    canvas: &IndexedCanvas,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(11).expect("ルール 11 は定義済み");
    let mut reported: Vec<(u8, u8)> = Vec::new();

    for region in regions.regions() {
        if region.area < cfg.min_region_area || canvas.transparent() == Some(region.index) {
            continue;
        }
        let Some(a) = palette.lab_of(region.index) else {
            continue;
        };
        for &id in &region.neighbors {
            let other = &regions.regions()[id as usize];
            if other.area < cfg.min_region_area
                || canvas.transparent() == Some(other.index)
                || other.index == region.index
            {
                continue;
            }
            let Some(b) = palette.lab_of(other.index) else {
                continue;
            };
            let pair = (region.index.min(other.index), region.index.max(other.index));
            if reported.contains(&pair) {
                continue;
            }
            let delta = (a.l - b.l).abs();
            if delta < cfg.min_lightness_delta {
                reported.push(pair);
                report.push(
                    Violation::new(
                        r,
                        format!(
                            "隣接する添字 {} と {} の ΔL が {delta:.3} (下限 {:.3})",
                            pair.0, pair.1, cfg.min_lightness_delta
                        ),
                    )
                    .area(region.bbox),
                );
            }
        }
    }
}

// --- ルール 15: ディザ過多 ---

fn rule_15_dither_ratio(canvas: &IndexedCanvas, cfg: &LintConfig, report: &mut Report) {
    let r = rule(15).expect("ルール 15 は定義済み");
    let total = canvas.size().area();
    if total == 0 {
        return;
    }
    let covered: usize = dither_areas(canvas, &DenoiseOptions::default())
        .iter()
        .map(|a| (a.w * a.h) as usize)
        .sum();
    let ratio = covered as f32 / total as f32;
    if ratio > cfg.max_dither_ratio {
        report.push(Violation::new(
            r,
            format!(
                "画面の {:.1}% がディザ (上限 {:.1}%)．低解像度では密度が過剰に見える",
                ratio * 100.0,
                cfg.max_dither_ratio * 100.0
            ),
        ));
    }
}

// --- ルール 16: 大面積の高彩度色 ---

fn rule_16_large_saturated(
    regions: &RegionMap,
    palette: &Palette,
    canvas: &IndexedCanvas,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(16).expect("ルール 16 は定義済み");
    let total = canvas.size().area() as f32;
    if total <= 0.0 {
        return;
    }
    // 同じ色の領域は面積を合算する — 面積効果は色ごとに効く
    let mut by_index: BTreeMap<u8, (u32, IRect)> = BTreeMap::new();
    for region in regions.regions() {
        if canvas.transparent() == Some(region.index) {
            continue;
        }
        let slot = by_index.entry(region.index).or_insert((0, region.bbox));
        slot.0 += region.area;
        slot.1 = slot.1.union(region.bbox);
    }

    for (index, (area, bbox)) in by_index {
        if (area as f32 / total) < cfg.large_area_ratio {
            continue;
        }
        let Some(lab) = palette.lab_of(index) else {
            continue;
        };
        if lab.chroma() > cfg.max_large_chroma || lab.l > cfg.max_large_lightness {
            report.push(
                Violation::new(
                    r,
                    format!(
                        "添字 {index} が画面の {:.1}% を占め，彩度 {:.3} / 明度 {:.3} が高い (面積効果)",
                        area as f32 / total * 100.0,
                        lab.chroma(),
                        lab.l
                    ),
                )
                .area(bbox),
            );
        }
    }
}

// --- ルール 17: 高コントラスト間のディザ ---

fn rule_17_high_contrast_dither(
    canvas: &IndexedCanvas,
    palette: &Palette,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(17).expect("ルール 17 は定義済み");
    for noise in detect_dither_noise(
        canvas,
        &DenoiseOptions {
            regularity: 2.0,
            ..DenoiseOptions::default()
        },
    ) {
        let (a, b) = noise.colors;
        let (Some(la), Some(lb)) = (palette.lab_of(a), palette.lab_of(b)) else {
            continue;
        };
        let delta = (la.l - lb.l).abs();
        if delta > cfg.high_contrast_delta {
            report.push(
                Violation::new(
                    r,
                    format!(
                        "添字 {a} と {b} (ΔL = {delta:.3}) をディザで混ぜている (上限 {:.3})",
                        cfg.high_contrast_delta
                    ),
                )
                .area(noise.area),
            );
        }
    }
}

// --- ルール 18: 純黒の使用 ---

fn rule_18_pure_black(palette: &Palette, cfg: &LintConfig, report: &mut Report) {
    let r = rule(18).expect("ルール 18 は定義済み");
    for (i, (c, lab)) in palette.entries().iter().zip(palette.lab()).enumerate() {
        if c.a == 0 {
            continue;
        }
        if lab.l <= cfg.pure_black_lightness && lab.chroma() <= cfg.pure_black_chroma {
            report.push(Violation::new(
                r,
                format!(
                    "添字 {i} が純黒に近い (L = {:.3}，彩度 = {:.3})．暗部にも色を残すこと",
                    lab.l,
                    lab.chroma()
                ),
            ));
        }
    }
}

/// `oklab_of` はルール実装から直接は呼ばないが，パレットを持たない検査で要る．
#[allow(dead_code)]
fn lab(c: px_core::Rgba8) -> Oklab {
    oklab_of(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_core::palette::{ChromaCurve, Ramp};
    use px_core::ramp::{RampSpec, generate_ramp};
    use px_core::{Rgba8, ivec2};

    fn ramp_palette() -> Palette {
        Palette::new(generate_ramp(&RampSpec::default())).unwrap()
    }

    fn has(report: &Report, id: u8) -> bool {
        report.violations.iter().any(|v| v.rule == id)
    }

    #[test]
    fn a_clean_sprite_has_no_violations() {
        let palette = ramp_palette();
        let mut canvas = IndexedCanvas::filled(16, 16, 1);
        for p in IRect::new(4, 4, 8, 8).iter() {
            canvas.set_at(p, 3);
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(report.is_empty(), "きれいな絵に違反: {report}");
    }

    #[test]
    fn rule_1_detects_indices_outside_the_palette() {
        let palette = Palette::new(vec![Rgba8::rgb(1, 2, 3), Rgba8::rgb(4, 5, 6)]).unwrap();
        let canvas = IndexedCanvas::from_pixels(2, 1, vec![0, 9]).unwrap();
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 1), "{report}");
        assert!(report.has_blocking());
        assert_eq!(report.violations[0].at, Some([1, 0]));
    }

    #[test]
    fn rule_3_detects_a_single_stray_pixel() {
        let palette = ramp_palette();
        let mut canvas = IndexedCanvas::filled(8, 8, 1);
        canvas.set(3, 3, 4);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 3), "{report}");
    }

    #[test]
    fn rule_3_ignores_the_transparent_index() {
        let palette = ramp_palette();
        let mut canvas = IndexedCanvas::filled(8, 8, 1).with_transparent(Some(0));
        canvas.set(3, 3, 0);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(
            !has(&report, 3),
            "透明の穴を孤立ピクセルと言っている: {report}"
        );
    }

    #[test]
    fn rule_5_flags_a_monotone_chroma_ramp() {
        // 彩度が明度に対し単調増加するランプ
        let mut palette = Palette::new(vec![
            Rgba8::rgb(0x30, 0x30, 0x30),
            Rgba8::rgb(0x70, 0x50, 0x50),
            Rgba8::rgb(0xc0, 0x50, 0x50),
        ])
        .unwrap();
        palette.add_ramp(Ramp::new(vec![0, 1, 2], ChromaCurve::Uniform));
        let report = lint_palette(&palette, &LintConfig::default());
        assert!(has(&report, 5), "{report}");
    }

    #[test]
    fn rule_5_accepts_the_generated_default_ramp() {
        let colors = generate_ramp(&RampSpec::default());
        let mut palette = Palette::new(colors).unwrap();
        let n = palette.len() as u8;
        palette.add_ramp(Ramp::new((0..n).collect(), ChromaCurve::PeakMiddle));
        let report = lint_palette(&palette, &LintConfig::default());
        assert!(
            !has(&report, 5),
            "自作のランプが自分の lint に落ちている: {report}"
        );
    }

    #[test]
    fn rule_11_flags_neighbours_that_are_too_close_in_lightness() {
        // 明度がほぼ同じ 2 色を隣接させる
        let palette = Palette::new(vec![
            Rgba8::rgb(0x80, 0x40, 0x40),
            Rgba8::rgb(0x40, 0x70, 0x48),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(8, 8, 0);
        for p in IRect::new(4, 0, 4, 8).iter() {
            canvas.set_at(p, 1);
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 11), "{report}");
        assert!(
            !report
                .violations
                .iter()
                .find(|v| v.rule == 11)
                .unwrap()
                .is_blocking()
        );
    }

    #[test]
    fn rule_16_flags_a_large_saturated_area() {
        let palette = Palette::new(vec![Rgba8::rgb(0xff, 0x00, 0x00)]).unwrap();
        let canvas = IndexedCanvas::filled(16, 16, 0);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 16), "{report}");
    }

    #[test]
    fn rule_16_accepts_a_muted_large_area() {
        let palette = Palette::new(vec![Rgba8::rgb(0x60, 0x62, 0x70)]).unwrap();
        let canvas = IndexedCanvas::filled(16, 16, 0);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(!has(&report, 16), "{report}");
    }

    #[test]
    fn rule_17_flags_dither_between_distant_lightnesses() {
        let palette = Palette::new(vec![
            Rgba8::rgb(0x10, 0x10, 0x18),
            Rgba8::rgb(0xf0, 0xf0, 0xe8),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(16, 16, 0);
        for p in canvas.bounds().iter() {
            canvas.set_at(p, if (p.x + p.y) % 2 == 0 { 0 } else { 1 });
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 17), "{report}");
    }

    #[test]
    fn rule_17_accepts_dither_between_close_lightnesses() {
        let colors = generate_ramp(&RampSpec::default());
        let palette = Palette::new(colors).unwrap();
        let mut canvas = IndexedCanvas::filled(16, 16, 2);
        for p in canvas.bounds().iter() {
            canvas.set_at(p, if (p.x + p.y) % 2 == 0 { 2 } else { 3 });
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(
            !has(&report, 17),
            "隣り合う段のディザを違反にしている: {report}"
        );
    }

    #[test]
    fn rule_18_flags_pure_black() {
        let palette = Palette::new(vec![Rgba8::rgb(0, 0, 0), Rgba8::rgb(200, 200, 200)]).unwrap();
        let report = lint_palette(&palette, &LintConfig::default());
        assert!(has(&report, 18), "{report}");
    }

    #[test]
    fn rule_18_accepts_the_generated_ramp() {
        let palette = ramp_palette();
        let report = lint_palette(&palette, &LintConfig::default());
        assert!(
            !has(&report, 18),
            "純黒回避したランプが純黒と言われている: {report}"
        );
    }

    #[test]
    fn rule_2_flags_an_image_without_a_grid() {
        let mut img = RgbaCanvas::filled(32, 32, Rgba8::TRANSPARENT);
        let mut state = 12345u32;
        for y in 0..32 {
            for x in 0..32 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let v = state as u8;
                img.set(x, y, Rgba8::rgb(v, v.wrapping_mul(3), v.wrapping_mul(7)));
            }
        }
        let report = lint_grid(&img, &LintConfig::default());
        assert!(has(&report, 2), "{report}");
    }

    #[test]
    fn rule_2_accepts_a_clean_upscale() {
        let mut small = RgbaCanvas::filled(8, 8, Rgba8::TRANSPARENT);
        for y in 0..8 {
            for x in 0..8 {
                let v = ((x * 31 + y * 17) % 4) as u8;
                small.set(x, y, Rgba8::rgb(v * 60, 40 + v * 30, 200 - v * 40));
            }
        }
        let mut big = RgbaCanvas::filled(32, 32, Rgba8::TRANSPARENT);
        for y in 0..32 {
            for x in 0..32 {
                big.set(x, y, small.get(x / 4, y / 4).unwrap());
            }
        }
        let report = lint_grid(&big, &LintConfig::default());
        assert!(
            !has(&report, 2),
            "きれいな 4 倍拡大が格子崩れと言われている: {report}"
        );
    }

    #[test]
    fn the_report_is_deterministic() {
        let palette = Palette::new(vec![Rgba8::rgb(0, 0, 0)]).unwrap();
        let mut canvas = IndexedCanvas::filled(8, 8, 0);
        canvas.set(1, 1, 5);
        let a = lint_canvas(&canvas, &palette, &LintConfig::default());
        let b = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert_eq!(a, b);
    }

    #[test]
    fn blocking_and_advisory_are_separated() {
        let palette = Palette::new(vec![Rgba8::rgb(0, 0, 0)]).unwrap();
        let mut canvas = IndexedCanvas::filled(8, 8, 0);
        canvas.set(1, 1, 5);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(report.has_blocking(), "{report}");
        assert!(!report.to_prompt_hint().is_empty());
        // advisory は生成ループの判定に影響しない
        let only_advisory = lint_palette(&palette, &LintConfig::default());
        assert!(!only_advisory.has_blocking(), "{only_advisory}");
    }

    #[test]
    fn every_declared_rule_number_is_unique() {
        let mut ids: Vec<u8> = crate::RULES.iter().map(|r| r.id).collect();
        ids.sort_unstable();
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(ids, unique);
        assert_eq!(ids.len(), 11, "M2 で実装するのは 11 ルール");
    }

    #[test]
    fn the_report_serialises_to_json() {
        let palette = Palette::new(vec![Rgba8::rgb(0, 0, 0)]).unwrap();
        let canvas = IndexedCanvas::filled(4, 4, 0);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        let json = serde_json::to_string(&report).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(report, back);
    }

    #[test]
    fn positions_are_reported_for_locatable_violations() {
        let palette = ramp_palette();
        let mut canvas = IndexedCanvas::filled(8, 8, 1);
        canvas.set(5, 6, 4);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        let v = report.violations.iter().find(|v| v.rule == 3).unwrap();
        assert_eq!(v.at, Some([5, 6]));
        assert_eq!(v.area, Some([5, 6, 1, 1]));
    }

    #[test]
    fn lint_frame_covers_palette_and_canvas_rules() {
        let palette = Palette::new(vec![Rgba8::rgb(0, 0, 0), Rgba8::rgb(0xff, 0, 0)]).unwrap();
        let mut frame = Frame::new(px_core::uvec2(8, 8), palette);
        let mut canvas = IndexedCanvas::filled(8, 8, 1);
        canvas.set(2, 2, 9);
        frame.layers.push(px_core::Layer::new(
            px_core::LayerMeta::named("art"),
            Surface::Indexed(canvas),
        ));
        let report = lint_frame(&frame, &LintConfig::default());
        assert!(
            has(&report, 1),
            "キャンバス側のルールが効いていない: {report}"
        );
        assert!(
            has(&report, 18),
            "パレット側のルールが効いていない: {report}"
        );
    }

    #[test]
    fn violations_carry_their_declared_severity_and_scope() {
        for r in crate::RULES {
            let v = Violation::new(r, "x");
            assert_eq!(v.severity, r.severity);
            assert_eq!(v.scope, r.scope);
        }
        let _ = ivec2(0, 0);
    }
}
