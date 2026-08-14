//! **フレーム間ルール (22 〜 27) の適用範囲と閾値を測る** (`px-calib lintseq`)．
//!
//! 静止画のときと同じ作法で進める (D70 ・D85 〜 D90) — **良い列に掛けて何が
//! 鳴るかを先に見て**から，欠陥を 1 つだけ入れた負例で捕捉率を測る．
//!
//! # 正例には真値がある
//!
//! | 群 | 作り方 | 真値 |
//! | --- | --- | --- |
//! | `translate` | 実素材を**整数画素ずつ平行移動**した 5 コマ | 揺れ 0 ・標数変化 0 ・体積変化 0 |
//! | `tween` | 2 枚から `px anim tween` で中割りを作った 5 コマ | **標数は保証されない** (設計書 6.9．D114 で実測) |
//! | `hold` | **1 枚を並べただけ**の 5 コマ | すべて 0 (動いていないので) |
//!
//! `translate` が «正しい動き» の定義そのものである — 書籍が線揺れの直し方を
//! «選択ツールでパーツをスライドさせる» とするので [^pl]，**平行移動で鳴ったら
//! それは誤爆**である．
//!
//! # 負例は «欠陥を 1 つだけ» 入れる
//!
//! すべて `translate` を元にする．元が鳴らないことが分かっているので，
//! **鳴ったぶんがその欠陥のせい**だと言える．
//!
//! [^pl]: Pixel Logic 第九章 (PAGE:234)．

use anyhow::Result;
use std::path::Path;

use px_core::canvas::IndexedCanvas;
use px_core::frame::{Frame, FrameKind, Layer, LayerMeta, Surface};
use px_core::geom::Mask;
use px_core::math::{IVec2, ivec2, uvec2};
use px_core::palette::Palette;
use px_core::tween::{TweenAlign, TweenOptions, tween_series};
use px_lint::rules::LintConfig;
use px_lint::{SequenceCoverage, lint_sequence};

use crate::animcal::{indexed, name_of, png_files};
use crate::rng::Rng;

/// 列 1 本．
pub struct Sequence {
    pub file: String,
    pub group: &'static str,
    /// 負例が狙っているルール (正例は `None`)．
    #[allow(dead_code)]
    pub target: Option<u8>,
    pub frames: Vec<Frame>,
}

const FRAMES: usize = 5;
/// 1 コマあたりの移動量 (画素)．**整数である** — 平行移動が真値になる条件．
const STEP: i32 = 2;

fn pad(canvas: &IndexedCanvas, margin: u32) -> IndexedCanvas {
    let fill = canvas.transparent().unwrap_or(0);
    canvas.crop(
        px_core::math::IRect {
            x: -(margin as i32),
            y: -(margin as i32),
            w: canvas.width() + margin * 2,
            h: canvas.height() + margin * 2,
        },
        fill,
    )
}

fn frame_of(canvas: IndexedCanvas, palette: &Palette, kind: FrameKind, exclude: bool) -> Frame {
    let mut f = Frame::new(uvec2(canvas.width(), canvas.height()), palette.clone());
    let mut meta = LayerMeta::named("art");
    meta.subpixel_exclude = exclude;
    f.kind = kind;
    f.layers.push(Layer::new(meta, Surface::Indexed(canvas)));
    f
}

/// 添字を平行移動する．空いたところは透明で埋める．
fn shift_canvas(canvas: &IndexedCanvas, d: IVec2) -> IndexedCanvas {
    let fill = canvas.transparent().unwrap_or(0);
    let mut out = IndexedCanvas::filled(canvas.width(), canvas.height(), fill);
    out.set_transparent(canvas.transparent());
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            if let Some(i) = canvas.get(x, y) {
                out.set(x + d.x, y + d.y, i);
            }
        }
    }
    out
}

fn opaque_index(canvas: &IndexedCanvas, palette: &Palette) -> Option<u8> {
    (0..=255u8).find(|i| {
        canvas.pixels().contains(i)
            && canvas.transparent() != Some(*i)
            && palette.get(*i).is_some_and(|c| c.a != 0)
    })
}

