//! 再構成誤差を帯ごとに分けて測る (校正の行き止まりを抜けるための実測)．
//!
//! 掃引で分かったのは，再構成検査が画像全体で 1 つの割合しか見ていないために
//! 2 つの別物が同じ数に潰れている，ということだった
//! (`docs/investigations/grid-calibration.md`)．
//!
//! 測ってみた結果，**当初の見込みは外れ，別の統計が当たった**．
//!
//! | 帯ごとに見る量 | 均衡正解率 |
//! | --- | --- |
//! | 全体の不一致率 (現行の再構成検査) | 76.6% |
//! | 不一致率の帯ごとのばらつき (当初の提案) | 77.3% |
//! | **帯ごとに最も合う位相のずれ** | **95.9%** |
//!
//! 不一致率が伸びなかったのは，帯ごとの値が**絵の中身に左右される**ためである．
//! ディザ帯のある行は不一致が多く，平坦な行は少ない — 格子の素性とは関係のない偏りが
//! そのまま混ざる．[`Profile`] はこの経緯を残すために置いてあるが，判定には使えない．
//!
//! 効いたのは [`PhaseDrift`] の方である．「その帯で最も合う位相」は中身に依らず，
//! 格子が本物ならどの帯でも同じ値になる．周期が非整数なら帯が進むほどずれていく．
//!
//! ずれは**単調増加とは限らない**．真の周期 $p = s \cdot r$ を整数 $q$ で近似すると，
//! $k$ 番目のセル境界のずれは $|q - p| \cdot k$ で増えるが，$p$ を超えると一周して
//! 再び合う．$s = 6$ ・$r = 1.3$ なら $p = 7.8$ ・$q = 8$ で，ずれが 1 セル分になるのは
//! 39 セル先である．よって**傾きではなく帯どうしの離れ具合**を見る．
//!
//! 不一致率の定義は `px_core::grid` の再構成検査と同じ — 画素とそのセルの平均色の
//! $\Delta E_{\mathrm{OKLab}}$ が $\delta$ を超えたら不一致とする．違うのは，
//! 全体で 1 つの割合に潰さずセルの位置で分けるところだけである．

use anyhow::{Context, Result};
use px_core::color::{delta_e, oklab_of};
use px_core::grid::{GridParams, estimate_grid};
use px_core::{IVec2, Rgba8, RgbaCanvas};
use rayon::prelude::*;

use crate::dataset::{Item, Manifest, Split};

/// 帯の数 (縦横それぞれ)．
pub const BANDS: usize = 4;

/// 帯ごとの不一致率が意味を持つために要るセル数 (1 軸あたり)．
pub const MIN_CELLS: usize = 8;

/// 帯ごとに分けた再構成誤差．
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// 画像全体の不一致率 (現行の再構成検査が見ている値そのもの)．
    pub overall: f32,
    /// 列方向に [`BANDS`] 等分した帯の不一致率．
    pub by_x: Vec<f32>,
    /// 行方向に [`BANDS`] 等分した帯の不一致率．
    pub by_y: Vec<f32>,
}

impl Profile {
    /// 帯のばらつき (最大 - 最小)．**当初はこれが主役になる見込みだったが外れた** —
    /// 絵の中身による偏りと格子のずれが混ざるため，現行の指標とほぼ同じ成績になる．
    pub fn spread(&self) -> f32 {
        spread(&self.by_x).max(spread(&self.by_y))
    }

    /// ばらつきを全体の水準で割ったもの．滲みが強い入力ほど `overall` も上がるので，
    /// 「滲みの量に対してどれだけ偏っているか」を見るにはこちらが要る．
    pub fn relative_spread(&self) -> f32 {
        if self.overall <= f32::EPSILON {
            0.0
        } else {
            self.spread() / self.overall
        }
    }

    /// 帯を左から右へ見たときの傾き (最小二乗) ．一周して戻る場合があるので，
    /// 判定には使わず記録だけする．
    pub fn slope(&self) -> f32 {
        slope(&self.by_x).abs().max(slope(&self.by_y).abs())
    }
}

