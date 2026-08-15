//! G3 — ラン長解析 (設計書 2.4 / 6.4)．
//!
//! `smooth` とジャギー lint (8)・バンディング lint (12) がこの上に乗る．
//!
//! # 単谷形と例外の 1 箇所
//!
//! 理想的なラン長列は端から中央へ単調非増加，中央から端へ単調非減少という
//! **単谷形**である．従って**谷そのものは理想形にも必ず存在する**ため，谷検出を
//! そのまま違反とすると理想形自身を違反と判定してしまう．
//!
//! 例外は**曲率符号が反転する点**の 1 箇所のみとし，それ以外の谷をジャギーとする
//! (D32)．反転点はラン長列だけからは決まらない — 単発の谷を「最小値だから反転点」と
//! みなすと，**最も普通のジャギーを毎回見逃す**ことになる．G2 の曲率場から求めるので
//! [`jaggy_valleys`] は反転点の集合を引数に取る (求め方は
//! [`crate::geom::jaggy::turn_runs`])．
//!
//! 補助規則 $\max r_i - \min r_i \le 1$ は採用しない．単谷形はラン長が段ごとに
//! 変化する形なので，この規則とは両立しない．

use std::collections::BTreeSet;

use crate::math::IVec2;

use super::contour::Chain;

/// チェーンのラン長列．
///
/// 主軸 (変位の大きい方の軸) に沿って進む画素をひとかたまりに数える．副軸の
/// 座標が変わるところが区切りになる．
pub fn run_lengths(chain: &Chain) -> Vec<u32> {
    let pts = chain.points();
    if pts.is_empty() {
        return Vec::new();
    }
    let horizontal = chain.is_horizontal();
    let minor = |p: &IVec2| if horizontal { p.y } else { p.x };

    let mut out = Vec::new();
    let mut current = minor(&pts[0]);
    let mut count = 0u32;
    for p in pts {
        if minor(p) == current {
            count += 1;
        } else {
            out.push(count);
            current = minor(p);
            count = 1;
        }
    }
    out.push(count);
    out
}

/// ラン長列と同じ区切りで，各ランに属する画素を返す．
///
/// `smooth` が「ラン $i$ の端の画素を隣に合わせる方向へ移動」するために要る．
pub fn run_pixels(chain: &Chain) -> Vec<Vec<IVec2>> {
    let pts = chain.points();
    if pts.is_empty() {
        return Vec::new();
    }
    let horizontal = chain.is_horizontal();
    let minor = |p: &IVec2| if horizontal { p.y } else { p.x };

    let mut out: Vec<Vec<IVec2>> = Vec::new();
    let mut current = minor(&pts[0]);
    let mut group = Vec::new();
    for p in pts {
        if minor(p) != current {
            out.push(std::mem::take(&mut group));
            current = minor(p);
        }
        group.push(*p);
    }
    out.push(group);
    out
}

/// 谷 — 両隣より短いラン．
///
/// 端のランは両隣が揃わないので谷にならない．
pub fn run_valleys(runs: &[u32]) -> Vec<usize> {
    (1..runs.len().saturating_sub(1))
        .filter(|&i| runs[i - 1] > runs[i] && runs[i] < runs[i + 1])
        .collect()
}

/// **理想の単谷形の底とみなす «続いた坂» の段数** (`sustained_valleys` の既定)．
///
/// **暫定値である．** 2 段にしたのは «平らな中の単発のくぼみ» (D60 が名指しした
/// `[3, 3, 1, 3]`) を除外しない最小の値だからで，上げ下げの影響は測っていない．
pub const SUSTAINED_SLOPE: usize = 2;