// ------------------------------------------------------------------ 正例

/// 平行移動の列．**«正しい動き» の定義そのもの**．
pub fn translate_sequence(file: &str, canvas: &IndexedCanvas, palette: &Palette) -> Sequence {
    let margin = (STEP * FRAMES as i32) as u32;
    let base = pad(canvas, margin);
    let frames = (0..FRAMES)
        .map(|t| {
            let c = shift_canvas(&base, ivec2(STEP * t as i32, 0));
            frame_of(c, palette, FrameKind::Key, false)
        })
        .collect();
    Sequence {
        file: file.to_string(),
        group: "translate",
        target: None,
        frames,
    }
}

/// 1 枚を並べただけ．**動いていないのだから何も鳴ってはいけない**．
pub fn hold_sequence(file: &str, canvas: &IndexedCanvas, palette: &Palette) -> Sequence {
    let base = pad(canvas, 4);
    let frames = (0..FRAMES)
        .map(|_| frame_of(base.clone(), palette, FrameKind::Key, false))
        .collect();
    Sequence {
        file: file.to_string(),
        group: "hold",
        target: None,
        frames,
    }
}

fn mask_of(canvas: &IndexedCanvas, palette: &Palette) -> Mask {
    let mut m = Mask::new(canvas.width(), canvas.height());
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            let Some(i) = canvas.get(x, y) else { continue };
            if canvas.transparent() == Some(i) || palette.get(i).is_some_and(|c| c.a == 0) {
                continue;
            }
            m.set(ivec2(x, y), true);
        }
    }
    m
}

fn paint(mask: &Mask, palette: &Palette, index: u8, transparent: u8) -> IndexedCanvas {
    let mut c = IndexedCanvas::filled(mask.width(), mask.height(), transparent);
    c.set_transparent(Some(transparent));
    let _ = palette;
    for p in mask.iter_set() {
        c.set(p.x, p.y, index);
    }
    c
}

/// `px anim tween` の中割りを含む列．**中の 3 枚に `kind = inbetween` を付ける**．
pub fn tween_sequence(file: &str, canvas: &IndexedCanvas, palette: &Palette) -> Option<Sequence> {
    let margin = (STEP * FRAMES as i32) as u32;
    let base = pad(canvas, margin);
    let index = opaque_index(&base, palette)?;
    let transparent = base.transparent()?;
    let a = mask_of(&base, palette);
    let b = mask_of(&shift_canvas(&base, ivec2(STEP * 4, 0)), palette);
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let opts = TweenOptions {
        align: TweenAlign::Centroid,
        ..Default::default()
    };
    let mid = tween_series(&a, &b, FRAMES as u32 - 2, &opts).ok()?;

    let mut frames = vec![frame_of(
        paint(&a, palette, index, transparent),
        palette,
        FrameKind::Key,
        false,
    )];
    for t in mid {
        frames.push(frame_of(
            paint(&t.mask, palette, index, transparent),
            palette,
            FrameKind::Inbetween,
            false,
        ));
    }
    frames.push(frame_of(
        paint(&b, palette, index, transparent),
        palette,
        FrameKind::Key,
        false,
    ));
    Some(Sequence {
        file: file.to_string(),
        group: "tween",
        target: None,
        frames,
    })
}

/// 市松を塗る．`object_fixed` なら**物体に貼り付ける** (コマの移動量ぶんずらす)．
fn dither_onto(canvas: &IndexedCanvas, other: u8, phase: IVec2) -> IndexedCanvas {
    let transparent = canvas.transparent();
    let mut out = canvas.clone();
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            let Some(i) = canvas.get(x, y) else { continue };
            if transparent == Some(i) {
                continue;
            }
            if (x - phase.x + y - phase.y).rem_euclid(2) == 0 {
                out.set(x, y, other);
            }
        }
    }
    out
}

