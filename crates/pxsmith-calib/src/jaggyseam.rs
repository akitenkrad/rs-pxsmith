//! **円板に残った谷を «継ぎ目» として許したら何が起きるかの上限を測る** (D169 の続き)．
//!
//! # なぜ実装より先に測るか
//!
//! D169 は «一定の傾きの直線として説明できる区間» を `pxsmith smooth` の対象から外し，
//! 清書の書き換えを 88 画素 ・17 枚から **60 画素 ・13 枚**へ減らした．
//! 残った 60 画素は**全部が円板**で，しかも散発ではない — 鳴るのは 90 度回転で
//! 移り合う 4 点で，形は 64 件中 40 件が `[2, 1, 2]` である (引き継ぎの «すべて»
//! は大きい円板だけを見た読みだった — 下の試験で数え直した) ．これは
//! **傾きの違う 2 つの直線区間の継ぎ目**に見える．
//!
//! だが**直線のときほど安全ではない**．直線の例外は健全だった —
//! 区間まるごとが digital straight なら，定義上そこに直すべきものは無い．
//! 継ぎ目はそうではなく，**«2 つに切れば両側とも直線» は本物のジャギーでも
//! 成り立ちうる**．D168 の測定でも，谷の局所の形 (深さ ・両隣の差 ・区間内の位置)
//! では偽と真がまったく分かれなかった．
//!
//! **だから認識器を書く前に上限を測る** — «継ぎ目に見える谷を全部許したら»
//! 清書がどれだけ守れて，負例をどれだけ落とすか．
//! **採れない結論なら実装せずに済む** (D92 «書いていないものを黙らない» の前段)．
//!
//! # 上限の測り方
//!
//! 谷 $i$ を挟んで**左右に窓を伸ばし，両方が digital straight になる取り方が
//! 1 つでもあれば «継ぎ目» とみなす**．窓の取り方は全部試すので，
//! これは «継ぎ目を見分ける認識器» が達しうる**最も緩い側**である —
//! ここで負例が落ちるなら，どんな認識器を書いても落ちる．
//!
//! 効くつまみは 2 つしかない．
//!
//! - **谷そのものをどちらの区間に入れるか** ([`Share`])．継ぎ目の谷は
//!   どちらの直線に属するとも言えるので 4 通り全部測る．
//! - **窓を何本以上要求するか** (`min_runs`)．短い窓は何でも直線に見える．
//!
//! # 何を «捕捉» と数えるか — 検出ではなく **直せたか** である
//!
//! D169 の例外は**検出を変えない** (lint ルール 8 は今までどおり鳴る) ．
//! だから «負例の捕捉 53 / 69» は定義上びくともしない — **その数字で
//! 継ぎ目の例外を評価すると，何を許しても «影響なし» と読める**．
//! ここでは**`pxsmith smooth` が崩した場所の画素を実際に動かしたか**を数える
//! (D148 «その指標が何を測っているか確かめる») ．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::geom::distance::{curvature_field, signed_distance};
use pxsmith_core::geom::jaggy::turn_runs;
use pxsmith_core::geom::runs::{
    is_digital_straight, is_digital_straight_span, jaggy_valleys, run_lengths, run_pixels,
};
use pxsmith_core::geom::{split_monotone, trace_contours};
use pxsmith_core::smooth::{SmoothOptions, smooth_canvas};
use rayon::prelude::*;

use crate::jaggytruth::{clean_scenes, defect_bases, from_heights, shift_one_step};
use crate::lintcal::index_exactly;

/// **谷そのものをどちらの直線区間に入れるか．**
///
/// 継ぎ目の谷は «左の直線の最後の走り» とも «右の直線の最初の走り» とも読める．
/// どう配るかで判定の緩さが変わるので**決め打ちにせず 4 通り測る**．
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Share {
    /// 両方に入れる (継ぎ目は 2 つの区間が重なる点だと読む) ．**最も厳しい**
    Both,
    /// 左だけに入れる
    Left,
    /// 右だけに入れる
    Right,
    /// どちらにも入れない (谷は継ぎ目そのもので，どちらの直線にも属さない) ．**最も緩い**
    Neither,
}

impl Share {
    pub fn label(self) -> &'static str {
        match self {
            Share::Both => "両方に入れる",
            Share::Left => "左に入れる",
            Share::Right => "右に入れる",
            Share::Neither => "どちらにも入れない",
        }
    }
}

