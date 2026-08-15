//! **lint の閾値を校正するための測定の口．**
//!
//! 格子推定と同じ作法で進める — まず «良い絵» に現行の閾値を掛けて，**何がどれだけ
//! 鳴るか**を測る．blocking のルールが良質なドット絵で鳴るなら，負例を 1 枚も作らずに
//! 閾値が間違っていると分かる．
//!
//! PNG は `pxsmith lint` では格子系のルール (2 ・9) しか通らない．色 ・パレット系の
//! ルールは添字の面を要求するので，**ここで PNG を添字化してから掛ける**．
//! 元絵が本物のドット絵なら色数は少なく，量子化ではなく**そのまま**添字にできる．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use pxsmith_core::canvas::{IndexedCanvas, RgbaCanvas};
use pxsmith_core::frame::{Frame, FrameKind, Layer, LayerMeta, Surface};
use pxsmith_core::math::uvec2;
use pxsmith_core::palette::Palette;
use pxsmith_lint::{LintConfig, RULES, Report, rules};
use rayon::prelude::*;

/// 1 枚分の測定．
#[derive(Clone, Debug)]
pub struct Record {
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub colors: usize,
    /// ルール番号ごとの違反数．
    pub hits: BTreeMap<u8, usize>,
}

pub const HEADER: &str = "file,width,height,colors,blocking,advisory,rules";

impl Record {
    pub fn to_csv(&self, cfg: &LintConfig) -> String {
        let _ = cfg;
        let blocking: usize = self
            .hits
            .iter()
            .filter(|(id, _)| is_blocking(**id))
            .map(|(_, n)| n)
            .sum();
        let advisory: usize = self.hits.values().sum::<usize>() - blocking;
        let rules = self
            .hits
            .iter()
            .map(|(id, n)| format!("{id}:{n}"))
            .collect::<Vec<_>>()
            .join("|");
        format!(
            "{},{},{},{},{},{},{}",
            self.file, self.width, self.height, self.colors, blocking, advisory, rules
        )
    }
}

fn is_blocking(id: u8) -> bool {
    pxsmith_lint::rule(id).is_some_and(|r| matches!(r.severity, pxsmith_lint::Severity::Blocking))
}

/// PNG をそのまま添字の面にする．**色を減らさない** — 減らすとその時点で
/// «良い絵» を壊してしまい，何を測っているのか分からなくなる．
///
/// > [!warning] **透明を «最も近い不透明色» で塗り潰さないこと．**
/// > [`Palette::nearest`] は透明色を色距離の対象から外すので，アルファ 0 の画素に
/// > 引くと**近くの不透明色が返る** — 背景が黙って «暗い面» になり，絵の 6 割が
/// > 塗り潰されている状態で lint を掛けていた．透明はパレットの透明添字へ直に送り，
/// > キャンバスにも透明添字を持たせる．
pub fn index_exactly(img: &RgbaCanvas) -> Result<(IndexedCanvas, Palette)> {
    let mut colors: Vec<_> = img.pixels().to_vec();
    colors.sort_unstable_by_key(|c| c.sort_key());
    colors.dedup();
    if colors.len() > 256 {
        bail!("色数が {} で添字に収まらない", colors.len());
    }
    let palette = Palette::new(colors)?;
    let transparent = palette
        .entries()
        .iter()
        .position(|c| c.a == 0)
        .map(|i| i as u8);
    let pixels: Vec<u8> = img
        .pixels()
        .iter()
        .map(|c| match (c.a, transparent) {
            (0, Some(i)) => i,
            _ => palette.nearest(*c, 1.0).unwrap_or(0),
        })
        .collect();
    let canvas = IndexedCanvas::from_pixels(img.width(), img.height(), pixels)?
        .with_transparent(transparent);
    Ok((canvas, palette))
}

/// **格子系のルール (2 ・9) は RGBA だけで判定できる．** 添字にできない絵 (補間や
/// JPEG で色数が爆発したもの) でも測れるので，色 ・パレット系と分けて扱う — まとめて
/// 落とすと，格子のルールを «色数の少ない絵» でしか測っていないことになる．
fn lint_one(path: &Path, cfg: &LintConfig) -> Result<Record> {
    let img = pxsmith_io::png::read_rgba(path)
        .with_context(|| format!("{} を読めない", path.display()))?;
    let (canvas, palette) = match index_exactly(&img) {
        Ok(v) => v,
        Err(_) => {
            let mut hits: BTreeMap<u8, usize> = BTreeMap::new();
            for v in &rules::lint_grid(&img, cfg).violations {
                *hits.entry(v.rule).or_default() += 1;
            }
            return Ok(Record {
                file: name_of(path),
                width: img.width(),
                height: img.height(),
                // 添字にできなかった印 (色数が 256 を超える)
                colors: 0,
                hits,
            });
        }
    };
    let colors = palette.len();
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

    let mut report: Report = pxsmith_lint::lint_frame(&frame, cfg);
    // 格子系 (2 ・9) は RGBA を見る
    report.extend(rules::lint_grid(&img, cfg));

    let mut hits: BTreeMap<u8, usize> = BTreeMap::new();
    for v in &report.violations {
        *hits.entry(v.rule).or_default() += 1;
    }
    Ok(Record {
        file: name_of(path),
        width: img.width(),
        height: img.height(),
        colors,
        hits,
    })
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 検査できなかった絵と，その理由．
pub type Skipped = Vec<(String, String)>;

/// ディレクトリの PNG をすべて検査する．
pub fn run(dir: &Path, cfg: &LintConfig) -> Result<(Vec<Record>, Skipped)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("{} を読めない", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    files.sort();

    let out: Vec<_> = files
        .par_iter()
        .map(|p| (p.clone(), lint_one(p, cfg)))
        .collect();

    let mut records = Vec::new();
    let mut skipped = Vec::new();
    for (path, r) in out {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        match r {
            Ok(rec) => records.push(rec),
            Err(e) => skipped.push((name, format!("{e}"))),
        }
    }
    Ok((records, skipped))
}

/// 正解つきの評価データセットに掛ける．**ルール 2 は «格子が崩れている件» でこそ
/// 鳴らなければならない**ので，格子の有無で分けて数える．
pub fn run_dataset(
    dir: &Path,
    manifest: &crate::dataset::Manifest,
    cfg: &LintConfig,
    only: Option<crate::dataset::Split>,
) -> Vec<(bool, Result<Record>)> {
    manifest
        .items
        .par_iter()
        .filter(|i| only.is_none_or(|s| i.split == s))
        .map(|item| {
            (
                item.has_integer_grid(),
                lint_one(&dir.join(&item.file), cfg),
            )
        })
        .collect()
}

/// ルールごとに «何枚で鳴ったか» を数える．
pub fn by_rule(records: &[Record]) -> Vec<(u8, &'static str, bool, usize, usize)> {
    RULES
        .iter()
        .map(|r| {
            let files = records
                .iter()
                .filter(|rec| rec.hits.contains_key(&r.id))
                .count();
            let total: usize = records.iter().filter_map(|rec| rec.hits.get(&r.id)).sum();
            (
                r.id,
                r.name,
                matches!(r.severity, pxsmith_lint::Severity::Blocking),
                files,
                total,
            )
        })
        .collect()
}
