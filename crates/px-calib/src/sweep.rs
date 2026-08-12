//! パラメータ掃引．
//!
//! $\varepsilon$ ・$\delta$ ・$\tau$ の組ごとに全件を推定し，1 行 1 件で CSV へ書く．
//!
//! **信頼度の下限 $\mathrm{min\_confidence}$ は掃引しない．** これは推定そのものの
//! 閾値ではなく「どこから先を棄却するか」の運転点であり，risk-coverage 曲線から
//! 後で選ぶ (設計書 6.1)．掃引中は 0 に固定し，信頼度を数値のまま記録する．

use std::path::Path;

use anyhow::{Context, Result};
use px_core::grid::{GridError, GridParams, estimate_grid};
use rayon::prelude::*;

use crate::dataset::{Item, Manifest, Split};

/// 掃引する閾値の組．
#[derive(Clone, Debug)]
pub struct ParamGrid {
    pub max_scale: u32,
    /// 位相ずれ検査の帯の数．掃引はしないが，検査ごと外して比べられるようにする
    pub phase_bands: usize,
    /// 帯どうしの位相のずれの許容 (複数指定可)．
    pub phase_tolerances: Vec<f32>,
    /// 帯ごとの位相**曲線**の食い違いの許容 (複数指定可)．1.0 以上で検査を外せる
    pub phase_agreements: Vec<f32>,
    /// 測れない候補を棄却するか．**掃引 1 回につき 1 通り**
    pub phase_require_measurable: bool,
    /// 帯ごとの位相を副画素で求めるか．**掃引 1 回につき 1 通り**
    pub phase_subpixel: bool,
    /// 信頼度の下限を $\hat{s}$ で割るか．**掃引 1 回につき 1 通り**
    pub confidence_per_scale: bool,
    /// 許容の下限 (画素)．**掃引する** — 帯を減らしてでも測る変更と同時に効くので，
    /// 割合だけの許容と比べられるようにしておく
    pub phase_tolerance_floors: Vec<f32>,
    pub phase_min_cells: usize,
    pub epsilons: Vec<f32>,
    pub deltas: Vec<f32>,
    pub taus: Vec<f32>,
    /// $\varepsilon$ を画像分散に対する割合として扱うか．**掃引 1 回につき 1 通り**
    /// — 絶対値と割合では意味のある水準の桁が違うので，同じ列に混ぜない
    pub normalize_epsilon: bool,
}

impl Default for ParamGrid {
    fn default() -> Self {
        Self {
            max_scale: 16,
            phase_bands: 4,
            phase_tolerances: vec![0.35],
            phase_agreements: vec![0.16],
            phase_require_measurable: true,
            phase_subpixel: false,
            confidence_per_scale: true,
            phase_tolerance_floors: vec![0.0],
            phase_min_cells: 2,
            // 予備調査で「補間を挟むと必要な ε が 1 桁以上大きくなる」ことが
            // 分かっている (開発ノート 5 節)．既定の 2e-4 から 2 桁上まで見る
            epsilons: vec![2.0e-4, 5.0e-4, 1.0e-3, 2.0e-3, 5.0e-3, 1.0e-2, 2.0e-2],
            deltas: vec![0.01, 0.02, 0.05, 0.10],
            taus: vec![0.01, 0.02, 0.05, 0.10],
            normalize_epsilon: false,
        }
    }
}

