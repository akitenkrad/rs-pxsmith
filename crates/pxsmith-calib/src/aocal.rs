//! **環境遮蔽 (`pxsmith shade --ao`) の閾値を測る．**
//!
//! 遮蔽は «谷折りの角は左右の面から光が届きにくい» を 1 画素単位で近似したもので，
//! 平滑化した距離場の曲率が $-\text{閾値}$ を下回った画素を遮蔽色へ落とす．
//! **閾値は «少し乗る» ところに置く** — 掛けすぎると絵が汚れる．
//!
//! lint と同じ作法で 3 群を測る (D70 の «正例が先») ．
//!
//! | 群 | 中身 | 望ましい向き |
//! | --- | --- | --- |
//! | **凸な形** | 円板 ・箱 (合成) | **乗らない** — 凹みが無いのだから 0 が正しい |
//! | 凹んだ形 | 谷折り ・胴体 (合成) | 乗る |
//! | **実素材** | CC0 の実物 64 枚のシルエット | **少しだけ乗る** |
//!
//! 3 群目が要点である．合成した形は滑らかなので «縁の階段» が出ず，**実素材でしか
//! 分からないことがある** (M3 で自己整合性が実素材でだけ落ちたのと同じ) ．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pxsmith_core::color::Rgba8;
use pxsmith_core::geom::Mask;
use pxsmith_core::math::ivec2;
use pxsmith_core::palette::ChromaCurve;
use pxsmith_core::ramp::{LightPreset, build_lighting};
use pxsmith_core::shade::{ShadeOptions, shade_mask};
use rayon::prelude::*;

/// 1 つの形 ・1 つの閾値ぶんの測定．
#[derive(Clone, Debug)]
pub struct Record {
    pub group: &'static str,
    pub file: String,
    pub threshold: f32,
    /// 距離場を均した回数．
    pub passes: usize,
    /// シルエットの画素数．
    pub pixels: usize,
    /// 遮蔽色になった画素数．
    pub occluded: usize,
    /// **遮蔽色の «ひとりぼっち» の画素数** — 4 近傍に仲間がいないもの．
    ///
    /// 遮蔽は «角に沿って» 乗るものなので，散らばっているなら閾値が低すぎる
    /// (lint ルール 3 の相手にもなる) ．
    pub scattered: usize,
}

impl Record {
    pub fn ratio(&self) -> f32 {
        if self.pixels == 0 {
            0.0
        } else {
            self.occluded as f32 / self.pixels as f32
        }
    }
}

pub const HEADER: &str = "group,file,threshold,passes,pixels,occluded,ratio,scattered";

pub fn to_csv(r: &Record) -> String {
    format!(
        "{},{},{:.3},{},{},{},{:.4},{}",
        r.group,
        r.file,
        r.threshold,
        r.passes,
        r.pixels,
        r.occluded,
        r.ratio(),
        r.scattered
    )
}

/// 円板 (凸)．
fn disc(size: u32, radius: f32) -> Mask {
    let mut m = Mask::new(size, size);
    let c = (size as f32 - 1.0) / 2.0;
    for p in m.bounds().iter() {
        let (dx, dy) = (p.x as f32 - c, p.y as f32 - c);
        if (dx * dx + dy * dy).sqrt() <= radius {
            m.set(p, true);
        }
    }
    m
}

/// 箱 (凸)．
fn box_mask(size: u32) -> Mask {
    let mut m = Mask::new(size, size);
    for p in m.bounds().iter() {
        if p.x >= 3 && p.y >= 3 && p.x < size as i32 - 3 && p.y < size as i32 - 3 {
            m.set(p, true);
        }
    }
    m
}

/// 谷折り — 2 つの円板が重なった形 (くびれが凹む)．
fn notched(size: u32) -> Mask {
    let mut m = Mask::new(size, size);
    let r = size as f32 * 0.28;
    for (cx, cy) in [
        (size as f32 * 0.35, size as f32 * 0.5),
        (size as f32 * 0.65, size as f32 * 0.5),
    ] {
        for p in m.bounds().iter() {
            let (dx, dy) = (p.x as f32 - cx, p.y as f32 - cy);
            if (dx * dx + dy * dy).sqrt() <= r {
                m.set(p, true);
            }
        }
    }
    m
}

/// L 字 — 内角がはっきりある形．
fn ell(size: u32) -> Mask {
    let mut m = Mask::new(size, size);
    let s = size as i32;
    for p in m.bounds().iter() {
        let in_arm = p.x >= 3 && p.x < s - 3 && p.y >= s / 2 && p.y < s - 3;
        let up_arm = p.x >= 3 && p.x < s / 2 && p.y >= 3 && p.y < s - 3;
        if in_arm || up_arm {
            m.set(p, true);
        }
    }
    m
}

/// 1 つのマスクを 1 つの閾値で測る．
fn measure(
    group: &'static str,
    file: &str,
    mask: &Mask,
    threshold: f32,
    passes: usize,
) -> Option<Record> {
    let pixels = mask.count();
    if pixels < 64 {
        return None;
    }
    let base = Rgba8::rgb(0x8a, 0x6a, 0x4a);
    let (_palette, model) =
        build_lighting(base, LightPreset::Clear, 5, ChromaCurve::PeakMiddle).ok()?;
    let opts = ShadeOptions {
        ambient_occlusion: Some(threshold),
        ao_smooth_passes: passes,
        ..ShadeOptions::default()
    };
    let field = shade_mask(mask, LightPreset::Clear.default_source(), &model, opts);

    let occluded_at =
        |p: pxsmith_core::math::IVec2| field.copied(p).flatten() == Some(model.occlusion);
    let mut occluded = 0usize;
    let mut scattered = 0usize;
    for p in mask.bounds().iter() {
        if !occluded_at(p) {
            continue;
        }
        occluded += 1;
        let alone = [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)]
            .iter()
            .all(|d| !occluded_at(p + *d));
        if alone {
            scattered += 1;
        }
    }
    Some(Record {
        group,
        file: file.to_string(),
        threshold,
        passes,
        pixels,
        occluded,
        scattered,
    })
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

