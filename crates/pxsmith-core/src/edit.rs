//! 編集操作とパッチ (設計書 3.6)．
//!
//! これらが支えるのは「将来 TUI/GUI を載せる際に `pxsmith-core` を改変しなくてよい」の
//! 1 点のみである．レシピの再現性は差分ビルドが担保する．
//!
//! 適用先は作業層のフレーム列 (`&mut [Frame]`) とし，[`FrameId`] がその添字，
//! [`LayerId`] がフレーム内のレイヤ添字を指す．

use std::collections::VecDeque;

use crate::canvas::IndexedCanvas;
use crate::color::Rgba8;
use crate::error::{CoreError, Result};
use crate::frame::Frame;
use crate::ink::{Brush, FillOpts, Ink};
use crate::math::{IRect, IVec2, ivec2, line};

/// フレーム列における添字．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FrameId(pub usize);

/// フレーム内のレイヤ添字．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(pub usize);

/// 編集操作．
#[derive(Clone, Debug, PartialEq)]
pub enum EditOp {
    DrawStroke {
        layer: LayerId,
        frame: FrameId,
        pts: Vec<IVec2>,
        brush: Brush,
        ink: Ink,
    },
    FloodFill {
        layer: LayerId,
        frame: FrameId,
        seed: IVec2,
        ink: Ink,
        opts: FillOpts,
    },
    /// パレット全体の差し替え．フレーム列の全フレームに適用する．
    SetPalette { entries: Vec<Rgba8> },
}

/// 取り消し可能な差分．
#[derive(Clone, Debug, PartialEq)]
pub enum Patch {
    Pixels {
        layer: LayerId,
        frame: FrameId,
        region: IRect,
        before: Vec<u8>,
        after: Vec<u8>,
    },
    Palette {
        before: Vec<Rgba8>,
        after: Vec<Rgba8>,
    },
}

impl Patch {
    /// 何も変えないパッチ．
    pub fn empty(layer: LayerId, frame: FrameId) -> Self {
        Self::Pixels {
            layer,
            frame,
            region: IRect::default(),
            before: Vec::new(),
            after: Vec::new(),
        }
    }

    pub fn is_noop(&self) -> bool {
        match self {
            Self::Pixels { before, after, .. } => before == after,
            Self::Palette { before, after } => before == after,
        }
    }

    /// 前向きに適用する．適用先が記録時の `before` と一致しなければエラー．
    pub fn apply(&self, frames: &mut [Frame]) -> Result<()> {
        match self {
            Self::Pixels {
                layer,
                frame,
                region,
                before,
                after,
            } => write_region(frames, *frame, *layer, *region, before, after),
            Self::Palette { before, after } => write_palette(frames, before, after),
        }
    }

    /// 逆向きに適用する．適用先が記録時の `after` と一致しなければエラー．
    pub fn revert(&self, frames: &mut [Frame]) -> Result<()> {
        match self {
            Self::Pixels {
                layer,
                frame,
                region,
                before,
                after,
            } => write_region(frames, *frame, *layer, *region, after, before),
            Self::Palette { before, after } => write_palette(frames, after, before),
        }
    }
}

fn indexed_mut(frames: &mut [Frame], frame: FrameId, layer: LayerId) -> Result<&mut IndexedCanvas> {
    let len = frames.len();
    let f = frames.get_mut(frame.0).ok_or(CoreError::FrameOutOfRange {
        index: frame.0,
        len,
    })?;
    let layer_count = f.layers.len();
    let l = f
        .layers
        .get_mut(layer.0)
        .ok_or(CoreError::LayerOutOfRange {
            index: layer.0,
            len: layer_count,
        })?;
    let name = l.meta.name.clone();
    l.surface
        .as_indexed_mut()
        .ok_or(CoreError::NotIndexed { name })
}

fn read_region(canvas: &IndexedCanvas, region: IRect) -> Vec<u8> {
    region
        .iter()
        .map(|p| canvas.get_at(p).unwrap_or_default())
        .collect()
}

fn write_region(
    frames: &mut [Frame],
    frame: FrameId,
    layer: LayerId,
    region: IRect,
    expect: &[u8],
    write: &[u8],
) -> Result<()> {
    if region.is_empty() {
        return Ok(());
    }
    let canvas = indexed_mut(frames, frame, layer)?;
    if read_region(canvas, region) != expect {
        return Err(CoreError::PatchMismatch);
    }
    for (i, p) in region.iter().enumerate() {
        canvas.set_at(p, write[i]);
    }
    Ok(())
}

