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
//!
//! > [!warning] 均衡正解率だけで採否を決めない
//! > **この口で高い分離能を出した統計が，関門としては役に立たないことがある．**
//! > 差分エネルギーの折り畳みは「真の $s$ vs 2 倍」で 87.1% を出しながら，選択規則
//! > $\hat{s} = \max \{ s \mid \text{関門を通る} \}$ に入れると完全一致 15 / 101
//! > だった (現行の再構成検査は 43 / 101) — 真の $s$ 自身も落とすためである．
//! >
//! > **関門は単独では測れない．** $\varepsilon$ と位相ずれ検査を通した後では，真の
//! > $s$ が残っているのは 59 / 101 しかなく，そこに何を足しても 59 が上限になる．
//! > CSV には `passes_epsilon` ・`passes_phase` を残してあるので，**他の関門を通した
//! > 状態で数え直すこと**．

use std::path::Path;

use anyhow::{Context, Result};
use pxsmith_core::grid::{
    BandAgreement, EdgeFit, GridParams, PhaseContrast, ProfileStats, ReconStats, band_agreement,
    band_phases, band_phases_subpixel, edge_fit, phase_contrast, phase_drift_spread, profile_stats,
    recon_stats, scale_candidates, split_gain, split_recon_gain,
};
use rayon::prelude::*;

use crate::dataset::{Manifest, Split};

/// 候補 1 つ分の測定．
#[derive(Clone, Debug)]
pub struct Record {
    pub item_id: u32,
    /// 目録から見た相対パス．**実データを測るときはこれが件の名前である**
    /// (合成データは `item_id` で足りるが，実データ枠は `local/other/009.png` の
    /// ように «どの絵か» が分からないと漏れを追えない) ．
    pub file: String,
    pub scale: u32,
    pub truth_scale: u32,
    /// 整数の格子がある件か．**無い件も測る** — 位相ずれ検査が本当に相手にしている
    /// のは非整数の周期であり，真の格子と並べないと「揺れ」と「ずれ」を比べられない．
    pub has_integer_grid: bool,
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
    /// 差分エネルギーの折り畳み．**セル平均の残差とは別の情報**である．
    pub profile: ProfileStats,
    /// 半セルずらしたときの崩れ方．**同じ画像で 2 度測った比**なので補間が相殺する．
    pub contrast: PhaseContrast,
    /// 他の関門を通るか．**関門は単独では効かない** — 位相ずれ検査が非整数倍と
    /// 非約数を落とした**後に**何が残っているかで，再構成検査の仕事が決まる．
    pub passes_epsilon: bool,
    pub passes_phase: bool,
    /// 帯ごとの位相の食い違い (画素)．検査を飛ばした場合は `None`．
    pub drift: Option<usize>,
    /// 帯の数を 2 ・3 ・4 と変えたときの，帯ごとの位相そのもの．
    ///
    /// **ずれの最大値 (`cyclic_spread`) は帯 1 本の外れで決まってしまう．** 揺れに強い
    /// まとめ方 (円周上の集中度など) と比べるために，生の並びを残す．帯の数は閾値では
    /// なく**検査の適用範囲**を決めている — 帯が薄いと検査そのものが飛ぶ．
    pub bands_by_count: [Option<(Vec<usize>, Vec<usize>)>; 3],
    /// 同じものを**副画素**で測ったもの．整数の刻みが分けたい量と同じ大きさなので，
    /// 細かくすれば «揺れ» と «ずれ» が分かれるかを見る．
    pub subpixel_by_count: [Option<(Vec<f32>, Vec<f32>)>; 3],
    /// セルを 4 分割したときに説明できる分散の割合．**倍数の抑止だけを担う候補**．
    pub split_gain: f32,
    /// 同じものを**不一致率**で測ったもの ($\delta$ の閾値を残したまま相対化する)．
    pub split_recon_gain: f32,
    /// 画像全体の分散 $\bar{V}_{\mathrm{image}}$．**信頼度の分母**である．
    ///
    /// これがあると CSV だけで **`estimate_grid` の答えを再現できる** — 関門を
    /// 掛け替えたときの完全一致数を，掃引を回さずに数えられる (`crate::replay`) ．
    pub image_var: f32,
    /// 画像の大きさと位相．**帯あたりのセル数**を出すのに要る．
    ///
    /// 非整数の周期のずれは «画像を横切る距離» に比例して溜まるので，同じずれでも
    /// セル数が違えば意味が違う．「どこで切るか」を疑うにはこの量が要る．
    pub width: u32,
    pub height: u32,
    pub phase: (i32, i32),
    /// 帯ごとの位相**曲線**を突き合わせた食い違い．
    ///
    /// 帯ずれ (argmin の食い違い) は落としたい候補で $\lfloor s/2 \rfloor$ に張り付く
    /// — **統計が飽和している**．正規化のしかたを CSV 側で試せるよう，生の 3 つ
    /// ($J$ ・$M$ ・$A$) をそのまま残す．
    pub agreement: Option<BandAgreement>,
    /// **セル境界の位置に直線を当てた当てはまり (1 階差分と 2 階差分)．**
    ///
    /// 帯の位相は標本が 4 つしかなく，浅い谷の argmin なので雑音に負ける．境界の位置は
    /// 数十本あり，しかも**対称な暈けが峰の位置を動かさない** — 補間で滲んだ本物の
    /// 格子を通せる見込みがここにある．
    ///
    /// 階数を両方残すのは，補間で «境界» の現れ方が変わるためである (nearest は 1 階が
    /// 尖り，bilinear はセル中心の間が直線になるので 2 階が尖る) ．**どちらで拾うかは
    /// CSV の上で決める．**
    pub edge1: EdgeFit,
    pub edge2: EdgeFit,
}