/// 十字 — 内角が 4 つある．**谷折りより深い凹み**である．
fn cross(size: u32) -> Mask {
    let mut m = Mask::new(size, size);
    let s = size as i32;
    let (a, b) = (s / 3, s * 2 / 3);
    for p in m.bounds().iter() {
        let vertical = p.x >= a && p.x < b && p.y >= 2 && p.y < s - 2;
        let horizontal = p.y >= a && p.y < b && p.x >= 2 && p.x < s - 2;
        if vertical || horizontal {
            m.set(p, true);
        }
    }
    m
}

/// コの字 — 深い «口» を持つ形．
fn bracket(size: u32) -> Mask {
    let mut m = Mask::new(size, size);
    let s = size as i32;
    for p in m.bounds().iter() {
        if p.x < 3 || p.y < 3 || p.x >= s - 3 || p.y >= s - 3 {
            continue;
        }
        // 右側の中央から «口» をくり抜く
        let mouth = p.x >= s / 2 && p.y >= s / 3 && p.y < s * 2 / 3;
        if !mouth {
            m.set(p, true);
        }
    }
    m
}

/// 合成した形 (凸 4 つ ・浅い凹み 4 つ ・深い凹み 4 つ)．
fn synthetic() -> Vec<(&'static str, String, Mask)> {
    vec![
        ("凸", "円板 24".to_string(), disc(24, 10.0)),
        ("凸", "円板 32".to_string(), disc(32, 14.0)),
        ("凸", "箱 24".to_string(), box_mask(24)),
        ("凸", "箱 32".to_string(), box_mask(32)),
        ("浅い凹", "谷折り 24".to_string(), notched(24)),
        ("浅い凹", "谷折り 32".to_string(), notched(32)),
        ("浅い凹", "L 字 24".to_string(), ell(24)),
        ("浅い凹", "L 字 32".to_string(), ell(32)),
        ("深い凹", "十字 24".to_string(), cross(24)),
        ("深い凹", "十字 32".to_string(), cross(32)),
        ("深い凹", "コの字 24".to_string(), bracket(24)),
        ("深い凹", "コの字 32".to_string(), bracket(32)),
    ]
}

/// 3 群を掃引して測る．
pub fn run(seeds: &Path, thresholds: &[f32], passes: &[usize]) -> Result<Vec<Record>> {
    let shapes = synthetic();
    let mut out: Vec<Record> = Vec::new();
    for &n in passes {
        for &t in thresholds {
            for (group, name, mask) in &shapes {
                if let Some(r) = measure(group, name, mask, t, n) {
                    out.push(r);
                }
            }
        }
    }

    let files = png_files(seeds)?;
    let real: Vec<Record> = files
        .par_iter()
        .flat_map(|p| {
            let Ok(img) = pxsmith_io::png::read_rgba(p) else {
                return Vec::new();
            };
            let mut mask = Mask::new(img.width(), img.height());
            for q in mask.bounds().iter() {
                if img.get(q.x, q.y).is_some_and(|c| c.a != 0) {
                    mask.set(q, true);
                }
            }
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            // **シルエットのある絵と «画面いっぱい» の絵を分ける．**
            // 全面が不透明なタイルには «外» が無いので，遮蔽が乗る «窪み» も無い —
            // 混ぜて数えると割合が別物になる
            let group: &'static str = if mask.count() == (mask.width() * mask.height()) as usize {
                "画面いっぱい"
            } else {
                "実素材"
            };
            passes
                .iter()
                .flat_map(|&n| {
                    thresholds
                        .iter()
                        .filter_map(|&t| measure(group, &name, &mask, t, n))
                        .collect::<Vec<_>>()
                })
                .collect()
        })
        .collect();
    out.extend(real);
    Ok(out)
}

/// 群 ・閾値ごとにまとめる (件数 ・割合の中央と最大 ・散らばりの合計)．
pub fn summarise(records: &[Record]) -> Vec<(&'static str, usize, f32, usize, f32, f32, usize)> {
    let mut keys: Vec<(usize, u32, &'static str)> = records
        .iter()
        .map(|r| (r.passes, (r.threshold * 1000.0).round() as u32, r.group))
        .collect();
    keys.sort_unstable();
    keys.dedup();

    keys.into_iter()
        .map(|(passes, t, group)| {
            let hit = |r: &&Record| {
                r.group == group && r.passes == passes && (r.threshold * 1000.0).round() as u32 == t
            };
            let mut ratios: Vec<f32> = records.iter().filter(hit).map(|r| r.ratio()).collect();
            ratios.sort_by(f32::total_cmp);
            let scattered: usize = records.iter().filter(hit).map(|r| r.scattered).sum();
            let median = ratios[ratios.len() / 2];
            let max = *ratios.last().expect("空でない");
            (
                group,
                passes,
                t as f32 / 1000.0,
                ratios.len(),
                median,
                max,
                scattered,
            )
        })
        .collect()
}