impl ParamGrid {
    /// 組を展開する．順序は $(\varepsilon, \delta, \tau, \text{下限}, \theta)$ の
    /// 辞書式で固定する．
    pub fn combinations(&self) -> Vec<GridParams> {
        let mut out = Vec::new();
        for &epsilon in &self.epsilons {
            for &delta in &self.deltas {
                for &tau in &self.taus {
                    for &floor in &self.phase_tolerance_floors {
                        for &tolerance in &self.phase_tolerances {
                            for &agreement in &self.phase_agreements {
                                out.push(GridParams {
                                    max_scale: self.max_scale,
                                    epsilon,
                                    delta,
                                    tau,
                                    phase_bands: self.phase_bands,
                                    phase_tolerance: tolerance,
                                    phase_agreement: agreement,
                                    phase_require_measurable: self.phase_require_measurable,
                                    phase_subpixel: self.phase_subpixel,
                                    phase_tolerance_floor: floor,
                                    phase_min_cells: self.phase_min_cells,
                                    // **0 で回す．** 信頼度を行に残しておけば，下限は集計側で
                                    // いくらでも掃ける (Row::outcome_at)
                                    min_confidence: 0.0,
                                    confidence_per_scale: self.confidence_per_scale,
                                    normalize_epsilon: self.normalize_epsilon,
                                });
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

/// 推定の結末．
///
/// **非整数倍リサイズを挟んだ件の正解は「棄却」である．** 周期が $s \cdot r$ (非整数)
/// になる入力に整数の格子は無く，$\mathrm{round}(s \cdot r)$ を採ってもセルごとに
/// 端数がずれる — $s = 6$ ・$r = 1.3$ なら 1 セルにつき 0.2 画素で，40 セル進むと
/// 丸ごと 1 セルずれる．返すべき答えは「格子を検出できない」であり，何かを返したら
/// それは黙って誤答したことになる (設計書 6.1 の再構成検査がここを受け持つ)．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// スケールも位相も正解と一致 (整数の格子がある件だけが到達しうる)．
    Exact,
    /// 整数の格子が無い件を正しく棄却した．
    CorrectRejection,
    /// スケールだけ一致．
    ScaleOnly,
    /// 推定はしたが正解と違う．整数の格子が無い件で何かを返した場合も含む．
    Wrong,
    /// 整数の格子があるのに棄却した (誤棄却)．
    Rejected,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::CorrectRejection => "correct_rejection",
            Self::ScaleOnly => "scale_only",
            Self::Wrong => "wrong",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "exact" => Some(Self::Exact),
            "correct_rejection" => Some(Self::CorrectRejection),
            "scale_only" => Some(Self::ScaleOnly),
            "wrong" => Some(Self::Wrong),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }

    /// 正解か．整数の格子がある件は完全一致，無い件は棄却が正解である．
    pub fn is_correct(self) -> bool {
        matches!(self, Self::Exact | Self::CorrectRejection)
    }
}

/// 掃引の 1 行．
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub param_id: usize,
    /// $\varepsilon$ が割合か絶対値か．**ファイルだけで意味が決まるように残す**
    pub normalized: bool,
    pub epsilon: f32,
    pub delta: f32,
    pub tau: f32,
    /// 帯のずれの許容の下限 (画素)．
    pub phase_floor: f32,
    /// 帯どうしの位相のずれの許容 ($s$ に対する割合)．
    pub phase_tolerance: f32,
    /// 帯ごとの位相**曲線**の食い違いの許容．**列は末尾に足す** — 途中に挿すと
    /// 既存の添字がすべてずれる
    pub phase_agreement: f32,
    /// 帯ごとの位相を副画素で求めたか．
    pub phase_subpixel: bool,
    /// 信頼度の下限を $\hat{s}$ で割るか．**掃引 1 回につき 1 通り** — 下限の意味が
    /// 変わるので，同じ列に混ぜると後から当てはめ直せなくなる
    pub confidence_per_scale: bool,
    pub item_id: u32,
    pub split: Split,
    pub has_integer_grid: bool,
    pub truth_scale: u32,
    pub truth_phase: Option<(u32, u32)>,
    pub effective_scale: f32,
    pub filter: String,
    pub resize: String,
    pub compression: String,
    /// 棄却の理由．推定できたときは空．
    pub error: Option<String>,
    pub scale_hat: Option<u32>,
    pub phase_hat: Option<(u32, u32)>,
    pub confidence: Option<f32>,
    pub mean_variance: Option<f32>,
}

pub const HEADER: &str = "param_id,normalized,epsilon,delta,tau,phase_floor,phase_tolerance,phase_subpixel,confidence_per_scale,item_id,split,has_integer_grid,truth_scale,\
truth_phase_x,truth_phase_y,effective_scale,filter,resize,compression,error,scale_hat,phase_hat_x,\
phase_hat_y,confidence,mean_variance,outcome,phase_agreement";

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

impl Row {
    pub fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            self.param_id,
            self.normalized,
            self.epsilon,
            self.delta,
            self.tau,
            self.phase_floor,
            self.phase_tolerance,
            self.phase_subpixel,
            self.confidence_per_scale,
            self.item_id,
            self.split.as_str(),
            self.has_integer_grid,
            self.truth_scale,
            opt(self.truth_phase.map(|p| p.0)),
            opt(self.truth_phase.map(|p| p.1)),
            self.effective_scale,
            self.filter,
            self.resize,
            self.compression,
            self.error.clone().unwrap_or_default(),
            opt(self.scale_hat),
            opt(self.phase_hat.map(|p| p.0)),
            opt(self.phase_hat.map(|p| p.1)),
            opt(self.confidence),
            opt(self.mean_variance),
            self.outcome().as_str(),
            self.phase_agreement,
        )
    }

    /// 結末．**行の内容から毎回求める．** 採点の定義を変えたときに，掃引をやり直さず
    /// 集計だけ作り直せるようにするためである (CSV の `outcome` 列は人が読むための
    /// 写しであって，判断の元ではない)．
    ///
    /// $\mathrm{min\_confidence}$ は掃引時に 0 で回してあるので，**閾値は後からいくらでも
    /// 掃ける** ([`Self::outcome_at`])．こちらはその 0 の場合である．
    pub fn outcome(&self) -> Outcome {
        self.outcome_at(0.0)
    }

    /// 信頼度の下限を後から当てはめた結末．
    ///
    /// 掃引を `min_confidence = 0` で回し，信頼度を行に残してあるので，**下限は再推定
    /// なしで掃引できる**．$\varepsilon$ ・$\delta$ ・$\tau$ と同時に決めるために要る —
    /// 実データで測ると，正例の誤棄却は閾値で落ちる件と信頼度で落ちる件に割れており，
    /// 片方だけ動かすと打ち消し合う．
    pub fn outcome_at(&self, min_confidence: f32) -> Outcome {
        // **下限は $\hat{s}$ で割る場合がある．** 行に記録した設定に従う —
        // 掃引をやり直さずに当てはめ直せる性質を保つため
        let floor = match (self.confidence_per_scale, self.scale_hat) {
            (true, Some(s)) => min_confidence / s.max(1) as f32,
            _ => min_confidence,
        };
        // 下限に届かない答えは「棄却した」ことになる
        if self.confidence.is_some_and(|c| c < floor) {
            return if self.has_integer_grid {
                Outcome::Rejected
            } else {
                Outcome::CorrectRejection
            };
        }
        self.raw_outcome()
    }

    fn raw_outcome(&self) -> Outcome {
        match (self.scale_hat, self.phase_hat) {
            // 何も返さなかった — 整数の格子が無い件ではこれが正解
            (None, _) | (_, None) => {
                if self.has_integer_grid {
                    Outcome::Rejected
                } else {
                    Outcome::CorrectRejection
                }
            }
            (Some(scale), Some(phase)) => {
                if !self.has_integer_grid {
                    // 整数の格子が無いのに答えを返した = 黙って誤答した
                    Outcome::Wrong
                } else if scale != self.truth_scale {
                    Outcome::Wrong
                } else if self.truth_phase == Some(phase) {
                    Outcome::Exact
                } else {
                    Outcome::ScaleOnly
                }
            }
        }
    }

    pub fn parse(line: &str) -> Result<Self> {
        let f: Vec<&str> = line.split(',').collect();
        anyhow::ensure!(f.len() == 27, "列数が 27 でない: {line}");
        let num = |i: usize| -> Result<f32> {
            f[i].parse()
                .with_context(|| format!("{} 列目が数値でない: {}", i + 1, f[i]))
        };
        let int = |i: usize| -> Result<u32> {
            f[i].parse()
                .with_context(|| format!("{} 列目が整数でない: {}", i + 1, f[i]))
        };
        let pair = |a: usize, b: usize| -> Option<(u32, u32)> {
            match (f[a].parse().ok(), f[b].parse().ok()) {
                (Some(x), Some(y)) => Some((x, y)),
                _ => None,
            }
        };
        // 列そのものは持ち回らない (結末は毎回求め直す) が，読めない値が混じって
        // いたら形式違いとして弾く
        Outcome::parse(f[25]).with_context(|| format!("結末を解釈できない: {}", f[25]))?;

        Ok(Self {
            param_id: int(0)? as usize,
            normalized: f[1] == "true",
            epsilon: num(2)?,
            delta: num(3)?,
            tau: num(4)?,
            phase_floor: num(5)?,
            phase_tolerance: num(6)?,
            phase_subpixel: f[7] == "true",
            confidence_per_scale: f[8] == "true",
            item_id: int(9)?,
            split: if f[10] == "validation" {
                Split::Validation
            } else {
                Split::Test
            },
            has_integer_grid: f[11] == "true",
            truth_scale: int(12)?,
            truth_phase: pair(13, 14),
            effective_scale: num(15)?,
            filter: f[16].to_string(),
            resize: f[17].to_string(),
            compression: f[18].to_string(),
            error: (!f[19].is_empty()).then(|| f[19].to_string()),
            scale_hat: f[20].parse().ok(),
            phase_hat: pair(21, 22),
            confidence: f[23].parse().ok(),
            mean_variance: f[24].parse().ok(),
            phase_agreement: num(26)?,
        })
    }
}

fn error_name(e: &GridError) -> &'static str {
    match e {
        GridError::NotFound => "not_found",
        GridError::TooSmall => "too_small",
        GridError::LowConfidence => "low_confidence",
    }
}

/// 1 件を全パラメータ組で推定する．画像の読み込みは 1 回で済ませる．
fn run_item(dir: &Path, item: &Item, combos: &[GridParams]) -> Result<Vec<Row>> {
    let img = px_io::png::read_rgba(dir.join(&item.file))
        .with_context(|| format!("{} を読めない", item.file))?;

    Ok(combos
        .iter()
        .enumerate()
        .map(|(param_id, params)| {
            let (error, scale_hat, phase_hat, confidence, mean_variance) =
                match estimate_grid(&img, params) {
                    Ok(e) => (
                        None,
                        Some(e.scale),
                        Some((e.phase.x.max(0) as u32, e.phase.y.max(0) as u32)),
                        Some(e.confidence),
                        Some(e.mean_variance),
                    ),
                    Err(e) => (Some(error_name(&e).to_string()), None, None, None, None),
                };
            Row {
                param_id,
                normalized: params.normalize_epsilon,
                epsilon: params.epsilon,
                delta: params.delta,
                phase_floor: params.phase_tolerance_floor,
                phase_tolerance: params.phase_tolerance,
                phase_agreement: params.phase_agreement,
                phase_subpixel: params.phase_subpixel,
                confidence_per_scale: params.confidence_per_scale,
                tau: params.tau,
                item_id: item.id,
                split: item.split,
                has_integer_grid: item.has_integer_grid(),
                truth_scale: item.truth_scale,
                truth_phase: item.truth_phase,
                effective_scale: item.effective_scale,
                filter: item.degradation.filter.as_str().to_string(),
                resize: item.degradation.resize.as_str().to_string(),
                compression: item.degradation.compression.as_str().to_string(),
                error,
                scale_hat,
                phase_hat,
                confidence,
                mean_variance,
            }
        })
        .collect())
}

/// 掃引する．`only` を与えるとその分割だけを対象にする．
///
/// 並列化しても結果は件の順・パラメータ組の順に並ぶ (設計書 6.15 規則 1)．
pub fn run(
    dir: &Path,
    manifest: &Manifest,
    only: Option<Split>,
    grid: &ParamGrid,
) -> Result<Vec<Row>> {
    let combos = grid.combinations();
    let items: Vec<&Item> = manifest
        .items
        .iter()
        .filter(|i| only.is_none_or(|s| i.split == s))
        .collect();

    let per_item: Vec<Result<Vec<Row>>> = items
        .par_iter()
        .map(|item| run_item(dir, item, &combos))
        .collect();

    let mut out = Vec::with_capacity(items.len() * combos.len());
    for rows in per_item {
        out.extend(rows?);
    }
    Ok(out)
}

/// CSV へ書く．
pub fn write_csv(path: &Path, rows: &[Row]) -> Result<()> {
    let mut text = String::with_capacity(rows.len() * 96 + HEADER.len());
    text.push_str(HEADER);
    text.push('\n');
    for row in rows {
        text.push_str(&row.to_csv());
        text.push('\n');
    }
    px_io::atomic::write(path, text.as_bytes())
        .with_context(|| format!("{} を書けない", path.display()))?;
    Ok(())
}

/// CSV を読む．
pub fn read_csv(path: &Path) -> Result<Vec<Row>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("{} が無い．先に sweep を実行すること", path.display()))?;
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default();
    anyhow::ensure!(header == HEADER, "見出し行が掃引の形式と違う");
    lines.map(Row::parse).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset;