fn write_palette(frames: &mut [Frame], expect: &[Rgba8], write: &[Rgba8]) -> Result<()> {
    if frames.iter().any(|f| f.palette.entries() != expect) {
        return Err(CoreError::PatchMismatch);
    }
    for f in frames.iter_mut() {
        f.palette.replace_entries(write.to_vec())?;
    }
    Ok(())
}

impl EditOp {
    /// 操作を適用し，取り消し用のパッチを返す．
    pub fn apply(&self, frames: &mut [Frame]) -> Result<Patch> {
        match self {
            Self::DrawStroke {
                layer,
                frame,
                pts,
                brush,
                ink,
            } => {
                let targets = stroke_pixels(pts, *brush);
                paint(frames, *frame, *layer, &targets, ink)
            }
            Self::FloodFill {
                layer,
                frame,
                seed,
                ink,
                opts,
            } => {
                let canvas = indexed_mut(frames, *frame, *layer)?;
                let targets = fill_pixels(canvas, *seed, *opts);
                paint(frames, *frame, *layer, &targets, ink)
            }
            Self::SetPalette { entries } => {
                let before = match frames.first() {
                    Some(f) => f.palette.entries().to_vec(),
                    None => Vec::new(),
                };
                // 全フレームが同じパレットを持つ前提でなければ，取り消しが破壊的になる．
                if frames.iter().any(|f| f.palette.entries() != before) {
                    return Err(CoreError::PatchMismatch);
                }
                let patch = Patch::Palette {
                    before,
                    after: entries.clone(),
                };
                patch.apply(frames)?;
                Ok(patch)
            }
        }
    }
}

/// ストロークが触れる画素の列 (重複を含む．描画順は決定論的)．
fn stroke_pixels(pts: &[IVec2], brush: Brush) -> Vec<IVec2> {
    let offsets = brush.offsets();
    let centers: Vec<IVec2> = match pts {
        [] => Vec::new(),
        [only] => vec![*only],
        _ => {
            let mut v = vec![pts[0]];
            for w in pts.windows(2) {
                // 直線の始点は前の区間の終点と重なるので落とす
                v.extend(line(w[0], w[1]).into_iter().skip(1));
            }
            v
        }
    };
    centers
        .into_iter()
        .flat_map(|c| offsets.iter().map(move |&o| c + o))
        .collect()
}

/// 塗りつぶしの対象画素．
fn fill_pixels(canvas: &IndexedCanvas, seed: IVec2, opts: FillOpts) -> Vec<IVec2> {
    let Some(target) = canvas.get_at(seed) else {
        return Vec::new();
    };
    if !opts.contiguous {
        return canvas
            .bounds()
            .iter()
            .filter(|&p| canvas.get_at(p) == Some(target))
            .collect();
    }

    let w = canvas.width() as usize;
    let mut seen = vec![false; w * canvas.height() as usize];
    let mut queue = VecDeque::from([seed]);
    let mut out = Vec::new();
    let neighbors: &[IVec2] = if opts.diagonal {
        &[
            ivec2(-1, -1),
            ivec2(0, -1),
            ivec2(1, -1),
            ivec2(-1, 0),
            ivec2(1, 0),
            ivec2(-1, 1),
            ivec2(0, 1),
            ivec2(1, 1),
        ]
    } else {
        &[ivec2(0, -1), ivec2(-1, 0), ivec2(1, 0), ivec2(0, 1)]
    };

    while let Some(p) = queue.pop_front() {
        if canvas.get_at(p) != Some(target) {
            continue;
        }
        let i = p.y as usize * w + p.x as usize;
        if seen[i] {
            continue;
        }
        seen[i] = true;
        out.push(p);
        for &d in neighbors {
            queue.push_back(p + d);
        }
    }
    // 走査順に依存しない決定論的な順序へ揃える (設計書 6.15 規則 1)
    out.sort_unstable_by_key(|p| (p.y, p.x));
    out
}

