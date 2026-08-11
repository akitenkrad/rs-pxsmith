//! **再構成検査を測り直す．**
//!
//! 誤棄却の主犯はこの検査である (実データ 50 件のうち 21 件) ．しかも落ちるのは補間が
//! 掛かった入力に限られ，nearest では 1 件も落ちない．
//!
//! 現行の判定は画像全体で「セル平均との色差が $\delta$ を超えた画素の割合」を 1 つ
//! 見るだけである．補間の滲みは**セルの境界に集中する**はずで，中まで滲むわけではない
//! — 本物の格子なら内側は平坦なまま残り，偽物なら内側も一様に汚れる，という見立てが
//! 立つ．これを測る．
//!
//! 手続きは D62 (位相ずれ検査) のときと同じにする．**候補ごとに統計を出し，
//! 「真の $s$ か否か」を単一閾値でどれだけ分けられるか**を均衡正解率で比べる．
//! 分かれない量を実装しても意味が無い．

use std::path::Path;

use anyhow::{Context, Result};
use px_core::grid::{GridParams, ReconStats, recon_stats, scale_candidates};
use rayon::prelude::*;

use crate::dataset::{Manifest, Split};

/// 候補 1 つ分の測定．
#[derive(Clone, Debug)]
pub struct Record {
    pub item_id: u32,
    pub scale: u32,
    pub truth_scale: u32,
    /// **これが真の $s$ か．** 分けたいのはここである．
    pub is_truth: bool,
    pub filter: String,
    pub stats: ReconStats,
    /// $\bar{V}(s)$．
    pub v: f32,
    /// $\bar{V}(\lfloor s/2 \rfloor)$．**$s$ が過大なら半分の $s$ で分散が激減する** —
    /// $2 s_*$ のセルは元の 4 画素を含むが，$s_*$ のセルは 1 画素しか含まないためである．
    /// 真の $s$ なら半分にしても平坦なままで比は 1 に近い．
    pub v_half: f32,
}

pub const HEADER: &str = "item_id,scale,truth_scale,is_truth,filter,\
overall,interior,border,median_delta_e,interior_median_delta_e,v,v_half";

impl Record {
    pub fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{},{:.5},{:.5},{:.5},{:.5},{:.5},{:.6},{:.6}",
            self.item_id,
            self.scale,
            self.truth_scale,
            self.is_truth,
            self.filter,
            self.stats.overall,
            self.stats.interior,
            self.stats.border,
            self.stats.median_delta_e,
            self.stats.interior_median_delta_e,
            self.v,
            self.v_half,
        )
    }
}

/// 整数の格子がある件だけを対象に，各 $s$ の再構成統計を測る．
///
/// 格子が無い件を混ぜない — 「真の $s$」が無いので分けようがなく，混ぜると
/// 何を測っているのか分からなくなる．そちらは位相ずれ検査の担当である．
pub fn run(
    dir: &Path,
    manifest: &Manifest,
    only: Option<Split>,
    params: &GridParams,
) -> Result<Vec<Record>> {
    let items: Vec<_> = manifest
        .items
        .iter()
        .filter(|i| only.is_none_or(|s| i.split == s))
        .filter(|i| i.has_integer_grid())
        .collect();

    let nested: Vec<Vec<Record>> = items
        .par_iter()
        .map(|item| -> Result<Vec<Record>> {
            let img = px_io::png::read_rgba(dir.join(&item.file))
                .with_context(|| format!("{} を読めない", item.file))?;
            let (candidates, _) = scale_candidates(&img, params);
            let v_of = |s: u32| {
                candidates
                    .iter()
                    .find(|c| c.scale == s)
                    .map_or(0.0, |c| c.mean_variance)
            };
            Ok(candidates
                .iter()
                .map(|c| Record {
                    item_id: item.id,
                    scale: c.scale,
                    truth_scale: item.truth_scale,
                    is_truth: c.scale == item.truth_scale,
                    filter: item.degradation.filter.as_str().to_string(),
                    stats: recon_stats(&img, c.scale, c.phase, params.delta),
                    v: c.mean_variance,
                    v_half: v_of(c.scale / 2),
                })
                .collect())
        })
        .collect::<Result<_>>()?;
    Ok(nested.into_iter().flatten().collect())
}

/// 単一閾値で「真の $s$」と「それ以外」を分けたときの均衡正解率．
///
/// 真の $s$ は 1 件につき 1 つしか無いので，件数が大きく偏る．**均衡で見る**．
pub fn separation(records: &[Record], key: impl Fn(&Record) -> f32) -> (f32, f32) {
    let mut values: Vec<f32> = records.iter().map(&key).collect();
    values.sort_by(f32::total_cmp);
    values.dedup();
    let (truth, other): (Vec<&Record>, Vec<&Record>) = records.iter().partition(|r| r.is_truth);
    if truth.is_empty() || other.is_empty() {
        return (0.0, 0.5);
    }

    let mut best = (0.0f32, 0.0f32);
    for &t in &values {
        // 閾値以下を「真の $s$」と見なす (小さいほど本物という向き)
        let tp = truth.iter().filter(|r| key(r) <= t).count() as f32 / truth.len() as f32;
        let tn = other.iter().filter(|r| key(r) > t).count() as f32 / other.len() as f32;
        let balanced = (tp + tn) / 2.0;
        if balanced > best.1 {
            best = (t, balanced);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(is_truth: bool, overall: f32) -> Record {
        Record {
            item_id: 0,
            scale: 4,
            truth_scale: 4,
            is_truth,
            filter: "nearest".to_string(),
            v: overall,
            v_half: overall,
            stats: ReconStats {
                overall,
                interior: overall,
                border: overall,
                median_delta_e: overall,
                interior_median_delta_e: overall,
            },
        }
    }

    #[test]
    fn a_perfectly_separating_statistic_scores_one() {
        let records = vec![
            rec(true, 0.1),
            rec(true, 0.2),
            rec(false, 0.8),
            rec(false, 0.9),
        ];
        let (threshold, balanced) = separation(&records, |r| r.stats.overall);
        assert!((balanced - 1.0).abs() < 1e-6, "均衡正解率 {balanced}");
        assert!((0.2..0.8).contains(&threshold), "閾値 {threshold}");
    }

    #[test]
    fn a_useless_statistic_scores_a_half() {
        // 完全に重なっていれば 0.5 付近にしかならない
        let records = vec![
            rec(true, 0.5),
            rec(false, 0.5),
            rec(true, 0.5),
            rec(false, 0.5),
        ];
        let (_, balanced) = separation(&records, |r| r.stats.overall);
        assert!(balanced <= 0.5 + 1e-6, "均衡正解率 {balanced}");
    }

    #[test]
    fn the_header_lists_as_many_columns_as_a_row_writes() {
        assert_eq!(
            HEADER.split(',').count(),
            rec(true, 0.1).to_csv().split(',').count()
        );
    }
}
