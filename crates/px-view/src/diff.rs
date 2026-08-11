//! 2 枚の差分 (`px diff`)．
//!
//! M1 の完了条件は「変化ピクセル数と位置を表示」である．**位置は 1 画素単位で
//! 数え上げる** — ドット絵では 1 画素の違いが意味を持つので，要約統計だけでは
//! 確認に使えない．

use px_core::canvas::IndexedCanvas;
use px_core::math::{IRect, IVec2, ivec2};

use crate::{Result, ViewError};

/// 1 画素の変化．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PixelChange {
    pub at: IVec2,
    pub before: u8,
    pub after: u8,
}

/// 差分の結果．
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Diff {
    pub changes: Vec<PixelChange>,
    pub total: usize,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn count(&self) -> usize {
        self.changes.len()
    }

    /// 変化した画素の割合．
    pub fn ratio(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.changes.len() as f32 / self.total as f32
        }
    }

    /// 変化を囲う最小の矩形．変化が無ければ `None`．
    pub fn bbox(&self) -> Option<IRect> {
        let first = self.changes.first()?;
        let (mut x0, mut y0) = (first.at.x, first.at.y);
        let (mut x1, mut y1) = (first.at.x, first.at.y);
        for c in &self.changes {
            x0 = x0.min(c.at.x);
            y0 = y0.min(c.at.y);
            x1 = x1.max(c.at.x);
            y1 = y1.max(c.at.y);
        }
        Some(IRect::new(
            x0,
            y0,
            (x1 - x0 + 1) as u32,
            (y1 - y0 + 1) as u32,
        ))
    }
}

/// 2 枚のインデックスキャンバスを比べる．走査順は上から下，左から右で固定する．
pub fn diff_indexed(a: &IndexedCanvas, b: &IndexedCanvas) -> Result<Diff> {
    if a.size() != b.size() {
        return Err(ViewError::SizeMismatch {
            a: (a.width(), a.height()),
            b: (b.width(), b.height()),
        });
    }
    let mut changes = Vec::new();
    for y in 0..a.height() as i32 {
        for x in 0..a.width() as i32 {
            let (before, after) = (a.get(x, y), b.get(x, y));
            if before != after {
                changes.push(PixelChange {
                    at: ivec2(x, y),
                    before: before.unwrap_or_default(),
                    after: after.unwrap_or_default(),
                });
            }
        }
    }
    Ok(Diff {
        changes,
        total: a.size().area(),
    })
}

/// 差分を人が読める形に整える．`limit` を超えた分は件数だけ示す．
pub fn format(diff: &Diff, limit: usize) -> String {
    if diff.is_empty() {
        return "差分なし".to_string();
    }
    let mut out = format!(
        "{} / {} 画素が変化 ({:.2}%)",
        diff.count(),
        diff.total,
        diff.ratio() * 100.0
    );
    if let Some(b) = diff.bbox() {
        out.push_str(&format!("\n範囲: ({}, {}) から {}x{}", b.x, b.y, b.w, b.h));
    }
    out.push('\n');
    for c in diff.changes.iter().take(limit) {
        out.push_str(&format!(
            "  ({:>3}, {:>3})  {:>3} -> {:>3}\n",
            c.at.x, c.at.y, c.before, c.after
        ));
    }
    if diff.count() > limit {
        out.push_str(&format!("  … 他 {} 件\n", diff.count() - limit));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(pixels: Vec<u8>) -> IndexedCanvas {
        IndexedCanvas::from_pixels(3, 2, pixels).unwrap()
    }

    #[test]
    fn identical_canvases_have_no_changes() {
        let a = canvas(vec![0, 1, 2, 3, 4, 5]);
        let d = diff_indexed(&a, &a).unwrap();
        assert!(d.is_empty());
        assert_eq!(d.ratio(), 0.0);
        assert_eq!(d.bbox(), None);
        assert_eq!(format(&d, 10), "差分なし");
    }

    #[test]
    fn reports_position_and_both_indices() {
        let a = canvas(vec![0, 1, 2, 3, 4, 5]);
        let b = canvas(vec![0, 9, 2, 3, 4, 5]);
        let d = diff_indexed(&a, &b).unwrap();
        assert_eq!(d.count(), 1);
        assert_eq!(
            d.changes[0],
            PixelChange {
                at: ivec2(1, 0),
                before: 1,
                after: 9
            }
        );
    }

    #[test]
    fn scan_order_is_row_major() {
        let a = canvas(vec![0, 0, 0, 0, 0, 0]);
        let b = canvas(vec![1, 0, 0, 0, 1, 0]);
        let d = diff_indexed(&a, &b).unwrap();
        assert_eq!(
            d.changes.iter().map(|c| c.at).collect::<Vec<_>>(),
            vec![ivec2(0, 0), ivec2(1, 1)]
        );
    }

    #[test]
    fn bbox_covers_every_change() {
        let a = canvas(vec![0; 6]);
        let b = canvas(vec![0, 1, 0, 0, 0, 1]);
        let d = diff_indexed(&a, &b).unwrap();
        assert_eq!(d.bbox(), Some(IRect::new(1, 0, 2, 2)));
    }

    #[test]
    fn size_mismatch_is_an_error() {
        let a = canvas(vec![0; 6]);
        let b = IndexedCanvas::filled(2, 2, 0);
        assert!(matches!(
            diff_indexed(&a, &b).unwrap_err(),
            ViewError::SizeMismatch { .. }
        ));
    }

    #[test]
    fn format_truncates_but_reports_the_total() {
        let a = canvas(vec![0; 6]);
        let b = canvas(vec![1; 6]);
        let text = format(&diff_indexed(&a, &b).unwrap(), 2);
        assert!(text.contains("6 / 6 画素が変化"), "{text}");
        assert!(text.contains("他 4 件"), "{text}");
    }
}