/// 対象画素へインクを乗せ，差分をパッチとして返す．
fn paint(
    frames: &mut [Frame],
    frame: FrameId,
    layer: LayerId,
    targets: &[IVec2],
    ink: &Ink,
) -> Result<Patch> {
    let canvas = indexed_mut(frames, frame, layer)?;
    let bounds = canvas.bounds();

    let writes: Vec<(IVec2, u8)> = targets
        .iter()
        .filter(|p| bounds.contains(**p))
        .filter_map(|&p| ink.resolve(p).map(|v| (p, v)))
        .collect();

    let Some(region) = bbox_of(writes.iter().map(|(p, _)| *p)) else {
        return Ok(Patch::empty(layer, frame));
    };

    let before = read_region(canvas, region);
    for &(p, v) in &writes {
        canvas.set_at(p, v);
    }
    let after = read_region(canvas, region);

    Ok(Patch::Pixels {
        layer,
        frame,
        region,
        before,
        after,
    })
}

fn bbox_of(points: impl Iterator<Item = IVec2>) -> Option<IRect> {
    let (mut x0, mut y0) = (i32::MAX, i32::MAX);
    let (mut x1, mut y1) = (i32::MIN, i32::MIN);
    let mut any = false;
    for p in points {
        any = true;
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    any.then(|| IRect::new(x0, y0, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::IndexedCanvas;
    use crate::frame::{Layer, LayerMeta, Surface};
    use crate::ink::PatternMask;
    use crate::math::UVec2;
    use crate::palette::Palette;

    fn frames() -> Vec<Frame> {
        let palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(255, 0, 0),
            Rgba8::rgb(0, 255, 0),
        ])
        .unwrap();
        let mut f = Frame::new(UVec2 { x: 8, y: 8 }, palette);
        f.layers.push(Layer::new(
            LayerMeta::named("body"),
            Surface::Indexed(IndexedCanvas::filled(8, 8, 0).with_transparent(Some(0))),
        ));
        vec![f]
    }

    fn pixels(fs: &[Frame]) -> Vec<u8> {
        fs[0].layers[0]
            .surface
            .as_indexed()
            .unwrap()
            .pixels()
            .to_vec()
    }

    fn assert_round_trip(op: EditOp) {
        let mut fs = frames();
        let original = fs.clone();
        let patch = op.apply(&mut fs).unwrap();
        assert_ne!(fs, original, "操作が何も変えていない: {op:?}");
        patch.revert(&mut fs).unwrap();
        assert_eq!(fs, original, "revert で元に戻らない: {op:?}");
        patch.apply(&mut fs).unwrap();
        assert_ne!(fs, original, "apply で再適用できない: {op:?}");
    }

    #[test]
    fn draw_stroke_round_trips() {
        assert_round_trip(EditOp::DrawStroke {
            layer: LayerId(0),
            frame: FrameId(0),
            pts: vec![ivec2(1, 1), ivec2(6, 4), ivec2(2, 7)],
            brush: Brush::Square { size: 2 },
            ink: Ink::Index(1),
        });
    }

    #[test]
    fn flood_fill_round_trips() {
        assert_round_trip(EditOp::FloodFill {
            layer: LayerId(0),
            frame: FrameId(0),
            seed: ivec2(0, 0),
            ink: Ink::Index(2),
            opts: FillOpts::default(),
        });
    }

    #[test]
    fn pattern_ink_round_trips() {
        assert_round_trip(EditOp::DrawStroke {
            layer: LayerId(0),
            frame: FrameId(0),
            pts: vec![ivec2(0, 0), ivec2(7, 7)],
            brush: Brush::Circle { radius: 2 },
            ink: Ink::Pattern {
                mask: PatternMask::Bayer { order: 4, level: 8 },
                a: 1,
                b: 2,
            },
        });
    }

    #[test]
    fn set_palette_round_trips() {
        assert_round_trip(EditOp::SetPalette {
            entries: vec![Rgba8::TRANSPARENT, Rgba8::rgb(1, 2, 3), Rgba8::rgb(4, 5, 6)],
        });
    }

    #[test]
    fn stroke_is_eight_connected_and_clipped() {
        let mut fs = frames();
        EditOp::DrawStroke {
            layer: LayerId(0),
            frame: FrameId(0),
            // 一部がキャンバス外
            pts: vec![ivec2(-4, 3), ivec2(11, 3)],
            brush: Brush::Pixel,
            ink: Ink::Index(1),
        }
        .apply(&mut fs)
        .unwrap();
        let c = fs[0].layers[0].surface.as_indexed().unwrap();
        for x in 0..8 {
            assert_eq!(c.get(x, 3), Some(1), "x={x} が塗られていない");
        }
        assert_eq!(c.get(0, 2), Some(0));
    }

    #[test]
    fn flood_fill_respects_barriers() {
        let mut fs = frames();
        // 縦の壁を引いて左右に分ける
        EditOp::DrawStroke {
            layer: LayerId(0),
            frame: FrameId(0),
            pts: vec![ivec2(4, 0), ivec2(4, 7)],
            brush: Brush::Pixel,
            ink: Ink::Index(1),
        }
        .apply(&mut fs)
        .unwrap();
        EditOp::FloodFill {
            layer: LayerId(0),
            frame: FrameId(0),
            seed: ivec2(0, 0),
            ink: Ink::Index(2),
            opts: FillOpts::default(),
        }
        .apply(&mut fs)
        .unwrap();
        let c = fs[0].layers[0].surface.as_indexed().unwrap();
        assert_eq!(c.get(0, 0), Some(2));
        assert_eq!(c.get(3, 7), Some(2));
        assert_eq!(c.get(5, 0), Some(0), "壁の向こうは塗られない");
    }

    #[test]
    fn non_contiguous_fill_reaches_across_barriers() {
        let mut fs = frames();
        EditOp::DrawStroke {
            layer: LayerId(0),
            frame: FrameId(0),
            pts: vec![ivec2(4, 0), ivec2(4, 7)],
            brush: Brush::Pixel,
            ink: Ink::Index(1),
        }
        .apply(&mut fs)
        .unwrap();
        EditOp::FloodFill {
            layer: LayerId(0),
            frame: FrameId(0),
            seed: ivec2(0, 0),
            ink: Ink::Index(2),
            opts: FillOpts {
                contiguous: false,
                diagonal: false,
            },
        }
        .apply(&mut fs)
        .unwrap();
        let c = fs[0].layers[0].surface.as_indexed().unwrap();
        assert_eq!(c.get(5, 0), Some(2));
        assert_eq!(c.get(4, 0), Some(1), "壁そのものは残る");
    }

    #[test]
    fn patch_apply_detects_stale_target() {
        let mut fs = frames();
        let patch = EditOp::DrawStroke {
            layer: LayerId(0),
            frame: FrameId(0),
            pts: vec![ivec2(2, 2)],
            brush: Brush::Pixel,
            ink: Ink::Index(1),
        }
        .apply(&mut fs)
        .unwrap();
        // 適用済みの状態にもう一度 apply すると before と食い違う
        assert!(matches!(
            patch.apply(&mut fs).unwrap_err(),
            CoreError::PatchMismatch
        ));
    }

    #[test]
    fn stamp_ink_leaving_holes_still_round_trips() {
        let stamp = IndexedCanvas::from_pixels(2, 2, vec![0, 1, 1, 0])
            .unwrap()
            .with_transparent(Some(0));
        assert_round_trip(EditOp::DrawStroke {
            layer: LayerId(0),
            frame: FrameId(0),
            pts: vec![ivec2(1, 1), ivec2(6, 6)],
            brush: Brush::Square { size: 3 },
            ink: Ink::Stamp(stamp),
        });
    }

    #[test]
    fn out_of_range_ids_are_errors() {
        let mut fs = frames();
        let op = EditOp::DrawStroke {
            layer: LayerId(9),
            frame: FrameId(0),
            pts: vec![ivec2(0, 0)],
            brush: Brush::Pixel,
            ink: Ink::Index(1),
        };
        assert!(matches!(
            op.apply(&mut fs).unwrap_err(),
            CoreError::LayerOutOfRange { .. }
        ));
        let op = EditOp::DrawStroke {
            layer: LayerId(0),
            frame: FrameId(9),
            pts: vec![ivec2(0, 0)],
            brush: Brush::Pixel,
            ink: Ink::Index(1),
        };
        assert!(matches!(
            op.apply(&mut fs).unwrap_err(),
            CoreError::FrameOutOfRange { .. }
        ));
    }

    #[test]
    fn empty_stroke_is_a_noop_patch() {
        let mut fs = frames();
        let before = pixels(&fs);
        let patch = EditOp::DrawStroke {
            layer: LayerId(0),
            frame: FrameId(0),
            pts: vec![],
            brush: Brush::Pixel,
            ink: Ink::Index(1),
        }
        .apply(&mut fs)
        .unwrap();
        assert!(patch.is_noop());
        assert_eq!(pixels(&fs), before);
        patch.revert(&mut fs).unwrap();
        assert_eq!(pixels(&fs), before);
    }
}