pub const SHARES: [Share; 4] = [Share::Both, Share::Left, Share::Right, Share::Neither];
/// 窓に要求する走りの本数 (端を落とした後) ．
pub const MIN_RUNS: [usize; 4] = [2, 3, 4, 5];

/// **窓が «一定の傾きの直線» として説明できるか．**
///
/// 端を落とすのは**単調区間の端に接しているときだけ**である — 区間の途中で
/// 切った側の走りは切り取られていないので，落とすと判定が甘くなる．
fn straight_window(runs: &[u32], a: usize, b: usize, min_runs: usize) -> bool {
    let trim_first = a == 0;
    let trim_last = b == runs.len();
    let effective = (b - a).saturating_sub(usize::from(trim_first) + usize::from(trim_last));
    effective >= min_runs && is_digital_straight_span(&runs[a..b], trim_first, trim_last)
}

/// **谷 $i$ が «傾きの違う 2 つの直線区間の継ぎ目» に見えるか** (上限側の判定)．
///
/// 窓の取り方をすべて試し，1 つでも両側が直線になれば真とする．
pub fn is_seam(runs: &[u32], i: usize, share: Share, min_runs: usize) -> bool {
    let n = runs.len();
    let (left_end, right_start) = match share {
        Share::Both => (i + 1, i),
        Share::Left => (i + 1, i + 1),
        Share::Right => (i, i),
        Share::Neither => (i, i + 1),
    };
    if left_end > n || right_start > n {
        return false;
    }
    let left = (0..left_end).any(|a| straight_window(runs, a, left_end, min_runs));
    if !left {
        return false;
    }
    (right_start + 1..=n).any(|b| straight_window(runs, right_start, b, min_runs))
}

/// 谷 1 つぶんの記録．
#[derive(Clone, Debug)]
pub struct Valley {
    /// 乗っている単調区間の走り列．
    pub runs: Vec<u32>,
    /// 走り列の中の位置．
    pub index: usize,
    /// この谷の画素の $x$ 座標 (崩した列の近くかを見るのに使う)．
    pub xs: Vec<i32>,
    /// 移動上限に収まるか — 収まらない谷は `smooth` がもともと触らない．
    pub within_limit: bool,
    /// 区間まるごとが直線か (D169 の例外．真なら `smooth` はもう触らない)．
    pub straight_chain: bool,
}

impl Valley {
    /// **いま `pxsmith smooth` が画素を動かす谷か．**
    pub fn movable(&self) -> bool {
        self.within_limit && !self.straight_chain
    }
    pub fn seam(&self, share: Share, min_runs: usize) -> bool {
        is_seam(&self.runs, self.index, share, min_runs)
    }
}

/// 画布の谷をすべて拾う．
///
/// **検出器と同じ関数を同じ順で呼ぶ** — 谷の集合の決め方が 2 か所にあっては
/// いけない (D110) ．`analyze_canvas` が返す [`pxsmith_core::geom::Jaggy`] には
/// 走り列の添字が入っていないので，ここだけ同じ経路をもう一度回している．
pub fn valleys_of(canvas: &IndexedCanvas, max_move: u32) -> Vec<Valley> {
    let mut out = Vec::new();
    let mut indices: Vec<u8> = canvas.pixels().to_vec();
    indices.sort_unstable();
    indices.dedup();
    for index in indices {
        if canvas.transparent() == Some(index) {
            continue;
        }
        let mask = canvas.mask_of(index);
        let k = curvature_field(&signed_distance(&mask));
        for contour in trace_contours(&mask) {
            for chain in split_monotone(&contour) {
                let runs = run_lengths(&chain);
                if runs.len() < 3 {
                    continue;
                }
                let groups = run_pixels(&chain);
                let turns = turn_runs(&chain, &k);
                let straight_chain = is_digital_straight(&runs);
                for i in jaggy_valleys(&runs, &turns) {
                    let target = runs[i - 1].min(runs[i + 1]);
                    let delta = target as i32 - runs[i] as i32;
                    out.push(Valley {
                        runs: runs.clone(),
                        index: i,
                        xs: groups
                            .get(i)
                            .map(|g| g.iter().map(|p| p.x).collect())
                            .unwrap_or_default(),
                        within_limit: delta.unsigned_abs() <= max_move,
                        straight_chain,
                    });
                }
            }
        }
    }
    out
}

