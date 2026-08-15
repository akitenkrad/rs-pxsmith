//! アンチエイリアス (`pxsmith aa`．設計書 6.5)．
//!
//! AA は**2 色の橋渡し**であり，段の角に «間の色» を置いて境目を和らげる．
//! 置く色は**曲率符号で決まる** — 凸な曲線では AA の中央部が明るい色 ・両端が暗い色，
//! 凹ではその逆である．
//!
//! # 中間色はランプから引けない
//!
//! 設計書の `select_aa_index` は `ramp.step(base, ±1)` で引くが，**ランプの宣言は
//! ファイルに残らない** (D81) ．受け取った絵に AA を付けるには，パレットから
//! «2 色の間にある色» を探すことになる．
//!
//! **実測では内部境界の色の組 2769 件のうち 2250 件 (81.3%) に間の色があった**ので，
//! 大半は既にある色で足りる．無ければ中点の色を作って足す (上限あり) ．
//!
//! # 付けない相手
//!
//! | 相手 | 理由 |
//! | --- | --- |
//! | 外郭 | 背景色が不定なので AA が機能せず，ゲーム内で縁が汚れる (D34)．明示的に頼まれたときだけ付ける |
//! | 45° 線 | 段が 1 画素ずつなので «角» が無い (設計書 6.5)．実測で境界チェーンの 43.7% がこれ |
//! | 色差の小さい組 | 間の色を置いても見えない．丸めで元の色に戻ることもある |
//! | **既に中間色が置いてある境界** | **AA の上に AA が乗る**．1 度目で置いた色が 2 度目の «境界の相手» になるので，掛けるたびに縁が太っていく — 冪等でなくなる |

use crate::canvas::IndexedCanvas;
use crate::color::{Oklab, distance_sq};
use crate::error::Result;
use crate::geom::distance::{curvature_field, signed_distance};
use crate::geom::runs::{run_lengths, run_pixels};
use crate::geom::{Mask, split_monotone, trace_contours};
use crate::math::{IVec2, ivec2};
use crate::palette::Palette;
use crate::quantize::oklab_to_rgba;

/// AA を付けるときの設定．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AaAddOptions {
    /// **外郭にも付けるか** (D34 の既定は `false`)．
    pub include_outline: bool,
    /// この色距離より近い 2 色には付けない．
    ///
    /// **暫定値である．** 内部境界の色の組の色距離は中央値 0.174 ・最小 0.003 で，
    /// 近い方の裾には «同じ面の中の微妙な質感» が入る — そこへ中間色を置いても
    /// 見えないうえ，8 ビットへ丸めると元の色に戻ることがある．
    pub min_span: f32,
    /// «2 色の間にある» とみなす遠回りの許容 ([`crate::clean::AaOptions`] の裏返し)．
    pub tolerance: f32,
    /// 中間色が無いときに**作ってよい色数**．0 なら作らない．
    ///
    /// 設計書 6.5 は «中間色の数 1 〜 2 色．上限 3 色» とする — ここは
    /// «1 つの絵に足す色の上限» なので別物である．
    pub max_new_colors: usize,
    /// **これより短い段には付けない．**
    ///
    /// 設計書 6.5 は «45° 線には通常付けない» とする — 段が 1 画素の並びだからである．
    /// 段 2 画素は約 27° でほぼ 45° に近く，AA の効き目より «増える色» の方が目立つ．
    ///
    /// **暫定値である．** 良い絵 61 枚 (不透明 27639 画素) で測った．
    ///
    /// | 下限 | 置いた画素 | 割合 | 2 巡目 | 3 巡目 | 4 巡目 |
    /// | --- | --- | --- | --- | --- | --- |
    /// | 2 | 2293 | 8.3% | 1109 | 592 | 349 |
    /// | **3** | **836** | **3.0%** | **324** | **138** | **44** |
    /// | 4 | 409 | 1.5% | 153 | 64 | 17 |
    /// | 5 | 247 | 0.9% | 87 | 37 | 12 |
    ///
    /// 下限 2 では画面の 8% が中間色になる — lint ルール 14 (AA 過多) の相手であり，
    /// 設計書 6.5 の «多すぎるより少ない方が良い» に反する．3 を採った．
    ///
    /// > [!warning] **`pxsmith aa` は冪等ではない．**
    /// > AA は輪郭の形を変えるので，2 度目には**その先に新しい角**ができる．
    /// > 量は巡ごとに 1/3 程度へ減っていくが 0 にはならない (上の表) ．
    /// > 窓を広げて抑える案は測って外れた — **冪等になる頃には何も塗らなくなる**
    /// > (半径 3 で置いた画素が 61 枚あわせて 17) ．
    /// > **掛けるのは 1 度だけにする**こと．
    pub min_run: u32,
    /// **中点からずらす量** (Oklab の色距離)．
    ///
    /// 設計書 6.5 は AA の中央部と両端で違う色を使うと規定する (曲率符号で明暗が
    /// 決まる) ので，中間色は 1 色では足りない．**中点の両側へ等距離にずらす．**
    ///
    /// > [!warning] **ずらす量は «AA 除去が中間色と認める範囲» に収める．**
    /// > [`crate::clean::remove_antialiasing`] は «2 色の**中点**から
    /// > [`crate::clean::AaOptions::tolerance`] 以内» を中間色とみなす．
    /// > 割合でずらす (例: 端から 35% ・65%) と，色差の大きい組でこの範囲を外れ，
    /// > **付けた AA を自分の道具で外せなくなる** (実際に往復の試験が落ちた) ．
    /// >
    /// > 除去の側を «線分の上» まで広げる案も測ったが，**良い絵で AA とみなす画素が
    /// > 5.76% → 7.16% に増える** (色 297 → 350) ので採らない — 付ける側を
    /// > 合わせる方が代償が小さい．
    ///
    /// **暫定値である．** 除去の許容 0.05 の内側という以上の根拠は無い．
    pub offset: f32,
}