fn spread(bands: &[f32]) -> f32 {
    let max = bands.iter().copied().fold(f32::MIN, f32::max);
    let min = bands.iter().copied().fold(f32::MAX, f32::min);
    if bands.is_empty() { 0.0 } else { max - min }
}

fn slope(bands: &[f32]) -> f32 {
    let n = bands.len() as f32;
    if n < 2.0 {
        return 0.0;
    }
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = bands.iter().sum::<f32>() / n;
    let mut num = 0.0;
    let mut den = 0.0;
    for (i, y) in bands.iter().enumerate() {
        let dx = i as f32 - mean_x;
        num += dx * (y - mean_y);
        den += dx * dx;
    }
    if den == 0.0 { 0.0 } else { num / den }
}

/// セルの平均色．`px_core::grid` と同じく `u8` の平均を採ってから OKLab へ移す．
fn cell_mean(img: &RgbaCanvas, x0: u32, y0: u32, s: u32) -> Rgba8 {
    let mut sum = [0u32; 4];
    let mut n = 0u32;
    for y in y0..y0 + s {
        for x in x0..x0 + s {
            let Some(c) = img.get(x as i32, y as i32) else {
                continue;
            };
            sum[0] += u32::from(c.r);
            sum[1] += u32::from(c.g);
            sum[2] += u32::from(c.b);
            sum[3] += u32::from(c.a);
            n += 1;
        }
    }
    if n == 0 {
        return Rgba8::TRANSPARENT;
    }
    Rgba8::new(
        (sum[0] / n) as u8,
        (sum[1] / n) as u8,
        (sum[2] / n) as u8,
        (sum[3] / n) as u8,
    )
}

/// 帯ごとの不一致率を測る．端の欠けたセルは数えない (再構成検査と同じ)．
///
/// セル数が [`MIN_CELLS`] に満たない軸があれば `None` — 帯に分けても意味が無い．
pub fn profile(img: &RgbaCanvas, scale: u32, phase: IVec2, delta: f32) -> Option<Profile> {
    let s = scale.max(1);
    let (dx, dy) = (phase.x.max(0) as u32, phase.y.max(0) as u32);
    let cells_x = (img.width().saturating_sub(dx) / s) as usize;
    let cells_y = (img.height().saturating_sub(dy) / s) as usize;
    if cells_x < MIN_CELLS || cells_y < MIN_CELLS {
        return None;
    }

    // (不一致画素数, 総画素数) を帯ごとに積む
    let mut acc_x = vec![(0usize, 0usize); BANDS];
    let mut acc_y = vec![(0usize, 0usize); BANDS];
    let mut total = (0usize, 0usize);

    for cy in 0..cells_y {
        for cx in 0..cells_x {
            let (x0, y0) = (dx + cx as u32 * s, dy + cy as u32 * s);
            let mean = oklab_of(cell_mean(img, x0, y0, s));

            let mut mismatched = 0usize;
            let mut n = 0usize;
            for y in y0..y0 + s {
                for x in x0..x0 + s {
                    let Some(c) = img.get(x as i32, y as i32) else {
                        continue;
                    };
                    n += 1;
                    if delta_e(oklab_of(c), mean) > delta {
                        mismatched += 1;
                    }
                }
            }

            let bx = cx * BANDS / cells_x;
            let by = cy * BANDS / cells_y;
            acc_x[bx].0 += mismatched;
            acc_x[bx].1 += n;
            acc_y[by].0 += mismatched;
            acc_y[by].1 += n;
            total.0 += mismatched;
            total.1 += n;
        }
    }

    let ratio = |(m, n): (usize, usize)| if n == 0 { 0.0 } else { m as f32 / n as f32 };
    Some(Profile {
        overall: ratio(total),
        by_x: acc_x.into_iter().map(ratio).collect(),
        by_y: acc_y.into_iter().map(ratio).collect(),
    })
}

