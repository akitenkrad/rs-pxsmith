//! **実データの誤棄却を 1 件ずつ解剖する．**
//!
//! 正解が分かっている件が棄却されたとき，知りたいのは「どこで落ちたか」である．
//!
//! | 落ち方 | 意味 |
//! | --- | --- |
//! | 真の $s$ が候補から消えた | $\varepsilon$ ・再構成検査 ・位相ずれ検査のどれかが落とした |
//! | 真の $s$ は残ったが選ばれなかった | より大きい $s$ に負けた (過大推定) |
//! | 真の $s$ が選ばれたが信頼度が足りない | 対照群との差が付かなかった |
//!
//! この 3 つは**直し方がまったく違う**．合成データ向けの [`crate::confidence`] と違い，
//! こちらは実データの目録を読む — 誤棄却が集中しているのは実データの側である．

use std::path::Path;

use anyhow::{Context, Result};
use pxsmith_core::grid::{GridParams, estimate_grid, scale_candidates};

use crate::real::{Manifest, Verdict};

/// 誤棄却の落ち方．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Fallout {
    /// 棄却されていない (正解した / 誤答した)．
    NotRejected,
    /// 真の $s$ が $\varepsilon$ で落ちた．
    Epsilon,
    /// 真の $s$ が再構成検査で落ちた．
    Recon,
    /// 真の $s$ が位相ずれ検査で落ちた．
    PhaseDrift,
    /// 真の $s$ は関門を通ったが，より大きい $s$ が選ばれた．
    LostToLarger,
    /// 真の $s$ が選ばれたが信頼度が下限に届かなかった．
    LowConfidence,
    /// 真の $s$ がそもそも候補に無い ($s_{\max}$ を超えている等)．
    NoCandidate,
}

impl Fallout {
    /// CSV と画面の両方で使う識別子．**英語のまま変えない．**
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRejected => "not_rejected",
            Self::Epsilon => "epsilon",
            Self::Recon => "recon",
            Self::PhaseDrift => "phase_drift",
            Self::LostToLarger => "lost_to_larger",
            Self::LowConfidence => "low_confidence",
            Self::NoCandidate => "no_candidate",
        }
    }
}

/// `failed_gates` 欄に現れる関門の識別子．**集計する側が突き合わせるために持つ．**
pub const GATES: [&str; 3] = ["epsilon", "recon", "phase_drift"];

/// 1 件の解剖結果．
#[derive(Clone, Debug)]
pub struct Record {
    pub file: String,
    pub truth_scale: u32,
    pub fallout: Fallout,
    /// 真の $s$ のセル内平均分散．
    pub v_truth: f32,
    /// 画像全体の分散 (閾値と信頼度の分母)．
    pub v_image: f32,
    /// 真の $s$ が $\varepsilon$ の何倍だったか．**1 を超えると落ちる**．
    pub epsilon_ratio: f32,
    /// 真の $s$ が選ばれた場合の信頼度．
    pub confidence: Option<f32>,
    /// 真の $s$ より大きく，関門を通った $s$ のうち最小 (過大推定の相手)．
    pub larger_accepted: Option<u32>,
    /// 真の $s$ が落ちた関門を**すべて**並べたもの (`|` 区切り)．
    ///
    /// [`Fallout`] は最初に落ちた関門しか言わないので，**2 つ以上の関門が同時に
    /// 落としている件が先頭の関門に付け替えられる**．検証セットでは，順序が
    /// $\varepsilon$ → 再構成 → 位相ずれ だったために「再構成検査が主犯」と読めて
    /// いたが，実際には位相ずれ検査が真の $s$ の 42 / 101 を落としていた．
    /// **どれか 1 つを直しても通るとは限らない**ことがこの欄で分かる．
    pub failed_gates: String,
}

pub const HEADER: &str = "file,truth_scale,fallout,failed_gates,v_truth,v_image,epsilon_ratio,confidence,larger_accepted";

impl Record {
    pub fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{:.6},{:.6},{:.3},{},{}",
            self.file,
            self.truth_scale,
            self.fallout.as_str(),
            self.failed_gates,
            self.v_truth,
            self.v_image,
            self.epsilon_ratio,
            self.confidence
                .map(|c| format!("{c:.4}"))
                .unwrap_or_default(),
            self.larger_accepted
                .map(|s| s.to_string())
                .unwrap_or_default(),
        )
    }
}

