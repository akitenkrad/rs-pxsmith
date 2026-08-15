//! **ジャギー検出を良い絵に掛けて何が鳴るかを数える．**
//!
//! `pxsmith smooth` は検出したところの画素を動かす．**動かす前に «良い絵で何件鳴るか» を
//! 測る** (D70 の作法) ．鳴りすぎるなら，`smooth` は良い絵を壊す道具になる．
//!
//! M1a の測定 (`docs/investigations/jaggy-false-positives.md`) は Aseprite の
//! テストスプライト (四角 ・点 ・文字) で 0.1% だったが，**手描きのドット絵では
//! ない**ので «意図的な凹凸» が入っていない．ここでは CC0 の実物 64 枚で測る．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pxsmith_core::geom::distance::{curvature_field, signed_distance};
use pxsmith_core::geom::jaggy::{DEFAULT_MAX_MOVE, analyze_canvas, turn_runs};
use pxsmith_core::geom::runs::{run_lengths, run_valleys};
use pxsmith_core::geom::{split_monotone, trace_contours};
use rayon::prelude::*;

use crate::lintcal::index_exactly;

/// 1 枚ぶんの測定．
#[derive(Clone, Debug)]
pub struct Record {
    pub file: String,
    pub runs: usize,
    pub chains: usize,
    pub jaggies: usize,
    /// 移動上限 (既定 1 画素) に収まるもの．`pxsmith smooth` が実際に動かす件数である．
    pub fixable: usize,
    /// 谷の «深さ» ごとの件数 ($\delta$ = 目標 − 長さ)．
    pub by_delta: BTreeMap<i32, usize>,
    /// 谷の周りの形ごとの件数．**理想の単谷形の底を «ジャギー» と呼んでいないか**を見る．
    pub by_shape: BTreeMap<&'static str, usize>,
    /// **1 本の単調区間に集まった谷の最大数**．
    ///
    /// «絵の中にジャギーが 1 つある» と «1 本の縁がぎざぎざである» を分ける量である．
    /// 前者は良い絵にも普通にあるが (実測で 61 枚中 36 枚) ，後者は縁の描き方の失敗
    /// でしか出ない．
    pub max_per_chain: usize,
    /// `max_per_chain` を取った区間のラン数 (密度の分母)．
    pub max_chain_runs: usize,
    /// `pxsmith smooth` を実際に掛けた結果 (`--apply` のときだけ)．
    pub applied: Option<Applied>,
}

/// **整形を実際に掛けて何が起きたか．** 直したと言うだけでなく，直った結果を見る．
#[derive(Clone, Debug)]
pub struct Applied {
    pub moved: usize,
    pub remaining: usize,
    pub passes: usize,
    /// 面積の変化 (絶対値)．**動かした画素数を超えてはいけない**．
    pub area_delta: i64,
    /// もう一度掛けたときに動く画素 — **0 でなければ収束していない**．
    pub second_pass_moved: usize,
    /// lint の blocking の数 (前 → 後)．**増えてはいけない**．
    pub blocking_before: usize,
    pub blocking_after: usize,
}

/// 谷 $i$ の周りが «続いた坂» か «単発のくぼみ» か．
///
/// 設計書 6.4 の理想形 (端から中央へ単調非増加 ・中央から端へ単調非減少) の底なら，
/// **両側に厳密な坂が 2 段以上続く**．`[3, 3, 1, 3]` のような単発のジャギーは
/// 平らな中に 1 つ凹んでいるだけなので続かない．
pub fn valley_shape(runs: &[u32], i: usize) -> &'static str {
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
    match (left >= 2, right >= 2) {
        (true, true) => "続いた坂 (理想の単谷形)",
        (true, false) | (false, true) => "片側だけ坂",
        (false, false) => "単発のくぼみ",
    }
}

pub const HEADER: &str = "file,chains,runs,jaggies,fixable,rate,max_per_chain,max_chain_runs,deep";

impl Record {
    pub fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{},{:.5},{},{},{}",
            self.file,
            self.chains,
            self.runs,
            self.jaggies,
            self.fixable,
            if self.runs == 0 {
                0.0
            } else {
                self.jaggies as f32 / self.runs as f32
            },
            self.max_per_chain,
            self.max_chain_runs,
            self.by_delta
                .iter()
                .filter(|(d, _)| **d >= 2)
                .map(|(_, n)| n)
                .sum::<usize>()
        )
    }
}

