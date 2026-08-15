//! **方向展開 (反転 + 陰影再導出) を測る．**
//!
//! 設計書 4.3 は «自動ミラーは陰影を持つ素材では光源方向を反転させる．自動ミラーで
//! 生成したタイルには lint ルール 7 を blocking で適用する» と言い，実装計画書は
//! «反転 + 陰影再導出で 8 方向生成» と言う．**どちらも «再導出すれば直る» を
//! 前提にしているが，測っていない．**
//!
//! `pxsmith-calib flip` (D89) が測ったのは **`pxsmith shade` の出力とその左右反転**だけである．
//! 方向展開が受け取るのは**手で描いた絵**でもあるので，2 つが未知のまま残っている．
//!
//! | 問い | なぜ要るか |
//! | --- | --- |
//! | **実素材を反転したらルール 7 は鳴るか** | 鳴らないなら «自動ミラーに blocking» は空振りする |
//! | **再導出は絵を何割書き換えるか** | 再導出は色を総取っ替えする．手描きの絵では失うものがある |
//!
//! 群は 2 つ．**`pxsmith shade` の出力** (再導出が素直に効くはずの側) と，
//! **実素材** (手で描かれた側) である．
//!
//! > [!warning] **1 度目は «宣言する光源» をプリセットの既定にして測り，外した．**
//! > 実素材は**元の絵の時点で 240 件中 196 件 (82%) が鳴った** — プリセットの
//! > 既定光源は作者が使った光源ではないので当たり前である．**正例が鳴っている
//! > 状態では反転の効果を測れない** (D70 «正例が先» と同じ穴) ．
//! >
//! > 直した測り方は «**元の絵から向きを取り，その宣言の下で反転した絵を検査する**»
//! > である．同語反復にはならない — 同語反復になるのは**同じ絵**を推定して検査する
//! > ときであって，ここで検査するのは**別の絵 (反転したもの)** である．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::color::Rgba8;
use pxsmith_core::geom::Mask;
use pxsmith_core::palette::{ChromaCurve, Palette};
use pxsmith_core::ramp::{LightPreset, LightSource, build_lighting};
use pxsmith_core::shade::{ShadeOptions, shade_to_canvas};

/// 1 枚 ・1 プリセットぶんの測定．
#[derive(Clone, Debug)]
pub struct Record {
    pub group: &'static str,
    pub file: String,
    pub preset: &'static str,
    /// 元の絵と光源の一致度．
    pub before: f32,
    /// **左右反転しただけ**の絵の一致度．**ここが下がらなければルール 7 は空振りする**．
    pub mirrored: f32,
    /// 反転してから陰影を導出し直した絵の一致度．
    pub reshaded: Option<f32>,
    /// 再導出が «反転しただけの絵» から書き換えた不透明画素の割合．
    pub rewritten: f32,
    /// **宣言した光源の横成分．** 反転で変わるのはこれだけである (D89) ．
    pub light_x: f32,
    /// 元の絵の色数．
    pub colors_before: usize,
    /// 再導出した絵の色数．
    pub colors_after: usize,
}

pub const HEADER: &str =
    "group,file,preset,before,mirrored,reshaded,rewritten,light_x,colors_before,colors_after";

pub fn to_csv(r: &Record) -> String {
    format!(
        "{},{},{},{:.4},{:.4},{},{:.4},{:.4},{},{}",
        r.group,
        r.file,
        r.preset,
        r.before,
        r.mirrored,
        r.reshaded
            .map(|v| format!("{v:.4}"))
            .unwrap_or_else(|| "na".to_string()),
        r.rewritten,
        r.light_x,
        r.colors_before,
        r.colors_after
    )
}

/// **平行光源のプリセットだけを測る．** 点光源 (`night`) では $\ell$ が画素ごとに
/// 違うのでルール 7 が掛からない (D89) ．
pub const DIRECTIONAL: &[(LightPreset, &str)] = &[
    (LightPreset::Clear, "clear"),
    (LightPreset::Overcast, "overcast"),
    (LightPreset::Sunset, "sunset"),
    (LightPreset::Moonlight, "moonlight"),
];