impl Default for AaAddOptions {
    fn default() -> Self {
        Self {
            include_outline: false,
            min_span: 0.08,
            tolerance: 0.05,
            max_new_colors: 4,
            offset: 0.04,
            min_run: 3,
        }
    }
}

/// AA を付けた結果．
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AaReport {
    /// 中間色に置き換えた画素の数．
    pub painted: usize,
    /// 新しく作った中間色の数．
    pub added_colors: usize,
    /// 45° として飛ばしたチェーンの数．
    pub skipped_diagonal: usize,
    /// 外郭として飛ばした画素の数．
    pub skipped_outline: usize,
    /// 中間色が用意できずに飛ばした画素の数．
    pub no_colour: usize,
}

/// チェーンの中でどこにいるか (設計書 6.5 の `ChainPosition`)．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ChainPosition {
    /// 中央部．
    Middle,
    /// 両端．
    End,
}

/// **明るい方の中間色を使うか** (設計書 6.5 の表)．
///
/// | 曲線 | 中央部 | 両端 |
/// | --- | --- | --- |
/// | 凸 ($\kappa > 0$) | 明るい色 | 暗い色 |
/// | 凹 | 暗い色 | 明るい色 |
pub fn wants_brighter(curvature: f32, pos: ChainPosition) -> bool {
    matches!(
        (curvature > 0.0, pos),
        (true, ChainPosition::Middle) | (false, ChainPosition::End)
    )
}