/// 帯ごとに最も合う位相．
///
/// **不一致率が絵の中身に左右されるのに対し，位相は左右されない．** 不一致率は
/// ディザ帯のある行で高く平坦な行で低いので，帯のばらつきに中身の偏りが混ざる．
/// 一方「その帯で最も合う位相」は，格子が本物なら中身に関係なくどこでも同じ値になる．
///
/// 周期が非整数なら，帯が右へ (下へ) 進むほど最適位相がずれていく．$s = 6$ ・
/// $r = 1.3$ なら 1 セルあたり 0.2 画素で，10 セル先では 2 画素ずれる．
#[derive(Clone, Debug, PartialEq)]
pub struct PhaseDrift {
    pub scale: u32,
    /// 左から右へ帯を切ったときの最適な $d_x$．
    pub by_x: Vec<u32>,
    /// 上から下へ帯を切ったときの最適な $d_y$．
    pub by_y: Vec<u32>,
}

impl PhaseDrift {
    /// 帯の間で位相がどれだけ離れているか (巡回距離の最大)．
    pub fn spread(&self) -> u32 {
        cyclic_spread(&self.by_x, self.scale).max(cyclic_spread(&self.by_y, self.scale))
    }

    /// ずれを「最大でありうるずれ」で割ったもの．$s$ が違う件どうしを比べるために要る．
    pub fn normalized(&self) -> f32 {
        let half = (self.scale / 2).max(1) as f32;
        (self.spread() as f32 / half).min(1.0)
    }
}

/// 巡回的な最大距離．位相は $s$ で一周するので，0 と $s - 1$ は隣どうしである．
fn cyclic_spread(phases: &[u32], s: u32) -> u32 {
    let mut worst = 0;
    for (i, a) in phases.iter().enumerate() {
        for b in &phases[i + 1..] {
            let d = a.abs_diff(*b);
            worst = worst.max(d.min(s.saturating_sub(d)));
        }
    }
    worst
}

/// セル内分散の和．`px_core::grid` と同じく `u64` で厳密に積む (設計書 6.15 規則 3)．
fn cell_variance_sum(img: &RgbaCanvas, s: u32, x0: u32, y0: u32) -> (f64, usize) {
    let mut sum = [0u64; 3];
    let mut sq = [0u64; 3];
    let mut n = 0u64;
    for y in y0..y0 + s {
        for x in x0..x0 + s {
            let Some(c) = img.get(x as i32, y as i32) else {
                continue;
            };
            for (k, v) in [c.r, c.g, c.b].into_iter().enumerate() {
                sum[k] += u64::from(v);
                sq[k] += u64::from(v) * u64::from(v);
            }
            n += 1;
        }
    }
    if n == 0 {
        return (0.0, 0);
    }
    let mut var = 0.0;
    for k in 0..3 {
        let mean = sum[k] as f64 / n as f64;
        var += (sq[k] as f64 / n as f64 - mean * mean).max(0.0);
    }
    // 画素値を [0, 1] に正規化した分散に合わせる
    (var / (255.0 * 255.0), 1)
}

/// 帯の中で最初にセルが始まる座標．
///
/// **位相は画像の原点を基準に測る．** 帯の左端を基準にすると，帯ごとに違う物差しで
/// 測ることになり，帯の間で比べられない (`start = x0 + p` としてはいけない)．
fn first_cell(band_start: u32, phase: u32, s: u32) -> u32 {
    let off = (i64::from(phase) - i64::from(band_start)).rem_euclid(i64::from(s)) as u32;
    band_start + off
}

