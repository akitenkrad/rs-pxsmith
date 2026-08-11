//! 格子推定 (設計書 6.1) と局所格子推定 (G4)．
//!
//! # 定式化
//!
//! 真のスケール $s_*$ の**すべての約数**がセル内分散 0 を与えるため，「分散を
//! 最小化する」定式化は原理的に誤りである．正しい原理は「**分散が閾値以下になる
//! 最大の $s$**」(D28)．
//!
//! $$ \hat{s} = \max \{ s \mid \bar{V}(s, d_s) \le \varepsilon \land \mathrm{ReconErr}(s, d_s) \le (\delta, \tau) \} $$
//!
//! 各 $s$ の位相 $d_s$ を先に確定させてから判定する．$\hat{d} = d_{\hat{s}}$ は
//! $\hat{s}$ が決まった後にしか定義されないので，条件式の中では使わない．
//!
//! # 再構成検査
//!
//! 倍数側の過大推定 ($s = 2 s_*$) を排除する．**厳密一致は採らない** — 適用範囲に
//! JPEG 圧縮・bicubic / lanczos・非整数倍リサイズを含むため，厳密一致は原理的に
//! 成立せず評価データセットのほぼ全件が棄却される．画素色差の許容 $\delta$ と
//! 不一致画素率の許容 $\tau$ の 2 段で判定する．
//!
//! # 位相ずれ検査
//!
//! 再構成検査だけでは**非整数の周期を落とせない**．画像全体で 1 つの割合しか見ないので，
//! 補間による一様な滲みと，非整数倍リサイズによる距離に比例したずれが同じ数に潰れる．
//! 合成 500 件では，どちらを優先しても他方が落ちる反比例になり，閾値の選び直しでは
//! 抜けられなかった (`docs/investigations/grid-calibration.md`)．
//!
//! そこで画像を帯に切り，帯ごとに最も合う位相を求めて揃っているかを見る．**位相は
//! 絵の中身に左右されない** — 本物の格子なら何が描いてあろうとどの帯でも同じ値になり，
//! 偽物なら帯が進むほどずれる．実測では単一閾値で均衡正解率 95.9% (再構成検査だけなら
//! 76.6%) だった．
//!
//! # 信頼度 (D63)
//!
//! $\hat{s}$ より**大きい** $s$ から倍数を除いた対照群に対する分離マージンを，画像全体の
//! 分散で正規化する．小さい $s$ を対照群へ入れてはいけない — $\bar{V}(s)$ は $s$ と
//! ともに単調に増えるので，最小値を必ず小さい $s$ が取り，マージンが負に潰れる．
//!
//! # 性能
//!
//! 位相探索は**積分画像 (summed-area table) で定数時間化する**．セル分散は
//! $\mathrm{Var} = E[x^2] - E[x]^2$ なので，チャネルごとに $\sum x$ と $\sum x^2$ の
//! 積分画像を 1 度作れば，任意の $(s, d_x, d_y)$ のセル統計が $O(1)$ で求まる．

use crate::canvas::RgbaCanvas;
use crate::color::{Rgba8, delta_e, oklab_of};
use crate::geom::mask::Field;
use crate::math::{IVec2, ivec2};

/// 推定に使う閾値．
///
/// 既定値は合成 500 件の**検証セット 300 件で校正した**もので，マクロ平均 (格子ありの
/// 完全一致率と格子なしの正棄却率の平均) が最大になる組である．
///
/// | 指標 | 値 |
/// | --- | --- |
/// | マクロ平均 | 88.1% |
/// | 格子あり 完全一致 | 83.2% |
/// | 格子なし 正しい棄却 | 93.0% |
///
/// > **まだ暫定である．** 実装計画書 M2 の目標は 95% で，届いていない．テストセット
/// > 200 件には 1 度も触れていない (目標を満たしてから 1 度だけ使う) ．実データ
/// > 20〜30 件も未調達なので，合成データと実運用対象の分布のずれは測れていない．
/// > 経緯は `docs/investigations/grid-calibration.md`．
#[derive(Copy, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GridParams {
    /// 探索するスケールの上限 $s_{\max}$．
    pub max_scale: u32,
    /// セル内平均分散の許容 $\varepsilon$ (画素値を $[0, 1]$ に正規化した値の分散)．
    pub epsilon: f32,
    /// 再構成の画素色差の許容 $\delta$ (OKLab の $\Delta E$)．
    pub delta: f32,
    /// 再構成の不一致画素率の許容 $\tau$．
    pub tau: f32,
    /// 位相ずれ検査で画像を切る帯の数 (縦横それぞれ)．0 で検査を飛ばす．
    pub phase_bands: usize,
    /// 帯どうしの位相のずれの許容．**$s$ に対する割合**で持つ．
    pub phase_tolerance: f32,
    /// 位相ずれ検査を行うために帯 1 本へ要るセル数．
    ///
    /// 下回ると検査そのものを行わない (候補は通る) ．**大きい $s$ ほど帯が薄くなるので，
    /// この値が大きいと過大推定の抜け穴になる** — 2 では候補の 23% で検査が働かず，
    /// $s = 12$ ・$16$ の過大推定がそこから漏れている．
    ///
    /// > **1 にすると評価データセットの成績は上がるが (88.1% → 90.1%) ，採らない．**
    /// > 帯あたり 1 セルでは位相が当てにならず，24 画素角のスプライトを 4 倍した画像で
    /// > 真のスケールを落とす (試験 `recovers_the_phase_when_the_image_is_cropped`) ．
    /// > 評価データセットの元絵は 16〜48 画素角なので**真のスケールでは常に 16 セル以上**
    /// > あり，この失敗を見られない — 掃引の数字が良くなるのは，データセットが
    /// > 見ていない場面を代償にしているからである．小さい絵を入れてから測り直す．
    pub phase_min_cells: usize,
    /// この値未満の信頼度は棄却する．
    pub min_confidence: f32,
}