    fn sample_row() -> Row {
        Row {
            param_id: 3,
            normalized: false,
            epsilon: 5.0e-3,
            delta: 0.02,
            tau: 0.05,
            phase_floor: 1.0,
            phase_tolerance: 1.0 / 6.0,
            phase_agreement: 1.0,
            phase_subpixel: false,
            confidence_per_scale: false,
            item_id: 17,
            split: Split::Validation,
            has_integer_grid: true,
            truth_scale: 6,
            truth_phase: Some((4, 3)),
            effective_scale: 6.0,
            filter: "bilinear".to_string(),
            resize: "keep".to_string(),
            compression: "jpeg80".to_string(),
            error: None,
            scale_hat: Some(6),
            phase_hat: Some((4, 3)),
            confidence: Some(0.25),
            mean_variance: Some(0.0012),
        }
    }

    /// 整数の格子が無い件 (リサイズあり)．
    fn resized_row() -> Row {
        Row {
            has_integer_grid: false,
            truth_phase: None,
            effective_scale: 7.8,
            resize: "1.30".to_string(),
            ..sample_row()
        }
    }

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

    #[test]
    fn a_row_survives_the_csv_round_trip() {
        let row = sample_row();
        assert_eq!(Row::parse(&row.to_csv()).unwrap(), row);
    }