fn second_index(canvas: &IndexedCanvas, palette: &Palette, not: u8) -> Option<u8> {
    (0..=255u8).find(|i| {
        *i != not
            && canvas.transparent() != Some(*i)
            && palette.get(*i).is_some_and(|c| c.a != 0)
            && canvas.pixels().contains(i)
    })
}

/// **ディザが物体に付いてくる列** (正例)．奇数画素ずつ動かす — 位相が反転しうる
/// 条件を作ったうえで «付いてくれば鳴らない» ことを見るためである (D105)．
pub fn dither_travel_sequence(
    file: &str,
    canvas: &IndexedCanvas,
    palette: &Palette,
) -> Option<Sequence> {
    let margin = FRAMES as u32 + 2;
    let base = pad(canvas, margin);
    let index = opaque_index(&base, palette)?;
    let other = second_index(&base, palette, index)?;
    // **先にディザを塗ってから動かす** = 物体に貼り付いている
    let dithered = dither_onto(&base, other, ivec2(0, 0));
    let frames = (0..FRAMES)
        .map(|t| {
            let c = shift_canvas(&dithered, ivec2(t as i32, 0));
            frame_of(c, palette, FrameKind::Key, false)
        })
        .collect();
    Some(Sequence {
        file: file.to_string(),
        group: "dithertravel",
        target: None,
        frames,
    })
}

/// **ディザが画布に貼り付いた列** (負例)．[`dither_travel_sequence`] と
/// **動きもディザの密度も同じ**で，違うのは «付いてくるかどうか» だけである．
pub fn dither_stuck_sequence(
    file: &str,
    canvas: &IndexedCanvas,
    palette: &Palette,
) -> Option<Sequence> {
    let margin = FRAMES as u32 + 2;
    let base = pad(canvas, margin);
    let index = opaque_index(&base, palette)?;
    let other = second_index(&base, palette, index)?;
    let frames = (0..FRAMES)
        .map(|t| {
            // **動かしてからディザを塗る** = 画布に貼り付いている
            let c = dither_onto(&shift_canvas(&base, ivec2(t as i32, 0)), other, ivec2(0, 0));
            frame_of(c, palette, FrameKind::Key, false)
        })
        .collect();
    Some(Sequence {
        file: file.to_string(),
        group: "ditherstuck",
        target: Some(24),
        frames,
    })
}

/// **道具が作った潰し** (正例)．`px anim squash` の出力が自らの検査に落ちないか．
pub fn squash_sequence(file: &str, canvas: &IndexedCanvas, palette: &Palette) -> Option<Sequence> {
    let base = pad(canvas, 8);
    let amounts = [0.0f32, -0.1, -0.2, -0.1, 0.0];
    let mut frames = Vec::new();
    for a in amounts {
        let (c, _) =
            px_core::deform::squash(&base, a, &px_core::deform::SquashOptions::default()).ok()?;
        frames.push(frame_of(c, palette, FrameKind::Key, false));
    }
    // 画布が揃っていないと比べられないので，一番大きいものへ合わせる
    let (w, h) = frames.iter().fold((0u32, 0u32), |(w, h), f| {
        let c = f.layers[0].surface.as_indexed().expect("添字の画布");
        (w.max(c.width()), h.max(c.height()))
    });
    for f in frames.iter_mut() {
        let c = f.layers[0].surface.as_indexed().expect("添字の画布");
        let fill = c.transparent().unwrap_or(0);
        let grown = c.crop(px_core::math::IRect { x: 0, y: 0, w, h }, fill);
        f.size = uvec2(w, h);
        f.layers[0].surface = Surface::Indexed(grown);
    }
    Some(Sequence {
        file: file.to_string(),
        group: "squash",
        target: None,
        frames,
    })
}

// ------------------------------------------------------------------ 負例