impl Default for GridParams {
    fn default() -> Self {
        Self {
            max_scale: 16,
            // 補間を挟むと必要な ε が 1 桁以上大きくなる (nearest なら 2e-4 で足りる)．
            // 0.05 も同成績だが，緩めても得が無いので小さい方を採る
            epsilon: 0.02,
            delta: 0.1,
            tau: 0.1,
            phase_bands: 4,
            // 0 で 67.3% ・1/8 で 72.3% ・1/6 で 75.2% ・1/4 で 73.0% ・3/8 で 60.2%
            phase_tolerance: 1.0 / 6.0,
            phase_min_cells: 2,
            // 運転点．0 だと非整数の周期に答えを返してしまう (正棄却 65.3%)．
            // 0.03 で 93.0% まで上がり，完全一致率の代償は 85.1% → 83.2% で済む
            min_confidence: 0.03,
        }
    }
}

/// 推定の結果．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GridEstimate {
    /// 推定したスケール $\hat{s}$．
    pub scale: u32,
    /// 位相 $\hat{d} = (d_x, d_y)$．
    pub phase: IVec2,
    /// 信頼度 $\mathrm{conf} \in [0, 1]$．
    pub confidence: f32,
    /// 採用した $s$ でのセル内平均分散．
    pub mean_variance: f32,
}

/// 推定に失敗する理由．
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GridError {
    /// 閾値を満たす $s$ が 1 つも無い．
    NotFound,
    /// 画像が小さすぎて格子を切れない．
    TooSmall,
    /// 信頼度が下限を下回った (棄却)．
    LowConfidence,
}

impl std::fmt::Display for GridError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "閾値を満たす格子が見つからない"),
            Self::TooSmall => write!(f, "画像が小さすぎて格子を切れない"),
            Self::LowConfidence => write!(f, "信頼度が下限を下回ったので棄却した"),
        }
    }
}

/// ある $s$ についての評価結果．信頼度の計算が全候補の $\bar{V}$ を要求するため，
/// 早期 return せずに全部集める．
#[derive(Copy, Clone, Debug)]
struct Candidate {
    scale: u32,
    mean_variance: f32,
    phase: IVec2,
}

/// 積分画像．チャネルごとに $\sum x$ と $\sum x^2$ を持つ．
///
/// 値は `u8` なので `u64` で厳密に積める．浮動小数で積むと大きな画像で
/// 桁落ちし，同じ入力でも積む順序で答えが変わりうる (設計書 6.15 規則 3)．
struct Integral {
    w: usize,
    h: usize,
    sum: [Vec<u64>; 3],
    sq: [Vec<u64>; 3],
}

impl Integral {
    fn new(img: &RgbaCanvas) -> Self {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let stride = w + 1;
        let mut sum = [
            vec![0u64; stride * (h + 1)],
            vec![0u64; stride * (h + 1)],
            vec![0u64; stride * (h + 1)],
        ];
        let mut sq = [
            vec![0u64; stride * (h + 1)],
            vec![0u64; stride * (h + 1)],
            vec![0u64; stride * (h + 1)],
        ];

        for y in 0..h {
            let mut row = [0u64; 3];
            let mut row_sq = [0u64; 3];
            for x in 0..w {
                let c = img.pixels()[y * w + x];
                let v = [c.r as u64, c.g as u64, c.b as u64];
                let i = (y + 1) * stride + (x + 1);
                for (k, value) in v.iter().enumerate() {
                    row[k] += value;
                    row_sq[k] += value * value;
                    sum[k][i] = sum[k][i - stride] + row[k];
                    sq[k][i] = sq[k][i - stride] + row_sq[k];
                }
            }
        }
        Self { w, h, sum, sq }
    }

    fn rect(&self, table: &[u64], x0: usize, y0: usize, x1: usize, y1: usize) -> u64 {
        let stride = self.w + 1;
        table[y1 * stride + x1] + table[y0 * stride + x0]
            - table[y0 * stride + x1]
            - table[y1 * stride + x0]
    }

