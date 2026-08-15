//! ジャギー正規化 (`px smooth`．設計書 6.4)．
//!
//! 検出は [`crate::geom::jaggy`] が済ませている — ここは**画素を動かす側**である．
//!
//! # 動かし方を «試して選ぶ» 理由
//!
//! 設計書の擬似コードは «ラン $i$ の端の画素を隣に合わせる方向へ移動» としか書いて
//! いないが，**どちら側から借りるかで結果が変わる**．階段 `[3, 3, 2, 3]` で測ると，
//!
//! | 借りる側 | 結果 | |
//! | --- | --- | --- |
//! | 後ろ (端の側) | `[3, 3, 3, 2]` | 谷が消える |
//! | 前 (中央の側) | `[3, 2, 3, 3]` | **新しい谷ができる** |
//!
//! どちらが良いかは形によって変わるので，**候補を当ててみて «ジャギーが実際に
//! 減ったか» で採否を決める**．減らない候補は当てない — 直したつもりで別の場所を
//! 壊すのが，この手の整形で最も起きやすい失敗である．
//!
//! # 数えるのは «触る色» の分だけ
//!
//! 判定は絵全体のジャギー数ではなく，**動かす画素が関わる 2 つの添字**についてだけ
//! 数え直す (D33 のとおり境界は色ごとに見るので，他の色の境界は動かない) ．
//! 全色を数え直すと 1 手ごとに絵全体を走査することになり，32x32 で桁が変わる．

use crate::canvas::IndexedCanvas;
use crate::geom::Mask;
use crate::geom::jaggy::{DEFAULT_MAX_MOVE, Jaggy, analyze_canvas, analyze_mask};
use crate::math::{IVec2, ivec2};

/// 整形の設定．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SmoothOptions {
    /// 画素の移動上限 (設計書 6.4 の $\delta_{\max}$)．
    ///
    /// これを超える谷は**直さずに報告する** — 意図的なディテールの可能性がある．
    pub max_move: u32,
    /// 走査を繰り返す上限．
    ///
    /// 1 回動かすと隣のランの長さも変わるので，収束するまで繰り返す．**上限は
    /// 安全網**であり，実測では良い絵 61 枚のどれも 3 巡以内で止まる．
    pub max_passes: usize,
}

impl Default for SmoothOptions {
    fn default() -> Self {
        Self {
            max_move: DEFAULT_MAX_MOVE,
            max_passes: 8,
        }
    }
}

/// 整形の結果．
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SmoothReport {
    /// 動かした画素の数．
    pub moved: usize,
    /// 残ったジャギーの数．
    pub remaining: usize,
    /// **移動上限を超えていて触らなかった数** (報告のみ)．
    pub over_limit: usize,
    /// **どの候補を当てても減らなかった数** — 直し方が無いということである．
    pub no_candidate: usize,
    /// **幾何が決めた刻みなので触らなかった数** (D169)．
    ///
    /// 一定の傾きの直線の digitization には谷が必ず現れる — それは描き損ねでは
    /// ないので，動かすと**正しく描いた線が壊れる** (D163 の実測: 清書 58 枚で
    /// 88 画素 ・17 枚) ．**黙って飛ばさずに数える** — «鳴らなかった» と
    /// «触らないと決めた» は別である (D77 ・D104 ・D164 の作法)．
    pub geometric: usize,
    /// 実際に回った巡回数．
    pub passes: usize,
}

/// **絵全体を整形する** (全色境界が対象．D33)．
pub fn smooth_canvas(canvas: &mut IndexedCanvas, opts: &SmoothOptions) -> SmoothReport {
    let mut report = SmoothReport::default();
    for pass in 1..=opts.max_passes.max(1) {
        report.passes = pass;
        let found = analyze_canvas(canvas, opts.max_move);
        let mut moved_this_pass = 0usize;

        for jaggy in &found.jaggies {
            let Some(index) = jaggy.index else { continue };
            if !jaggy.within_limit {
                continue;
            }
            // **幾何が決めた刻みは動かさない** (D169)．検出は今までどおり鳴るが，
            // 画素を動かすのはここだけなので，壊すのを止められるのもここだけである
            if jaggy.on_straight_chain {
                continue;
            }
            // 前の手で形が変わっていることがある．**当てる前に確かめる**
            if !jaggy
                .pixels
                .iter()
                .all(|&p| canvas.get_at(p) == Some(index))
            {
                continue;
            }
            if let Some(at) = best_move(canvas, jaggy, index, opts) {
                canvas.set_at(at.0, at.1);
                moved_this_pass += 1;
            }
        }

        report.moved += moved_this_pass;
        if moved_this_pass == 0 {
            break;
        }
    }

    let last = analyze_canvas(canvas, opts.max_move);
    report.remaining = last.jaggies.len();
    report.over_limit = last.jaggies.iter().filter(|j| !j.within_limit).count();
    report.geometric = last
        .jaggies
        .iter()
        .filter(|j| j.within_limit && j.on_straight_chain)
        .count();
    report.no_candidate = report
        .remaining
        .saturating_sub(report.over_limit + report.geometric);
    report
}

