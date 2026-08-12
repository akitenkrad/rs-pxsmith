//! 指標の算出 — 正解率・完全一致率・risk-coverage 曲線・校正誤差 (ECE)．
//!
//! 採点は**全 500 件**に対して行う．正解は件の種類で 2 通りある (定義とその理由は
//! [`crate::sweep::Outcome`])．
//!
//! | 件の種類 | 正解 |
//! | --- | --- |
//! | リサイズなし (整数の格子がある) | $(s, d_x, d_y)$ の完全一致 |
//! | リサイズあり (整数の格子が無い) | 棄却 |
//!
//! 内訳として部分集合ごとの率も出す．全部棄却すれば満点になる，という抜け道は無い —
//! 格子がある件を棄却すると誤棄却として数えられるので，両側から締まる．
//!
//! risk-coverage は「信頼度がある値以上の答えだけを採用したとき，どれだけ答えを返し
//! (coverage) ，そのうちどれだけ誤る (risk) か」を見る．運転点はここから選ぶ．
//! **整数の格子が無い件へ返した答えはすべて risk に数える** — 黙って誤答することの
//! 代償をそのまま値にしたものである．

use std::collections::BTreeMap;

use crate::sweep::{Outcome, Row};

/// 信頼度の刻み．0.00 から 1.00 まで 0.01 刻みで見る．
pub const THRESHOLD_STEPS: usize = 101;

/// ECE のビン数．
pub const ECE_BINS: usize = 10;

/// パラメータ組 1 つ分のまとめ．
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub param_id: usize,
    /// $\varepsilon$ が画像分散に対する割合か．
    pub normalized: bool,
    /// 当てはめた信頼度の下限．
    pub min_confidence: f32,
    pub epsilon: f32,
    pub delta: f32,
    pub tau: f32,
    /// 帯のずれの許容の下限 (画素)．
    pub phase_floor: f32,
    pub phase_tolerance: f32,
    pub phase_subpixel: bool,
    /// 全件数．
    pub n: usize,
    /// 正解率 (完全一致 + 正しい棄却)．**実装計画書の目標 95% に対応する．**
    pub correct_rate: f32,
    /// 校正誤差．
    pub ece: f32,
    /// 整数の格子がある件数．
    pub grid_n: usize,
    /// そのうち完全一致した率．
    pub grid_exact_rate: f32,
    /// そのうちスケールだけ当てた率 (完全一致を含む)．
    pub grid_scale_rate: f32,
    /// そのうち誤って棄却した率．
    pub grid_false_reject_rate: f32,
    /// 整数の格子が無い件数．
    pub resized_n: usize,
    /// そのうち正しく棄却した率．
    pub resized_reject_rate: f32,
    /// **診断値** — $\hat{s} = \mathrm{round}(s \cdot r)$ を返した率．正解ではない
    /// (どれだけ「惜しい誤答」をしているかを見るための値)．
    pub resized_effective_rate: f32,
}

impl Summary {
    /// 2 種類の件を等しく重く見た正解率 (マクロ平均)．
    ///
    /// **閾値を選ぶときはこちらを使う．** 素の正解率で選ぶと「ほぼ全部棄却する」組が
    /// 勝ってしまう — 劣化条件の水準がリサイズなし 1 に対してあり 2 なので，格子の
    /// 無い件が全体の 2/3 を占めるためである．件数比は要因計画の副産物であって，
    /// どちらの誤りをどれだけ嫌うかという判断ではない．
    pub fn macro_rate(&self) -> f32 {
        (self.grid_exact_rate + self.resized_reject_rate) / 2.0
    }
}

pub const SUMMARY_HEADER: &str = "param_id,normalized,min_confidence,epsilon,delta,tau,phase_floor,phase_tolerance,phase_subpixel,n,macro_rate,correct_rate,ece,grid_n,\
grid_exact_rate,grid_scale_rate,grid_false_reject_rate,resized_n,resized_reject_rate,\
resized_effective_rate";