/// 与えた矩形の中で，セル内平均分散が最小になる位相を返す．返す値は**画像の原点を
/// 基準にした位相**である．**同点は小さい方**を採る (設計書 6.15 規則 2)．
fn best_phase_in(img: &RgbaCanvas, s: u32, rect: (u32, u32, u32, u32), fixed_dy: u32) -> u32 {
    let (x0, y0, x1, y1) = rect;
    let mut best = (f64::MAX, 0u32);
    for p in 0..s {
        let mut total = 0.0;
        let mut cells = 0usize;
        let mut y = first_cell(y0, fixed_dy, s);
        while y + s <= y1 {
            let mut x = first_cell(x0, p, s);
            while x + s <= x1 {
                let (v, n) = cell_variance_sum(img, s, x, y);
                total += v;
                cells += n;
                x += s;
            }
            y += s;
        }
        if cells == 0 {
            continue;
        }
        let mean = total / cells as f64;
        if mean < best.0 {
            best = (mean, p);
        }
    }
    best.1
}

/// 帯ごとの最適位相を求める．
pub fn phase_drift(img: &RgbaCanvas, scale: u32, phase: IVec2) -> Option<PhaseDrift> {
    let s = scale.max(1);
    let (dx, dy) = (phase.x.max(0) as u32, phase.y.max(0) as u32);
    let (w, h) = (img.width(), img.height());
    if (w.saturating_sub(dx) / s) < MIN_CELLS as u32
        || (h.saturating_sub(dy) / s) < MIN_CELLS as u32
    {
        return None;
    }

    let by_x = (0..BANDS)
        .map(|b| {
            let x0 = w * b as u32 / BANDS as u32;
            let x1 = w * (b + 1) as u32 / BANDS as u32;
            best_phase_in(img, s, (x0, 0, x1, h), dy)
        })
        .collect();
    // 縦方向は画像を転置する代わりに，行と列の役割を入れ替えて同じ計算をする
    let by_y = (0..BANDS)
        .map(|b| {
            let y0 = h * b as u32 / BANDS as u32;
            let y1 = h * (b + 1) as u32 / BANDS as u32;
            best_phase_y_in(img, s, (0, y0, w, y1), dx)
        })
        .collect();

    Some(PhaseDrift {
        scale: s,
        by_x,
        by_y,
    })
}

/// [`best_phase_in`] の縦版．
fn best_phase_y_in(img: &RgbaCanvas, s: u32, rect: (u32, u32, u32, u32), fixed_dx: u32) -> u32 {
    let (x0, y0, x1, y1) = rect;
    let mut best = (f64::MAX, 0u32);
    for p in 0..s {
        let mut total = 0.0;
        let mut cells = 0usize;
        let mut y = first_cell(y0, p, s);
        while y + s <= y1 {
            let mut x = first_cell(x0, fixed_dx, s);
            while x + s <= x1 {
                let (v, n) = cell_variance_sum(img, s, x, y);
                total += v;
                cells += n;
                x += s;
            }
            y += s;
        }
        if cells == 0 {
            continue;
        }
        let mean = total / cells as f64;
        if mean < best.0 {
            best = (mean, p);
        }
    }
    best.1
}

/// 1 件分の実測．
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub item_id: u32,
    pub has_integer_grid: bool,
    pub filter: String,
    pub resize: String,
    pub compression: String,
    pub truth_scale: u32,
    pub scale_hat: u32,
    /// 推定した $(s, d)$ が正解と一致するか．**帯の profile が受け入れるべき件**である．
    pub should_accept: bool,
    pub overall: f32,
    pub spread: f32,
    pub relative_spread: f32,
    pub slope: f32,
    pub by_x: Vec<f32>,
    pub by_y: Vec<f32>,
    /// 帯ごとの最適位相の離れ具合 (画素)．
    pub phase_spread: u32,
    /// それを $s/2$ で割ったもの．
    pub phase_drift: f32,
    pub phase_by_x: Vec<u32>,
    pub phase_by_y: Vec<u32>,
}

pub const RECORD_HEADER: &str = "item_id,has_integer_grid,filter,resize,compression,truth_scale,\
scale_hat,should_accept,overall,spread,relative_spread,slope,phase_spread,phase_drift,by_x,by_y,\
phase_by_x,phase_by_y";