    #[test]
    fn a_rejected_row_survives_the_csv_round_trip() {
        let row = rejected(sample_row());
        assert_eq!(Row::parse(&row.to_csv()).unwrap(), row);
    }

    #[test]
    fn the_outcome_column_is_a_copy_not_the_source_of_truth() {
        // 掃引をやり直さずに採点の定義を変えられること
        let row = sample_row();
        let mut line: Vec<String> = row.to_csv().split(',').map(str::to_string).collect();
        // **列番号を直書きしない** — 列を足すたびに別の欄を壊して落ちる．
        // 末尾でもない (末尾に列を足したら別の欄を壊す) ．見出しから引く
        let at = HEADER
            .split(',')
            .position(|h| h == "outcome")
            .expect("outcome 列がある");
        line[at] = "wrong".to_string();
        assert_eq!(
            Row::parse(&line.join(",")).unwrap().outcome(),
            Outcome::Exact,
            "CSV の結末欄を信じてしまっている"
        );
    }

    #[test]
    fn an_unknown_outcome_is_rejected_as_a_format_error() {
        let mut line: Vec<String> = sample_row()
            .to_csv()
            .split(',')
            .map(str::to_string)
            .collect();
        *line.last_mut().expect("列がある") = "???".to_string();
        assert!(Row::parse(&line.join(",")).is_err());
    }