    /// 矩形内の分散 (3 チャネルの和)．画素値は $[0, 1]$ に正規化する．
    fn variance(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> f64 {
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        if n <= 0.0 {
            return 0.0;
        }
        let mut acc = 0.0;
        for k in 0..3 {
            let s = self.rect(&self.sum[k], x0, y0, x1, y1) as f64;
            let q = self.rect(&self.sq[k], x0, y0, x1, y1) as f64;
            let mean = s / n;
            // 丸め誤差で僅かに負になることがある
            acc += (q / n - mean * mean).max(0.0);
        }
        // 0..255 の分散を 0..1 の尺度へ
        acc / (255.0 * 255.0)
    }

    fn mean(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> Rgba8 {
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        if n <= 0.0 {
            return Rgba8::TRANSPARENT;
        }
        let mut c = [0u8; 3];
        for (k, slot) in c.iter_mut().enumerate() {
            let s = self.rect(&self.sum[k], x0, y0, x1, y1) as f64;
            *slot = (s / n).round().clamp(0.0, 255.0) as u8;
        }
        Rgba8::rgb(c[0], c[1], c[2])
    }
}

/// 完全なセルの並び．端の欠けたセルは含めない．
fn cells(
    w: usize,
    h: usize,
    s: usize,
    dx: usize,
    dy: usize,
) -> impl Iterator<Item = (usize, usize)> {
    let nx = if w > dx { (w - dx) / s } else { 0 };
    let ny = if h > dy { (h - dy) / s } else { 0 };
    (0..ny).flat_map(move |j| (0..nx).map(move |i| (dx + i * s, dy + j * s)))
}

/// セル内平均分散 $\bar{V}(s, d_x, d_y)$．完全なセルが無ければ `None`．
fn mean_cell_variance(it: &Integral, s: usize, dx: usize, dy: usize) -> Option<f32> {
    let mut acc = 0.0f64;
    let mut count = 0usize;
    for (x, y) in cells(it.w, it.h, s, dx, dy) {
        acc += it.variance(x, y, x + s, y + s);
        count += 1;
    }
    (count > 0).then(|| (acc / count as f64) as f32)
}

/// 平均分散が最小の位相．**同点は $(d_x, d_y)$ の辞書式順**で決める
/// (設計書 6.15 規則 2)．
fn best_phase(it: &Integral, s: usize) -> Option<(f32, IVec2)> {
    let mut best: Option<(f32, IVec2)> = None;
    for dy in 0..s {
        for dx in 0..s {
            let Some(v) = mean_cell_variance(it, s, dx, dy) else {
                continue;
            };
            let phase = ivec2(dx as i32, dy as i32);
            match best {
                // 走査順が辞書式なので，同点は先に見たものが残る
                Some((bv, _)) if bv <= v => {}
                _ => best = Some((v, phase)),
            }
        }
    }
    best
}

/// 矩形の中だけでセル内平均分散を測る．位相は**画像の原点を基準**に与える．
fn mean_cell_variance_in(
    it: &Integral,
    s: usize,
    rect: (usize, usize, usize, usize),
    dx: usize,
    dy: usize,
) -> Option<f32> {
    let (x0, y0, x1, y1) = rect;
    let mut acc = 0.0f64;
    let mut count = 0usize;
    let mut y = first_cell(y0, dy, s);
    while y + s <= y1 {
        let mut x = first_cell(x0, dx, s);
        while x + s <= x1 {
            acc += it.variance(x, y, x + s, y + s);
            count += 1;
            x += s;
        }
        y += s;
    }
    (count > 0).then(|| (acc / count as f64) as f32)
}

/// 帯の中で最初にセルが始まる座標．
///
/// **位相は画像の原点を基準に測る．** 帯の左端を基準にすると帯ごとに違う物差しで
/// 測ることになり，本物の格子でもずれが出て何も分からなくなる．
fn first_cell(band_start: usize, phase: usize, s: usize) -> usize {
    band_start + (phase + s - band_start % s) % s
}

/// 帯 1 つの中で最も合う位相．**同点は小さい方**を採る (設計書 6.15 規則 2)．
fn best_phase_in(
    it: &Integral,
    s: usize,
    rect: (usize, usize, usize, usize),
    fixed: usize,
    horizontal: bool,
) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for p in 0..s {
        let (dx, dy) = if horizontal { (p, fixed) } else { (fixed, p) };
        let Some(v) = mean_cell_variance_in(it, s, rect, dx, dy) else {
            continue;
        };
        match best {
            Some((bv, _)) if bv <= v => {}
            _ => best = Some((v, p)),
        }
    }
    best.map(|(_, p)| p)
}

/// 巡回的な最大距離．位相は $s$ で一周するので 0 と $s - 1$ は隣どうしである．
fn cyclic_spread(phases: &[usize], s: usize) -> usize {
    let mut worst = 0;
    for (i, a) in phases.iter().enumerate() {
        for b in &phases[i + 1..] {
            let d = a.abs_diff(*b);
            worst = worst.max(d.min(s - d));
        }
    }
    worst
}

/// 位相ずれ検査 — 帯ごとに最も合う位相を求め，帯の間で揃っているかを見る．
///
/// **再構成検査だけでは非整数の周期を落とせない．** 周期 $s \cdot r$ ($r$ が非整数) の
/// 入力を整数 $q$ で近似すると，セル境界は 1 セルあたり $|q - s r|$ ずつずれていく．
/// ところが再構成検査は画像全体で 1 つの割合しか見ないので，このずれが「補間による
/// 一様な滲み」と同じ数に潰れてしまう．評価データセットではどちらを優先しても
/// もう一方が落ちる反比例になり，閾値の選び直しでは抜けられなかった．
///
/// 位相は**絵の中身に左右されない**ところが違う．格子が本物なら，何が描いてあろうと
/// どの帯でも同じ位相が最も合う．偽物なら帯が進むほどずれる．
///
/// 帯あたりのセル数が足りない場合は検査を行わない (`true` を返す) ．少ないセルから
/// 求めた位相は当てにならず，落とす根拠にならない．
fn phase_drift_ok(
    it: &Integral,
    s: usize,
    d: IVec2,
    bands: usize,
    tolerance: f32,
    min_cells: usize,
) -> bool {
    if bands < 2 {
        return true;
    }
    let (dx, dy) = (d.x.max(0) as usize, d.y.max(0) as usize);
    let cells_x = it.w.saturating_sub(dx) / s;
    let cells_y = it.h.saturating_sub(dy) / s;
    if cells_x < min_cells * bands || cells_y < min_cells * bands {
        return true;
    }

    let mut by_x = Vec::with_capacity(bands);
    let mut by_y = Vec::with_capacity(bands);
    for b in 0..bands {
        let x0 = it.w * b / bands;
        let x1 = it.w * (b + 1) / bands;
        let y0 = it.h * b / bands;
        let y1 = it.h * (b + 1) / bands;
        let Some(px) = best_phase_in(it, s, (x0, 0, x1, it.h), dy, true) else {
            return true;
        };
        let Some(py) = best_phase_in(it, s, (0, y0, it.w, y1), dx, false) else {
            return true;
        };
        by_x.push(px);
        by_y.push(py);
    }

    let spread = cyclic_spread(&by_x, s).max(cyclic_spread(&by_y, s));
    spread as f32 <= s as f32 * tolerance
}

/// 再構成誤差の判定 — 画素色差 $\delta$ を超える画素の割合が $\tau$ 以下か．
fn recon_ok(img: &RgbaCanvas, it: &Integral, s: usize, d: IVec2, delta: f32, tau: f32) -> bool {
    let (dx, dy) = (d.x as usize, d.y as usize);
    let mut mismatched = 0usize;
    let mut total = 0usize;

    for (x, y) in cells(it.w, it.h, s, dx, dy) {
        let mean = oklab_of(it.mean(x, y, x + s, y + s));
        for j in 0..s {
            for i in 0..s {
                let Some(c) = img.get((x + i) as i32, (y + j) as i32) else {
                    continue;
                };
                total += 1;
                if delta_e(oklab_of(c), mean) > delta {
                    mismatched += 1;
                }
            }
        }
    }
    if total == 0 {
        return false;
    }
    (mismatched as f32 / total as f32) <= tau
}

fn divisors_and_multiples(s: u32, max: u32) -> impl Fn(u32) -> bool {
    move |t: u32| t != 0 && (s.is_multiple_of(t) || (t.is_multiple_of(s) && t <= max))
}

/// 画像全体の分散 $\bar{V}_{\mathrm{image}}$．信頼度の正規化に使う．
fn image_variance(it: &Integral) -> f32 {
    it.variance(0, 0, it.w, it.h) as f32
}

/// 対照群 — $\hat{s}$ より**大きい** $s$ から，倍数を除いたもの (D63)．
///
/// $\bar{V}(s)$ は $s$ とともに単調に増える (セルが大きいほど中に色が混ざる) ．合成
/// 500 件で測ると $s = 2$ で 0.0006 ・$s = 16$ で 0.0204 と 30 倍以上ちがう．
/// そのため**小さい $s$ を対照群へ入れると，最小値は必ずその小さい $s$ が取る** —
/// 正解した件の 52% でマージンが負になり，信頼度が 0 へ潰れていた．
///
/// $\hat{s}$ は「閾値を満たす最大の $s$」なので，問うべきは「1 つ上の $s$ がどれだけ
/// 悪いか」である．小さい $s$ は条件を満たして当たり前で，何の情報も持たない．
/// 倍数を除くのは従来どおり — 分散が同等に小さく，マージンを不当に縮めるためである．
///
/// 実測での分離能は，現行の定義が 70.8% に対しこの定義で 92.2% (均衡正解率)．
fn control_group(hat: u32, all: &[Candidate], max: u32) -> Vec<&Candidate> {
    let excluded = divisors_and_multiples(hat, max);
    all.iter()
        .filter(|c| c.scale > hat && !excluded(c.scale))
        .collect()
}

fn confidence(hat: &Candidate, all: &[Candidate], image_var: f32, max: u32) -> f32 {
    let group = control_group(hat.scale, all, max);
    // 退化ケース: 対照群なし / 画像が完全に平坦 — どちらも本質的に曖昧なので棄却
    if group.is_empty() || image_var <= 0.0 {
        return 0.0;
    }
    let min_other = group
        .iter()
        .map(|c| c.mean_variance)
        .fold(f32::INFINITY, f32::min);
    ((min_other - hat.mean_variance) / image_var).clamp(0.0, 1.0)
}

/// 与えた候補集合を評価する．
fn evaluate(
    img: &RgbaCanvas,
    it: &Integral,
    scales: &[u32],
    params: &GridParams,
) -> (Option<Candidate>, Vec<Candidate>) {
    let mut all = Vec::new();
    // 早期 return しない — 信頼度が全候補の分散を要求する
    for &s in scales {
        let Some((v, phase)) = best_phase(it, s as usize) else {
            continue;
        };
        all.push(Candidate {
            scale: s,
            mean_variance: v,
            phase,
        });
    }

    let accepted: Vec<&Candidate> = all
        .iter()
        .filter(|c| c.mean_variance <= params.epsilon)
        .filter(|c| recon_ok(img, it, c.scale as usize, c.phase, params.delta, params.tau))
        // 非整数の周期を落とす．再構成検査と違い，絵の中身に左右されない
        .filter(|c| {
            phase_drift_ok(
                it,
                c.scale as usize,
                c.phase,
                params.phase_bands,
                params.phase_tolerance,
                params.phase_min_cells,
            )
        })
        .collect();

    // 閾値を満たす最大の s．集合の最大値なので同点は起きない
    let hat = accepted.into_iter().max_by_key(|c| c.scale).copied();
    (hat, all)
}

/// 候補 1 つの評価結果 (診断用)．
///
/// どの検査で落ちたのか，対照群の中でどれと競っているのかを外から見るための型である．
/// 推定そのものには使わない．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ScaleCandidate {
    pub scale: u32,
    /// セル内平均分散 $\bar{V}(s, d_s)$．
    pub mean_variance: f32,
    /// その $s$ で最も合う位相．
    pub phase: IVec2,
    pub passes_epsilon: bool,
    pub passes_recon: bool,
    pub passes_phase: bool,
}

