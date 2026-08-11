//! 信頼度の中身を測る．
//!
//! 位相ずれ検査を入れた後，残った誤答の 86% が $\hat{s} = 2$ への転落だった．
//! これを止めるのは信頼度の役目だが (設計書 6.1) ，測ると**逆を向いている** —
//! 誤答の方が正解より自信がある．
//!
//! $$ \mathrm{conf} = \mathrm{clamp}\left( \frac{\min_{s \in G} \bar{V}(s) - \bar{V}(\hat{s})}{\bar{V}_{\mathrm{image}}},\ 0,\ 1 \right) $$
//!
//! **式を直す前に，この式の各項が実際にどうなっているのかを見る．** 分子が潰れて
//! いるのか，分母が大きすぎるのか，対照群の誰が最小なのかで打ち手が変わる．

use std::path::Path;

use anyhow::{Context, Result};
use px_core::grid::{
    GridParams, ScaleCandidate, control_group_of, estimate_grid, scale_candidates,
};
use rayon::prelude::*;

use crate::dataset::{Item, Manifest, Split};

/// 1 件分の測定．
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub item_id: u32,
    pub has_integer_grid: bool,
    pub truth_scale: u32,
    pub scale_hat: u32,
    /// 答えが正解か (格子ありで $(s, d)$ が一致)．
    pub correct: bool,
    pub confidence: f32,
    /// $\bar{V}(\hat{s})$．
    pub v_hat: f32,
    /// 対照群の最小分散．
    pub v_rival: f32,
    /// その最小を与えた $s$ — **誰と競っているのか**．
    pub rival_scale: u32,
    /// $\bar{V}_{\mathrm{image}}$ (信頼度の分母)．
    pub v_image: f32,
    /// 分子 (クランプ前)．
    pub margin: f32,
    /// 分子を $\bar{V}(\hat{s})$ で割ったもの — 分母を変えたらどうなるかの目安．
    pub relative_margin: f32,
    /// 全候補の $(s, \bar{V}(s))$．**別の定義を試すには曲線ごと要る**．
    pub curve: Vec<(u32, f32)>,
    /// 候補ごとの関門の通過状況 `s:erp` (通ったものだけ文字が立つ) ．
    /// **真のスケールがどこで落ちたのかを見るために要る**．
    pub gates: Vec<(u32, bool, bool, bool)>,
}

pub const HEADER: &str = "item_id,has_integer_grid,truth_scale,scale_hat,correct,confidence,\
v_hat,v_rival,rival_scale,v_image,margin,relative_margin,curve,gates";