/// **AA を付ける** (設計書 6.5)．
///
/// 内部境界の «段の角» を中間色に置き換える．パレットに中間色が無ければ作って足す．
pub fn add_antialiasing(
    canvas: &mut IndexedCanvas,
    palette: &mut Palette,
    opts: &AaAddOptions,
) -> Result<AaReport> {
    let mut report = AaReport::default();
    let mut indices: Vec<u8> = canvas.pixels().to_vec();
    indices.sort_unstable();
    indices.dedup();

    // **既に置いてある AA を剥がしてから «角» を探す．** 剥がさないと，1 度目で
    // 塗った画素が形を変え，2 度目に新しい角ができて縁が太っていく (冪等でなくなる)
    let (source, already) = strip_aa(canvas, palette, opts.tolerance);
    let mut paint: Vec<(IVec2, u8)> = Vec::new();

    for index in indices {
        if source.transparent() == Some(index) {
            continue;
        }
        let mask = source.mask_of(index);
        let curvature = curvature_field(&signed_distance(&mask));

        for contour in trace_contours(&mask) {
            for chain in split_monotone(&contour) {
                let runs = run_lengths(&chain);
                if runs.len() < 2 {
                    continue;
                }
                // 45° 線には付けない
                if runs.iter().all(|&r| r == 1) {
                    report.skipped_diagonal += 1;
                    continue;
                }
                let groups = run_pixels(&chain);
                let total: usize = groups.iter().map(|g| g.len()).sum();
                let mut seen = 0usize;

                for (i, group) in groups.iter().enumerate() {
                    // 短い段には付けない (45° に近いほど効き目より色数が目立つ)
                    if group.len() < opts.min_run.max(2) as usize {
                        seen += group.len();
                        continue;
                    }
                    // 段の角 — 隣のランがある側の端だけ
                    let mut ends: Vec<IVec2> = Vec::new();
                    if i > 0 {
                        ends.extend(group.first().copied());
                    }
                    if i + 1 < groups.len() {
                        ends.extend(group.last().copied());
                    }

                    for at in ends {
                        // そこは既に塗ってある
                        if already.get(at) {
                            continue;
                        }
                        let position = position_in_chain(seen, total);
                        let k = curvature.copied(at).unwrap_or(0.0);
                        match plan_pixel(&source, palette, at, index, k, position, opts) {
                            Ok(Some((to, added))) => {
                                paint.push((at, to));
                                report.painted += 1;
                                report.added_colors += usize::from(added);
                            }
                            Ok(None) => {}
                            Err(skip) => match skip {
                                Skip::Outline => report.skipped_outline += 1,
                                Skip::NoColour => report.no_colour += 1,
                            },
                        }
                    }
                    seen += group.len();
                }
            }
        }
    }

    for (at, to) in paint {
        canvas.set_at(at, to);
    }
    Ok(report)
}

/// 飛ばした理由．
enum Skip {
    Outline,
    NoColour,
}

fn position_in_chain(seen: usize, total: usize) -> ChainPosition {
    // 前後 1/3 を «両端» とみなす
    let third = total.max(3) / 3;
    if seen < third || seen + third >= total {
        ChainPosition::End
    } else {
        ChainPosition::Middle
    }
}

/// 1 画素ぶんの «何色にするか» を決める．**まだ塗らない**．
///
/// 返り値の `bool` は «中間色を新しく作ったか»．
#[allow(clippy::result_large_err)]
fn plan_pixel(
    canvas: &IndexedCanvas,
    palette: &mut Palette,
    at: IVec2,
    index: u8,
    curvature: f32,
    position: ChainPosition,
    opts: &AaAddOptions,
) -> std::result::Result<Option<(u8, bool)>, Skip> {
    // 境界の相手 — 4 近傍で最も多い «自分以外の不透明な色» (同数は小さい添字)
    let mut counts: std::collections::BTreeMap<u8, usize> = Default::default();
    let mut outline = false;
    for d in [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)] {
        match canvas.get_at(at + d) {
            Some(i) if i == index => {}
            Some(i) if canvas.transparent() == Some(i) => outline = true,
            Some(i) => *counts.entry(i).or_default() += 1,
            // 画像の外も外郭である
            None => outline = true,
        }
    }
    if outline && !opts.include_outline {
        return Err(Skip::Outline);
    }
    let Some((&other, _)) = counts
        .iter()
        .max_by_key(|(i, n)| (**n, std::cmp::Reverse(**i)))
    else {
        return Ok(None);
    };

    let (Some(a), Some(b)) = (palette.lab_of(index), palette.lab_of(other)) else {
        return Ok(None);
    };
    let span = distance_sq(a, b, 1.0).sqrt();
    if span < opts.min_span {
        return Ok(None);
    }

    // 明るい側 ・暗い側のどちらへ寄せるか (設計書 6.5 の表)
    let brighter = wants_brighter(curvature, position);
    // 中間色は «2 色の間» にしかないので，寄せる先は «明るい方の色» か «暗い方の色»
    let toward_bright = a.l < b.l;
    // **中点から等距離にずらす** (割合でずらすと AA 除去の見方から外れる)．
    // 端へ届かないように，ずらす量は半分の 8 割まで
    let shift = opts.offset.min(span * 0.4) * if brighter == toward_bright { 1.0 } else { -1.0 };
    let t = 0.5 + shift / span;
    let target = mix(a, b, t);

    if let Some(existing) = nearest_between(palette, index, other, target, opts.tolerance) {
        return Ok(Some((existing, false)));
    }
    if opts.max_new_colors == 0 || palette.len() >= Palette::MAX_COLORS {
        return Err(Skip::NoColour);
    }
    match palette.push(oklab_to_rgba(target)) {
        Ok(i) => Ok(Some((i, true))),
        Err(_) => Err(Skip::NoColour),
    }
}

