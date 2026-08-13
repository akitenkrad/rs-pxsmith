//! **`px compose` の «置き方» を測る．**
//!
//! 設計書は «アンカー付きパーツ合成» としか決めていないので，3 つを実測で決めた．
//!
//! | 測るもの | 決まること |
//! | --- | --- |
//! | **画布の余白** | アンカーで動かしたとき切れるか — 画布を広げるか切るか |
//! | **併合したパレットの色数** | 添字をそのまま通せるか ・L0 の 62 色に収まるか |
//! | **実際に合成して壊れないか** | 切り捨て ・被覆 ・lint の blocking |
//!
//! 3 つ目が `px-calib aa` ・`outline` と同じ «自分の道具が自分の検査に落ちていないか»
//! の口である (D58) ．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use px_core::compose::{Alignment, ComposeOptions, Part, compose};
use px_core::frame::{Frame, FrameKind, Layer, LayerMeta, Surface};
use px_core::math::{IVec2, ivec2, uvec2};

use crate::lintcal;

/// 1 枚の余白 (シルエットの外接矩形から四辺までの画素数)．
#[derive(Clone, Debug)]
pub struct Margin {
    pub file: String,
    /// 全面が不透明な «画面いっぱい» の絵かどうか．
    pub full_bleed: bool,
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

impl Margin {
    pub fn min(&self) -> u32 {
        self.left.min(self.right).min(self.top).min(self.bottom)
    }
}

/// 2 パーツを併合したときの色数．
#[derive(Clone, Debug)]
pub struct Merge {
    pub base: String,
    pub part: String,
    pub base_colors: usize,
    pub part_colors: usize,
    pub merged: usize,
    /// 装備の色のうち胴体と共有しているものの割合．
    pub shared: f32,
}

/// 実際に合成した結果．
#[derive(Clone, Debug)]
pub struct Run {
    pub base: String,
    pub part: String,
    pub shift: IVec2,
    pub clip: bool,
    pub canvas: (u32, u32),
    pub grew: bool,
    /// 画布の外へ出て捨てた画素数．
    pub clipped: usize,
    /// 上のパーツに隠された画素数．
    pub covered: usize,
    /// 合成の前後で blocking がいくつ増えたか．
    pub blocking_delta: i64,
    /// **どのルールが増えたか．** 総数だけ見ると «合成が壊した» のか
    /// «重ねた絵が本当にそうなっている» のかを分けられない (D58) ．
    pub by_rule: BTreeMap<u8, i64>,
}

pub const MARGIN_HEADER: &str = "file,full_bleed,left,right,top,bottom,min";
pub const MERGE_HEADER: &str = "base,part,base_colors,part_colors,merged,shared";
pub const RUN_HEADER: &str =
    "base,part,shift_x,shift_y,clip,canvas_w,canvas_h,grew,clipped,covered,blocking_delta";

pub fn margin_csv(m: &Margin) -> String {
    format!(
        "{},{},{},{},{},{},{}",
        m.file,
        m.full_bleed,
        m.left,
        m.right,
        m.top,
        m.bottom,
        m.min()
    )
}

pub fn merge_csv(m: &Merge) -> String {
    format!(
        "{},{},{},{},{},{:.3}",
        m.base, m.part, m.base_colors, m.part_colors, m.merged, m.shared
    )
}

pub fn run_csv(r: &Run) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{},{},{}",
        r.base,
        r.part,
        r.shift.x,
        r.shift.y,
        r.clip,
        r.canvas.0,
        r.canvas.1,
        r.grew,
        r.clipped,
        r.covered,
        r.blocking_delta
    )
}

fn name_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn png_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("{} を読めない", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    files.sort();
    Ok(files)
}

/// PNG を 1 レイヤ ・1 フレームのパーツにする．
fn part_of(path: &Path) -> Result<Part> {
    let img = px_io::png::read_rgba(path)?;
    let (canvas, palette) = lintcal::index_exactly(&img)?;
    let frame = Frame {
        size: uvec2(img.width(), img.height()),
        layers: vec![Layer::new(
            LayerMeta {
                name: "art".to_string(),
                ..LayerMeta::default()
            },
            Surface::Indexed(canvas),
        )],
        palette,
        duration_ms: 100,
        kind: FrameKind::Key,
    };
    Ok(Part::new(name_of(path), vec![frame]))
}

