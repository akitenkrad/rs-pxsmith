//! **ルール 13 (pillow shading) の閾値を測る．**
//!
//! 特徴量は距離場と明度の相関 $\rho = \mathrm{corr}(d(p), L(p))$ である
//! (設計書 7.3) ．3 つの群で測る．
//!
//! | 群 | 中身 | 望ましい向き |
//! | --- | --- | --- |
//! | 正例 | CC0 の実物のドット絵 (良い絵) | 鳴らない |
//! | 負例 | 良い絵を縁からの距離だけで塗り直したもの (`lint-gen pillow`) | 鳴る |
//! | **`px shade`** | 同じ形に `px shade` で陰影を付けたもの | **鳴らない (D58)** |
//!
//! 3 群目が要点である．**設計書は «正しく陰影付けされた凸形状でも相当に相関する» と
//! 予告している**ので，自作の出力が自作の検査に落ちるなら，落ちるのは検査の方である．
//!
//! 群 3 は光源プリセット 5 種すべてで測る — 向きが変われば相関も変わる．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use px_core::canvas::RgbaCanvas;
use px_core::color::Rgba8;
use px_core::palette::ChromaCurve;
use px_core::ramp::{LightPreset, build_lighting};
use px_core::shade::{ShadeOptions, shade_to_canvas};
use rayon::prelude::*;

use crate::lintcal::index_exactly;

/// 1 枚ぶんの測定．
#[derive(Clone, Debug)]
pub struct Record {
    pub file: String,
    /// 光源プリセット (`px shade` の群だけ)．
    pub preset: Option<&'static str>,
    pub pixels: usize,
    pub rho: f32,
    /// その絵で鳴った blocking 違反の数 (自己整合性の確認)．
    pub blocking: usize,
    /// 鳴ったルール番号 (深刻度を問わない)．
    pub rules: Vec<u8>,
}

pub const HEADER: &str = "group,file,preset,pixels,rho,blocking,rules";

/// PNG をそのまま添字にして $\rho$ を測る．
fn measure(img: &RgbaCanvas, file: &str) -> Option<Record> {
    let (canvas, palette) = index_exactly(img).ok()?;
    let rho = px_lint::rules::pillow_correlation(&canvas, &palette)?;
    let (blocking, rules) = lint_counts(&canvas, &palette);
    Some(Record {
        file: file.to_string(),
        preset: None,
        pixels: canvas.silhouette().count(),
        rho,
        blocking,
        rules,
    })
}

/// **自己整合性の確認** — その面に lint を掛けて blocking の数と鳴ったルールを取る．
fn lint_counts(
    canvas: &px_core::canvas::IndexedCanvas,
    palette: &px_core::palette::Palette,
) -> (usize, Vec<u8>) {
    let cfg = px_lint::LintConfig::default();
    let mut report = px_lint::rules::lint_palette(palette, &cfg);
    report.extend(px_lint::lint_canvas(canvas, palette, &cfg));
    let blocking = report.blocking().count();
    let mut rules: Vec<u8> = report.violations.iter().map(|v| v.rule).collect();
    rules.sort_unstable();
    rules.dedup();
    (blocking, rules)
}

/// 絵の中で最も広い不透明な色 (`px shade --base` に渡す固有色にする)．
fn dominant(img: &RgbaCanvas) -> Rgba8 {
    let mut counts: std::collections::BTreeMap<[u8; 4], usize> = std::collections::BTreeMap::new();
    for p in img.pixels() {
        if p.a != 0 {
            *counts.entry([p.r, p.g, p.b, p.a]).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(c, n)| (*n, std::cmp::Reverse(*c)))
        .map(|(c, _)| Rgba8::rgb(c[0], c[1], c[2]))
        .unwrap_or(Rgba8::rgb(128, 128, 128))
}

/// **同じ形に `px shade` で陰影を付けて測る** (D58 の群)．
fn measure_shaded(img: &RgbaCanvas, file: &str, steps: u8) -> Vec<Record> {
    let mut mask = px_core::geom::Mask::new(img.width(), img.height());
    for p in mask.bounds().iter() {
        if img.get(p.x, p.y).is_some_and(|c| c.a != 0) {
            mask.set(p, true);
        }
    }
    if mask.is_empty() {
        return Vec::new();
    }
    let base = dominant(img);
    let pixels = mask.count();

    LightPreset::ALL
        .iter()
        .filter_map(|&preset| {
            let (palette, model) =
                build_lighting(base, preset, steps, ChromaCurve::PeakMiddle).ok()?;
            let (canvas, palette) = shade_to_canvas(
                &mask,
                preset.default_source(),
                &model,
                &palette,
                ShadeOptions::default(),
            )
            .ok()?;
            let rho = px_lint::rules::pillow_correlation(&canvas, &palette)?;
            let (blocking, rules) = lint_counts(&canvas, &palette);
            Some(Record {
                file: file.to_string(),
                preset: Some(preset.as_str()),
                pixels,
                rho,
                blocking,
                rules,
            })
        })
        .collect()
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

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 3 つの群を測る．負例のディレクトリが無ければその群は空になる．
pub fn run(
    seeds: &Path,
    negatives: Option<&Path>,
    steps: u8,
) -> Result<(Vec<Record>, Vec<Record>, Vec<Record>)> {
    let files = png_files(seeds)?;
    let good: Vec<Record> = files
        .par_iter()
        .filter_map(|p| {
            let img = px_io::png::read_rgba(p).ok()?;
            measure(&img, &name_of(p))
        })
        .collect();
    let shaded: Vec<Record> = files
        .par_iter()
        .flat_map(|p| match px_io::png::read_rgba(p) {
            Ok(img) => measure_shaded(&img, &name_of(p), steps),
            Err(_) => Vec::new(),
        })
        .collect();

    let bad: Vec<Record> = match negatives {
        Some(dir) if dir.exists() => png_files(dir)?
            .par_iter()
            // 負例のディレクトリには 7 種類の欠陥が混ざっている．**pillow だけを採る**
            .filter(|p| name_of(p).starts_with("pillow-"))
            .filter_map(|p| {
                let img = px_io::png::read_rgba(p).ok()?;
                measure(&img, &name_of(p))
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok((good, bad, shaded))
}

/// 分位点 (最小 ・5% ・中央 ・95% ・最大)．
pub fn quantiles(records: &[Record]) -> Option<(f32, f32, f32, f32, f32)> {
    if records.is_empty() {
        return None;
    }
    let mut v: Vec<f32> = records.iter().map(|r| r.rho).collect();
    v.sort_by(f32::total_cmp);
    let at = |q: f32| v[((v.len() - 1) as f32 * q).round() as usize];
    Some((v[0], at(0.05), at(0.5), at(0.95), v[v.len() - 1]))
}

/// 閾値ごとの «鳴る割合»．
pub fn rate_at(records: &[Record], threshold: f32) -> f32 {
    if records.is_empty() {
        return 0.0;
    }
    records.iter().filter(|r| r.rho > threshold).count() as f32 / records.len() as f32
}

pub fn to_csv(group: &str, r: &Record) -> String {
    format!(
        "{group},{},{},{},{:.4},{},{}",
        r.file,
        r.preset.unwrap_or(""),
        r.pixels,
        r.rho,
        r.blocking,
        r.rules
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("|")
    )
}

/// ルール番号ごとに «何件で鳴ったか» を数える．
pub fn rule_hits(records: &[Record]) -> std::collections::BTreeMap<u8, usize> {
    let mut out = std::collections::BTreeMap::new();
    for r in records {
        for &id in &r.rules {
            *out.entry(id).or_default() += 1;
        }
    }
    out
}