/// マスク 1 枚を整形する (シルエットだけを直したいとき)．
pub fn smooth_mask(mask: &mut Mask, opts: &SmoothOptions) -> SmoothReport {
    // マスクを 2 色のキャンバスに写して同じ経路を通す — 判定を 1 か所にまとめる
    let mut canvas =
        IndexedCanvas::filled(mask.width(), mask.height(), 0).with_transparent(Some(0));
    for p in mask.bounds().iter() {
        if mask.get(p) {
            canvas.set_at(p, 1);
        }
    }
    let report = smooth_canvas(&mut canvas, opts);
    for p in mask.bounds().iter() {
        mask.set(p, canvas.get_at(p) == Some(1));
    }
    report
}

/// 当てる候補と，当てた後のジャギー数を測って**最も減るものを選ぶ**．
///
/// 同点のときは候補の並び順で決める (決定論性の規則 2) ．**減らない候補は返さない．**
fn best_move(
    canvas: &IndexedCanvas,
    jaggy: &Jaggy,
    index: u8,
    opts: &SmoothOptions,
) -> Option<(IVec2, u8)> {
    let candidates = candidates(canvas, jaggy, index);
    if candidates.is_empty() {
        return None;
    }
    // 触る添字だけ数える
    let mut touched: Vec<u8> = candidates.iter().map(|(_, i)| *i).collect();
    touched.push(index);
    touched.sort_unstable();
    touched.dedup();

    let before = count_jaggies(canvas, &touched, opts.max_move);
    let mut best: Option<((IVec2, u8), usize)> = None;
    for (at, to) in candidates {
        let mut trial = canvas.clone();
        trial.set_at(at, to);
        let after = count_jaggies(&trial, &touched, opts.max_move);
        if after < before && best.as_ref().is_none_or(|(_, b)| after < *b) {
            best = Some(((at, to), after));
        }
    }
    best.map(|(m, _)| m)
}