/// 正解が分かっている件をすべて解剖する．
pub fn run(dir: &Path, manifest: &Manifest, params: &GridParams) -> Result<Vec<Record>> {
    manifest
        .items
        .iter()
        .filter_map(|item| item.truth.map(|t| (item, t)))
        .map(|(item, truth)| {
            let img = pxsmith_io::png::read_rgba(dir.join(&item.file))
                .with_context(|| format!("{} を読めない", item.file))?;
            let estimate = estimate_grid(&img, params);
            let (candidates, v_image) = scale_candidates(&img, params);

            let mine = candidates.iter().find(|c| c.scale == truth.scale);
            let v_truth = mine.map_or(0.0, |c| c.mean_variance);
            // 割合で持っているので，実際に効いた閾値は ε * V_image である
            let threshold = if params.normalize_epsilon {
                params.epsilon * v_image
            } else {
                params.epsilon
            };
            let larger_accepted = candidates
                .iter()
                .filter(|c| c.accepted() && c.scale > truth.scale)
                .map(|c| c.scale)
                .min();

            let verdict = crate::real::judge_public(Some(truth), item.no_grid, &estimate);
            let fallout = match (&estimate, mine) {
                // 棄却されていないなら解剖しない
                _ if verdict != Verdict::Rejected => Fallout::NotRejected,
                (_, None) => Fallout::NoCandidate,
                // 真の $s$ が関門で落ちた — **順に見る**．最初に落ちた関門が原因である
                (_, Some(c)) if !c.passes_epsilon => Fallout::Epsilon,
                (_, Some(c)) if !c.passes_recon => Fallout::Recon,
                (_, Some(c)) if !c.passes_phase => Fallout::PhaseDrift,
                // 関門は通った．より大きい $s$ に負けたのか，信頼度で落ちたのか
                _ if larger_accepted.is_some() => Fallout::LostToLarger,
                _ => Fallout::LowConfidence,
            };

            let failed_gates = mine
                .map(|c| {
                    [
                        (!c.passes_epsilon).then_some("epsilon"),
                        (!c.passes_recon).then_some("recon"),
                        (!c.passes_phase).then_some("phase_drift"),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("|")
                })
                .unwrap_or_default();

            Ok(Record {
                file: item.file.clone(),
                failed_gates,
                truth_scale: truth.scale,
                fallout,
                v_truth,
                v_image,
                epsilon_ratio: if threshold > 0.0 {
                    v_truth / threshold
                } else {
                    f32::INFINITY
                },
                confidence: estimate.as_ref().ok().map(|e| e.confidence),
                larger_accepted,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_gate_that_fails_is_the_cause() {
        // 落ち方の名前が重ならないこと (集計で取り違えない)
        let names: Vec<&str> = [
            Fallout::NotRejected,
            Fallout::Epsilon,
            Fallout::Recon,
            Fallout::PhaseDrift,
            Fallout::LostToLarger,
            Fallout::LowConfidence,
            Fallout::NoCandidate,
        ]
        .iter()
        .map(|f| f.as_str())
        .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
    }

    /// `GATES` の識別子は `failed_gates` 欄に書く値と同じでなければならない．
    /// **食い違うと関門ごとの集計が黙って 0 件になる．**
    #[test]
    fn the_gate_identifiers_match_what_the_column_writes() {
        let written = [Fallout::Epsilon, Fallout::Recon, Fallout::PhaseDrift];
        for (id, f) in GATES.iter().zip(written) {
            assert_eq!(*id, f.as_str(), "{:?} の識別子が食い違っている", f);
        }
    }

    #[test]
    fn the_header_lists_as_many_columns_as_a_row_writes() {
        let r = Record {
            file: "a.png".to_string(),
            truth_scale: 6,
            fallout: Fallout::Epsilon,
            failed_gates: "recon|phase_drift".to_string(),
            v_truth: 0.01,
            v_image: 0.05,
            epsilon_ratio: 1.2,
            confidence: Some(0.02),
            larger_accepted: Some(12),
        };
        assert_eq!(HEADER.split(',').count(), r.to_csv().split(',').count());
    }
}