fn mix(a: Oklab, b: Oklab, t: f32) -> Oklab {
    Oklab {
        l: a.l + (b.l - a.l) * t,
        a: a.a + (b.a - a.a) * t,
        b: a.b + (b.b - a.b) * t,
    }
}

/// **既に置いてある AA を剥がした写しと，その画素のマスク．**
///
/// 画素ごとに «8 近傍に現れる 2 色の間にあるか» を見て，間にあるなら近い方の端の色へ
/// 戻す．[`crate::clean::remove_antialiasing`] と同じ見方を**画素単位**で行うものである．
///
/// > [!warning] **これが無いと `pxsmith aa` は冪等にならない．**
/// > 角を塗ると輪郭の形が変わり，2 度目には**その先に新しい角ができる**．
/// > 掛けるたびに縁が太っていく — 良い絵 61 枚のうち 29 枚がそうなっていた．
/// >
/// > 窓を広げて «近くに AA があるなら塗らない» とする案は測って外れた．
/// > **冪等になる頃には何も塗らなくなる**．
/// >
/// > | 窓の半径 | 置いた画素 | 2 度目で塗る絵 |
/// > | --- | --- | --- |
/// > | 1 | 636 | 29 枚 |
/// > | 2 | 98 | 3 枚 |
/// > | 3 | **17** | 0 枚 |
/// >
/// > **剥がしてから見れば «元の角» が毎回同じに出る**ので，2 度目は «そこは塗って
/// > ある» と分かる．窓の広さと引き換えにしなくてよい．
///
/// 返すマスクは**その絵で中間色として置かれている画素**であり，lint ルール 14
/// (AA 過多) の分子でもある — **付ける側 ・外す側 ・数える側で «中間色» の定義を
/// 1 か所にまとめる**ためにここを共有する (D83 でずれて往復が壊れた) ．
pub fn strip_aa(
    canvas: &IndexedCanvas,
    palette: &Palette,
    tolerance: f32,
) -> (IndexedCanvas, Mask) {
    let mut base = canvas.clone();
    let mut placed = Mask::new(canvas.width(), canvas.height());

    for p in canvas.bounds().iter() {
        let Some(m) = canvas.get_at(p) else { continue };
        if canvas.transparent() == Some(m) {
            continue;
        }
        let Some(lm) = palette.lab_of(m) else {
            continue;
        };

        let mut near: Vec<u8> = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(i) = canvas.get_at(p + ivec2(dx, dy))
                    && i != m
                    && canvas.transparent() != Some(i)
                    && !near.contains(&i)
                {
                    near.push(i);
                }
            }
        }

        // **どちらの端へ戻すかは «色の近さ» ではなく «周りの多さ» で決める．**
        // AA は面から 1 画素を借りて置くものなので，戻す先は**元の持ち主の面**である．
        // 色で決めると，向こう側へ寄せた画素が相手の面へ戻り，**剥がした絵が元の絵と
        // 違う形になる** — 角の位置がずれて «そこは塗ってある» と分からなくなる
        let mut around: std::collections::BTreeMap<u8, usize> = Default::default();
        for d in [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)] {
            if let Some(i) = canvas.get_at(p + d)
                && i != m
                && canvas.transparent() != Some(i)
            {
                *around.entry(i).or_default() += 1;
            }
        }
        let mut best: Option<(f32, u8)> = None;
        for &x in &near {
            for &y in &near {
                if x >= y {
                    continue;
                }
                let (Some(lx), Some(ly)) = (palette.lab_of(x), palette.lab_of(y)) else {
                    continue;
                };
                let span = distance_sq(lx, ly, 1.0).sqrt();
                let (da, db) = (
                    distance_sq(lm, lx, 1.0).sqrt(),
                    distance_sq(lm, ly, 1.0).sqrt(),
                );
                if span <= f32::EPSILON || da + db - span > tolerance || da >= span || db >= span {
                    continue;
                }
                let (nx, ny) = (
                    around.get(&x).copied().unwrap_or(0),
                    around.get(&y).copied().unwrap_or(0),
                );
                // 周りに多い方へ戻す．同数なら色の近い方，それも同じなら小さい添字
                let to = match nx.cmp(&ny) {
                    std::cmp::Ordering::Greater => x,
                    std::cmp::Ordering::Less => y,
                    std::cmp::Ordering::Equal if da <= db => x,
                    std::cmp::Ordering::Equal => y,
                };
                let d = da.min(db);
                if best.as_ref().is_none_or(|(b, _)| d < *b) {
                    best = Some((d, to));
                }
            }
        }
        if let Some((_, to)) = best {
            base.set_at(p, to);
            placed.set(p, true);
        }
    }
    (base, placed)
}