/// 左右反転．**シルエットの形をそのまま写す** — 補間は挟まない．
pub fn mirror(canvas: &IndexedCanvas) -> IndexedCanvas {
    let (w, h) = (canvas.width(), canvas.height());
    let mut out = canvas.clone();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let v = canvas.get(w as i32 - 1 - x, y).expect("範囲内");
            out.set(x, y, v);
        }
    }
    out
}

fn mask_of(canvas: &IndexedCanvas) -> Mask {
    let mut m = Mask::new(canvas.width(), canvas.height());
    for p in canvas.bounds().iter() {
        if !canvas.is_transparent_at(p) {
            m.set(p, true);
        }
    }
    m
}

/// 面積が最も大きい不透明色 — 再導出の基準色にする．
fn dominant(canvas: &IndexedCanvas, palette: &Palette) -> Rgba8 {
    let mut counts = [0usize; 256];
    for v in canvas.pixels() {
        if canvas.transparent() != Some(*v) {
            counts[*v as usize] += 1;
        }
    }
    let best = counts
        .iter()
        .enumerate()
        .max_by_key(|(i, n)| (**n, std::cmp::Reverse(*i)))
        .map(|(i, _)| i as u8)
        .unwrap_or(0);
    palette
        .get(best)
        .filter(|c| c.a != 0)
        .unwrap_or(Rgba8::rgb(0x8a, 0x6a, 0x4a))
}

fn agreement(canvas: &IndexedCanvas, palette: &Palette, source: LightSource) -> Option<f32> {
    pxsmith_lint::rules::shading_agreement(canvas, palette, source)
}

/// **その絵が «どちらから照らされているように見えるか» を宣言に使う．**
///
/// プリセットの既定光源では実素材の 82% が元の絵の時点で鳴ってしまい，反転の
/// 効果が測れない．元の絵から取った向きを宣言とし，**反転した絵**を検査する．
fn declared_light(canvas: &IndexedCanvas, palette: &Palette) -> Option<LightSource> {
    let g = pxsmith_lint::rules::mean_lightness_direction(canvas, palette)?;
    // `dir` は光源から面へ向かう向きなので，明度が増す向きの逆になる
    Some(LightSource::Directional {
        dir: pxsmith_core::math::Vec2 { x: -g.x, y: -g.y },
    })
}

/// 測る対象．**`source` は呼ぶ側が決める** — 何を宣言として使うかがこの測定の
/// 要点なので，ここに既定は置かない．
struct Subject<'a> {
    group: &'static str,
    file: &'a str,
    canvas: &'a IndexedCanvas,
    palette: &'a Palette,
    preset: LightPreset,
    name: &'static str,
    steps: u8,
    source: LightSource,
    base: Rgba8,
}