/// 清書 1 枚の結果．
#[derive(Clone, Debug)]
pub struct CleanCase {
    pub name: String,
    pub kind: &'static str,
    /// いま `smooth` が動かす谷の数．
    pub movable: usize,
    /// `smooth` が実際に動かした画素 (谷の数とは一致しない — 巡回するため)．
    pub moved: usize,
    /// (配り方, 本数) ごとに «継ぎ目» と読める谷の数．
    pub seam: BTreeMap<(Share, usize), usize>,
}

/// 負例 1 件の結果．
#[derive(Clone, Debug)]
pub struct DefectCase {
    pub name: String,
    /// 谷ができたか (捕捉率の正しい分母．D163)．
    pub has_valley: bool,
    /// **`smooth` が崩した場所の画素を実際に動かしたか** — これが «直せた» である．
    pub repaired: bool,
    /// 崩した場所にあって，いま `smooth` が動かす谷の数．
    pub movable_near: usize,
    /// 崩した場所で動かした画素．
    pub moved_here: usize,
    /// **崩していない場所で動かした画素** — 巻き添えの量．
    pub moved_elsewhere: usize,
    /// **D169 の例外を外したときに動かせる谷の数** — 直線の例外が直す力を
    /// 削っていないかを見る («鳴らなかった» と «検査していない» を分ける．D104)．
    pub near_before_d169: usize,
    /// (配り方, 本数) ごとに «動かせる谷が 1 つも残らないか» = **直せなくなる**．
    ///
    /// **谷が 1 つも無い件は数えない** — «全部が継ぎ目» は空集合でも真になるので，
    /// そのまま数えると**例外を何も入れていなくても «直せなくなる» が立つ**
    /// (この計画で何度も踏んでいる «負例が欠陥になっているか» の裏返しである)．
    pub lost: BTreeMap<(Share, usize), bool>,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub clean: Vec<CleanCase>,
    pub defects: Vec<DefectCase>,
    /// 実素材の谷 (ファイル名, 谷) ．
    pub real: Vec<(String, Valley)>,
    pub real_files: usize,
}