/// 余白を測る．
pub fn margins(dir: &Path) -> Result<Vec<Margin>> {
    let mut out = Vec::new();
    for path in png_files(dir)? {
        let Ok(img) = px_io::png::read_rgba(&path) else {
            continue;
        };
        let (w, h) = (img.width() as i32, img.height() as i32);
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        let mut opaque = 0usize;
        for y in 0..h {
            for x in 0..w {
                if img.get(x, y).is_some_and(|c| c.a != 0) {
                    opaque += 1;
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        if x0 > x1 {
            continue;
        }
        out.push(Margin {
            file: name_of(&path),
            full_bleed: opaque == (w * h) as usize,
            left: x0 as u32,
            right: (w - 1 - x1) as u32,
            top: y0 as u32,
            bottom: (h - 1 - y1) as u32,
        });
    }
    Ok(out)
}

/// 併合したパレットの色数を，胴体 x 装備の総当たりで測る．
pub fn merges(dir: &Path, bases: &[String], equips: &[String]) -> Result<Vec<Merge>> {
    let mut out = Vec::new();
    for b in bases {
        let Ok(bimg) = px_io::png::read_rgba(dir.join(format!("{b}.png"))) else {
            continue;
        };
        let bc = used_colors(&bimg);
        for e in equips {
            let Ok(eimg) = px_io::png::read_rgba(dir.join(format!("{e}.png"))) else {
                continue;
            };
            let ec = used_colors(&eimg);
            let shared = if ec.is_empty() {
                0.0
            } else {
                ec.iter().filter(|c| bc.contains(*c)).count() as f32 / ec.len() as f32
            };
            let mut merged: Vec<u32> = bc.iter().chain(&ec).copied().collect();
            merged.sort_unstable();
            merged.dedup();
            out.push(Merge {
                base: b.clone(),
                part: e.clone(),
                base_colors: bc.len(),
                part_colors: ec.len(),
                merged: merged.len(),
                shared,
            });
        }
    }
    Ok(out)
}

fn used_colors(img: &px_core::canvas::RgbaCanvas) -> Vec<u32> {
    let mut v: Vec<u32> = img
        .pixels()
        .iter()
        .filter(|c| c.a != 0)
        .map(|c| u32::from_be_bytes([c.r, c.g, c.b, c.a]))
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// **実際に合成して «壊していないか» を測る．**
///
/// アンカーは素材に付いていないので，ずらし量を掃く — アンカーの中身が何であれ
/// 結果は «ずれ» でしか効かないので，ずれを直接与えるのと同じことである．
pub fn runs(
    dir: &Path,
    bases: &[String],
    equips: &[String],
    shifts: &[IVec2],
    clip: bool,
) -> Result<Vec<Run>> {
    let cfg = px_lint::LintConfig::default();
    let mut out = Vec::new();
    for b in bases {
        let Ok(base) = part_of(&dir.join(format!("{b}.png"))) else {
            continue;
        };
        let (before, before_rules) = blocking_of(&base.frames[0], &cfg);
        for e in equips {
            let Ok(top) = part_of(&dir.join(format!("{e}.png"))) else {
                continue;
            };
            for shift in shifts {
                let mut a = base.clone();
                a.anchors.insert("joint".into(), *shift);
                let mut c = top.clone();
                c.anchors.insert("joint".into(), ivec2(0, 0));
                c.align = Some(Alignment {
                    part: "joint".into(),
                    base: "joint".into(),
                });
                let opts = ComposeOptions {
                    clip,
                    ..ComposeOptions::default()
                };
                let Ok((frames, report)) = compose(&[a, c], &opts) else {
                    continue;
                };
                let (after, after_rules) = blocking_of(&frames[0], &cfg);
                let mut by_rule: BTreeMap<u8, i64> = after_rules.clone();
                for (rule, n) in &before_rules {
                    *by_rule.entry(*rule).or_default() -= n;
                }
                by_rule.retain(|_, d| *d != 0);
                out.push(Run {
                    base: b.clone(),
                    part: e.clone(),
                    shift: *shift,
                    clip,
                    canvas: (report.canvas.x, report.canvas.y),
                    grew: report.grew,
                    clipped: report.clipped(),
                    covered: report.placements.iter().map(|p| p.covered).sum(),
                    blocking_delta: after as i64 - before as i64,
                    by_rule,
                });
            }
        }
    }
    Ok(out)
}

/// フレームの blocking 違反の数．**レイヤを 1 枚に潰してから測る** — 合成の出力は
/// パーツごとにレイヤが分かれており，レイヤ単位で見ると «穴だらけの絵» を検査する
/// ことになる．
fn blocking_of(frame: &Frame, cfg: &px_lint::LintConfig) -> (usize, BTreeMap<u8, i64>) {
    let Some(flat) = flatten(frame) else {
        return (0, BTreeMap::new());
    };
    let mut r = px_lint::rules::lint_palette(&frame.palette, cfg);
    r.extend(px_lint::lint_canvas(&flat, &frame.palette, cfg));
    let mut by_rule: BTreeMap<u8, i64> = BTreeMap::new();
    let mut n = 0usize;
    for v in r.blocking() {
        *by_rule.entry(v.rule).or_default() += 1;
        n += 1;
    }
    (n, by_rule)
}

/// レイヤを下から順に載せて 1 枚にする．
fn flatten(frame: &Frame) -> Option<px_core::canvas::IndexedCanvas> {
    let first = frame.layers.first()?.surface.as_indexed()?;
    let transparent = first.transparent()?;
    let mut out = px_core::canvas::IndexedCanvas::filled(frame.size.x, frame.size.y, transparent)
        .with_transparent(Some(transparent));
    for layer in &frame.layers {
        if let Some(c) = layer.surface.as_indexed() {
            out.blit(c, ivec2(0, 0), true);
        }
    }
    Some(out)
}

/// 余白を «シルエットのある絵» と «画面いっぱいの絵» に分けてまとめる．
///
/// 分けるのは D91 と同じ理由である — 画面いっぱいのタイルは余白が必ず 0 なので，
/// 混ぜると «どれも縁に接している» という当たり前の数しか出ない．
pub fn summarise_margins(margins: &[Margin]) -> Vec<(&'static str, usize, usize, u32, u32)> {
    [("シルエット", false), ("画面いっぱい", true)]
        .into_iter()
        .filter_map(|(label, full)| {
            let group: Vec<&Margin> = margins.iter().filter(|m| m.full_bleed == full).collect();
            if group.is_empty() {
                return None;
            }
            let touching = group.iter().filter(|m| m.min() == 0).count();
            let mut mins: Vec<u32> = group.iter().map(|m| m.min()).collect();
            mins.sort_unstable();
            let median = mins[mins.len() / 2];
            let max = *mins.last().expect("空でない");
            Some((label, group.len(), touching, median, max))
        })
        .collect()
}

/// 併合の色数をまとめる (件数 ・中央 ・最大 ・上限を超えた件数)．
pub fn summarise_merges(merges: &[Merge]) -> (usize, usize, usize, usize, usize, f32) {
    let mut counts: Vec<usize> = merges.iter().map(|m| m.merged).collect();
    counts.sort_unstable();
    let median = counts.get(counts.len() / 2).copied().unwrap_or(0);
    let max = counts.last().copied().unwrap_or(0);
    let over62 = merges.iter().filter(|m| m.merged > 62).count();
    let over256 = merges.iter().filter(|m| m.merged > 256).count();
    let mut shared: Vec<f32> = merges.iter().map(|m| m.shared).collect();
    shared.sort_by(f32::total_cmp);
    let shared_median = shared.get(shared.len() / 2).copied().unwrap_or(0.0);
    (merges.len(), median, max, over62, over256, shared_median)
}

/// ずらし量ごとに «切り捨て ・被覆 ・blocking 増» をまとめる．
pub fn summarise_runs(runs: &[Run]) -> Vec<(IVec2, bool, usize, usize, usize, usize)> {
    let mut keys: Vec<(i32, i32, bool)> = runs
        .iter()
        .map(|r| (r.shift.x, r.shift.y, r.clip))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|(x, y, clip)| {
            let hit = |r: &&Run| r.shift.x == x && r.shift.y == y && r.clip == clip;
            let group: Vec<&Run> = runs.iter().filter(hit).collect();
            let clipped: usize = group.iter().map(|r| r.clipped).sum();
            let grew = group.iter().filter(|r| r.grew).count();
            let worse = group.iter().filter(|r| r.blocking_delta > 0).count();
            (ivec2(x, y), clip, group.len(), grew, clipped, worse)
        })
        .collect()
}

/// 増えた blocking をルールごとに数える．
pub fn blocking_by_rule(runs: &[Run]) -> Vec<(u8, i64, usize)> {
    let mut total: BTreeMap<u8, (i64, usize)> = BTreeMap::new();
    for r in runs {
        for (rule, d) in &r.by_rule {
            if *d <= 0 {
                continue;
            }
            let e = total.entry(*rule).or_insert((0, 0));
            e.0 += d;
            e.1 += 1;
        }
    }
    total.into_iter().map(|(k, (d, n))| (k, d, n)).collect()
}

/// 既定の «胴体» — Dungeon Crawl のドール素材のうち生き物の絵 (CC0)．
pub const DEFAULT_BASES: &[&str] = &[
    "crawl_goblin",
    "crawl_donald",
    "crawl_saint_roka",
    "crawl_urand_fencer",
    "crawl_naga_warrior",
    "crawl_deformed_orc",
    "crawl_centaur_darkgrey_f",
    "crawl_salamander",
    "crawl_elephant",
    "crawl_holy_dragon",
    "crawl_tentacled_monstrosity",
    "crawl_siren_water",
];

/// 既定の «装備» — 同じ素材集の被り物と衣服 (CC0)．**実際に重ねて使う素材である**．
pub const DEFAULT_EQUIPS: &[&str] = &["crawl_cap1", "crawl_helmet1_visored", "crawl_cloth"];

/// 既定のずらし量．0 は «既に揃っている» 場合で，画布が動いてはいけない．
pub fn default_shifts() -> Vec<IVec2> {
    vec![
        ivec2(0, 0),
        ivec2(1, 0),
        ivec2(0, -2),
        ivec2(-3, 2),
        ivec2(4, -5),
    ]
}

/// 名前の一覧を `BTreeMap` 越しに整えて返す (重複の除去と決定論的な並び)．
pub fn names(list: &[&str]) -> Vec<String> {
    let m: BTreeMap<&str, ()> = list.iter().map(|s| (*s, ())).collect();
    m.into_keys().map(|s| s.to_string()).collect()
}