impl ScaleCandidate {
    pub fn accepted(&self) -> bool {
        self.passes_epsilon && self.passes_recon && self.passes_phase
    }
}

/// すべての $s$ を評価して返す (診断用)．返り値は候補と画像全体の分散
/// $\bar{V}_{\mathrm{image}}$．
///
/// [`estimate_grid`] は絞り込みと全探索フォールバックを経るが，こちらは常に
/// $2 \ldots s_{\max}$ を素通しで評価する．**校正で「何と何が競っているのか」を
/// 見るための口**であって，推定の経路ではない．
pub fn scale_candidates(img: &RgbaCanvas, params: &GridParams) -> (Vec<ScaleCandidate>, f32) {
    let max = params
        .max_scale
        .min(img.width().max(1))
        .min(img.height().max(1));
    let it = Integral::new(img);
    let image_var = image_variance(&it);

    let out = (2..=max)
        .filter_map(|s| {
            let (v, phase) = best_phase(&it, s as usize)?;
            Some(ScaleCandidate {
                scale: s,
                mean_variance: v,
                phase,
                passes_epsilon: v <= params.epsilon,
                passes_recon: recon_ok(img, &it, s as usize, phase, params.delta, params.tau),
                passes_phase: phase_drift_ok(
                    &it,
                    s as usize,
                    phase,
                    params.phase_bands,
                    params.phase_tolerance,
                    params.phase_min_cells,
                ),
            })
        })
        .collect();
    (out, image_var)
}