impl Summary {
    pub fn clean_movable(&self) -> usize {
        self.clean.iter().map(|c| c.movable).sum()
    }
    pub fn clean_moved(&self) -> usize {
        self.clean.iter().map(|c| c.moved).sum()
    }
    pub fn clean_sheets_damaged(&self) -> usize {
        self.clean.iter().filter(|c| c.moved > 0).count()
    }
    pub fn clean_seam(&self, share: Share, min_runs: usize) -> usize {
        self.clean
            .iter()
            .filter_map(|c| c.seam.get(&(share, min_runs)))
            .sum()
    }
    /// 継ぎ目を許した後も `smooth` が動かす谷が残る絵の数．
    pub fn clean_sheets_left(&self, share: Share, min_runs: usize) -> usize {
        self.clean
            .iter()
            .filter(|c| c.movable > c.seam.get(&(share, min_runs)).copied().unwrap_or(0))
            .count()
    }
    pub fn defects_with_valley(&self) -> usize {
        self.defects.iter().filter(|d| d.has_valley).count()
    }
    pub fn repaired(&self) -> usize {
        self.defects
            .iter()
            .filter(|d| d.has_valley && d.repaired)
            .count()
    }
    /// **直したのに，崩した場所に «動かす谷» が 1 つも無い件** — 谷の数え上げが
    /// `smooth` の挙動を説明できていない件数である．黙って 0 と読まないために数える．
    pub fn repaired_unexplained(&self) -> usize {
        self.defects
            .iter()
            .filter(|d| d.has_valley && d.repaired && d.movable_near == 0)
            .count()
    }
    /// **動かす谷があるのに直らなかった件** — 当てる候補が無い側 (`no_candidate`)．
    pub fn movable_but_unrepaired(&self) -> usize {
        self.defects
            .iter()
            .filter(|d| d.has_valley && !d.repaired && d.movable_near > 0)
            .count()
    }
    /// **D169 の直線の例外で動かせなくなった負例** — 例外が直す力を削った分．
    pub fn silenced_by_d169(&self) -> usize {
        self.defects
            .iter()
            .filter(|d| d.has_valley && d.near_before_d169 > 0 && d.movable_near == 0)
            .count()
    }
    /// 崩した場所で動かした画素の合計．
    pub fn moved_here(&self) -> usize {
        self.defects
            .iter()
            .filter(|d| d.has_valley)
            .map(|d| d.moved_here)
            .sum()
    }
    /// **崩していない場所で動かした画素の合計** — 1 か所の崩れが巻き添えにした量．
    pub fn moved_elsewhere(&self) -> usize {
        self.defects
            .iter()
            .filter(|d| d.has_valley)
            .map(|d| d.moved_elsewhere)
            .sum()
    }
    pub fn with_valley_near(&self) -> usize {
        self.defects
            .iter()
            .filter(|d| d.has_valley && d.near_before_d169 > 0)
            .count()
    }
    pub fn lost(&self, share: Share, min_runs: usize) -> usize {
        self.defects
            .iter()
            .filter(|d| {
                d.has_valley
                    && d.repaired
                    && d.lost.get(&(share, min_runs)).copied().unwrap_or(false)
            })
            .count()
    }
    pub fn real_movable(&self) -> usize {
        self.real.iter().filter(|(_, v)| v.movable()).count()
    }
    pub fn real_seam(&self, share: Share, min_runs: usize) -> usize {
        self.real
            .iter()
            .filter(|(_, v)| v.movable() && v.seam(share, min_runs))
            .count()
    }
    /// 直せなくなる負例の名前 (**1 件ずつ人が確かめられるように出す**)．
    pub fn lost_names(&self, share: Share, min_runs: usize) -> Vec<&str> {
        self.defects
            .iter()
            .filter(|d| {
                d.has_valley
                    && d.repaired
                    && d.lost.get(&(share, min_runs)).copied().unwrap_or(false)
            })
            .map(|d| d.name.as_str())
            .collect()
    }
    /// 挙動を説明できていない負例の名前．
    pub fn unexplained_names(&self) -> Vec<&str> {
        self.defects
            .iter()
            .filter(|d| d.has_valley && d.repaired && d.movable_near == 0)
            .map(|d| d.name.as_str())
            .collect()
    }
    /// 継ぎ目を許しても `smooth` が動かす谷が残る清書の名前．
    pub fn left_names(&self, share: Share, min_runs: usize) -> Vec<&str> {
        self.clean
            .iter()
            .filter(|c| c.movable > c.seam.get(&(share, min_runs)).copied().unwrap_or(0))
            .map(|c| c.name.as_str())
            .collect()
    }
}

fn smooth_opts(max_move: u32) -> SmoothOptions {
    SmoothOptions {
        max_move,
        ..SmoothOptions::default()
    }
}

/// 崩した列の近くか (`jaggytruth::caught_near` と同じ幅)．
const NEAR: i32 = 2;

fn near(xs: &[i32], x: usize) -> bool {
    xs.iter().any(|q| (q - x as i32).abs() <= NEAR)
}

/// **`smooth` を掛けて «崩した列の画素» と «それ以外の画素» を数える．**
///
/// 直せたかだけでなく**巻き添えも数える** — D169 の例外は単調区間まるごとに
/// 掛かるので，**1 か所崩れると同じ区間の «正しく描けている部分» まで
/// 書き換えの対象に戻る**．直った件数だけ見ているとこれが見えない．
fn smooth_near(canvas: &IndexedCanvas, x: usize, max_move: u32) -> (usize, usize) {
    let mut after = canvas.clone();
    smooth_canvas(&mut after, &smooth_opts(max_move));
    let w = canvas.width() as i32;
    let (mut here, mut elsewhere) = (0usize, 0usize);
    for (k, (a, b)) in canvas.pixels().iter().zip(after.pixels()).enumerate() {
        if a != b {
            if ((k as i32 % w) - x as i32).abs() <= NEAR {
                here += 1;
            } else {
                elsewhere += 1;
            }
        }
    }
    (here, elsewhere)
}