/// 欠陥の種類．**平行移動の列に 1 つだけ入れる**．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SeqDefect {
    /// 輪郭を 1 画素ずつ出したり入れたりする (線揺れ)．
    Wobble,
    /// 中割りに穴を開ける (トポロジー変化)．
    Topology,
    /// 新しくできた列を 1 ドットまで削る．
    OrphanColumn,
    /// 除外レイヤをずらす．
    Exclusion,
    /// 片方の辺を伸ばしもう片方を縮めつつ体積を変える．
    Volume,
}

impl SeqDefect {
    pub const ALL: [SeqDefect; 5] = [
        SeqDefect::Wobble,
        SeqDefect::Topology,
        SeqDefect::OrphanColumn,
        SeqDefect::Exclusion,
        SeqDefect::Volume,
    ];

    pub fn rule(self) -> u8 {
        match self {
            Self::Topology => 22,
            Self::Wobble => 23,
            Self::OrphanColumn => 25,
            Self::Exclusion => 26,
            Self::Volume => 27,
        }
    }

    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wobble => "wobble",
            Self::Topology => "topology",
            Self::OrphanColumn => "orphan",
            Self::Exclusion => "exclusion",
            Self::Volume => "volume",
        }
    }
}

fn boundary_pixels(canvas: &IndexedCanvas, palette: &Palette) -> Vec<IVec2> {
    let m = mask_of(canvas, palette);
    m.iter_set().filter(|p| m.is_boundary(*p)).collect()
}