/// **中間色として置かれている画素の数** (lint ルール 14 の分子)．
///
/// 「その色が，自分より広く使われている 2 色のちょうど中点にある」ものを中間色と
/// みなし，その色の画素をすべて数える．[`crate::clean::remove_antialiasing`] の
/// 判定から**面積の上限だけを外した**ものである — 上限は «消してよいか» の安全弁で
/// あって «中間色か» の定義ではない．
///
/// > [!warning] **面積の上限を残すと «AA が多いほど数が減る»．**
/// > 除去の側は 16 画素以下しか対象にしないので，AA を敷き詰めると中間色が
/// > その枠を超えて**数から外れる**．実測でも，縁を全部ぼかした負例が
/// > 3.9 〜 10.2% と良い絵の 90% 点 (11.2%) を下回った．割合を測る用途では外す．
///
/// > [!note] 画素ごとに «8 近傍の 2 色の間か» を見る [`strip_aa`] は**使えない**．
/// > 密な質感では色の大半が誰かの «間» に入り，良い絵 61 枚で中央 17.5% ・
/// > 最大 67.0% になる．あちらは `pxsmith aa` が «元の角» を復元するための道具である．
pub fn intermediate_pixels(canvas: &IndexedCanvas, palette: &Palette, tolerance: f32) -> usize {
    let mut areas: std::collections::BTreeMap<u8, u32> = Default::default();
    for &i in canvas.pixels() {
        if canvas.transparent() != Some(i) {
            *areas.entry(i).or_default() += 1;
        }
    }

    let mut count = 0usize;
    for (&i, &area) in &areas {
        let Some(mid) = palette.lab_of(i) else {
            continue;
        };
        let between = areas.iter().any(|(&a, &na)| {
            na > area
                && areas.iter().any(|(&b, &nb)| {
                    if b <= a || b == i || a == i || nb <= area {
                        return false;
                    }
                    let (Some(la), Some(lb)) = (palette.lab_of(a), palette.lab_of(b)) else {
                        return false;
                    };
                    let midpoint = Oklab::new(
                        (la.l + lb.l) * 0.5,
                        (la.a + lb.a) * 0.5,
                        (la.b + lb.b) * 0.5,
                    );
                    distance_sq(mid, midpoint, 1.0).sqrt() <= tolerance
                })
        });
        if between {
            count += area as usize;
        }
    }
    count
}