impl Summary {
    pub fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{},{},{:.4},{:.4},{:.4},{},{:.4},{:.4},{:.4},{},{:.4},{:.4}",
            self.param_id,
            self.normalized,
            self.min_confidence,
            self.epsilon,
            self.delta,
            self.tau,
            self.phase_floor,
            self.phase_tolerance,
            self.phase_subpixel,
            self.n,
            self.macro_rate(),
            self.correct_rate,
            self.ece,
            self.grid_n,
            self.grid_exact_rate,
            self.grid_scale_rate,
            self.grid_false_reject_rate,
            self.resized_n,
            self.resized_reject_rate,
            self.resized_effective_rate,
        )
    }
}

fn rate(n: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        n as f32 / total as f32
    }
}

fn count(rows: &[&Row], f: impl Fn(&Row) -> bool) -> usize {
    rows.iter().filter(|r| f(r)).count()
}

/// パラメータ組ごとにまとめる (信頼度の下限 0)．
pub fn summarize(rows: &[Row]) -> Vec<Summary> {
    summarize_at(rows, 0.0)
}

/// 信頼度の下限を当てはめてまとめる．並びは `param_id` の昇順．
///
/// **掃引をやり直さずに下限を変えられる** — 掃引は下限 0 で回し信頼度を行に残して
/// あるので，ここで当てはめ直せばよい ([`Row::outcome_at`])．
pub fn summarize_at(rows: &[Row], min_confidence: f32) -> Vec<Summary> {
    let mut by_param: BTreeMap<usize, Vec<&Row>> = BTreeMap::new();
    for row in rows {
        by_param.entry(row.param_id).or_default().push(row);
    }

    by_param
        .into_iter()
        .map(|(param_id, rows)| {
            let (grid, resized): (Vec<&Row>, Vec<&Row>) =
                rows.iter().copied().partition(|r| r.has_integer_grid);

            let first = rows[0];
            let outcome = |r: &Row| r.outcome_at(min_confidence);
            Summary {
                param_id,
                normalized: first.normalized,
                min_confidence,
                epsilon: first.epsilon,
                delta: first.delta,
                tau: first.tau,
                phase_floor: first.phase_floor,
                phase_tolerance: first.phase_tolerance,
                phase_subpixel: first.phase_subpixel,
                n: rows.len(),
                correct_rate: rate(count(&rows, |r| outcome(r).is_correct()), rows.len()),
                ece: ece(&rows),
                grid_n: grid.len(),
                grid_exact_rate: rate(count(&grid, |r| outcome(r) == Outcome::Exact), grid.len()),
                grid_scale_rate: rate(
                    count(&grid, |r| {
                        matches!(outcome(r), Outcome::Exact | Outcome::ScaleOnly)
                    }),
                    grid.len(),
                ),
                grid_false_reject_rate: rate(
                    count(&grid, |r| outcome(r) == Outcome::Rejected),
                    grid.len(),
                ),
                resized_n: resized.len(),
                resized_reject_rate: rate(
                    count(&resized, |r| outcome(r) == Outcome::CorrectRejection),
                    resized.len(),
                ),
                resized_effective_rate: rate(
                    count(&resized, |r| {
                        outcome(r) == Outcome::Wrong
                            && r.scale_hat
                                .is_some_and(|s| s == r.effective_scale.round() as u32)
                    }),
                    resized.len(),
                ),
            }
        })
        .collect()
}

