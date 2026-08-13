//! ジャギー検出 — 幾何基盤の API 検証 (R19)．
//!
//! M1a の完了条件は「`smooth` の骨格を試作し基盤 API の妥当性を確認」である．
//! 基盤単体のテストでは API の妥当性が確認できないので，**G1 → G3 を実際に
//! 繋いだものをここに置く**．
//!
//! ここで行うのは**検出と提案まで**で，画素の移動は行わない．移動は M3 の
//! `px smooth` の仕事である．検出だけでも lint ルール 8 (ジャギー) が成立する．
//!
//! 対象は輪郭線ではなく**全色境界**である (D33)．線を一度も引かなくても異なる色の
//! 面が接するところに線が生まれ，陰影の境界にジャギーがあってはならない．

use std::collections::BTreeSet;

use crate::canvas::IndexedCanvas;
use crate::math::{IVec2, ivec2};

use super::contour::{Chain, split_monotone, trace_contours};
use super::distance::{curvature_field, signed_distance};
use super::mask::{Field, Mask};
use super::runs::{jaggy_valleys, run_lengths, run_pixels};

/// 見つかったジャギー 1 箇所．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Jaggy {
    /// この輪郭が属する色の添字 (マスクから直接調べた場合は `None`)．
    pub index: Option<u8>,
    /// 谷になっているランの長さ．
    pub length: u32,
    /// 両隣に合わせるときの目標の長さ．
    pub target: u32,
    /// 目標との差．`smooth` はこの分だけ画素を動かす．
    pub delta: i32,
    /// 移動上限に収まるか．収まらないものは**警告として報告する**だけで
    /// 直さない — 意図的なディテールの可能性がある (R22)．
    pub within_limit: bool,
    /// 谷になっているランの画素．
    pub pixels: Vec<IVec2>,
    /// ランが伸びている向き (単位ベクトル)．**1 画素のランでは画素からは決まらない**
    /// ので，チェーンの主軸から取る — `smooth` が «どちらへ伸ばすか» に要る．
    pub major: IVec2,
}

impl Jaggy {
    /// 報告の位置 — ランの先頭の画素．
    pub fn at(&self) -> Option<IVec2> {
        self.pixels.first().copied()
    }
}

/// 検出の結果．
#[derive(Clone, Debug, Default)]
pub struct JaggyReport {
    pub jaggies: Vec<Jaggy>,
    /// 調べた単調区間の数．誤検出率の分母に使う．
    pub chains: usize,
    /// 調べたランの数．
    pub runs: usize,
}

impl JaggyReport {
    pub fn is_clean(&self) -> bool {
        self.jaggies.is_empty()
    }

    /// 直せるもの (移動上限に収まるもの) の数．
    pub fn fixable(&self) -> usize {
        self.jaggies.iter().filter(|j| j.within_limit).count()
    }

    /// 全ランのうち谷と判定された割合．手描き素材に対しては誤検出率の目安になる．
    pub fn valley_rate(&self) -> f32 {
        if self.runs == 0 {
            0.0
        } else {
            self.jaggies.len() as f32 / self.runs as f32
        }
    }

    fn merge(&mut self, other: Self) {
        self.jaggies.extend(other.jaggies);
        self.chains += other.chains;
        self.runs += other.runs;
    }
}

/// 既定の移動上限 (画素)．
///
/// 1 に固定してあるのは，2 以上動かすと**元の絵の意図を変えてしまう**ためである．
/// 上限を超えたものは警告として返し，人の判断に委ねる (設計書 6.4)．
pub const DEFAULT_MAX_MOVE: u32 = 1;

/// 曲率符号が反転しているランの添字 (設計書 6.4 の `Turn(chain)`)．
///
/// ラン $i$ の前後で曲率の符号が入れ替わっていれば，そこは**理想形にも現れる谷**
/// なので違反にしない．符号は G2 の曲率場から読む — ラン長列だけからは
/// 「凹凸が入れ替わったのか，単に凸凹しているのか」が区別できない．
pub fn turn_runs(chain: &Chain, curvature: &Field<f32>) -> BTreeSet<usize> {
    /// 符号を読むときの不感帯．曲率場は離散微分なので 0 付近が揺れる
    const EPS: f32 = 1e-3;

    let signs: Vec<i32> = run_pixels(chain)
        .iter()
        .map(|group| {
            let Some(p) = group.get(group.len() / 2) else {
                return 0;
            };
            match curvature.copied(*p) {
                Some(k) if k > EPS => 1,
                Some(k) if k < -EPS => -1,
                _ => 0,
            }
        })
        .collect();

    (1..signs.len().saturating_sub(1))
        .filter(|&i| {
            let (a, b) = (signs[i - 1], signs[i + 1]);
            a != 0 && b != 0 && a != b
        })
        .collect()
}

/// マスク 1 枚のジャギーを調べる．
pub fn analyze_mask(mask: &Mask, max_move: u32) -> JaggyReport {
    let curvature = curvature_field(&signed_distance(mask));
    let mut report = JaggyReport::default();
    for contour in trace_contours(mask) {
        for chain in split_monotone(&contour) {
            report.merge(analyze_chain(&chain, None, &curvature, max_move));
        }
    }
    report
}

/// 全色境界を調べる (D33)．
pub fn analyze_canvas(canvas: &IndexedCanvas, max_move: u32) -> JaggyReport {
    let mut report = JaggyReport::default();
    let mut indices: Vec<u8> = canvas.pixels().to_vec();
    indices.sort_unstable();
    indices.dedup();

    for index in indices {
        if canvas.transparent() == Some(index) {
            continue;
        }
        let mask = canvas.mask_of(index);
        let curvature = curvature_field(&signed_distance(&mask));
        for contour in trace_contours(&mask) {
            for chain in split_monotone(&contour) {
                report.merge(analyze_chain(&chain, Some(index), &curvature, max_move));
            }
        }
    }
    report
}