/// **2 色の間にあり，狙った色に最も近いパレットの色**．
///
/// «間» は «$a$ と $b$ を結ぶ線からの遠回りが許容以内» とする — [`crate::clean`] の
/// AA 除去が «中間色» とみなす条件と同じ形なので，付けた AA は同じ道具で外せる．
pub fn nearest_between(
    palette: &Palette,
    a: u8,
    b: u8,
    target: Oklab,
    tolerance: f32,
) -> Option<u8> {
    let (Some(x), Some(y)) = (palette.lab_of(a), palette.lab_of(b)) else {
        return None;
    };
    let span = distance_sq(x, y, 1.0).sqrt();
    if span <= f32::EPSILON {
        return None;
    }
    let mut best: Option<(f32, u8)> = None;
    for (i, lab) in palette.lab().iter().enumerate() {
        let i = i as u8;
        if i == a || i == b || palette.get(i).is_some_and(|c| c.a == 0) {
            continue;
        }
        let (da, db) = (
            distance_sq(*lab, x, 1.0).sqrt(),
            distance_sq(*lab, y, 1.0).sqrt(),
        );
        // 直線から外れた分．0 なら 2 色をまっすぐ結ぶ線の上にある
        if da + db - span > tolerance || da >= span || db >= span {
            continue;
        }
        let off = distance_sq(*lab, target, 1.0).sqrt();
        // 同点は小さい添字 (決定論性の規則 2)
        if best.as_ref().is_none_or(|(b0, _)| off < *b0) {
            best = Some((off, i));
        }
    }
    best.map(|(_, i)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clean::{AaOptions, remove_antialiasing};
    use crate::color::Rgba8;

    /// 暗い面と明るい面が緩い階段で接する絵 (内部境界)．
    fn two_faces() -> (IndexedCanvas, Palette) {
        // 添字 0 = 透明 ・1 = 暗い ・2 = 明るい
        let palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0x30, 0x34, 0x5a),
            Rgba8::rgb(0xc8, 0xcc, 0xe0),
        ])
        .unwrap();
        let mut c = IndexedCanvas::filled(16, 10, 1).with_transparent(Some(0));
        // 上側を明るい色にする．段は 4 画素ずつ (緩い階段)
        for (step, x0) in [0i32, 4, 8, 12].iter().enumerate() {
            for x in *x0..(*x0 + 4) {
                for y in 0..(2 + step as i32) {
                    c.set(x, y, 2);
                }
            }
        }
        (c, palette)
    }

    /// 45° の階段 (ラン長がすべて 1)．
    fn diagonal() -> (IndexedCanvas, Palette) {
        let palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0x30, 0x34, 0x5a),
            Rgba8::rgb(0xc8, 0xcc, 0xe0),
        ])
        .unwrap();
        let mut c = IndexedCanvas::filled(12, 12, 1).with_transparent(Some(0));
        for y in 0..12i32 {
            for x in 0..(y + 1) {
                c.set(x, y, 2);
            }
        }
        (c, palette)
    }

    /// **段の角に中間色が置かれる．**
    #[test]
    fn a_shallow_staircase_gets_intermediate_pixels_at_its_corners() {
        let (mut c, mut palette) = two_faces();
        let before = palette.len();
        let report = add_antialiasing(&mut c, &mut palette, &AaAddOptions::default()).unwrap();
        assert!(report.painted > 0, "1 画素も置いていない: {report:?}");
        assert!(
            palette.len() > before,
            "中間色が無いのに作っていない ({before} 色のまま)"
        );
        // 置いた色は 2 色の «間» にある
        for &i in c.pixels() {
            if i == 0 || i == 1 || i == 2 {
                continue;
            }
            let (l, a, b) = (
                palette.lab_of(i).unwrap(),
                palette.lab_of(1).unwrap(),
                palette.lab_of(2).unwrap(),
            );
            let span = distance_sq(a, b, 1.0).sqrt();
            let detour = distance_sq(l, a, 1.0).sqrt() + distance_sq(l, b, 1.0).sqrt() - span;
            assert!(
                detour < 0.05,
                "添字 {i} が 2 色の間に無い (遠回り {detour})"
            );
        }
    }

    /// **45° 線には付けない** (設計書 6.5)．
    #[test]
    fn a_forty_five_degree_edge_is_left_alone() {
        let (mut c, mut palette) = diagonal();
        let before = c.clone();
        let report = add_antialiasing(&mut c, &mut palette, &AaAddOptions::default()).unwrap();
        assert_eq!(report.painted, 0, "45° に AA を付けた: {report:?}");
        assert!(report.skipped_diagonal > 0, "45° と数えていない");
        assert_eq!(c, before);
    }

    /// **外郭には付けない** (D34)．シルエットは 1 画素も変わらない．
    #[test]
    fn the_outer_silhouette_is_not_touched_by_default() {
        let (mut c, mut palette) = two_faces();
        // 右端を透明にして «外郭» を作る
        for y in 0..10i32 {
            c.set(15, y, 0);
        }
        let silhouette = c.mask_of(0);
        let report = add_antialiasing(&mut c, &mut palette, &AaAddOptions::default()).unwrap();
        assert!(report.skipped_outline > 0, "外郭を数えていない: {report:?}");
        assert_eq!(c.mask_of(0), silhouette, "シルエットが変わった");
    }

    /// **色差が小さい組には付けない．**
    #[test]
    fn two_nearly_identical_colours_get_no_antialiasing() {
        let palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0x80, 0x82, 0x88),
            Rgba8::rgb(0x82, 0x84, 0x8a),
        ])
        .unwrap();
        let mut c = IndexedCanvas::filled(16, 10, 1).with_transparent(Some(0));
        for (step, x0) in [0i32, 4, 8, 12].iter().enumerate() {
            for x in *x0..(*x0 + 4) {
                for y in 0..(2 + step as i32) {
                    c.set(x, y, 2);
                }
            }
        }
        let mut palette = palette;
        let before = c.clone();
        let report = add_antialiasing(&mut c, &mut palette, &AaAddOptions::default()).unwrap();
        assert_eq!(report.painted, 0, "見えない AA を置いた: {report:?}");
        assert_eq!(c, before);
    }

    /// **`pxsmith clean --remove-aa` が外せる．**
    ///
    /// 付ける側と外す側が «中間色» を同じ形で見ていることの確認である
    /// (定義がずれていると，付けた AA を自分の道具で外せない) ．
    ///
    /// > [!note] **«元の絵に戻る» ことまでは求めない．**
    /// > AA の色は中点の両側に置く (設計書 6.5 の «中央部は明るい色 ・両端は暗い色»)
    /// > ので，向こう側へ寄せた画素は**近い方の端が元の持ち主ではない**．
    /// > 外すと元と違う側の色になるのは AA の定義どおりである．
    #[test]
    fn what_this_adds_can_be_removed_again() {
        let (original, palette) = two_faces();
        let mut c = original.clone();
        let mut palette = palette;
        let report = add_antialiasing(&mut c, &mut palette, &AaAddOptions::default()).unwrap();
        assert!(report.painted > 0);
        assert_ne!(c, original, "何も変わっていない");
        let added: Vec<u8> = (3..palette.len() as u8).collect();
        assert!(
            c.pixels().iter().any(|i| added.contains(i)),
            "作った中間色が 1 画素も使われていない"
        );

        let removed = remove_antialiasing(&mut c, &palette, &AaOptions::default());
        assert!(removed > 0, "AA 除去が 1 画素も戻していない");
        assert!(
            !c.pixels().iter().any(|i| added.contains(i)),
            "中間色が残っている"
        );
        // 形は変わらない (透明の位置が同じ)
        assert_eq!(c.mask_of(0), original.mask_of(0));
    }

    /// **もう一度掛けても増えない** (冪等)．
    #[test]
    fn adding_antialiasing_twice_paints_nothing_new() {
        let (mut c, mut palette) = two_faces();
        add_antialiasing(&mut c, &mut palette, &AaAddOptions::default()).unwrap();
        let once = c.clone();
        let again = add_antialiasing(&mut c, &mut palette, &AaAddOptions::default()).unwrap();
        assert_eq!(again.painted, 0, "2 回目で塗った: {again:?}");
        assert_eq!(c, once);
    }

    /// 曲率符号で明暗が入れ替わる (設計書 6.5 の表)．
    #[test]
    fn the_curvature_sign_decides_which_side_the_aa_leans_to() {
        assert!(wants_brighter(1.0, ChainPosition::Middle));
        assert!(!wants_brighter(1.0, ChainPosition::End));
        assert!(!wants_brighter(-1.0, ChainPosition::Middle));
        assert!(wants_brighter(-1.0, ChainPosition::End));
    }

    /// 中間色が既にあるならそれを使う (色を増やさない)．
    #[test]
    fn an_existing_intermediate_colour_is_reused() {
        let (mut c, palette) = two_faces();
        // 2 色の中点を先に入れておく
        let mid = mix(palette.lab_of(1).unwrap(), palette.lab_of(2).unwrap(), 0.5);
        let mut palette = palette;
        palette.push(oklab_to_rgba(mid)).unwrap();
        let before = palette.len();

        let report = add_antialiasing(&mut c, &mut palette, &AaAddOptions::default()).unwrap();
        assert!(report.painted > 0);
        assert_eq!(
            palette.len(),
            before,
            "既にある中間色を使わずに色を増やした ({report:?})"
        );
    }
}