/// 平行移動の列に欠陥を 1 つ入れる．
pub fn corrupt(
    base: &Sequence,
    defect: SeqDefect,
    palette: &Palette,
    seed: u64,
) -> Option<Sequence> {
    let mut frames = base.frames.clone();
    let mut rng = Rng::new(seed);

    match defect {
        SeqDefect::Wobble => {
            // **コマごとに輪郭の一部を出したり入れたりする** — 行きつ戻りつ
            // させるので，偶奇で向きを変える
            for (t, f) in frames.iter_mut().enumerate() {
                let canvas = f.layers[0].surface.as_indexed()?.clone();
                let transparent = canvas.transparent()?;
                let index = opaque_index(&canvas, palette)?;
                let edge = boundary_pixels(&canvas, palette);
                if edge.is_empty() {
                    return None;
                }
                let mut next = canvas.clone();
                for (k, p) in edge.iter().enumerate() {
                    // 1 / 4 の画素だけ触る．偶数コマと奇数コマで逆向きにする
                    if !(k + rng.below(4) as usize).is_multiple_of(4) {
                        continue;
                    }
                    if t % 2 == 0 {
                        next.set(p.x, p.y, transparent);
                    } else {
                        next.set(p.x, p.y, index);
                    }
                }
                f.layers[0].surface = Surface::Indexed(next);
            }
        }
        SeqDefect::Topology => {
            // 中の 1 枚に穴を開け，`inbetween` の印を付ける
            let t = frames.len() / 2;
            let canvas = frames[t].layers[0].surface.as_indexed()?.clone();
            let transparent = canvas.transparent()?;
            let m = mask_of(&canvas, palette);
            let bbox = m.bbox()?;
            let mut next = canvas.clone();
            let c = ivec2(bbox.x + bbox.w as i32 / 2, bbox.y + bbox.h as i32 / 2);
            let mut opened = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if next.get(c.x + dx, c.y + dy).is_some() {
                        next.set(c.x + dx, c.y + dy, transparent);
                        opened += 1;
                    }
                }
            }
            if opened == 0 {
                return None;
            }
            frames[t].layers[0].surface = Surface::Indexed(next);
            for f in frames.iter_mut().skip(1).take(FRAMES - 2) {
                f.kind = FrameKind::Inbetween;
            }
        }
        SeqDefect::OrphanColumn => {
            // **新しくできた列を 1 ドットまで削る**
            for t in 1..frames.len() {
                let canvas = frames[t].layers[0].surface.as_indexed()?.clone();
                let previous = frames[t - 1].layers[0].surface.as_indexed()?.clone();
                let transparent = canvas.transparent()?;
                let mut next = canvas.clone();
                for x in 0..canvas.width() as i32 {
                    let was = (0..canvas.height() as i32)
                        .any(|y| previous.get(x, y).is_some_and(|i| i != transparent));
                    let now: Vec<i32> = (0..canvas.height() as i32)
                        .filter(|y| canvas.get(x, *y).is_some_and(|i| i != transparent))
                        .collect();
                    if !was && now.len() > 1 {
                        for y in now.iter().skip(1) {
                            next.set(x, *y, transparent);
                        }
                    }
                }
                frames[t].layers[0].surface = Surface::Indexed(next);
            }
        }
        SeqDefect::Exclusion => {
            // 除外レイヤを足し，コマごとにずらす (**動いてはいけない層が動く**)
            for (t, f) in frames.iter_mut().enumerate() {
                let canvas = f.layers[0].surface.as_indexed()?.clone();
                let face = shift_canvas(&canvas, ivec2(t as i32, 0));
                let mut meta = LayerMeta::named("face");
                meta.subpixel_exclude = true;
                f.layers.push(Layer::new(meta, Surface::Indexed(face)));
            }
        }
        SeqDefect::Volume => {
            // **片方の辺を伸ばしもう片方を縮めるが，体積は保たない**
            for (t, f) in frames.iter_mut().enumerate() {
                if t == 0 {
                    continue;
                }
                let canvas = f.layers[0].surface.as_indexed()?.clone();
                let transparent = canvas.transparent()?;
                let m = mask_of(&canvas, palette);
                let bbox = m.bbox()?;
                let mut next = IndexedCanvas::filled(canvas.width(), canvas.height(), transparent);
                next.set_transparent(Some(transparent));
                // 横 (1 + 0.15 t) 倍 ・縦 (1 - 0.05 t) 倍 — 積が 1 にならない
                let (sx, sy) = (1.0 + 0.15 * t as f32, 1.0 - 0.05 * t as f32);
                for y in 0..canvas.height() as i32 {
                    for x in 0..canvas.width() as i32 {
                        let Some(i) = canvas.get(x, y) else { continue };
                        if i == transparent {
                            continue;
                        }
                        let nx = bbox.x + (((x - bbox.x) as f32) * sx).round() as i32;
                        let ny = bbox.y + (((y - bbox.y) as f32) * sy).round() as i32;
                        next.set(nx, ny, i);
                    }
                }
                f.layers[0].surface = Surface::Indexed(next);
            }
        }
    }

    Some(Sequence {
        file: base.file.clone(),
        group: match defect {
            SeqDefect::Wobble => "wobble",
            SeqDefect::Topology => "topology",
            SeqDefect::OrphanColumn => "orphan",
            SeqDefect::Exclusion => "exclusion",
            SeqDefect::Volume => "volume",
        },
        target: Some(defect.rule()),
        frames,
    })
}

// ------------------------------------------------------------------ 測る

/// 飛ばした件の内訳 (D128)．
#[derive(Clone, Debug, Default)]
pub struct SeqSkipped {
    pub not_indexable: usize,
    pub empty: usize,
    /// 欠陥を入れられなかった (絵，欠陥) の組．
    pub not_corrupted: usize,
    /// 中割りを作れなかった絵．
    pub no_tween: usize,
}