pub const HEADER: &str = "file,item_id,scale,truth_scale,has_integer_grid,is_truth,filter,\
overall,interior,border,median_delta_e,interior_median_delta_e,v,v_half,\
edge_share_x,edge_share_y,echo1_x,echo1_y,echo2_x,echo2_y,\
relief1_x,relief1_y,relief2_x,relief2_y,\
vmargin_x,vmargin_y,vratio_x,vratio_y,rratio_x,rratio_y,\
passes_epsilon,passes_phase,drift,bx2,by2,bx3,by3,bx4,by4,\
sx2,sy2,sx3,sy3,sx4,sy4,split_gain,split_recon_gain,image_var,width,height,dx,dy,\
agree_bands,jx,jy,mx,my,ax,ay,\
e1nx,e1ny,e1cx,e1cy,e1rx,e1ry,e1sx,e1sy,\
e2nx,e2ny,e2cx,e2cy,e2rx,e2ry,e2sx,e2sy,\
e2mx,e2my,e2fx,e2fy";

/// 帯ごとの位相を `|` でつなぐ (CSV の 1 欄に収めるため)．
fn join(v: Option<&Vec<usize>>) -> String {
    v.map(|v| v.iter().map(usize::to_string).collect::<Vec<_>>().join("|"))
        .unwrap_or_default()
}

/// 測れなかった量は**空欄**にする．$-1$ などの番兵は，傾きが負を取りうる
/// (2 倍の候補で $-0.5$) この量では «測れない» と区別できない．
fn optf(v: Option<f32>) -> String {
    v.map(|x| format!("{x:.5}")).unwrap_or_default()
}

/// 副画素の位相を `|` でつなぐ．
fn join_f(v: Option<&Vec<f32>>) -> String {
    v.map(|v| {
        v.iter()
            .map(|x| format!("{x:.3}"))
            .collect::<Vec<_>>()
            .join("|")
    })
    .unwrap_or_default()
}