/// **理想の単谷形の底** — 両側に厳密な坂が `min_slope` 段以上続く谷．
///
/// 設計書 6.4 の理想形は «端から中央へ単調非増加，中央から端へ単調非減少» なので，
/// **その底は理想形にも必ず現れる**．`[4, 3, 2, 3, 4]` の底がこれである．
///
/// > [!warning] **D60 の «ラン長列の最小値» とは別物である．**
/// > D60 が禁じたのは «内部の最小値を反転点とみなす» ことで，それだと
/// > `[3, 3, 1, 3]` の谷が毎回除外され**最も普通のジャギーが丸ごと見逃される**．
/// > ここは «両側に坂が続くか» を見るので，平らな中の単発のくぼみは除外しない
/// > (`[3, 3, 1, 3]` は左が $3 = 3$ で坂が続かない) ．
/// >
/// > **曲率場からは取れないことを測って確かめた** (D80) ．単調区間の中では
/// > 距離場の曲率はどのランでも正で，理想形 `[4, 3, 2, 3, 4]` とジャギー
/// > `[3, 3, 2, 3]` の符号の並びが**同じ**である (均し 0 〜 5 回のいずれでも) ．
/// > 単調区間は既に向きで切ってあるので，その中で符号が反転する場面がほとんど無い．
pub fn sustained_valleys(runs: &[u32], min_slope: usize) -> BTreeSet<usize> {
    run_valleys(runs)
        .into_iter()
        .filter(|&i| {
            let mut left = 0usize;
            let mut j = i;
            while j > 0 && runs[j - 1] > runs[j] {
                left += 1;
                j -= 1;
            }
            let mut right = 0usize;
            let mut j = i;
            while j + 1 < runs.len() && runs[j + 1] > runs[j] {
                right += 1;
                j += 1;
            }
            left >= min_slope && right >= min_slope
        })
        .collect()
}

/// ジャギーとみなす谷 — 理想形にも現れる谷を除いたもの (D32 ・D80)．
///
/// `turns` は曲率符号が反転しているランの添字 ([`crate::geom::jaggy::turn_runs`]) ．
/// **それに加えて «続いた坂の底» を除く** — 実測では曲率がほとんど反転しないので，
/// これが無いと理想の単谷形そのものを違反と判定する．
pub fn jaggy_valleys(runs: &[u32], turns: &BTreeSet<usize>) -> Vec<usize> {
    let sustained = sustained_valleys(runs, SUSTAINED_SLOPE);
    run_valleys(runs)
        .into_iter()
        .filter(|i| !turns.contains(i) && !sustained.contains(i))
        .collect()
}

/// 単谷形か — 谷が高々 1 つ．
pub fn is_unimodal(runs: &[u32]) -> bool {
    run_valleys(runs).len() <= 1
}

/// **走りの列が «一定の傾きの直線» の digitization として説明できるか** (D169)．
///
/// 一定の傾き $a/b$ で引いた直線の走りは，長さが $\lfloor b/a \rfloor$ と
/// $\lceil b/a \rceil$ の 2 種類しか取らず，**どちらが現れるかの並びが
/// «できるだけ均等» になる** (Sturmian) ．この性質は再帰的で，
/// «珍しい方の間に普通の方が何個あるか» を数え直すと同じ条件になる．
/// 閾値は 1 つも無い — **数え上げだけで決まる**．
///
/// # 何に使うか — 検出ではなく **抑制** に使う
///
/// 設計書 6.4 (D32) は «理想 Bresenham 列との比較» を**検出器としては採らない**
/// と決めている (意図的に描かれた曲線をすべて違反と判定してしまうため) ．
/// **ここはその逆向きである** — 直線として説明できるときに «直さない» と言うだけで，
/// **曲線を違反にする方向には決して働かない**．だから D32 とは両立する．
///
/// # 端の走りは落とす
///
/// 単調区間の最初と最後の走りは**途中で切り取られている**ことがあるので，
/// 長さの条件から外す (切られた走りは短く出るだけで，傾きの証拠にならない) ．
///
/// # 繰り返していないものを «幾何» と呼ばない
///
/// 珍しい方の値が **1 度しか出てこない列は偽とする**．`[3, 3, 2, 3]` は
/// 数としては «2 種類が 1 違い» を満たすが，**2 が 1 度出るだけでは並びを
/// 確かめようがない** — これは D80 が «最も普通のジャギー» として挙げた形なので，
/// ここを真にすると `px smooth` が本物のジャギーを直さなくなる．
/// **模様は繰り返して初めて模様である．**
pub fn is_digital_straight(runs: &[u32]) -> bool {
    if runs.len() < 3 {
        // 谷が存在しえない長さ (`run_valleys` は 3 本以上を要求する)
        return true;
    }
    is_digital_straight_span(runs, true, true)
}