/// 与えた $\hat{s}$ に対する対照群の添字 (診断用)．[`control_group`] と同じ規則．
///
/// 信頼度の分子は「対照群の最小分散 - $\bar{V}(\hat{s})$」なので，**誰が最小なのか**が
/// 分からないと式の当否を論じられない．
pub fn control_group_of(hat: u32, candidates: &[ScaleCandidate], max_scale: u32) -> Vec<u32> {
    let excluded = divisors_and_multiples(hat, max_scale);
    candidates
        .iter()
        .map(|c| c.scale)
        .filter(|s| *s > hat && !excluded(*s))
        .collect()
}

/// 格子を推定する (設計書 6.1)．
pub fn estimate_grid(
    img: &RgbaCanvas,
    params: &GridParams,
) -> std::result::Result<GridEstimate, GridError> {
    let max = params
        .max_scale
        .min(img.width().max(1))
        .min(img.height().max(1));
    if max < 2 {
        return Err(GridError::TooSmall);
    }
    let it = Integral::new(img);

    // 候補で絞ってから全探索へ落ちる 2 段構成．候補を取りこぼしたときに
    // 黙って誤答しないよう，フォールバックを必ず持つ
    let narrowed = candidate_scales(img, max);
    let (mut hat, mut all) = evaluate(img, &it, &narrowed, params);
    let needs_fallback = match &hat {
        None => true,
        Some(c) => control_group(c.scale, &all, max).is_empty(),
    };
    if needs_fallback {
        let full: Vec<u32> = (2..=max).collect();
        let (h2, a2) = evaluate(img, &it, &full, params);
        hat = h2;
        all = a2;
    }

    let hat = hat.ok_or(GridError::NotFound)?;
    let conf = confidence(&hat, &all, image_variance(&it), max);
    if conf < params.min_confidence {
        return Err(GridError::LowConfidence);
    }
    Ok(GridEstimate {
        scale: hat.scale,
        phase: hat.phase,
        confidence: conf,
        mean_variance: hat.mean_variance,
    })
}

/// 自己相関からスケールの候補を絞る．
///
/// 列ごとの差分エネルギーは格子の境界で山になる．その自己相関のピーク位置が
/// 周期，すなわちスケールの候補になる．**取りこぼしうるので，これだけに
/// 頼ってはいけない** — 呼び出し側は必ず全探索へ落ちられるようにする．
pub fn candidate_scales(img: &RgbaCanvas, max_scale: u32) -> Vec<u32> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w < 2 || h < 2 {
        return (2..=max_scale).collect();
    }

    let edge_energy = |along_x: bool| -> Vec<f64> {
        let n = if along_x { w } else { h };
        let m = if along_x { h } else { w };
        let mut out = vec![0.0f64; n];
        for (i, slot) in out.iter_mut().enumerate().skip(1) {
            let mut acc = 0.0;
            for j in 0..m {
                let (a, b) = if along_x {
                    (img.pixels()[j * w + i], img.pixels()[j * w + i - 1])
                } else {
                    (img.pixels()[i * w + j], img.pixels()[(i - 1) * w + j])
                };
                acc += (a.r as f64 - b.r as f64).abs()
                    + (a.g as f64 - b.g as f64).abs()
                    + (a.b as f64 - b.b as f64).abs();
            }
            *slot = acc;
        }
        out
    };

    let mut votes = vec![0.0f64; (max_scale + 1) as usize];
    for signal in [edge_energy(true), edge_energy(false)] {
        let n = signal.len();
        let mean = signal.iter().sum::<f64>() / n as f64;
        let centered: Vec<f64> = signal.iter().map(|v| v - mean).collect();
        let norm: f64 = centered.iter().map(|v| v * v).sum();
        if norm <= f64::EPSILON {
            continue;
        }
        for lag in 2..=max_scale as usize {
            if lag >= n {
                break;
            }
            let acc: f64 = (lag..n).map(|i| centered[i] * centered[i - lag]).sum();
            votes[lag] += acc / norm;
        }
    }

    let mut candidates: Vec<u32> = (2..=max_scale)
        .filter(|&s| votes[s as usize] > 0.0)
        .collect();
    // 真のスケールの約数もセル内分散 0 を与えるので，候補に含めておく
    let peaks = candidates.clone();
    for s in peaks {
        for d in 2..=s {
            if s % d == 0 && !candidates.contains(&d) {
                candidates.push(d);
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    if candidates.is_empty() {
        (2..=max_scale).collect()
    } else {
        candidates
    }
}

/// 局所格子推定 (G4)．窓ごとの $\hat{s}$．
///
/// ミクセル検出 (lint 9) と非一様格子の棄却が共有する (D37)．
///
/// **信頼度 0 の窓は投票しない．** 平坦な窓ではすべての $s$ が閾値を満たすので
/// 「窓に収まる最大の $s$」が選ばれてしまう．これは本質的に曖昧なケースであり
/// (設計書 6.1 の退化ケース)，票に混ぜると一致率が下がって非一様と誤判定される．
pub fn local_grid(img: &RgbaCanvas, window: u32, params: &GridParams) -> Field<Option<u32>> {
    let step = window.max(2);
    let nx = img.width().div_ceil(step).max(1);
    let ny = img.height().div_ceil(step).max(1);
    let mut out: Field<Option<u32>> = Field::filled(nx, ny, None);

    for wy in 0..ny {
        for wx in 0..nx {
            let x0 = wx * step;
            let y0 = wy * step;
            let w = step.min(img.width().saturating_sub(x0));
            let h = step.min(img.height().saturating_sub(y0));
            if w < 4 || h < 4 {
                continue;
            }
            let mut sub = RgbaCanvas::filled(w, h, Rgba8::TRANSPARENT);
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    if let Some(c) = img.get(x0 as i32 + x, y0 as i32 + y) {
                        sub.set(x, y, c);
                    }
                }
            }
            let local = GridParams {
                max_scale: params.max_scale.min(w.min(h) / 2).max(2),
                ..*params
            };
            let value = estimate_grid(&sub, &local)
                .ok()
                .filter(|e| e.confidence > 0.0)
                .map(|e| e.scale);
            out.set(ivec2(wx as i32, wy as i32), value);
        }
    }
    out
}