impl Record {
    pub fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{},{:.6},{:.8},{:.8},{},{:.8},{:.8},{:.4},{},{}",
            self.item_id,
            self.has_integer_grid,
            self.truth_scale,
            self.scale_hat,
            self.correct,
            self.confidence,
            self.v_hat,
            self.v_rival,
            self.rival_scale,
            self.v_image,
            self.margin,
            self.relative_margin,
            self.curve
                .iter()
                .map(|(s, v)| format!("{s}:{v:.8}"))
                .collect::<Vec<_>>()
                .join(" "),
            self.gates
                .iter()
                .map(|(s, e, r, p)| {
                    let flag = |ok: bool, c: char| if ok { c } else { '-' };
                    format!("{s}:{}{}{}", flag(*e, 'e'), flag(*r, 'r'), flag(*p, 'p'))
                })
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

fn measure(item: &Item, cands: &[ScaleCandidate], v_image: f32, hat: u32, correct: bool) -> Record {
    let group = control_group_of(hat, cands, 16);
    let rival = cands
        .iter()
        .filter(|c| group.contains(&c.scale))
        .min_by(|a, b| a.mean_variance.total_cmp(&b.mean_variance));
    let v_hat = cands
        .iter()
        .find(|c| c.scale == hat)
        .map(|c| c.mean_variance)
        .unwrap_or(0.0);
    let (v_rival, rival_scale) = rival.map_or((0.0, 0), |c| (c.mean_variance, c.scale));
    let margin = v_rival - v_hat;

    Record {
        item_id: item.id,
        has_integer_grid: item.has_integer_grid(),
        truth_scale: item.truth_scale,
        scale_hat: hat,
        correct,
        confidence: if v_image > 0.0 {
            (margin / v_image).clamp(0.0, 1.0)
        } else {
            0.0
        },
        v_hat,
        v_rival,
        rival_scale,
        v_image,
        margin,
        relative_margin: if v_hat > 0.0 {
            margin / v_hat
        } else {
            f32::NAN
        },
        curve: cands.iter().map(|c| (c.scale, c.mean_variance)).collect(),
        gates: cands
            .iter()
            .map(|c| (c.scale, c.passes_epsilon, c.passes_recon, c.passes_phase))
            .collect(),
    }
}

/// 答えを返した件について，信頼度の各項を測る．
pub fn run(
    dir: &Path,
    manifest: &Manifest,
    only: Option<Split>,
    params: &GridParams,
) -> Result<Vec<Record>> {
    // 運転点は測定の対象外にする — 棄却された件こそ「なぜ落ちたのか」を見たい
    let params = &GridParams {
        min_confidence: 0.0,
        ..*params
    };

    let items: Vec<&Item> = manifest
        .items
        .iter()
        .filter(|i| only.is_none_or(|s| i.split == s))
        .collect();

    let measured: Vec<Result<Option<Record>>> = items
        .par_iter()
        .map(|item| -> Result<Option<Record>> {
            let img = px_io::png::read_rgba(dir.join(&item.file))
                .with_context(|| format!("{} を読めない", item.file))?;
            // 答えを返さなかった件は信頼度を持たないので対象外
            let Ok(e) = estimate_grid(&img, params) else {
                return Ok(None);
            };
            let (cands, v_image) = scale_candidates(&img, params);
            let phase = (e.phase.x.max(0) as u32, e.phase.y.max(0) as u32);
            let correct = item.has_integer_grid()
                && e.scale == item.truth_scale
                && item.truth_phase == Some(phase);
            Ok(Some(measure(item, &cands, v_image, e.scale, correct)))
        })
        .collect();

    let mut out = Vec::with_capacity(items.len());
    for r in measured {
        out.extend(r?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use px_core::ivec2;

    use super::*;
    use crate::dataset;

    fn candidate(scale: u32, mean_variance: f32) -> ScaleCandidate {
        ScaleCandidate {
            scale,
            mean_variance,
            phase: ivec2(0, 0),
            passes_epsilon: true,
            passes_recon: true,
            passes_phase: true,
        }
    }

    fn item() -> Item {
        dataset::build(0, 4, &[]).items.remove(0)
    }

    #[test]
    fn the_rival_is_the_smallest_variance_outside_the_control_group() {
        let cands = [
            candidate(2, 0.001), // ŝ=4 の約数なので対照群に入らない
            candidate(3, 0.020),
            candidate(4, 0.002), // これが ŝ
            candidate(5, 0.008), // 対照群の最小
            candidate(8, 0.001), // 倍数なので入らない
        ];
        let r = measure(&item(), &cands, 0.05, 4, true);
        assert_eq!(r.rival_scale, 5, "約数か倍数を対戦相手にしている");
        assert!((r.v_rival - 0.008).abs() < 1e-6);
        assert!((r.margin - 0.006).abs() < 1e-6);
    }

    #[test]
    fn the_margin_is_reported_before_the_clamp() {
        // 対照群の方が分散が小さい (負のマージン) 場合，信頼度は 0 に潰れるが
        // margin には符号が残る — これが無いと「なぜ 0 なのか」が分からない
        let cands = [candidate(4, 0.010), candidate(5, 0.004)];
        let r = measure(&item(), &cands, 0.05, 4, true);
        assert!(r.margin < 0.0);
        assert_eq!(r.confidence, 0.0);
    }

    #[test]
    fn a_flat_image_has_no_confidence() {
        let cands = [candidate(4, 0.0), candidate(5, 0.0)];
        let r = measure(&item(), &cands, 0.0, 4, true);
        assert_eq!(r.confidence, 0.0, "分母 0 で割っている");
    }

    #[test]
    fn a_record_survives_the_csv_shape() {
        let cands = [candidate(4, 0.002), candidate(5, 0.008)];
        let line = measure(&item(), &cands, 0.05, 4, true).to_csv();
        assert_eq!(line.split(',').count(), HEADER.split(',').count());
    }
}