/// **区間の一部だけを見て同じ判定をする** — 端を落とすかどうかを呼ぶ側が決める．
///
/// [`is_digital_straight`] は単調区間まるごとを見るので両端を落とすが，
/// **区間の途中で切った窓では，切り口の側の走りは切り取られていない** —
/// そこを落とすと «見なかったことにした走り» の分だけ判定が甘くなる．
/// どちらを落とすかは場面で変わるので引数にした (判定の本体は 1 つである．D110)．
///
/// 落とした結果が 1 本以下なら **偽** を返す — 走り 1 本に傾きは無い．
/// ([`is_digital_straight`] が短い列を真とするのは «谷が存在しえない» からで，
/// 窓の側にその理屈は無い．)
pub fn is_digital_straight_span(runs: &[u32], trim_first: bool, trim_last: bool) -> bool {
    let from = usize::from(trim_first);
    let to = runs.len().saturating_sub(usize::from(trim_last));
    if from >= to {
        return false;
    }
    balanced(&runs[from..to], true)
}

/// [`is_digital_straight`] の本体 — 再帰する．
///
/// `top` は最上位の呼び出しか．**繰り返しの要求は最上位でだけ課す** —
/// 数え直した列は既に «珍しい方が 2 度以上出た» ことの帰結なので，
/// そこで再び 2 度以上を求めると長い直線まで落ちる．
fn balanced(v: &[u32], top: bool) -> bool {
    if v.len() <= 1 {
        return !top;
    }
    let lo = *v.iter().min().expect("空でない");
    let hi = *v.iter().max().expect("空でない");
    if hi == lo {
        return true;
    }
    if hi - lo > 1 {
        return false;
    }

    // **連続してよいのは一方だけ** — 両方が 2 個以上並ぶ列は直線にならない
    let longest = |val: u32| -> usize {
        let (mut best, mut cur) = (0usize, 0usize);
        for &x in v {
            if x == val {
                cur += 1;
                best = best.max(cur);
            } else {
                cur = 0;
            }
        }
        best
    };
    let (lo_max, hi_max) = (longest(lo), longest(hi));
    if lo_max >= 2 && hi_max >= 2 {
        return false;
    }

    // 孤立する方を «区切り» に取る (どちらも孤立するなら短い方)
    let rare = if lo_max == 1 { lo } else { hi };
    let occurrences = v.iter().filter(|&&x| x == rare).count();
    if top && occurrences < 2 {
        // 1 度きりの食い違いは «傾き» の証拠にならない
        return false;
    }

    // 区切りの間に «普通の方» が何個あるかを数え直す
    // (先頭と末尾の半端は落とす — 端の走りと同じ理由)
    let mut counts = Vec::new();
    let mut seen = false;
    let mut run = 0u32;
    for &x in v {
        if x == rare {
            if seen {
                counts.push(run);
            }
            seen = true;
            run = 0;
        } else {
            run += 1;
        }
    }
    if counts.is_empty() {
        return true;
    }
    balanced(&counts, false)
}