/// 列を作る．
pub fn build(dir: &Path) -> Result<(Vec<Sequence>, Vec<Palette>, SeqSkipped)> {
    let mut out = Vec::new();
    let mut palettes = Vec::new();
    let mut skipped = SeqSkipped::default();

    for path in png_files(dir)? {
        let file = name_of(&path);
        let Some((canvas, palette)) = indexed(&path) else {
            skipped.not_indexable += 1;
            continue;
        };
        if canvas.transparent().is_none() || opaque_index(&canvas, &palette).is_none() {
            skipped.empty += 1;
            continue;
        }

        let translate = translate_sequence(&file, &canvas, &palette);
        for defect in SeqDefect::ALL {
            match corrupt(&translate, defect, &palette, 0) {
                Some(s) => {
                    out.push(s);
                    palettes.push(palette.clone());
                }
                None => skipped.not_corrupted += 1,
            }
        }
        out.push(translate);
        palettes.push(palette.clone());
        out.push(hold_sequence(&file, &canvas, &palette));
        palettes.push(palette.clone());
        if let Some(seq) = dither_stuck_sequence(&file, &canvas, &palette) {
            out.push(seq);
            palettes.push(palette.clone());
        }
        if let Some(seq) = dither_travel_sequence(&file, &canvas, &palette) {
            out.push(seq);
            palettes.push(palette.clone());
        }
        if let Some(seq) = squash_sequence(&file, &canvas, &palette) {
            out.push(seq);
            palettes.push(palette.clone());
        }
        match tween_sequence(&file, &canvas, &palette) {
            Some(s) => {
                out.push(s);
                palettes.push(palette.clone());
            }
            None => skipped.no_tween += 1,
        }
    }
    Ok((out, palettes, skipped))
}

/// 群ごとに «どのルールが何本の列で鳴ったか» を数える．
pub fn measure(sequences: &[Sequence], cfg: &LintConfig) -> Vec<(String, usize, [usize; 6])> {
    let mut groups: std::collections::BTreeMap<String, (usize, [usize; 6])> =
        std::collections::BTreeMap::new();
    for s in sequences {
        let (report, _) = lint_sequence(&s.frames, cfg);
        let entry = groups.entry(s.group.to_string()).or_insert((0, [0; 6]));
        entry.0 += 1;
        for (k, id) in (22..=27u8).enumerate() {
            if report.violations.iter().any(|v| v.rule == id) {
                entry.1[k] += 1;
            }
        }
    }
    groups
        .into_iter()
        .map(|(g, (n, counts))| (g, n, counts))
        .collect()
}

/// 閾値を掃く．返り値は (値, 正例で鳴った本数, 正例の本数, 負例で捕捉, 負例の本数)．
///
/// 正例は `translate` ・`tween` ・`hold` ・`dithertravel` ・**`squash`** —
/// `squash` は**道具が作ったもの**なので，ここが鳴るなら «自分の出力が自分の
/// 検査に落ちる» ということである (D58)．
pub fn sweep(
    sequences: &[Sequence],
    rule: u8,
    values: &[f32],
    set: impl Fn(&mut LintConfig, f32),
) -> Vec<(f32, usize, usize, usize, usize)> {
    let positives = ["translate", "tween", "hold", "dithertravel", "squash"];
    let negative = match rule {
        23 => "wobble",
        24 => "ditherstuck",
        27 => "volume",
        _ => return Vec::new(),
    };
    values
        .iter()
        .map(|&v| {
            let mut cfg = LintConfig::default();
            set(&mut cfg, v);
            let (mut fp, mut np, mut tp, mut nn) = (0, 0, 0, 0);
            for s in sequences {
                let hit = {
                    let (report, _) = lint_sequence(&s.frames, &cfg);
                    report.violations.iter().any(|x| x.rule == rule)
                };
                if positives.contains(&s.group) {
                    np += 1;
                    if hit {
                        fp += 1;
                    }
                } else if s.group == negative {
                    nn += 1;
                    if hit {
                        tp += 1;
                    }
                }
            }
            (v, fp, np, tp, nn)
        })
        .collect()
}

/// 群ごとに «正例のうち道具が作ったもの» を分けて数える．
pub fn positive_groups() -> [&'static str; 5] {
    ["translate", "tween", "hold", "dithertravel", "squash"]
}

/// 検査できなかったものを数える．
pub fn coverage(sequences: &[Sequence], cfg: &LintConfig) -> Vec<(String, SequenceCoverage)> {
    sequences
        .iter()
        .map(|s| {
            let (_, cov) = lint_sequence(&s.frames, cfg);
            (s.group.to_string(), cov)
        })
        .collect()
}