impl Record {
    pub fn to_csv(&self) -> String {
        let join = |v: &[f32]| {
            v.iter()
                .map(|x| format!("{x:.4}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let join_u = |v: &[u32]| v.iter().map(u32::to_string).collect::<Vec<_>>().join(" ");
        format!(
            "{},{},{},{},{},{},{},{},{:.4},{:.4},{:.4},{:.4},{},{:.4},{},{},{},{}",
            self.item_id,
            self.has_integer_grid,
            self.filter,
            self.resize,
            self.compression,
            self.truth_scale,
            self.scale_hat,
            self.should_accept,
            self.overall,
            self.spread,
            self.relative_spread,
            self.slope,
            self.phase_spread,
            self.phase_drift,
            join(&self.by_x),
            join(&self.by_y),
            join_u(&self.phase_by_x),
            join_u(&self.phase_by_y),
        )
    }
}

/// 全件について，推定器が出した候補で帯を測る．
///
/// **閾値は掃引で完全一致率が最大だった水準を使う** ($\varepsilon = 0.02$ ・
/// $\delta = 0.15$ ・$\tau = 0.02$) ．問うているのは「今の再構成検査が通してしまった
/// 答えを，帯の統計で選り分けられるか」なので，検査を切ってはいけない —
/// 切ると過大推定が野放しになり，正解する件が nearest ばかりになって
/// 「滲んだ整数格子と非整数格子の区別」という肝心の問いが標本から消える．
pub fn run(
    dir: &std::path::Path,
    manifest: &Manifest,
    only: Option<Split>,
    epsilon: f32,
    delta: f32,
    tau: f32,
) -> Result<Vec<Record>> {
    let params = GridParams {
        max_scale: 16,
        epsilon,
        delta,
        tau,
        normalize_epsilon: false,
        // 帯の統計は自前で測る．推定器の側の検査は切って，
        // 「検査が通した答えを選り分けられるか」という元の問いのまま測る
        phase_bands: 0,
        phase_tolerance: 1.0,
        phase_min_cells: 2,
        min_confidence: 0.0,
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
            let Ok(e) = estimate_grid(&img, &params) else {
                return Ok(None);
            };
            let Some(p) = profile(&img, e.scale, e.phase, delta) else {
                return Ok(None);
            };
            let Some(d) = phase_drift(&img, e.scale, e.phase) else {
                return Ok(None);
            };
            let phase = (e.phase.x.max(0) as u32, e.phase.y.max(0) as u32);
            Ok(Some(Record {
                item_id: item.id,
                has_integer_grid: item.has_integer_grid(),
                filter: item.degradation.filter.as_str().to_string(),
                resize: item.degradation.resize.as_str().to_string(),
                compression: item.degradation.compression.as_str().to_string(),
                truth_scale: item.truth_scale,
                scale_hat: e.scale,
                should_accept: item.has_integer_grid()
                    && e.scale == item.truth_scale
                    && item.truth_phase == Some(phase),
                overall: p.overall,
                spread: p.spread(),
                relative_spread: p.relative_spread(),
                slope: p.slope(),
                by_x: p.by_x,
                by_y: p.by_y,
                phase_spread: d.spread(),
                phase_drift: d.normalized(),
                phase_by_x: d.by_x,
                phase_by_y: d.by_y,
            }))
        })
        .collect();

    let mut out = Vec::with_capacity(items.len());
    for r in measured {
        out.extend(r?);
    }
    Ok(out)
}

/// 帯の統計で「受け入れるべき件」と「棄却すべき件」を分けられるか．
///
/// `key` が小さいほど受け入れる，という向きの単一閾値を全通り試して，均衡正解率
/// (受け入れるべき件を通す率と，棄却すべき件を止める率の平均) が最大になる点を返す．
/// **0.5 は「分けられていない」ことを意味する** (どちらかへ倒すだけで達成できる)．
pub fn best_threshold(records: &[Record], key: impl Fn(&Record) -> f32) -> (f32, f32) {
    let positives = records.iter().filter(|r| r.should_accept).count();
    let negatives = records.len() - positives;
    if positives == 0 || negatives == 0 {
        return (0.0, 0.5);
    }

    let mut candidates: Vec<f32> = records.iter().map(&key).collect();
    candidates.sort_by(f32::total_cmp);
    candidates.dedup_by(|a, b| a == b);

    let mut best = (0.0f32, 0.0f32);
    for &t in &candidates {
        let accepted_ok = records
            .iter()
            .filter(|r| r.should_accept && key(r) <= t)
            .count();
        let rejected_ok = records
            .iter()
            .filter(|r| !r.should_accept && key(r) > t)
            .count();
        let balanced =
            (accepted_ok as f32 / positives as f32 + rejected_ok as f32 / negatives as f32) / 2.0;
        // 同率なら閾値が小さい方 — 受け入れる範囲を広げない
        if balanced > best.1 {
            best = (t, balanced);
        }
    }
    best
}

/// 四分位 (Q1 ・中央値 ・Q3)．
pub fn quartiles(values: &[f32]) -> (f32, f32, f32) {
    if values.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut v = values.to_vec();
    v.sort_by(f32::total_cmp);
    let at = |q: f32| v[((v.len() - 1) as f32 * q).round() as usize];
    (at(0.25), at(0.5), at(0.75))
}

#[cfg(test)]
mod tests {
    use px_core::ivec2;

    use super::*;
    use crate::degrade::{Compression, Degradation, Filter, Resize};
    use crate::sprite;

    fn nearest(scale: u32) -> Degradation {
        Degradation {
            scale,
            filter: Filter::Nearest,
            resize: Resize::Keep,
            compression: Compression::Png,
            crop: (0, 0),
        }
    }

    #[test]
    fn an_exact_grid_has_no_mismatch_anywhere() {
        // nearest で拡大しただけの画像は，正しい s と位相ならセル内が完全に平坦
        let src = sprite::synthesize(11);
        let img = nearest(6).apply(&src).unwrap();
        let p = profile(&img, 6, ivec2(0, 0), 0.02).expect("セル数は足りる");
        assert_eq!(p.overall, 0.0);
        assert_eq!(p.spread(), 0.0);
        assert!(p.by_x.iter().all(|r| *r == 0.0));
    }

    #[test]
    fn a_wrong_phase_breaks_every_band_alike() {
        // 位相がずれるとセルが 2 つの色にまたがる．**ずれ方は画面のどこでも同じ**
        let src = sprite::synthesize(12);
        let img = nearest(6).apply(&src).unwrap();
        let p = profile(&img, 6, ivec2(3, 3), 0.02).expect("セル数は足りる");
        assert!(p.overall > 0.05, "不一致が出ていない: {}", p.overall);
        // どの帯も無傷では済まない．**ただし帯ごとの値は絵の中身に左右される** —
        // ディザ帯のある行は不一致が多く，平坦な行は少ない．だから帯のばらつきは
        // 「格子が本物か」の証拠にならない (実測でも現行の指標とほぼ同じ成績だった)
        assert!(
            p.by_x.iter().all(|r| *r > 0.05),
            "一様なずれなのに無傷の帯がある: {:?}",
            p.by_x
        );
    }

    #[test]
    fn a_real_grid_has_the_same_phase_in_every_band() {
        // 本物の格子なら，どの帯で測っても最適位相は同じ値になる
        let src = sprite::synthesize(14);
        let img = Degradation {
            filter: Filter::Bicubic,
            crop: (2, 5),
            ..nearest(8)
        }
        .apply(&src)
        .unwrap();
        let d = phase_drift(&img, 8, ivec2(6, 3)).expect("セル数は足りる");
        assert_eq!(d.by_x, vec![6; BANDS], "帯によって位相が違う");
        assert_eq!(d.by_y, vec![3; BANDS]);
        assert_eq!(d.spread(), 0);
    }

    #[test]
    fn a_non_integer_grid_shifts_its_phase_across_the_bands() {
        // 周期 7.8 を 8 で近似すると，帯が進むほど最適位相がずれる
        let src = sprite::synthesize(15);
        let img = Degradation {
            resize: Resize::Up13,
            ..nearest(6)
        }
        .apply(&src)
        .unwrap();
        let d = phase_drift(&img, 8, ivec2(0, 0)).expect("セル数は足りる");
        assert!(
            d.spread() > 0,
            "非整数の周期なのに位相が動かない: {:?} {:?}",
            d.by_x,
            d.by_y
        );
    }

    #[test]
    fn the_phase_is_measured_from_the_image_origin() {
        // 帯の左端を基準にすると帯ごとに違う物差しになる．画像の原点で測ること
        assert_eq!(first_cell(0, 3, 8), 3);
        assert_eq!(first_cell(8, 3, 8), 11, "次の帯でも同じ位相の位置に来る");
        assert_eq!(first_cell(10, 3, 8), 11);
        assert_eq!(first_cell(11, 3, 8), 11);
        assert_eq!(first_cell(12, 3, 8), 19);
    }

    #[test]
    fn phases_wrap_around_the_scale() {
        // 位相 0 と s-1 は 1 画素しか離れていない
        assert_eq!(cyclic_spread(&[0, 7], 8), 1);
        assert_eq!(cyclic_spread(&[0, 4], 8), 4);
        assert_eq!(cyclic_spread(&[2, 2, 2, 2], 8), 0);
        assert_eq!(cyclic_spread(&[7, 0, 1], 8), 2);
    }

    #[test]
    fn the_drift_is_normalized_by_half_the_scale() {
        // s が違う件どうしを比べるため．最大のずれは s/2 (それ以上は巡回で戻る)
        let d = PhaseDrift {
            scale: 8,
            by_x: vec![0, 0, 2, 2],
            by_y: vec![0; BANDS],
        };
        assert_eq!(d.spread(), 2);
        assert!((d.normalized() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_profile_needs_enough_cells() {
        let src = sprite::synthesize(13);
        let img = nearest(2).apply(&src).unwrap();
        // 帯に分けられないほど大きい s では測らない
        assert!(profile(&img, img.width() / 4, ivec2(0, 0), 0.02).is_none());
    }

    #[test]
    fn spread_is_the_gap_between_the_widest_bands() {
        let p = Profile {
            overall: 0.2,
            by_x: vec![0.1, 0.2, 0.3, 0.4],
            by_y: vec![0.2, 0.2, 0.2, 0.2],
        };
        assert!((p.spread() - 0.3).abs() < 1e-6);
        assert!((p.relative_spread() - 1.5).abs() < 1e-6);
    }

    #[test]
    fn slope_reads_a_rising_profile() {
        let rising = Profile {
            overall: 0.25,
            by_x: vec![0.1, 0.2, 0.3, 0.4],
            by_y: vec![0.25, 0.25, 0.25, 0.25],
        };
        assert!((slope(&rising.by_x) - 0.1).abs() < 1e-6);
        // 一周して戻る形では傾きが消える — だから判定には使わない
        let wrapped = Profile {
            overall: 0.25,
            by_x: vec![0.1, 0.4, 0.4, 0.1],
            by_y: vec![0.25, 0.25, 0.25, 0.25],
        };
        assert!(slope(&wrapped.by_x).abs() < 1e-6);
        assert!(wrapped.spread() > 0.29, "ばらつきなら捉えられる");
    }

    #[test]
    fn a_flat_profile_has_no_spread() {
        let p = Profile {
            overall: 0.0,
            by_x: vec![0.0; BANDS],
            by_y: vec![0.0; BANDS],
        };
        assert_eq!(p.spread(), 0.0);
        assert_eq!(p.relative_spread(), 0.0, "0 除算していない");
    }
}