/// 局所推定のばらつき．非一様格子の判定に使う．
///
/// 返り値は `(最頻のスケール, 一致した窓の割合)`．窓が 1 つも推定できなければ
/// `None`．**割合が閾値を下回ったら非一様として棄却する** (D29)．
pub fn uniformity(local: &Field<Option<u32>>) -> Option<(u32, f32)> {
    let values: Vec<u32> = local.data().iter().filter_map(|v| *v).collect();
    if values.is_empty() {
        return None;
    }
    let mut counts: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for v in &values {
        *counts.entry(*v).or_default() += 1;
    }
    // 同数のときは小さいスケールを採る (BTreeMap の走査順で決まる)
    let (best, count) = counts
        .iter()
        .max_by_key(|(scale, count)| (**count, std::cmp::Reverse(**scale)))
        .map(|(s, c)| (*s, *c))?;
    Some((best, count as f32 / values.len() as f32))
}

/// 格子に沿って最頻色へ縮小する (`px conform` の中核)．
///
/// **平均ではなく最頻色を採る**．平均だと境界のセルで元のパレットに無い色が
/// できてしまい，インデックスカラーへ戻せなくなる．
pub fn downscale_modal(img: &RgbaCanvas, scale: u32, phase: IVec2) -> RgbaCanvas {
    let s = scale.max(1) as usize;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (dx, dy) = (phase.x.max(0) as usize, phase.y.max(0) as usize);
    let nx = if w > dx { (w - dx) / s } else { 0 };
    let ny = if h > dy { (h - dy) / s } else { 0 };

    let mut out = RgbaCanvas::filled(nx as u32, ny as u32, Rgba8::TRANSPARENT);
    for j in 0..ny {
        for i in 0..nx {
            let mut counts: std::collections::BTreeMap<(u8, u8, u8, u8), usize> =
                std::collections::BTreeMap::new();
            for y in 0..s {
                for x in 0..s {
                    if let Some(c) = img.get((dx + i * s + x) as i32, (dy + j * s + y) as i32) {
                        *counts.entry((c.r, c.g, c.b, c.a)).or_default() += 1;
                    }
                }
            }
            // 同数のときは色の並び順で決める (決定論性，設計書 6.15 規則 2)
            if let Some(((r, g, b, a), _)) = counts.into_iter().max_by_key(|(k, v)| (*v, *k)) {
                out.set(i as i32, j as i32, Rgba8::new(r, g, b, a));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 添字の並びを `scale` 倍に拡大した画像を作る．`phase` だけずらして切る．
    fn upscaled(pattern: &[&str], colors: &[Rgba8], scale: u32, phase: (u32, u32)) -> RgbaCanvas {
        let ph = pattern.len() as u32;
        let pw = pattern[0].len() as u32;
        let full_w = pw * scale;
        let full_h = ph * scale;
        let (dx, dy) = phase;
        let w = full_w - dx;
        let h = full_h - dy;

        let mut out = RgbaCanvas::filled(w, h, Rgba8::TRANSPARENT);
        for y in 0..h {
            for x in 0..w {
                let sx = ((x + dx) / scale) as usize;
                let sy = ((y + dy) / scale) as usize;
                let ch = pattern[sy].as_bytes()[sx];
                let index = (ch - b'0') as usize;
                out.set(x as i32, y as i32, colors[index]);
            }
        }
        out
    }

    fn palette() -> Vec<Rgba8> {
        vec![
            Rgba8::rgb(0x1a, 0x1c, 0x2c),
            Rgba8::rgb(0xb1, 0x3e, 0x53),
            Rgba8::rgb(0xff, 0xcd, 0x75),
            Rgba8::rgb(0x38, 0xb7, 0x64),
        ]
    }

    const PATTERN: [&str; 6] = ["001122", "011223", "112233", "223300", "233001", "330011"];

    /// 12 x 12 の広めの模様．帯に切って測るには 1 帯あたり 2 セル以上が要る．
    const WIDE: [&str; 12] = [
        "001122330011",
        "011223300112",
        "112233001122",
        "122330011223",
        "223300112233",
        "233001122330",
        "330011223300",
        "300112233001",
        "001122330011",
        "011223300112",
        "112233001122",
        "122330011223",
    ];

    /// 非整数倍で最近傍リサンプルした画像．周期 `period` は整数にならない．
    ///
    /// これが位相ずれ検査の相手である — 周期 7.8 を 8 と読むと，セル境界が
    /// 1 セルあたり 0.2 画素ずつずれていく．
    fn resampled(pattern: &[&str], colors: &[Rgba8], period: f32) -> RgbaCanvas {
        let pw = pattern[0].len() as f32;
        let ph = pattern.len() as f32;
        let w = (pw * period) as u32;
        let h = (ph * period) as u32;
        let mut out = RgbaCanvas::filled(w, h, Rgba8::TRANSPARENT);
        for y in 0..h {
            for x in 0..w {
                let sx = ((x as f32 / period) as usize).min(pattern[0].len() - 1);
                let sy = ((y as f32 / period) as usize).min(pattern.len() - 1);
                let index = (pattern[sy].as_bytes()[sx] - b'0') as usize;
                out.set(x as i32, y as i32, colors[index]);
            }
        }
        out
    }

    #[test]
    fn a_true_grid_fits_the_same_phase_in_every_band() {
        for scale in [4u32, 6, 8] {
            for phase in [(0u32, 0u32), (2, 3)] {
                let img = upscaled(&WIDE, &palette(), scale, phase);
                let it = Integral::new(&img);
                let d = ivec2(
                    ((scale - phase.0) % scale) as i32,
                    ((scale - phase.1) % scale) as i32,
                );
                assert!(
                    phase_drift_ok(&it, scale as usize, d, 4, 0.0, 2),
                    "{scale} 倍 ・位相 {phase:?} で帯ごとに位相が違う"
                );
            }
        }
    }

    #[test]
    fn a_non_integer_period_fails_the_phase_check() {
        // 周期 7.8 を 8 と読ませる．1 セルあたり 0.2 画素ずつずれる
        let img = resampled(&WIDE, &palette(), 7.8);
        let it = Integral::new(&img);
        let d = best_phase(&it, 8).expect("位相はある").1;
        assert!(
            !phase_drift_ok(&it, 8, d, 4, 1.0 / 6.0, 2),
            "非整数の周期を通してしまった"
        );
    }

    #[test]
    fn the_phase_check_is_skipped_when_the_bands_would_be_too_thin() {
        // 帯あたり 2 セルに満たないときは，位相が当てにならないので落とす根拠にしない
        let img = upscaled(&PATTERN, &palette(), 4, (0, 0));
        let it = Integral::new(&img);
        assert!(
            phase_drift_ok(&it, 4, ivec2(0, 0), 8, 0.0, 2),
            "セルが足りないのに棄却している"
        );
    }

    #[test]
    fn the_phase_check_can_be_turned_off() {
        let img = resampled(&WIDE, &palette(), 7.8);
        let it = Integral::new(&img);
        let d = best_phase(&it, 8).expect("位相はある").1;
        assert!(
            phase_drift_ok(&it, 8, d, 0, 0.0, 2),
            "帯 0 で検査が働いている"
        );
        assert!(
            phase_drift_ok(&it, 8, d, 1, 0.0, 2),
            "帯 1 では比べようがない"
        );
    }

    #[test]
    fn phases_wrap_around_the_scale() {
        assert_eq!(cyclic_spread(&[0, 7], 8), 1, "0 と s-1 は隣どうし");
        assert_eq!(cyclic_spread(&[0, 4], 8), 4);
        assert_eq!(cyclic_spread(&[2, 2, 2, 2], 8), 0);
    }

    #[test]
    fn a_band_measures_its_phase_from_the_image_origin() {
        // 帯の左端を基準にすると帯ごとに違う物差しになる
        assert_eq!(first_cell(0, 3, 8), 3);
        assert_eq!(first_cell(8, 3, 8), 11);
        assert_eq!(first_cell(10, 3, 8), 11);
        assert_eq!(first_cell(11, 3, 8), 11);
        assert_eq!(first_cell(12, 3, 8), 19);
    }

    #[test]
    fn the_diagnostic_view_agrees_with_the_estimator() {
        // 診断用の口が推定と食い違うと，校正で見ているものが別物になる
        let img = upscaled(&WIDE, &palette(), 6, (2, 3));
        let params = GridParams::default();
        let (cands, image_var) = scale_candidates(&img, &params);
        let e = estimate_grid(&img, &params).unwrap();

        let hat = cands
            .iter()
            .filter(|c| c.accepted())
            .max_by_key(|c| c.scale)
            .expect("受け入れられた候補がある");
        assert_eq!(hat.scale, e.scale);
        assert_eq!(hat.phase, e.phase);
        assert!((hat.mean_variance - e.mean_variance).abs() < 1e-6);
        assert!(image_var > 0.0);
    }

    #[test]
    fn the_control_group_is_larger_non_multiples_only() {
        let img = upscaled(&WIDE, &palette(), 4, (0, 0));
        let (cands, _) = scale_candidates(&img, &GridParams::default());
        let group = control_group_of(4, &cands, 16);

        assert!(group.iter().all(|s| *s > 4), "ŝ 以下の s が残っている");
        for s in [8u32, 12, 16] {
            assert!(!group.contains(&s), "倍数 {s} が残っている");
        }
        assert!(group.contains(&5) && group.contains(&7));
    }

    #[test]
    fn a_clean_upscale_is_confident() {
        let img = upscaled(&WIDE, &palette(), 4, (0, 0));
        let e = estimate_grid(&img, &GridParams::default()).unwrap();
        assert!(e.confidence > 0.0, "きれいな格子なのに信頼度が 0 である");
    }

    #[test]
    fn recovers_the_scale_of_a_clean_upscale() {
        for scale in [2u32, 3, 4, 6, 8] {
            let img = upscaled(&PATTERN, &palette(), scale, (0, 0));
            let e = estimate_grid(&img, &GridParams::default())
                .unwrap_or_else(|err| panic!("{scale} 倍で失敗: {err}"));
            assert_eq!(e.scale, scale, "{scale} 倍を {} と推定した", e.scale);
            assert_eq!(e.phase, ivec2(0, 0));
        }
    }

    #[test]
    fn recovers_the_phase_when_the_image_is_cropped() {
        for (dx, dy) in [(1u32, 0u32), (0, 1), (2, 3), (3, 3)] {
            let img = upscaled(&PATTERN, &palette(), 4, (dx, dy));
            let e = estimate_grid(&img, &GridParams::default()).unwrap();
            assert_eq!(e.scale, 4, "位相 ({dx},{dy}) でスケールを外した");
            // 位相は「切り落とした分だけ格子がずれる」
            let expect = ivec2(((4 - dx) % 4) as i32, ((4 - dy) % 4) as i32);
            assert_eq!(e.phase, expect, "位相 ({dx},{dy})");
        }
    }

    /// D28 の要点 — 約数を選んではいけない．
    #[test]
    fn picks_the_largest_scale_not_a_divisor() {
        let img = upscaled(&PATTERN, &palette(), 8, (0, 0));
        let e = estimate_grid(&img, &GridParams::default()).unwrap();
        assert_eq!(e.scale, 8, "約数 (2 や 4) を選んでいる");
    }

    /// 再構成検査の要点 — 倍数側の過大推定を排除する．
    #[test]
    fn reconstruction_check_rejects_overestimation() {
        let img = upscaled(&PATTERN, &palette(), 3, (0, 0));
        // 6 は 3 の倍数だが，隣り合うセルの色が違うので再構成が合わない
        let e = estimate_grid(&img, &GridParams::default()).unwrap();
        assert_eq!(e.scale, 3);
    }

    #[test]
    fn confidence_is_zero_for_a_flat_image() {
        let img = RgbaCanvas::filled(32, 32, Rgba8::rgb(10, 20, 30));
        // 平坦な画像はすべての s が閾値を満たす退化ケース (設計書 6.1) なので，
        // 自信を持って答えてはいけない．棄却されるか，信頼度 0 で返るかのどちらかである．
        // 既定の min_confidence が 0 より大きいので通常は LowConfidence になる
        match estimate_grid(&img, &GridParams::default()) {
            Ok(e) => assert_eq!(e.confidence, 0.0, "平坦な画像に信頼度が付いている"),
            Err(GridError::NotFound | GridError::LowConfidence) => {}
            Err(e) => panic!("想定外: {e}"),
        }
    }

    #[test]
    fn a_flat_image_is_rejected_when_a_minimum_confidence_is_set() {
        let img = RgbaCanvas::filled(32, 32, Rgba8::rgb(10, 20, 30));
        let params = GridParams {
            min_confidence: 0.01,
            ..GridParams::default()
        };
        assert!(matches!(
            estimate_grid(&img, &params),
            Err(GridError::LowConfidence) | Err(GridError::NotFound)
        ));
    }

    #[test]
    fn confidence_is_positive_for_a_clear_grid() {
        let img = upscaled(&PATTERN, &palette(), 4, (0, 0));
        let e = estimate_grid(&img, &GridParams::default()).unwrap();
        assert!(e.confidence > 0.0, "はっきりした格子に信頼度が付かない");
    }

    #[test]
    fn noise_beyond_epsilon_is_not_accepted_as_a_grid() {
        // 全画素がばらばら — 格子は無い
        let mut img = RgbaCanvas::filled(32, 32, Rgba8::TRANSPARENT);
        let mut state = 12345u32;
        for y in 0..32 {
            for x in 0..32 {
                state = state.wrapping_mul(1103515245).wrapping_add(12345);
                let v = (state >> 16) as u8;
                img.set(x, y, Rgba8::rgb(v, v.wrapping_mul(3), v.wrapping_mul(7)));
            }
        }
        assert!(estimate_grid(&img, &GridParams::default()).is_err());
    }

    #[test]
    fn tiny_images_are_rejected() {
        let img = RgbaCanvas::filled(1, 1, Rgba8::rgb(0, 0, 0));
        assert_eq!(
            estimate_grid(&img, &GridParams::default()),
            Err(GridError::TooSmall)
        );
    }

    #[test]
    fn phase_ties_are_broken_lexicographically() {
        // 縦縞だけの画像は y 方向の位相が全て同点になる
        let mut img = RgbaCanvas::filled(16, 16, Rgba8::TRANSPARENT);
        for y in 0..16 {
            for x in 0..16 {
                let c = if (x / 4) % 2 == 0 {
                    Rgba8::rgb(0, 0, 0)
                } else {
                    Rgba8::rgb(255, 255, 255)
                };
                img.set(x, y, c);
            }
        }
        let a = estimate_grid(&img, &GridParams::default()).unwrap();
        let b = estimate_grid(&img, &GridParams::default()).unwrap();
        assert_eq!(a, b, "同じ入力で結果が揺れている");
        assert_eq!(a.phase.y, 0, "同点なのに辞書式で最小になっていない");
    }

    #[test]
    fn candidate_scales_include_the_true_scale() {
        for scale in [2u32, 3, 4, 6, 8] {
            let img = upscaled(&PATTERN, &palette(), scale, (0, 0));
            let candidates = candidate_scales(&img, 16);
            assert!(
                candidates.contains(&scale),
                "{scale} 倍が候補に入っていない: {candidates:?}"
            );
        }
    }

    #[test]
    fn downscale_modal_recovers_the_original_pattern() {
        let colors = palette();
        let img = upscaled(&PATTERN, &colors, 5, (0, 0));
        let small = downscale_modal(&img, 5, ivec2(0, 0));
        assert_eq!((small.width(), small.height()), (6, 6));
        for (y, row) in PATTERN.iter().enumerate() {
            for (x, ch) in row.bytes().enumerate() {
                let expect = colors[(ch - b'0') as usize];
                assert_eq!(small.get(x as i32, y as i32), Some(expect), "({x},{y})");
            }
        }
    }

    #[test]
    fn downscale_modal_is_deterministic() {
        let img = upscaled(&PATTERN, &palette(), 4, (0, 0));
        assert_eq!(
            downscale_modal(&img, 4, ivec2(0, 0)),
            downscale_modal(&img, 4, ivec2(0, 0))
        );
    }

    #[test]
    fn local_grid_agrees_with_the_global_estimate_on_a_uniform_image() {
        let big: Vec<String> = (0..24)
            .map(|y| {
                (0..24)
                    .map(|x| char::from(b'0' + ((x + y) % 4) as u8))
                    .collect()
            })
            .collect();
        let refs: Vec<&str> = big.iter().map(|s| s.as_str()).collect();
        let img = upscaled(&refs, &palette(), 4, (0, 0));

        let local = local_grid(&img, 32, &GridParams::default());
        let (scale, ratio) = uniformity(&local).expect("局所推定が 1 つも取れない");
        assert_eq!(scale, 4);
        assert!(ratio > 0.5, "窓ごとの一致率が低すぎる: {ratio}");
    }

    /// 平坦な窓は票を投じない．窓に収まる最大の $s$ を選んでしまうため．
    #[test]
    fn flat_windows_do_not_vote() {
        let img = RgbaCanvas::filled(64, 64, Rgba8::rgb(10, 20, 30));
        let local = local_grid(&img, 32, &GridParams::default());
        assert!(
            local.data().iter().all(|v| v.is_none()),
            "平坦な画像から局所推定が出ている: {:?}",
            local.data()
        );
        assert_eq!(uniformity(&local), None);
    }

    #[test]
    fn uniformity_is_none_without_any_estimate() {
        let empty: Field<Option<u32>> = Field::filled(2, 2, None);
        assert_eq!(uniformity(&empty), None);
    }
}