impl Record {
    pub fn to_csv(&self) -> String {
        let p = &self.profile;
        let c = &self.contrast;
        format!(
            "{},{},{},{},{},{},{},{:.5},{:.5},{:.5},{:.5},{:.5},{:.6},{:.6},\
{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},\
{:.5},{:.5},{:.4},{:.4},{:.4},{:.4},{},{},{},{},{},{},{},{},{},\
{},{},{},{},{},{},{:.5},{:.5},{:.6},{},{},{},{},\
{},{:.7},{:.7},{:.7},{:.7},{:.7},{:.7},\
{},{},{:.4},{:.4},{},{},{},{},\
{},{},{:.4},{:.4},{},{},{},{},{},{},{},{}",
            self.file,
            self.item_id,
            self.scale,
            self.truth_scale,
            self.has_integer_grid,
            self.is_truth,
            self.filter,
            self.stats.overall,
            self.stats.interior,
            self.stats.border,
            self.stats.median_delta_e,
            self.stats.interior_median_delta_e,
            self.v,
            self.v_half,
            p.edge_share[0],
            p.edge_share[1],
            p.echo1[0],
            p.echo1[1],
            p.echo2[0],
            p.echo2[1],
            p.relief1[0],
            p.relief1[1],
            p.relief2[0],
            p.relief2[1],
            c.variance_margin[0],
            c.variance_margin[1],
            c.variance_ratio[0],
            c.variance_ratio[1],
            c.recon_ratio[0],
            c.recon_ratio[1],
            self.passes_epsilon,
            self.passes_phase,
            self.drift.map(|d| d as i32).unwrap_or(-1),
            join(self.bands_by_count[0].as_ref().map(|b| &b.0)),
            join(self.bands_by_count[0].as_ref().map(|b| &b.1)),
            join(self.bands_by_count[1].as_ref().map(|b| &b.0)),
            join(self.bands_by_count[1].as_ref().map(|b| &b.1)),
            join(self.bands_by_count[2].as_ref().map(|b| &b.0)),
            join(self.bands_by_count[2].as_ref().map(|b| &b.1)),
            join_f(self.subpixel_by_count[0].as_ref().map(|b| &b.0)),
            join_f(self.subpixel_by_count[0].as_ref().map(|b| &b.1)),
            join_f(self.subpixel_by_count[1].as_ref().map(|b| &b.0)),
            join_f(self.subpixel_by_count[1].as_ref().map(|b| &b.1)),
            join_f(self.subpixel_by_count[2].as_ref().map(|b| &b.0)),
            join_f(self.subpixel_by_count[2].as_ref().map(|b| &b.1)),
            self.split_gain,
            self.split_recon_gain,
            self.image_var,
            self.width,
            self.height,
            self.phase.0,
            self.phase.1,
            self.agreement.map_or(0, |a| a.bands),
            self.agreement.map_or(-1.0, |a| a.joint[0]),
            self.agreement.map_or(-1.0, |a| a.joint[1]),
            self.agreement.map_or(-1.0, |a| a.separate[0]),
            self.agreement.map_or(-1.0, |a| a.separate[1]),
            self.agreement.map_or(-1.0, |a| a.level[0]),
            self.agreement.map_or(-1.0, |a| a.level[1]),
            self.edge1.count[0],
            self.edge1.count[1],
            self.edge1.coverage[0],
            self.edge1.coverage[1],
            optf(self.edge1.residual[0]),
            optf(self.edge1.residual[1]),
            optf(self.edge1.slope[0]),
            optf(self.edge1.slope[1]),
            self.edge2.count[0],
            self.edge2.count[1],
            self.edge2.coverage[0],
            self.edge2.coverage[1],
            optf(self.edge2.residual[0]),
            optf(self.edge2.residual[1]),
            optf(self.edge2.slope[0]),
            optf(self.edge2.slope[1]),
            optf(self.edge2.residual_median[0]),
            optf(self.edge2.residual_median[1]),
            optf(self.edge2.residual_folded[0]),
            optf(self.edge2.residual_folded[1]),
        )
    }