    #[test]
    fn the_confidence_floor_can_be_applied_afterwards() {
        // 掃引は下限 0 で回すので，下限は行から当てはめ直せる — 再推定はいらない
        let answered = Row {
            confidence: Some(0.02),
            scale_hat: Some(6),
            phase_hat: Some((0, 0)),
            truth_scale: 6,
            truth_phase: Some((0, 0)),
            has_integer_grid: true,
            ..sample_row()
        };
        assert_eq!(answered.outcome_at(0.0), Outcome::Exact);
        assert_eq!(answered.outcome_at(0.01), Outcome::Exact);
        // 下限に届かなければ「棄却した」ことになる
        assert_eq!(answered.outcome_at(0.03), Outcome::Rejected);

        // 格子が無い件で下限に届かなければ，正しく棄却したことになる
        let noise = Row {
            has_integer_grid: false,
            ..answered.clone()
        };
        assert_eq!(noise.outcome_at(0.01), Outcome::Wrong);
        assert_eq!(noise.outcome_at(0.03), Outcome::CorrectRejection);

        // 信頼度を持たない件 (別の検査で落ちた) は下限に影響されない
        let rejected = Row {
            confidence: None,
            scale_hat: None,
            phase_hat: None,
            ..answered.clone()
        };
        assert_eq!(rejected.outcome_at(0.0), Outcome::Rejected);
        assert_eq!(rejected.outcome_at(0.20), Outcome::Rejected);
    }