pub fn run(max_move: u32, real_dir: Option<&Path>) -> Result<Summary> {
    let clean: Vec<CleanCase> = clean_scenes()
        .par_iter()
        .map(|(name, kind, canvas)| {
            let valleys = valleys_of(canvas, max_move);
            let mut smoothed = canvas.clone();
            let moved = smooth_canvas(&mut smoothed, &smooth_opts(max_move)).moved;
            let mut seam = BTreeMap::new();
            for share in SHARES {
                for min_runs in MIN_RUNS {
                    let n = valleys
                        .iter()
                        .filter(|v| v.movable() && v.seam(share, min_runs))
                        .count();
                    seam.insert((share, min_runs), n);
                }
            }
            CleanCase {
                name: name.clone(),
                kind,
                movable: valleys.iter().filter(|v| v.movable()).count(),
                moved,
                seam,
            }
        })
        .collect();

    let defects: Vec<DefectCase> = defect_bases()
        .par_iter()
        .flat_map(|(name, heights, h)| {
            (0..3)
                .filter_map(|k| {
                    let (broken, x) = shift_one_step(heights, k * 5)?;
                    let canvas = from_heights(&broken, *h);
                    let valleys = valleys_of(&canvas, max_move);
                    let has_valley = !valleys.is_empty();
                    let nearby: Vec<&Valley> = valleys
                        .iter()
                        .filter(|v| v.movable() && near(&v.xs, x))
                        .collect();
                    let mut lost = BTreeMap::new();
                    for share in SHARES {
                        for min_runs in MIN_RUNS {
                            // **崩した場所で動かせる谷が 1 つも残らないなら直せなくなる**．
                            // 空集合を «全部が継ぎ目» と読まないよう先に弾く
                            lost.insert(
                                (share, min_runs),
                                !nearby.is_empty()
                                    && nearby.iter().all(|v| v.seam(share, min_runs)),
                            );
                        }
                    }
                    let (here, elsewhere) = smooth_near(&canvas, x, max_move);
                    Some(DefectCase {
                        name: format!("{name}#{k}"),
                        has_valley,
                        repaired: here > 0,
                        moved_here: here,
                        moved_elsewhere: elsewhere,
                        movable_near: nearby.len(),
                        near_before_d169: valleys
                            .iter()
                            .filter(|v| v.within_limit && near(&v.xs, x))
                            .count(),
                        lost,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();

    let mut summary = Summary {
        clean,
        defects,
        ..Summary::default()
    };

    if let Some(dir) = real_dir {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .with_context(|| format!("{} を読めない", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
            .collect();
        files.sort();
        let got: Vec<(String, Vec<Valley>)> = files
            .par_iter()
            .filter_map(|p| {
                let img = pxsmith_io::png::read_rgba(p).ok()?;
                let (canvas, _) = index_exactly(&img).ok()?;
                let name = p.file_name()?.to_string_lossy().to_string();
                Some((name, valleys_of(&canvas, max_move)))
            })
            .collect();
        summary.real_files = got.len();
        for (name, vs) in got {
            for v in vs {
                summary.real.push((name.clone(), v));
            }
        }
    }

    Ok(summary)
}

/// 清書を種類ごとにまとめる (枚数 ・動かす谷 ・動かした画素)．
pub fn by_kind(summary: &Summary) -> BTreeMap<&'static str, (usize, usize, usize)> {
    let mut m: BTreeMap<&'static str, (usize, usize, usize)> = BTreeMap::new();
    for c in &summary.clean {
        let e = m.entry(c.kind).or_default();
        e.0 += 1;
        e.1 += c.movable;
        e.2 += c.moved;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **円板に残った谷の形を数える — 引き継ぎの «すべて `[2, 1, 2]`» は外れていた．**
    ///
    /// 大きい円板だけを見ると `[2, 1, 2]` に揃って見えるが，全 38 枚で数えると
    /// **64 件中 40 件 (62.5%)** でしかない．半径が小さいほど平らな頭が近いので
    /// 谷の左隣が伸び，`(3, 1, 2)` ・`(5, 1, 2)` になる．`(3, 2, 3)` は
    /// 深さ 1 の別の形である．
    ///
    /// 動かす谷があるのは 38 枚中 **14 枚**で，`smooth` が実際に画素を動かすのは
    /// **13 枚**である (1 枚は当てる候補がどれもジャギーを減らさない) ．
    /// **枚数も数える** — 0 枚のまま «全部 `[2, 1, 2]` だった» と読まないため
    /// (D104 の作法)．
    ///
    /// 壊れると: 測っている対象が引き継ぎに書いた形と違うことに気付けない．
    #[test]
    fn the_disk_valleys_are_not_all_the_same_shape() {
        let mut sheets = 0usize;
        let mut shapes: BTreeMap<(u32, u32, u32), usize> = BTreeMap::new();
        for radius in 3u32..=40 {
            let vs: Vec<Valley> = valleys_of(&crate::jaggytruth::disk(radius), 1)
                .into_iter()
                .filter(|v| v.movable())
                .collect();
            if vs.is_empty() {
                continue;
            }
            sheets += 1;
            for v in &vs {
                let (i, r) = (v.index, &v.runs);
                *shapes.entry((r[i - 1], r[i], r[i + 1])).or_default() += 1;
            }
        }
        assert_eq!(sheets, 14, "動かす谷がある円板の枚数が変わった");
        let total: usize = shapes.values().sum();
        assert_eq!(total, 64, "動かす谷の数が変わった");
        // **90 度回転で移り合う 4 点**なので，どの形も 4 の倍数で現れる
        assert!(
            shapes.values().all(|n| n % 4 == 0),
            "4 の倍数でない形がある: {shapes:?}"
        );
        assert_eq!(
            shapes,
            BTreeMap::from([
                ((2, 1, 2), 40),
                ((3, 1, 2), 12),
                ((3, 2, 3), 8),
                ((5, 1, 2), 4)
            ]),
            "谷の形の分布が変わった"
        );
    }

    /// **窓は端を落とすかどうかで判定が変わる** — 切り口の側を落としてはいけない．
    ///
    /// 壊れると: 見なかったことにした走りの分だけ «直線» が甘くなり，
    /// 上限が実際よりも良く見える．
    #[test]
    fn trimming_the_cut_side_would_make_a_broken_staircase_look_straight() {
        // 段を 1 つ手前へ動かした階段の «右側» — 4 が切り口に立っている
        let right = [4u32, 3, 3, 3, 3, 3];
        assert!(
            !is_digital_straight_span(&right, false, true),
            "切り口の走りを見ているのに直線と言った"
        );
        assert!(
            is_digital_straight_span(&right, true, true),
            "切り口を落とせば直線に見えるはず — この試験の前提が消えた"
        );
    }

    /// 谷の周りの走りを 5 本切り出す (前 2 本 ・谷 ・後ろ 2 本)．
    fn around(v: &Valley) -> Vec<u32> {
        v.runs[v.index - 2..v.index + 3].to_vec()
    }

    /// **継ぎ目の規則は «正しい円板の縁» と «段を 1 つ崩した円板の縁» を分けられない．**
    ///
    /// これがこの測定の結論である — 落とすかどうかの差ではなく，
    /// **2 つの場面が同じ走り列を作る**ので，どんな窓の取り方をしても同じ答えになる．
    ///
    /// 壊れると: «閾値を選び直せば分けられる» と読んでしまう．
    #[test]
    fn a_broken_disk_step_looks_exactly_like_a_correct_disk_edge() {
        // (a) 崩していない円板の縁に残る谷 (幾何が決めた刻み — 直してはいけない)
        let clean = valleys_of(&crate::jaggytruth::disk(35), 1);
        let a = clean
            .iter()
            .find(|v| v.movable())
            .expect("円板 r35 に動かす谷が無い");

        // (b) 円板の上端の段を 1 つ手前へ動かしたもの (本物の欠陥 — 直すべきもの)
        let h = crate::jaggytruth::disk_heights(20);
        let (broken, x) = shift_one_step(&h, 10).expect("崩せる");
        let canvas = from_heights(&broken, 20 * 2 + 5);
        let b = valleys_of(&canvas, 1)
            .into_iter()
            .find(|v| v.movable() && near(&v.xs, x))
            .expect("崩した場所に動かす谷が無い");

        assert_eq!(
            around(a),
            around(&b),
            "この試験の前提 (2 つの場面が同じ走り列を作る) が変わった"
        );
        assert!(
            a.seam(Share::Right, 2) && b.seam(Share::Right, 2),
            "継ぎ目の規則が 2 つを別々に扱った — 上限の測定をやり直すこと"
        );
    }

    /// **走り 1 本の窓は直線ではない** — 短い窓を許すと何でも継ぎ目になる．
    #[test]
    fn a_single_run_window_is_not_a_line() {
        assert!(!is_digital_straight_span(&[3], false, false));
        assert!(
            !straight_window(&[3, 3, 3], 0, 2, 1),
            "端を落として 1 本になる窓を通した"
        );
    }
}