fn opaque_area(canvas: &pxsmith_core::canvas::IndexedCanvas) -> i64 {
    canvas
        .pixels()
        .iter()
        .filter(|i| canvas.transparent() != Some(**i))
        .count() as i64
}

fn blocking_count(
    canvas: &pxsmith_core::canvas::IndexedCanvas,
    palette: &pxsmith_core::palette::Palette,
) -> usize {
    let cfg = pxsmith_lint::LintConfig::default();
    let mut report = pxsmith_lint::rules::lint_palette(palette, &cfg);
    report.extend(pxsmith_lint::lint_canvas(canvas, palette, &cfg));
    report.blocking().count()
}

fn measure(path: &Path, max_move: u32, apply: bool) -> Result<Record> {
    let img = pxsmith_io::png::read_rgba(path)
        .with_context(|| format!("{} を読めない", path.display()))?;
    let (canvas, palette) = index_exactly(&img)?;
    let report = analyze_canvas(&canvas, max_move);
    let mut by_delta: BTreeMap<i32, usize> = BTreeMap::new();
    for j in &report.jaggies {
        *by_delta.entry(j.delta).or_default() += 1;
    }

    // 谷の周りの形を数え直す (報告には形が入っていないので同じ経路をもう一度回す)
    let mut by_shape: BTreeMap<&'static str, usize> = BTreeMap::new();
    let (mut max_per_chain, mut max_chain_runs) = (0usize, 0usize);
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
                let turns = turn_runs(&chain, &k);
                let mut here = 0usize;
                for i in run_valleys(&runs) {
                    if turns.contains(&i) {
                        *by_shape.entry("曲率で除外済み").or_default() += 1;
                    } else {
                        *by_shape.entry(valley_shape(&runs, i)).or_default() += 1;
                        here += 1;
                    }
                }
                // 同点は «ランの少ない方» を採る (密度が高い方が悪い縁である)
                if here > max_per_chain
                    || (here == max_per_chain && here > 0 && runs.len() < max_chain_runs)
                {
                    max_per_chain = here;
                    max_chain_runs = runs.len();
                }
            }
        }
    }
    let applied = if apply {
        let mut smoothed = canvas.clone();
        let opts = pxsmith_core::smooth::SmoothOptions {
            max_move,
            ..pxsmith_core::smooth::SmoothOptions::default()
        };
        let before_area = opaque_area(&canvas);
        let before_blocking = blocking_count(&canvas, &palette);
        let r = pxsmith_core::smooth::smooth_canvas(&mut smoothed, &opts);
        // **もう一度掛けて動かないか** (収束しているか)
        let again = pxsmith_core::smooth::smooth_canvas(&mut smoothed.clone(), &opts);
        Some(Applied {
            moved: r.moved,
            remaining: r.remaining,
            passes: r.passes,
            area_delta: (opaque_area(&smoothed) - before_area).abs(),
            second_pass_moved: again.moved,
            blocking_before: before_blocking,
            blocking_after: blocking_count(&smoothed, &palette),
        })
    } else {
        None
    };

    Ok(Record {
        file: path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        runs: report.runs,
        chains: report.chains,
        jaggies: report.jaggies.len(),
        fixable: report.fixable(),
        by_delta,
        by_shape,
        max_per_chain,
        max_chain_runs,
        applied,
    })
}

/// ディレクトリの PNG をすべて測る．**測れなかった絵は落とさずに数える**．
pub fn run(
    dir: &Path,
    max_move: u32,
    apply: bool,
) -> Result<(Vec<Record>, crate::lintcal::Skipped)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("{} を読めない", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    files.sort();

    let out: Vec<_> = files
        .par_iter()
        .map(|p| (p.clone(), measure(p, max_move, apply)))
        .collect();

    let mut records = Vec::new();
    let mut skipped = Vec::new();
    for (path, r) in out {
        match r {
            Ok(rec) => records.push(rec),
            Err(e) => skipped.push((
                path.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                format!("{e}"),
            )),
        }
    }
    Ok((records, skipped))
}

/// 既定の移動上限 (`pxsmith-core` から引く．数値を書き写さない)．
pub const DEFAULT_MOVE: u32 = DEFAULT_MAX_MOVE;