    /// この候補が真の $s$ の何倍か．約数なら 1 未満になる．
    pub fn ratio(&self) -> f32 {
        self.scale as f32 / self.truth_scale as f32
    }

    /// 真の $s$ の整数倍 ($2$ 倍以上) か．**再構成検査が唯一止めている相手**である．
    pub fn is_multiple(&self) -> bool {
        !self.is_truth && self.scale.is_multiple_of(self.truth_scale)
    }

    /// 真の $s$ の約数か．検査を入れると勝ってしまう相手である．
    pub fn is_divisor(&self) -> bool {
        !self.is_truth && self.truth_scale.is_multiple_of(self.scale)
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
    include_resized: bool,
) -> Result<Vec<Record>> {
    let items: Vec<_> = manifest
        .items
        .iter()
        .filter(|i| only.is_none_or(|s| i.split == s))
        .filter(|i| include_resized || i.has_integer_grid())
        .collect();

    let nested: Vec<Vec<Record>> = items
        .par_iter()
        .map(|item| -> Result<Vec<Record>> {
            of_image(
                dir,
                &Subject {
                    id: item.id,
                    file: item.file.clone(),
                    truth_scale: item.truth_scale,
                    has_integer_grid: item.has_integer_grid(),
                    filter: item.degradation.filter.as_str().to_string(),
                },
                params,
            )
        })
        .collect::<Result<_>>()?;
    Ok(nested.into_iter().flatten().collect())
}

/// 測る対象 1 件．**合成データと実データで共通の «正解の持ち方»．**
///
/// 実データの目録 ([`crate::real::Item`]) は補間法も添字も持たないが，測る中身は
/// 同じである — **同じ CSV に出せれば，同じ道具で解析できる**．
pub struct Subject {
    pub id: u32,
    pub file: String,
    pub truth_scale: u32,
    pub has_integer_grid: bool,
    pub filter: String,
}

/// 1 枚から候補ぶんの行を作る．
fn of_image(dir: &Path, subject: &Subject, params: &GridParams) -> Result<Vec<Record>> {
    let img = pxsmith_io::png::read_rgba(dir.join(&subject.file))
        .with_context(|| format!("{} を読めない", subject.file))?;
    let (candidates, image_var) = scale_candidates(&img, params);
    let v_of = |s: u32| {
        candidates
            .iter()
            .find(|c| c.scale == s)
            .map_or(0.0, |c| c.mean_variance)
    };
    Ok(candidates
        .iter()
        .map(|c| Record {
            item_id: subject.id,
            file: subject.file.clone(),
            scale: c.scale,
            truth_scale: subject.truth_scale,
            has_integer_grid: subject.has_integer_grid,
            is_truth: subject.has_integer_grid && c.scale == subject.truth_scale,
            filter: subject.filter.clone(),
            stats: recon_stats(&img, c.scale, c.phase, params.delta),
            v: c.mean_variance,
            v_half: v_of(c.scale / 2),
            profile: profile_stats(&img, c.scale, c.phase),
            contrast: phase_contrast(&img, c.scale, c.phase, params.delta),
            passes_epsilon: c.passes_epsilon,
            passes_phase: c.passes_phase,
            bands_by_count: [2, 3, 4]
                .map(|b| band_phases(&img, c.scale, c.phase, b, params.phase_min_cells)),
            subpixel_by_count: [2, 3, 4]
                .map(|b| band_phases_subpixel(&img, c.scale, c.phase, b, params.phase_min_cells)),
            image_var,
            width: img.width(),
            height: img.height(),
            phase: (c.phase.x, c.phase.y),
            agreement: band_agreement(
                &img,
                c.scale,
                c.phase,
                params.phase_bands,
                params.phase_min_cells,
            ),
            edge1: edge_fit(&img, c.scale, 1, params),
            edge2: edge_fit(&img, c.scale, 2, params),
            split_gain: split_gain(&img, c.scale, c.phase),
            split_recon_gain: split_recon_gain(&img, c.scale, c.phase, params.delta),
            drift: phase_drift_spread(
                &img,
                c.scale,
                c.phase,
                params.phase_bands,
                params.phase_min_cells,
            ),
        })
        .collect())
}

/// **実データの目録で同じ測定を行う．**
///
/// `diagnose` は正解が分かっている件の**誤棄却**しか見ない — 負例に何が起きたかは
/// 判定行 (「s=14 位相=(12,13) 信頼度 0.074」) からしか読めなかった．
/// D72 ・D73 の判断はどちらも «漏れた件の $\hat{s}$ と信頼度» だけを頼りに下しており，
/// **候補ごとの統計を見ないまま «塞げない» と結論する**ところだった．
///
/// 実データ枠が採否を決める枠になった以上，ここも合成データと同じ CSV へ出す．
pub fn run_real(
    dir: &Path,
    manifest: &crate::real::Manifest,
    params: &GridParams,
) -> Result<Vec<Record>> {
    let subjects: Vec<Subject> = manifest
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| Subject {
            id: i as u32,
            file: item.file.clone(),
            // 正解が分からない件は «格子なし» と混ぜない — `truth` があるものだけ
            // 正例として扱い，`no_grid` の件は truth_scale 0 のまま負例にする
            truth_scale: item.truth.map_or(0, |t| t.scale),
            has_integer_grid: item.truth.is_some(),
            // 補間法は分からないので «出どころ» を入れる (分布のずれを見る区分)
            filter: format!("{:?}", item.category).to_lowercase(),
        })
        .collect();

