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
    pub epsilons: Vec<f32>,
    pub deltas: Vec<f32>,
    pub taus: Vec<f32>,
}

impl Default for ParamGrid {
    fn default() -> Self {
        Self {
            max_scale: 16,
            // 予備調査で「補間を挟むと必要な ε が 1 桁以上大きくなる」ことが
            // 分かっている (開発ノート 5 節)．既定の 2e-4 から 2 桁上まで見る
            epsilons: vec![2.0e-4, 5.0e-4, 1.0e-3, 2.0e-3, 5.0e-3, 1.0e-2, 2.0e-2],
            deltas: vec![0.01, 0.02, 0.05, 0.10],
            taus: vec![0.01, 0.02, 0.05, 0.10],
        }
    }
}

impl ParamGrid {
    /// 組を展開する．順序は $(\varepsilon, \delta, \tau)$ の辞書式で固定する．
    pub fn combinations(&self) -> Vec<GridParams> {
        let mut out = Vec::new();
        for &epsilon in &self.epsilons {
            for &delta in &self.deltas {
                for &tau in &self.taus {
                    out.push(GridParams {
                        max_scale: self.max_scale,
                        epsilon,
                        delta,
                        tau,
                        min_confidence: 0.0,
                    });
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
    pub epsilon: f32,
    pub delta: f32,
    pub tau: f32,
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

pub const HEADER: &str = "param_id,epsilon,delta,tau,item_id,split,has_integer_grid,truth_scale,\
truth_phase_x,truth_phase_y,effective_scale,filter,resize,compression,error,scale_hat,phase_hat_x,\
phase_hat_y,confidence,mean_variance,outcome";

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

impl Row {
    pub fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            self.param_id,
            self.epsilon,
            self.delta,
            self.tau,
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
        )
    }

    /// 結末．**行の内容から毎回求める．** 採点の定義を変えたときに，掃引をやり直さず
    /// 集計だけ作り直せるようにするためである (CSV の `outcome` 列は人が読むための
    /// 写しであって，判断の元ではない)．
    pub fn outcome(&self) -> Outcome {
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
        anyhow::ensure!(f.len() == 21, "列数が 21 でない: {line}");
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
        Outcome::parse(f[20]).with_context(|| format!("結末を解釈できない: {}", f[20]))?;

        Ok(Self {
            param_id: int(0)? as usize,
            epsilon: num(1)?,
            delta: num(2)?,
            tau: num(3)?,
            item_id: int(4)?,
            split: if f[5] == "validation" {
                Split::Validation
            } else {
                Split::Test
            },
            has_integer_grid: f[6] == "true",
            truth_scale: int(7)?,
            truth_phase: pair(8, 9),
            effective_scale: num(10)?,
            filter: f[11].to_string(),
            resize: f[12].to_string(),
            compression: f[13].to_string(),
            error: (!f[14].is_empty()).then(|| f[14].to_string()),
            scale_hat: f[15].parse().ok(),
            phase_hat: pair(16, 17),
            confidence: f[18].parse().ok(),
            mean_variance: f[19].parse().ok(),
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
                epsilon: params.epsilon,
                delta: params.delta,
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
            epsilon: 5.0e-3,
            delta: 0.02,
            tau: 0.05,
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
        line[20] = "wrong".to_string();
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
        line[20] = "???".to_string();
        assert!(Row::parse(&line.join(",")).is_err());
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
        for item in dataset::build(0, 40).items {
            assert_eq!(item.has_integer_grid(), item.truth_phase.is_some());
        }
    }
}