/// 当てる候補 — **ランの両端の «外側» 1 画素**．
///
/// 前へ伸ばす (端の隣を自分の色にする) か，後ろへ伸ばす (手前の隣を自分の色にする)
/// かの 2 通りである．既に自分の色なら，その画素を**隣の色へ渡す** (ランが短くなる
/// 側の手当て) ．
fn candidates(canvas: &IndexedCanvas, jaggy: &Jaggy, index: u8) -> Vec<(IVec2, u8)> {
    let (Some(&first), Some(&last)) = (jaggy.pixels.first(), jaggy.pixels.last()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for at in [last + jaggy.major, first - jaggy.major] {
        let Some(current) = canvas.get_at(at) else {
            continue;
        };
        if current == index {
            // 自分の色なら «渡す» 側．渡す先は 4 近傍で最も多い別の色 (同数は小さい添字)
            if let Some(to) = donor(canvas, at, index) {
                out.push((at, to));
            }
        } else {
            out.push((at, index));
        }
    }
    out
}

/// 画素を渡す相手の添字 — 4 近傍で最も多い «自分以外» の色．
///
/// 同数のときは小さい添字を採る (決定論性の規則 2) ．
fn donor(canvas: &IndexedCanvas, at: IVec2, index: u8) -> Option<u8> {
    let mut counts: std::collections::BTreeMap<u8, usize> = std::collections::BTreeMap::new();
    for d in [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)] {
        if let Some(i) = canvas.get_at(at + d)
            && i != index
        {
            *counts.entry(i).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(i, n)| (*n, std::cmp::Reverse(*i)))
        .map(|(i, _)| i)
}

/// 指定した添字のマスクだけでジャギーを数える．
fn count_jaggies(canvas: &IndexedCanvas, indices: &[u8], max_move: u32) -> usize {
    indices
        .iter()
        .filter(|i| canvas.transparent() != Some(**i))
        .map(|&i| analyze_mask(&canvas.mask_of(i), max_move).jaggies.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::jaggy::analyze_canvas;
    use crate::geom::regions::label_mask;
    use crate::geom::runs::run_lengths;
    use crate::geom::{split_monotone, trace_contours};

    /// 上から `widths[i]` 画素の幅で下がっていく階段．
    fn staircase(widths: &[i32]) -> Mask {
        let total: i32 = widths.iter().sum();
        let h = widths.len() as i32 + 2;
        let mut m = Mask::new(total as u32 + 2, h as u32 + 1);
        let mut x = 1i32;
        for (step, w) in widths.iter().enumerate() {
            for dx in 0..*w {
                for y in (step as i32 + 1)..=h {
                    m.set(ivec2(x + dx, y), true);
                }
            }
            x += w;
        }
        m
    }

    /// 上端の輪郭のラン長列 (最も長いチェーンを採る)．
    fn top_runs(mask: &Mask) -> Vec<u32> {
        trace_contours(mask)
            .iter()
            .flat_map(split_monotone)
            .map(|c| run_lengths(&c))
            .max_by_key(|r| r.iter().sum::<u32>())
            .unwrap_or_default()
    }

    fn jaggies(mask: &Mask) -> usize {
        analyze_mask(mask, DEFAULT_MAX_MOVE).jaggies.len()
    }

    /// **谷が消える．** 直した結果が «直っている» ことを見る (直したと言うだけでなく)．
    #[test]
    fn a_lone_short_step_is_lengthened_until_the_valley_is_gone() {
        let mut m = staircase(&[3, 3, 2, 3]);
        assert_eq!(jaggies(&m), 1, "元の絵に谷が無い");
        let report = smooth_mask(&mut m, &SmoothOptions::default());
        assert_eq!(report.moved, 1, "動かした画素が 1 でない: {report:?}");
        assert_eq!(report.remaining, 0, "谷が残っている: {:?}", top_runs(&m));
    }

    /// **新しい谷を作らない．** 借りる側を間違えると `[3, 2, 3, 3]` になり谷が残る．
    #[test]
    fn smoothing_does_not_open_a_new_valley_next_door() {
        let mut m = staircase(&[3, 3, 2, 3]);
        smooth_mask(&mut m, &SmoothOptions::default());
        let runs = top_runs(&m);
        assert!(
            !runs.windows(3).any(|w| w[0] > w[1] && w[1] < w[2]),
            "隣に谷ができた: {runs:?}"
        );
    }

    /// **理想の単谷形は 1 画素も動かない．** 直す相手ではない (D80)．
    #[test]
    fn an_ideal_single_valley_is_left_alone() {
        let mut m = staircase(&[4, 3, 2, 3, 4]);
        let before = m.clone();
        let report = smooth_mask(&mut m, &SmoothOptions::default());
        assert_eq!(report.moved, 0, "理想形を削った: {report:?}");
        assert_eq!(m, before);
    }

    /// **移動上限を超える谷は触らない** — 意図的なディテールの可能性がある．
    #[test]
    fn a_deep_valley_is_reported_but_not_touched() {
        let mut m = staircase(&[3, 3, 1, 3]);
        let before = m.clone();
        let report = smooth_mask(&mut m, &SmoothOptions::default());
        assert_eq!(report.moved, 0, "上限 1 を超える谷 (δ = 2) を動かした");
        assert_eq!(report.over_limit, 1, "報告に残っていない: {report:?}");
        assert_eq!(m, before);
    }

    /// 上限を上げれば直る (直せないのは上限のせいだと確かめる)．
    #[test]
    fn raising_the_limit_lets_the_deep_valley_be_fixed() {
        let mut m = staircase(&[3, 3, 1, 3]);
        let report = smooth_mask(
            &mut m,
            &SmoothOptions {
                max_move: 2,
                ..SmoothOptions::default()
            },
        );
        assert!(report.moved > 0, "上限を上げても動かない: {report:?}");
        assert_eq!(report.remaining, 0, "残った: {:?}", top_runs(&m));
    }

    /// **もう一度掛けても何も起きない** (冪等)．
    #[test]
    fn smoothing_twice_changes_nothing_the_second_time() {
        let mut m = staircase(&[3, 3, 2, 3]);
        smooth_mask(&mut m, &SmoothOptions::default());
        let once = m.clone();
        let again = smooth_mask(&mut m, &SmoothOptions::default());
        assert_eq!(again.moved, 0, "2 回目で動いた: {again:?}");
        assert_eq!(m, once);
    }

    /// **形が割れない．** 1 画素動かすたびに連結成分が増えていないか見る．
    #[test]
    fn smoothing_does_not_split_the_shape() {
        let mut m = staircase(&[3, 3, 2, 3, 3, 2, 3]);
        let before = label_mask(&m, true).components().len();
        smooth_mask(&mut m, &SmoothOptions::default());
        assert_eq!(
            label_mask(&m, true).components().len(),
            before,
            "形が割れた"
        );
    }

    /// **面積は動かした画素の数までしか変わらない．**
    #[test]
    fn the_area_changes_by_at_most_one_pixel_per_move() {
        let mut m = staircase(&[3, 3, 2, 3, 3, 2, 3]);
        let before = m.count() as i64;
        let report = smooth_mask(&mut m, &SmoothOptions::default());
        let delta = (m.count() as i64 - before).unsigned_abs() as usize;
        assert!(
            delta <= report.moved,
            "面積が {delta} 変わったのに動かしたのは {} 画素",
            report.moved
        );
    }

    /// **色境界も直す** (D33)．シルエットは真四角のままである．
    #[test]
    fn a_colour_boundary_inside_a_square_is_smoothed_too() {
        let mut c = IndexedCanvas::filled(14, 9, 1);
        let widths = [3i32, 3, 2, 3];
        let mut x = 1i32;
        for (step, w) in widths.iter().enumerate() {
            for dx in 0..*w {
                for y in (step as i32 + 1)..9 {
                    c.set(x + dx, y, 2);
                }
            }
            x += w;
        }
        let before = analyze_canvas(&c, DEFAULT_MAX_MOVE).jaggies.len();
        assert!(before > 0, "色境界に谷が無い");
        let report = smooth_canvas(&mut c, &SmoothOptions::default());
        assert!(report.moved > 0, "色境界を直していない: {report:?}");
        assert!(
            report.remaining < before,
            "減っていない ({before} → {})",
            report.remaining
        );
        // シルエット (透明でない画素) は変わらない — 中の境界だけが動く
        assert_eq!(c.width() * c.height(), 14 * 9);
    }

    /// きれいな階段は 1 画素も動かさない．
    #[test]
    fn an_even_staircase_is_left_alone() {
        let mut m = staircase(&[3, 3, 3, 3]);
        let before = m.clone();
        let report = smooth_mask(&mut m, &SmoothOptions::default());
        assert_eq!(report.moved, 0);
        assert_eq!(m, before);
    }

    /// 円も動かさない (滑らかな曲線を «直す» のは誤りである)．
    #[test]
    fn a_circle_is_left_alone() {
        let mut m = Mask::new(21, 21);
        for p in m.bounds().iter() {
            let (dx, dy) = (p.x as f32 - 10.0, p.y as f32 - 10.0);
            if (dx * dx + dy * dy).sqrt() <= 8.0 {
                m.set(p, true);
            }
        }
        let before = m.clone();
        let report = smooth_mask(&mut m, &SmoothOptions::default());
        assert_eq!(report.moved, 0, "円を削った: {report:?}");
        assert_eq!(m, before);
    }

    /// **壊れると: `px smooth` が正しく描いた直線を書き換える** (R22 ・D169)．
    ///
    /// 傾き 2/5 の理想の階段は走りが `[3, 2, 3, 2, ...]` になり，2 のところが
    /// 定義上すべて谷である．**幾何が決めた刻みなので 1 画素も動かしてはいけない．**
    /// D163 の実測では，これを動かして清書 58 枚のうち 17 枚を壊していた．
    #[test]
    fn an_ideal_slope_is_left_alone() {
        // 3 と 2 が交互に並ぶ = 傾き 2/5 の digitization
        let widths: Vec<i32> = (0..12).map(|i| if i % 2 == 0 { 3 } else { 2 }).collect();
        let mut mask = staircase(&widths);
        let before = mask.clone();
        let report = smooth_mask(&mut mask, &SmoothOptions::default());

        assert_eq!(report.moved, 0, "理想の傾きを書き換えている");
        assert!(report.geometric > 0, "幾何として数えていない: {report:?}");
        for p in before.bounds().iter() {
            assert_eq!(before.get(p), mask.get(p), "画素 {p:?} が変わった");
        }
    }

    /// **壊れると: 幾何の例外が広すぎて，本物のジャギーまで直さなくなる．**
    ///
    /// `[3, 3, 2, 3]` は D80 が理想形と対比させたジャギーである．走りは 2 種類で
    /// 1 しか違わないが，**短い方が 1 度しか出てこないので «並び» を確かめようが
    /// ない** — 直線としては説明できない．**今までどおり直さなければならない．**
    ///
    /// > [!warning] **負例を 2 度続けて外した** (この計画で 5 度目 ・6 度目)．
    /// > `[3, 3, 1, 3, 3]` は深さ 2 で**移動上限を超える**ので元から «報告のみ»，
    /// > `[3, 3, 2, 3, 3]` は**どちらへ借りても谷が残る**ので元から動かせない．
    /// > どちらも `geometric: 0` で **例外は 1 度も働いていなかった** —
    /// > 道具ではなく試験が誤っていた．**«直る形» であることを先に確かめること．**
    #[test]
    fn a_lone_dip_is_not_excused_as_geometry() {
        let mut mask = staircase(&[3, 3, 2, 3]);
        assert_eq!(jaggies(&mask), 1, "元の絵に谷が無い");
        let report = smooth_mask(&mut mask, &SmoothOptions::default());
        assert_eq!(report.moved, 1, "本物のジャギーを直していない: {report:?}");
        assert_eq!(report.geometric, 0, "幾何として除いている: {report:?}");
        assert_eq!(report.remaining, 0, "谷が残っている: {:?}", top_runs(&mask));
    }
}