/// 1 枚 ・1 プリセットを測る．
fn measure(s: &Subject<'_>) -> Option<Record> {
    let Subject {
        group,
        file,
        canvas,
        palette,
        preset,
        name,
        steps,
        source,
        base,
    } = *s;
    let before = agreement(canvas, palette, source)?;

    let flipped = mirror(canvas);
    let mirrored = agreement(&flipped, palette, source)?;

    // 反転したシルエットへ陰影を導出し直す
    let (ramp_palette, model) =
        build_lighting(base, preset, steps, ChromaCurve::PeakMiddle).ok()?;
    let (reshaded_canvas, reshaded_palette) = shade_to_canvas(
        &mask_of(&flipped),
        source,
        &model,
        &ramp_palette,
        ShadeOptions::default(),
    )
    .ok()?;
    let reshaded = agreement(&reshaded_canvas, &reshaded_palette, source);

    // 再導出が «反転しただけの絵» から何割の不透明画素を書き換えたか
    let (mut opaque, mut changed) = (0usize, 0usize);
    for p in flipped.bounds().iter() {
        if flipped.is_transparent_at(p) {
            continue;
        }
        opaque += 1;
        let was = flipped.get_at(p).and_then(|i| palette.get(i));
        let now = reshaded_canvas
            .get_at(p)
            .and_then(|i| reshaded_palette.get(i));
        if was != now {
            changed += 1;
        }
    }

    Some(Record {
        group,
        file: file.to_string(),
        preset: name,
        before,
        mirrored,
        reshaded,
        rewritten: if opaque == 0 {
            0.0
        } else {
            changed as f32 / opaque as f32
        },
        light_x: pxsmith_core::outline::light_direction(source).x,
        colors_before: palette.len(),
        colors_after: reshaded_palette.len(),
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

fn name_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 2 群を測る．
pub fn run(seeds: &Path, steps: u8) -> Result<Vec<Record>> {
    let mut out: Vec<Record> = Vec::new();
    for path in png_files(seeds)? {
        let Ok(img) = pxsmith_io::png::read_rgba(&path) else {
            continue;
        };
        let Ok((canvas, palette)) = crate::lintcal::index_exactly(&img) else {
            continue;
        };
        let file = name_of(&path);

        let base = dominant(&canvas, &palette);

        for (preset, name) in DIRECTIONAL {
            // 群 1 — 実素材 (手で描かれた絵)．**宣言はその絵から取る**
            if let Some(source) = declared_light(&canvas, &palette)
                && let Some(r) = measure(&Subject {
                    group: "実素材",
                    file: &file,
                    canvas: &canvas,
                    palette: &palette,
                    preset: *preset,
                    name,
                    steps,
                    source,
                    base,
                })
            {
                out.push(r);
            }

            // 群 2 — `pxsmith shade` の出力．**宣言は導出に使った光源そのもの**であり，
            // 再導出も**同じ base ・同じプリセット**で行う — こうしないとパレットが
            // 別物になり，«書き換えた割合» が «色が違う» を数えるだけになる
            let source = preset.default_source();
            let Ok((ramp_palette, model)) =
                build_lighting(base, *preset, steps, ChromaCurve::PeakMiddle)
            else {
                continue;
            };
            let Ok((shaded, shaded_palette)) = shade_to_canvas(
                &mask_of(&canvas),
                source,
                &model,
                &ramp_palette,
                ShadeOptions::default(),
            ) else {
                continue;
            };
            if let Some(r) = measure(&Subject {
                group: "pxsmith shade の出力",
                file: &file,
                canvas: &shaded,
                palette: &shaded_palette,
                preset: *preset,
                name,
                steps,
                source,
                base,
            }) {
                out.push(r);
            }
        }
    }
    Ok(out)
}

/// 群 ・段階ごとに一致度の分布をまとめる．
///
/// 返すのは (群，段階，件数，最小，中央，最大，閾値を下回った件数)．
pub fn summarise(
    records: &[Record],
    threshold: f32,
) -> Vec<(&'static str, &'static str, usize, f32, f32, f32, usize)> {
    let mut out = Vec::new();
    for group in ["実素材", "pxsmith shade の出力"] {
        for (stage, pick) in [
            ("元の絵", 0usize),
            ("反転しただけ", 1),
            ("反転 + 再導出", 2),
        ] {
            let mut values: Vec<f32> = records
                .iter()
                .filter(|r| r.group == group)
                .filter_map(|r| match pick {
                    0 => Some(r.before),
                    1 => Some(r.mirrored),
                    _ => r.reshaded,
                })
                .collect();
            if values.is_empty() {
                continue;
            }
            values.sort_by(f32::total_cmp);
            let below = values.iter().filter(|v| **v < threshold).count();
            out.push((
                group,
                stage,
                values.len(),
                values[0],
                values[values.len() / 2],
                *values.last().expect("空でない"),
                below,
            ));
        }
    }
    out
}

/// 再導出が書き換えた割合を群ごとにまとめる (件数，中央，最大)．
pub fn summarise_rewritten(records: &[Record]) -> Vec<(&'static str, usize, f32, f32)> {
    ["実素材", "pxsmith shade の出力"]
        .into_iter()
        .filter_map(|group| {
            let mut v: Vec<f32> = records
                .iter()
                .filter(|r| r.group == group)
                .map(|r| r.rewritten)
                .collect();
            if v.is_empty() {
                return None;
            }
            v.sort_by(f32::total_cmp);
            Some((group, v.len(), v[v.len() / 2], *v.last().expect("空でない")))
        })
        .collect()
}