    #[test]
    fn the_header_lists_as_many_columns_as_a_row_writes() {
        assert_eq!(
            HEADER.split(',').count(),
            sample_row().to_csv().split(',').count()
        );
    }

    #[test]
    fn combinations_are_the_product_of_the_levels() {
        let grid = ParamGrid::default();
        assert_eq!(
            grid.combinations().len(),
            grid.epsilons.len() * grid.deltas.len() * grid.taus.len()
        );
        // 掃引中の運転点は 0 に固定する
        assert!(grid.combinations().iter().all(|p| p.min_confidence == 0.0));
    }

    #[test]
    fn a_phase_mismatch_is_only_a_scale_match() {
        let row = sample_row();
        assert_eq!(row.outcome(), Outcome::Exact);
        assert_eq!(
            Row {
                phase_hat: Some((0, 0)),
                ..row.clone()
            }
            .outcome(),
            Outcome::ScaleOnly
        );
        assert_eq!(
            Row {
                scale_hat: Some(7),
                ..row
            }
            .outcome(),
            Outcome::Wrong
        );
    }

    #[test]
    fn rejecting_a_grid_that_exists_is_an_error() {
        assert_eq!(rejected(sample_row()).outcome(), Outcome::Rejected);
        assert!(!Outcome::Rejected.is_correct());
    }

    #[test]
    fn rejecting_a_non_integer_grid_is_the_right_answer() {
        // s*r = 7.8 に整数の格子は無い．棄却が正解である
        assert_eq!(rejected(resized_row()).outcome(), Outcome::CorrectRejection);
        assert!(Outcome::CorrectRejection.is_correct());
    }

    #[test]
    fn answering_a_non_integer_grid_is_always_wrong() {
        // round(7.8) = 8 を当てても正解にはしない — セルごとに 0.2 画素ずれる
        for scale in [6, 7, 8] {
            let row = Row {
                scale_hat: Some(scale),
                ..resized_row()
            };
            assert_eq!(row.outcome(), Outcome::Wrong, "s={scale}");
        }
    }

    #[test]
    fn the_dataset_and_the_scoring_agree_on_which_items_have_a_grid() {
        for item in dataset::build(0, 40, &[]).items {
            assert_eq!(item.has_integer_grid(), item.truth_phase.is_some());
        }
    }
}