/// 校正誤差 (expected calibration error)．
///
/// 信頼度を [`ECE_BINS`] 個の等幅ビンに分け，ビンごとの平均信頼度と実際の正解率の
/// 差を件数で重み付けして足す．**棄却された件は信頼度を持たないので対象外**である
/// (信頼度が付いていないものの校正は問えない)．整数の格子が無い件へ返した答えは，
/// どれだけ自信があっても不正解として数える．
pub fn ece(rows: &[&Row]) -> f32 {
    let scored: Vec<(f32, bool)> = rows
        .iter()
        // **下限を当てはめる前の正誤で測る．** 下限で棄却した件を「正解」に数えると，
        // 「信頼度が低い答えは当たっていた」という校正の情報が消える
        .filter_map(|r| r.confidence.map(|c| (c, r.outcome().is_correct())))
        .collect();
    if scored.is_empty() {
        return 0.0;
    }

    let mut sum = 0.0;
    for b in 0..ECE_BINS {
        let lo = b as f32 / ECE_BINS as f32;
        let hi = (b + 1) as f32 / ECE_BINS as f32;
        // 最後のビンだけ右端を含める
        let in_bin: Vec<(f32, bool)> = scored
            .iter()
            .copied()
            .filter(|(c, _)| *c >= lo && (*c < hi || (b == ECE_BINS - 1 && *c <= hi)))
            .collect();
        if in_bin.is_empty() {
            continue;
        }
        let n = in_bin.len() as f32;
        let mean_conf = in_bin.iter().map(|(c, _)| *c).sum::<f32>() / n;
        let acc = in_bin.iter().filter(|(_, ok)| *ok).count() as f32 / n;
        sum += (n / scored.len() as f32) * (acc - mean_conf).abs();
    }
    sum
}

/// risk-coverage 曲線の 1 点．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Point {
    pub threshold: f32,
    /// 答えを返した件の割合．
    pub coverage: f32,
    /// 返した答えのうち誤っていた割合．返答が 0 件なら 0．
    pub risk: f32,
    pub n_covered: usize,
}

pub const CURVE_HEADER: &str = "param_id,threshold,coverage,risk,n_covered";

/// risk-coverage 曲線．**全件**を対象にする．
///
/// 閾値 $t$ では信頼度が $t$ 以上の答えだけを採用する．棄却された件 (信頼度が無い) は
/// どの閾値でも採用されないので，coverage の分母には残るが分子には入らない．
pub fn risk_coverage(rows: &[&Row]) -> Vec<Point> {
    let total = rows.len();
    (0..THRESHOLD_STEPS)
        .map(|i| {
            let threshold = i as f32 / (THRESHOLD_STEPS - 1) as f32;
            let covered: Vec<&&Row> = rows
                .iter()
                .filter(|r| r.confidence.is_some_and(|c| c >= threshold))
                .collect();
            // 採用した答えのうち誤っていたもの．整数の格子が無い件へ返した答えは
            // 完全一致になりようがないので，すべてここへ入る
            let wrong = covered
                .iter()
                .filter(|r| r.outcome() != Outcome::Exact)
                .count();
            Point {
                threshold,
                coverage: rate(covered.len(), total),
                risk: rate(wrong, covered.len()),
                n_covered: covered.len(),
            }
        })
        .collect()
}