fn analyze_chain(
    chain: &Chain,
    index: Option<u8>,
    curvature: &Field<f32>,
    max_move: u32,
) -> JaggyReport {
    let runs = run_lengths(chain);
    let mut report = JaggyReport {
        chains: 1,
        runs: runs.len(),
        ..JaggyReport::default()
    };
    if runs.len() < 3 {
        return report;
    }
    let groups = run_pixels(chain);
    let turns = turn_runs(chain, curvature);

    let major = if chain.is_horizontal() {
        ivec2(1, 0)
    } else {
        ivec2(0, 1)
    };
    for i in jaggy_valleys(&runs, &turns) {
        let target = runs[i - 1].min(runs[i + 1]);
        let delta = target as i32 - runs[i] as i32;
        report.jaggies.push(Jaggy {
            index,
            length: runs[i],
            target,
            delta,
            within_limit: delta.unsigned_abs() <= max_move,
            pixels: groups.get(i).cloned().unwrap_or_default(),
            major,
        });
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::IRect;

    /// 段の長さが揃った階段 — ジャギーは無いはず．
    fn clean_staircase() -> Mask {
        let mut m = Mask::new(12, 8);
        for step in 0..4i32 {
            for dx in 0..3i32 {
                for y in (step + 1)..8 {
                    m.set(ivec2(step * 3 + dx, y), true);
                }
            }
        }
        m
    }

    /// 1 段だけ短い階段 — そこがジャギーになる．
    fn jagged_staircase() -> Mask {
        let mut m = Mask::new(12, 8);
        let widths = [3i32, 3, 1, 3];
        let mut x = 0i32;
        for (step, w) in widths.iter().enumerate() {
            for dx in 0..*w {
                for y in (step as i32 + 1)..8 {
                    m.set(ivec2(x + dx, y), true);
                }
            }
            x += w;
        }
        m
    }

    #[test]
    fn a_rectangle_has_no_jaggies() {
        let mut m = Mask::new(10, 10);
        for p in IRect::new(2, 2, 6, 6).iter() {
            m.set(p, true);
        }
        let report = analyze_mask(&m, DEFAULT_MAX_MOVE);
        assert!(report.is_clean(), "四角にジャギー: {:?}", report.jaggies);
        assert!(report.chains > 0, "チェーンが 1 本も取れていない");
    }

    #[test]
    fn a_circle_has_no_jaggies() {
        // ラン長が単調に変化する形．単谷形なので違反にならない
        let mut m = Mask::new(21, 21);
        for p in m.bounds().iter() {
            let (dx, dy) = (p.x as f32 - 10.0, p.y as f32 - 10.0);
            if (dx * dx + dy * dy).sqrt() <= 8.0 {
                m.set(p, true);
            }
        }
        let report = analyze_mask(&m, DEFAULT_MAX_MOVE);
        assert!(report.is_clean(), "円にジャギー: {:?}", report.jaggies);
    }

    #[test]
    fn an_even_staircase_has_no_jaggies() {
        let report = analyze_mask(&clean_staircase(), DEFAULT_MAX_MOVE);
        assert!(
            report.is_clean(),
            "揃った階段にジャギー: {:?}",
            report.jaggies
        );
    }

    #[test]
    fn a_short_step_is_reported() {
        let report = analyze_mask(&jagged_staircase(), DEFAULT_MAX_MOVE);
        assert!(!report.is_clean(), "短い段が見つかっていない");
        let j = &report.jaggies[0];
        assert!(j.length < j.target, "谷でないものを報告している: {j:?}");
        assert!(j.at().is_some());
    }

    #[test]
    fn a_large_gap_is_reported_but_not_fixable() {
        // 3, 3, 1, 3 なので差は 2．上限 1 では直せない
        let report = analyze_mask(&jagged_staircase(), 1);
        assert!(!report.is_clean());
        assert_eq!(report.fixable(), 0, "上限を超えるものを直せると言っている");
        let loose = analyze_mask(&jagged_staircase(), 3);
        assert!(
            loose.fixable() > 0,
            "上限を上げても直せないままになっている"
        );
    }

    #[test]
    fn colour_boundaries_are_scanned_not_just_the_silhouette() {
        // シルエットは真四角だが，内側の色境界が階段状になっている (D33)
        let mut c = IndexedCanvas::filled(12, 8, 1);
        let widths = [3i32, 3, 1, 3];
        let mut x = 0i32;
        for (step, w) in widths.iter().enumerate() {
            for dx in 0..*w {
                for y in (step as i32 + 1)..8 {
                    c.set(x + dx, y, 2);
                }
            }
            x += w;
        }
        let report = analyze_canvas(&c, DEFAULT_MAX_MOVE);
        assert!(!report.is_clean(), "色境界のジャギーを見落としている (D33)");
        assert!(report.jaggies.iter().any(|j| j.index == Some(2)));
    }

    #[test]
    fn the_transparent_index_is_skipped() {
        let c = IndexedCanvas::filled(8, 8, 0).with_transparent(Some(0));
        let report = analyze_canvas(&c, DEFAULT_MAX_MOVE);
        assert_eq!(report.chains, 0, "透明だけの絵を走査している");
    }

    #[test]
    fn the_report_counts_what_it_looked_at() {
        let report = analyze_mask(&clean_staircase(), DEFAULT_MAX_MOVE);
        assert!(report.runs >= report.chains, "ランの数がチェーン数を下回る");
        assert_eq!(report.valley_rate(), 0.0);
    }
}