    let nested: Vec<Vec<Record>> = subjects
        .par_iter()
        .map(|s| of_image(dir, s, params))
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
            file: "0000.png".to_string(),
            scale: 4,
            truth_scale: 4,
            has_integer_grid: true,
            is_truth,
            filter: "nearest".to_string(),
            v: overall,
            v_half: overall,
            profile: ProfileStats {
                edge_share: [overall; 2],
                echo1: [overall; 2],
                echo2: [overall; 2],
                relief1: [overall; 2],
                relief2: [overall; 2],
            },
            contrast: PhaseContrast {
                variance_margin: [overall; 2],
                variance_ratio: [overall; 2],
                recon_ratio: [overall; 2],
            },
            passes_epsilon: true,
            passes_phase: true,
            drift: None,
            bands_by_count: [None, None, None],
            subpixel_by_count: [None, None, None],
            split_gain: overall,
            split_recon_gain: overall,
            image_var: overall,
            width: 64,
            height: 64,
            phase: (0, 0),
            agreement: None,
            edge1: EdgeFit {
                count: [8, 8],
                coverage: [1.0; 2],
                residual: [Some(overall); 2],
                slope: [Some(overall); 2],
                residual_median: [Some(overall); 2],
                residual_folded: [Some(overall); 2],
            },
            edge2: EdgeFit {
                count: [0, 0],
                coverage: [0.0; 2],
                residual: [None; 2],
                slope: [None; 2],
                residual_median: [None; 2],
                residual_folded: [None; 2],
            },
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
    fn a_candidate_knows_whether_it_is_a_multiple_or_a_divisor() {
        let mut r = rec(false, 0.0);
        r.truth_scale = 4;
        r.scale = 8;
        assert!(r.is_multiple() && !r.is_divisor());
        r.scale = 2;
        assert!(r.is_divisor() && !r.is_multiple());
        // 真の s 自身はどちらでもない (自分自身の倍数でも約数でもある値だが除く)
        r.scale = 4;
        r.is_truth = true;
        assert!(!r.is_multiple() && !r.is_divisor());
    }

    #[test]
    fn the_header_lists_as_many_columns_as_a_row_writes() {
        assert_eq!(
            HEADER.split(',').count(),
            rec(true, 0.1).to_csv().split(',').count()
        );
    }
}