/// 目標精度を満たす**最小の**信頼度閾値を選ぶ．
///
/// 閾値を上げるほど採用件数が減って coverage が下がるので，条件を満たす中で最小の
/// ものを採る．**検証セットで選び，テストセットで選び直さない** (実装計画書 M2)．
pub fn operating_point(curve: &[Point], target: f32) -> Option<Point> {
    curve
        .iter()
        .find(|p| p.n_covered > 0 && p.risk <= 1.0 - target)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Split;

    /// 整数の格子がある件．`scale_hat` などを差し替えて結末を作る．
    fn grid_row(param_id: usize, confidence: Option<f32>) -> Row {
        Row {
            param_id,
            normalized: false,
            epsilon: 1.0e-3,
            delta: 0.02,
            tau: 0.02,
            phase_floor: 1.0,
            phase_tolerance: 1.0 / 6.0,
            phase_agreement: 1.0,
            phase_contrast_min: 1.0,
            phase_subpixel: false,
            confidence_per_scale: false,
            item_id: 0,
            split: Split::Validation,
            has_integer_grid: true,
            truth_scale: 6,
            truth_phase: Some((0, 0)),
            effective_scale: 6.0,
            filter: "nearest".to_string(),
            resize: "keep".to_string(),
            compression: "png".to_string(),
            error: None,
            scale_hat: Some(6),
            phase_hat: Some((0, 0)),
            confidence,
            mean_variance: confidence.map(|_| 0.001),
        }
    }

    /// 整数の格子が無い件 (リサイズあり)．
    fn resized_row(param_id: usize, confidence: Option<f32>) -> Row {
        Row {
            has_integer_grid: false,
            truth_phase: None,
            effective_scale: 7.8,
            resize: "1.30".to_string(),
            ..grid_row(param_id, confidence)
        }
    }

    /// 答えを返さなかった行．
    fn rejected(row: Row) -> Row {
        Row {
            error: Some("not_found".to_string()),
            scale_hat: None,
            phase_hat: None,
            confidence: None,
            mean_variance: None,
            ..row
        }
    }

    fn wrong(row: Row) -> Row {
        Row {
            scale_hat: Some(3),
            ..row
        }
    }

    #[test]
    fn the_macro_rate_does_not_reward_rejecting_everything() {
        // 格子なしが 2 倍あるので，全部棄却すると素の正解率は 2/3 になる．
        // マクロ平均なら 1/2 に落ちる
        let rows = [
            rejected(grid_row(0, None)),
            rejected(resized_row(0, None)),
            rejected(resized_row(0, None)),
        ];
        let s = &summarize(&rows)[0];
        assert!((s.correct_rate - 2.0 / 3.0).abs() < 1e-6);
        assert!((s.macro_rate() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_correct_rate_counts_both_kinds_of_right_answer() {
        let rows = [
            grid_row(0, Some(0.9)),         // 完全一致
            rejected(resized_row(0, None)), // 正しい棄却
            wrong(grid_row(0, Some(0.2))),  // 誤り
            resized_row(0, Some(0.4)),      // 格子が無いのに答えた = 誤り
        ];
        let s = &summarize(&rows)[0];
        assert_eq!(s.n, 4);
        assert!((s.correct_rate - 0.5).abs() < 1e-6);
        assert_eq!((s.grid_n, s.resized_n), (2, 2));
        assert!((s.grid_exact_rate - 0.5).abs() < 1e-6);
        assert!((s.resized_reject_rate - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rejecting_everything_does_not_score_full_marks() {
        // 全部棄却する推定器は，格子がある件を落とすぶんだけ減点される
        let rows = [
            rejected(grid_row(0, None)),
            rejected(grid_row(0, None)),
            rejected(resized_row(0, None)),
        ];
        let s = &summarize(&rows)[0];
        assert!((s.correct_rate - 1.0 / 3.0).abs() < 1e-6);
        assert!((s.grid_false_reject_rate - 1.0).abs() < 1e-6);
    }

    #[test]
    fn answering_everything_does_not_score_full_marks_either() {
        let rows = [
            grid_row(0, Some(0.9)),
            resized_row(0, Some(0.9)),
            resized_row(0, Some(0.9)),
        ];
        let s = &summarize(&rows)[0];
        assert!((s.correct_rate - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(s.resized_reject_rate, 0.0);
    }

    #[test]
    fn the_resized_subset_keeps_the_effective_scale_as_a_diagnostic() {
        // effective_scale = 7.8 なので round は 8．正解ではないが記録は残す
        let near = Row {
            scale_hat: Some(8),
            ..resized_row(0, Some(0.4))
        };
        let far = Row {
            scale_hat: Some(3),
            ..resized_row(0, Some(0.4))
        };
        let s = &summarize(&[near, far])[0];
        assert!((s.resized_effective_rate - 0.5).abs() < 1e-6);
        assert_eq!(s.correct_rate, 0.0, "惜しい誤答も誤答である");
    }

    #[test]
    fn summaries_are_grouped_by_param_id() {
        let rows = [grid_row(1, Some(0.9)), wrong(grid_row(0, Some(0.1)))];
        let s = summarize(&rows);
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].param_id, s[1].param_id),
            (0, 1),
            "param_id の昇順でない"
        );
        assert_eq!(s[0].correct_rate, 0.0);
        assert_eq!(s[1].correct_rate, 1.0);
    }

    #[test]
    fn a_perfectly_calibrated_set_has_no_calibration_error() {
        // 信頼度 0.95 の件が 20 件中 19 件正解 — ビン [0.9, 1.0] の平均信頼度と一致する
        let mut rows: Vec<Row> = (0..19).map(|_| grid_row(0, Some(0.95))).collect();
        rows.push(wrong(grid_row(0, Some(0.95))));
        let refs: Vec<&Row> = rows.iter().collect();
        assert!(ece(&refs) < 1e-6, "ECE = {}", ece(&refs));
    }

    #[test]
    fn overconfidence_shows_up_as_calibration_error() {
        // 信頼度 0.95 と言いながら全件外している
        let rows: Vec<Row> = (0..10).map(|_| wrong(grid_row(0, Some(0.95)))).collect();
        let refs: Vec<&Row> = rows.iter().collect();
        assert!((ece(&refs) - 0.95).abs() < 1e-5, "ECE = {}", ece(&refs));
    }

    #[test]
    fn confident_answers_to_a_missing_grid_are_miscalibration() {
        // 格子が無い件へ自信満々で答えるのは校正の誤りとして表に出る
        let rows: Vec<Row> = (0..10).map(|_| resized_row(0, Some(0.9))).collect();
        let refs: Vec<&Row> = rows.iter().collect();
        assert!((ece(&refs) - 0.9).abs() < 1e-5, "ECE = {}", ece(&refs));
    }

    #[test]
    fn rejected_items_are_outside_the_calibration_error() {
        let rows = [rejected(grid_row(0, None))];
        let refs: Vec<&Row> = rows.iter().collect();
        assert_eq!(ece(&refs), 0.0);
    }

    #[test]
    fn coverage_falls_as_the_threshold_rises() {
        let rows = [
            grid_row(0, Some(0.9)),
            grid_row(0, Some(0.5)),
            wrong(grid_row(0, Some(0.1))),
            rejected(grid_row(0, None)),
        ];
        let refs: Vec<&Row> = rows.iter().collect();
        let curve = risk_coverage(&refs);

        assert_eq!(curve.len(), THRESHOLD_STEPS);
        // 棄却された 1 件はどの閾値でも採用されない
        assert!((curve[0].coverage - 0.75).abs() < 1e-6);
        assert!((curve[0].risk - 1.0 / 3.0).abs() < 1e-6);
        for w in curve.windows(2) {
            assert!(w[1].coverage <= w[0].coverage, "coverage が単調でない");
        }
        assert_eq!(curve[THRESHOLD_STEPS - 1].n_covered, 0);
    }

    #[test]
    fn answers_to_a_missing_grid_count_as_risk() {
        let rows = [grid_row(0, Some(0.9)), resized_row(0, Some(0.9))];
        let refs: Vec<&Row> = rows.iter().collect();
        let curve = risk_coverage(&refs);
        assert!(
            (curve[0].risk - 0.5).abs() < 1e-6,
            "黙って誤答した分が risk に出ていない"
        );
    }

    #[test]
    fn the_operating_point_is_the_lowest_threshold_that_meets_the_target() {
        let rows = [
            grid_row(0, Some(0.9)),
            grid_row(0, Some(0.5)),
            wrong(grid_row(0, Some(0.1))),
        ];
        let refs: Vec<&Row> = rows.iter().collect();
        let curve = risk_coverage(&refs);

        // 誤りは信頼度 0.1 の 1 件なので，0.11 まで上げれば risk = 0 になる
        let p = operating_point(&curve, 1.0).expect("運転点がある");
        assert!((p.threshold - 0.11).abs() < 1e-6, "{}", p.threshold);
        assert!((p.coverage - 2.0 / 3.0).abs() < 1e-6);

        // 3 件中 1 件の誤り (risk = 0.333) を許すなら閾値 0 で足りる
        let loose = operating_point(&curve, 0.6).expect("運転点がある");
        assert_eq!(loose.threshold, 0.0);
    }

    #[test]
    fn an_unreachable_target_has_no_operating_point() {
        let rows = [wrong(grid_row(0, Some(0.9)))];
        let refs: Vec<&Row> = rows.iter().collect();
        assert!(operating_point(&risk_coverage(&refs), 0.95).is_none());
    }
}