/// バンディング — 同じ長さのランが並走している箇所 (lint 12)．
///
/// `min_repeat` 個以上続いたところを返す．返り値は開始位置と長さ．
pub fn banding(runs: &[u32], min_repeat: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if runs.is_empty() || min_repeat == 0 {
        return out;
    }
    let mut start = 0usize;
    for i in 1..=runs.len() {
        if i == runs.len() || runs[i] != runs[start] {
            let len = i - start;
            if len >= min_repeat {
                out.push((start, len));
            }
            start = i;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::contour::{split_monotone, trace_contours};
    use crate::geom::mask::Mask;
    use crate::math::{IRect, ivec2};

    /// **理想の単谷形の底は違反にしない．** 谷は理想形にも必ず現れる (設計書 6.4)．
    #[test]
    fn the_bottom_of_an_ideal_single_valley_is_not_a_jaggy() {
        let runs = [4, 3, 2, 3, 4];
        assert_eq!(run_valleys(&runs), vec![2], "谷が取れていない");
        assert!(
            sustained_valleys(&runs, SUSTAINED_SLOPE).contains(&2),
            "理想の単谷形の底を除外していない"
        );
        assert!(
            jaggy_valleys(&runs, &BTreeSet::new()).is_empty(),
            "理想形そのものをジャギーと呼んでいる"
        );
    }

    /// **D60 の反例 — 単発のくぼみは «理想形» を名乗って逃げられない．**
    ///
    /// ラン長列の最小値を反転点にすると `[3, 3, 1, 3]` の谷が毎回除外され，
    /// **最も普通のジャギーが丸ごと見逃される**．除外の条件は «両側に坂が続くか» で
    /// あって «最小値か» ではない．
    #[test]
    fn a_lone_dip_in_a_flat_run_is_still_a_jaggy() {
        for runs in [
            vec![3u32, 3, 1, 3],
            vec![3, 3, 2, 3],
            vec![5, 5, 4, 5, 5],
            vec![2, 2, 1, 2],
        ] {
            assert!(
                sustained_valleys(&runs, SUSTAINED_SLOPE).is_empty(),
                "{runs:?} の単発のくぼみを理想形として除外した"
            );
            assert_eq!(
                jaggy_valleys(&runs, &BTreeSet::new()).len(),
                1,
                "{runs:?} のジャギーを見逃した"
            );
        }
    }

    /// **片側だけ坂が続く形は除外しない．** 理想形は両側に坂がある．
    #[test]
    fn a_valley_with_a_slope_on_only_one_side_is_still_a_jaggy() {
        let runs = [5, 4, 3, 2, 3];
        assert!(sustained_valleys(&runs, SUSTAINED_SLOPE).is_empty());
        assert_eq!(jaggy_valleys(&runs, &BTreeSet::new()), vec![3]);
    }

    /// 曲率の反転点は今までどおり除外する (2 つの規則は足し算である)．
    #[test]
    fn a_curvature_turn_is_still_excluded() {
        let runs = [3, 3, 1, 3];
        let turns: BTreeSet<usize> = [2usize].into_iter().collect();
        assert!(jaggy_valleys(&runs, &turns).is_empty());
    }

    /// テスト用に，与えたラン長列そのものを持つチェーンを作るのは手間なので，
    /// ラン長列を直接扱う関数はそのまま試す．
    #[test]
    fn valleys_need_both_sides_to_be_longer() {
        assert_eq!(run_valleys(&[3, 1, 3]), vec![1]);
        assert_eq!(run_valleys(&[3, 3, 3]), Vec::<usize>::new());
        assert_eq!(run_valleys(&[1, 2, 3]), Vec::<usize>::new());
        assert_eq!(run_valleys(&[3, 2, 1]), Vec::<usize>::new());
    }

    #[test]
    fn ends_are_never_valleys() {
        assert_eq!(run_valleys(&[1, 5, 5, 1]), Vec::<usize>::new());
    }

    #[test]
    fn a_valley_at_a_curvature_reversal_is_excused() {
        let runs = [4, 3, 2, 3, 4];
        assert_eq!(run_valleys(&runs), vec![2]);
        assert!(
            jaggy_valleys(&runs, &BTreeSet::from([2])).is_empty(),
            "反転点を違反と判定している"
        );
    }

    /// 反転点を「ラン長の最小値」で決めると，**単発のジャギーを毎回見逃す**．
    #[test]
    fn a_lone_valley_is_a_jaggy_when_the_curvature_does_not_reverse() {
        let runs = [3, 3, 1, 3];
        assert_eq!(run_valleys(&runs), vec![2]);
        assert_eq!(
            jaggy_valleys(&runs, &BTreeSet::new()),
            vec![2],
            "曲率が反転していない谷を見逃している"
        );
    }

    #[test]
    fn only_the_listed_turns_are_excused() {
        let runs = [4, 2, 4, 1, 4];
        assert_eq!(run_valleys(&runs), vec![1, 3]);
        assert_eq!(jaggy_valleys(&runs, &BTreeSet::from([1])), vec![3]);
        assert!(!is_unimodal(&runs));
    }

    #[test]
    fn a_straight_run_of_equal_lengths_is_unimodal() {
        assert!(is_unimodal(&[2, 2, 2, 2]));
    }

    #[test]
    fn run_lengths_of_a_horizontal_edge() {
        // 幅 8 の 1 行．上辺は 8 画素の 1 ラン
        let mut m = Mask::new(8, 3);
        for p in IRect::new(0, 1, 8, 1).iter() {
            m.set(p, true);
        }
        let contour = &trace_contours(&m)[0];
        let chains = split_monotone(contour);
        let horizontal = chains
            .iter()
            .find(|c| c.is_horizontal() && c.len() >= 5)
            .expect("水平なチェーンが無い");
        let runs = run_lengths(horizontal);
        assert_eq!(runs.len(), 1, "1 行なのでランは 1 つ: {runs:?}");
        assert!(runs[0] >= 5);
    }

    #[test]
    fn run_lengths_of_a_staircase() {
        // 2 画素ずつ下がる階段 — ラン長は 2 の並びになる
        let mut m = Mask::new(9, 6);
        for step in 0..4i32 {
            for dx in 0..2 {
                for y in (step + 1)..6 {
                    m.set(ivec2(step * 2 + dx, y), true);
                }
            }
        }
        let contour = &trace_contours(&m)[0];
        let runs: Vec<Vec<u32>> = split_monotone(contour)
            .iter()
            .map(run_lengths)
            .filter(|r| r.len() > 2)
            .collect();
        assert!(
            runs.iter()
                .any(|r| r.iter().filter(|&&v| v == 2).count() >= 2),
            "階段のラン長が 2 で揃っていない: {runs:?}"
        );
    }

    #[test]
    fn banding_finds_repeated_equal_runs() {
        assert_eq!(banding(&[2, 2, 2, 3, 4], 3), vec![(0, 3)]);
        assert_eq!(banding(&[2, 2, 3, 3], 3), Vec::<(usize, usize)>::new());
        assert_eq!(banding(&[1, 1, 1, 1], 2), vec![(0, 4)]);
    }

    #[test]
    fn banding_is_empty_for_an_empty_input() {
        assert!(banding(&[], 2).is_empty());
        assert!(banding(&[1, 2, 3], 0).is_empty());
    }

    #[test]
    fn run_pixels_agree_with_run_lengths() {
        let mut m = Mask::new(10, 10);
        for p in IRect::new(2, 2, 6, 5).iter() {
            m.set(p, true);
        }
        let contour = &trace_contours(&m)[0];
        for chain in split_monotone(contour) {
            let lengths = run_lengths(&chain);
            let groups = run_pixels(&chain);
            assert_eq!(groups.len(), lengths.len());
            for (g, l) in groups.iter().zip(&lengths) {
                assert_eq!(g.len() as u32, *l);
            }
        }
    }

    #[test]
    fn run_lengths_sum_to_the_chain_length() {
        let mut m = Mask::new(10, 10);
        for p in IRect::new(2, 2, 6, 5).iter() {
            m.set(p, true);
        }
        let contour = &trace_contours(&m)[0];
        for chain in split_monotone(contour) {
            let runs = run_lengths(&chain);
            assert_eq!(
                runs.iter().sum::<u32>() as usize,
                chain.len(),
                "ランの合計がチェーンの長さと合わない"
            );
        }
    }

    /// 傾き $a/b$ の理想の階段の走り列 (`jaggytruth` と同じ作り方)．
    fn staircase_runs(a: u32, b: u32, w: u32) -> Vec<u32> {
        let heights: Vec<u32> = (0..w).map(|x| (a * x) / b).collect();
        let mut runs = Vec::new();
        let mut cur = 1u32;
        for i in 1..heights.len() {
            if heights[i] == heights[i - 1] {
                cur += 1;
            } else {
                runs.push(cur);
                cur = 1;
            }
        }
        runs.push(cur);
        runs
    }

    /// **壊れると: 正しく描いた直線を «幾何ではない» と読み，`px smooth` が壊す．**
    #[test]
    fn every_ideal_staircase_reads_as_a_straight_line() {
        for (a, b) in [
            (1u32, 1u32),
            (1, 2),
            (1, 3),
            (1, 4),
            (1, 5),
            (2, 3),
            (2, 5),
            (2, 7),
            (3, 4),
            (3, 5),
            (3, 7),
            (3, 8),
            (4, 5),
            (4, 7),
            (4, 9),
            (4, 11),
            (5, 6),
            (5, 8),
            (5, 9),
            (5, 14),
        ] {
            let runs = staircase_runs(a, b, 96);
            assert!(
                is_digital_straight(&runs),
                "傾き {a}/{b} の理想の階段が直線と読めない: {runs:?}"
            );
        }
    }

    /// **壊れると: 本物のジャギーを «幾何» と読み，`px smooth` が直さなくなる．**
    ///
    /// `[3, 3, 1, 3]` は D60 が «最も普通のジャギー» として挙げた形，
    /// `[3, 3, 2, 3]` は D80 が理想形と対比させたジャギーである．
    /// **どちらも直線と読んではいけない．**
    #[test]
    fn a_single_dip_is_not_a_straight_line() {
        for runs in [
            vec![3, 3, 1, 3],
            vec![3, 3, 2, 3],
            vec![3, 3, 3, 2, 3, 3, 3],
            vec![4, 3, 2, 3, 4],
        ] {
            assert!(
                !is_digital_straight(&runs),
                "単発のくぼみを直線と読んでいる: {runs:?}"
            );
        }
    }

    /// **壊れると: 段をずらした階段 (設計書 6.4 の «揃っていない階段») を見逃す．**
    #[test]
    fn a_shifted_step_breaks_straightness() {
        let mut runs = staircase_runs(1, 3, 96);
        // 走りを 1 つ削り，隣へ足す = 段が 1 画素早く落ちる
        let n = runs.len() / 2;
        runs[n] -= 1;
        runs[n + 1] += 1;
        assert!(
            !is_digital_straight(&runs),
            "崩した階段を直線と読んでいる: {runs:?}"
        );
    }

    /// **壊れると: 端が切れているだけの直線を «幾何ではない» と読む．**
    #[test]
    fn truncated_end_runs_do_not_break_straightness() {
        let mut runs = staircase_runs(1, 3, 96);
        let last = runs.len() - 1;
        runs[0] = 1;
        runs[last] = 1;
        assert!(is_digital_straight(&runs), "端の欠けで落ちている: {runs:?}");
    }

    /// **落とした結果が空になる窓は偽である** — 走り 0 本 ・1 本に傾きは無い．
    ///
    /// [`is_digital_straight`] が短い列を**真**とするのとは向きが逆なので，
    /// 引き写すと «何も見ていない窓» が直線として通る．
    ///
    /// **壊れると: 窓を短く取るだけで何でも «直線» になり，抑制が青天井になる．**
    #[test]
    fn a_window_with_nothing_left_after_trimming_is_not_a_line() {
        assert!(!is_digital_straight_span(&[], false, false), "空の窓");
        assert!(
            !is_digital_straight_span(&[3], true, false),
            "落として 0 本"
        );
        assert!(
            !is_digital_straight_span(&[3, 4], true, true),
            "落として 0 本"
        );
        assert!(
            !is_digital_straight_span(&[3], false, false),
            "1 本だけの窓"
        );
        // 落とさなければ 2 本は読める (同じ長さが 2 本 = 傾き一定)
        assert!(is_digital_straight_span(&[3, 3], false, false), "2 本の窓");
    }
}
