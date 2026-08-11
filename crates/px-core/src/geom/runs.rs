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

/// ジャギーとみなす谷 — 曲率符号の反転点を除いたもの (D32)．
///
/// `turns` は反転しているランの添字．[`crate::geom::jaggy::turn_runs`] が
/// 距離場の曲率から求める．
pub fn jaggy_valleys(runs: &[u32], turns: &BTreeSet<usize>) -> Vec<usize> {
    run_valleys(runs)
        .into_iter()
        .filter(|i| !turns.contains(i))
        .collect()
}

/// 単谷形か — 谷が高々 1 つ．
pub fn is_unimodal(runs: &[u32]) -> bool {
    run_valleys(runs).len() <= 1
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
}
