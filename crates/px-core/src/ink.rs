//! 描画インクとブラシ (設計書 3.4)．
//!
//! 描画プリミティブは持つが入力デバイスは持たない (中心となる設計判断 2)．

use crate::canvas::IndexedCanvas;
use crate::math::{IVec2, ivec2};

/// 2 色パターンのマスク (D11)．表現力はパターンまでに限定し，2 色を超えない．
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternMask {
    /// 市松．`size` はマスの一辺．
    Checker { size: u8 },
    /// Bayer 順序ディザ．`order` は行列の一辺 (2 / 4 / 8)，`level` は閾値．
    Bayer { order: u8, level: u8 },
    /// 任意ビットパターン．`bits` は 1 行 1 ワード．
    Custom { w: u8, h: u8, bits: Vec<u64> },
}

impl PatternMask {
    /// この座標が `a` 側 (真) か `b` 側 (偽) か．
    pub fn is_a(&self, p: IVec2) -> bool {
        match self {
            Self::Checker { size } => {
                let s = (*size).max(1) as i32;
                let cx = p.x.div_euclid(s);
                let cy = p.y.div_euclid(s);
                (cx + cy).rem_euclid(2) == 0
            }
            Self::Bayer { order, level } => {
                let n = bayer_side(*order);
                let t = bayer_value(
                    n,
                    p.x.rem_euclid(n as i32) as u32,
                    p.y.rem_euclid(n as i32) as u32,
                );
                t >= *level as u32
            }
            Self::Custom { w, h, bits } => {
                if *w == 0 || *h == 0 || bits.is_empty() {
                    return true;
                }
                let x = p.x.rem_euclid(*w as i32) as u32;
                let y = p.y.rem_euclid(*h as i32) as u32;
                let row = bits[y as usize % bits.len()];
                (row >> (x % 64)) & 1 == 1
            }
        }
    }
}

/// Bayer 行列の一辺．`order` は 2 / 4 / 8 のいずれかへ丸める．
fn bayer_side(order: u8) -> u32 {
    match order {
        0..=2 => 2,
        3 | 4 => 4,
        _ => 8,
    }
}

/// $n \times n$ Bayer 行列の値 ($0 \le v < n^2$)．
///
/// $M_{2n} = \begin{pmatrix} 4M_n & 4M_n + 2 \\ 4M_n + 3 & 4M_n + 1 \end{pmatrix}$ の再帰で作る．
fn bayer_value(n: u32, x: u32, y: u32) -> u32 {
    if n <= 1 {
        return 0;
    }
    let half = n / 2;
    let quadrant = match (x >= half, y >= half) {
        (false, false) => 0,
        (true, false) => 2,
        (false, true) => 3,
        (true, true) => 1,
    };
    4 * bayer_value(half, x % half, y % half) + quadrant
}

/// 描画インク．
#[derive(Clone, Debug, PartialEq)]
pub enum Ink {
    /// 単色．
    Index(u8),
    /// 2 色パターン．
    Pattern { mask: PatternMask, a: u8, b: u8 },
    /// スタンプ．透明画素は書き込まない．
    Stamp(IndexedCanvas),
}

impl Ink {
    /// `None` は「この画素には書き込まない」を意味する (透明を書くのではない)．
    pub fn resolve(&self, p: IVec2) -> Option<u8> {
        match self {
            Self::Index(i) => Some(*i),
            Self::Pattern { mask, a, b } => Some(if mask.is_a(p) { *a } else { *b }),
            Self::Stamp(canvas) => {
                let w = canvas.width() as i32;
                let h = canvas.height() as i32;
                if w == 0 || h == 0 {
                    return None;
                }
                let q = ivec2(p.x.rem_euclid(w), p.y.rem_euclid(h));
                if canvas.is_transparent_at(q) {
                    None
                } else {
                    canvas.get_at(q)
                }
            }
        }
    }
}

/// ブラシ形状．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Brush {
    /// 1 画素 (既定)．
    #[default]
    Pixel,
    /// 一辺 `size` の正方形．
    Square { size: u32 },
    /// 半径 `radius` の円．
    Circle { radius: u32 },
}

impl Brush {
    /// 中心からの相対座標．決定論性のため走査順を固定する．
    pub fn offsets(self) -> Vec<IVec2> {
        match self {
            Self::Pixel => vec![IVec2::ZERO],
            Self::Square { size } => {
                let s = size.max(1) as i32;
                let lo = -(s - 1) / 2;
                let hi = lo + s - 1;
                (lo..=hi)
                    .flat_map(|dy| (lo..=hi).map(move |dx| ivec2(dx, dy)))
                    .collect()
            }
            Self::Circle { radius } => {
                let r = radius as i32;
                let rr = radius as i64 * radius as i64;
                (-r..=r)
                    .flat_map(|dy| (-r..=r).map(move |dx| ivec2(dx, dy)))
                    .filter(|p| (p.x as i64 * p.x as i64 + p.y as i64 * p.y as i64) <= rr)
                    .collect()
            }
        }
    }
}

/// 塗りつぶしの条件．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FillOpts {
    /// 連結領域のみ塗る (偽なら同じ添字の画素を全て塗る)．
    pub contiguous: bool,
    /// 8 近傍で連結とみなす (偽なら 4 近傍)．
    pub diagonal: bool,
}

impl Default for FillOpts {
    fn default() -> Self {
        Self {
            contiguous: true,
            diagonal: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checker_alternates() {
        let m = PatternMask::Checker { size: 1 };
        assert!(m.is_a(ivec2(0, 0)));
        assert!(!m.is_a(ivec2(1, 0)));
        assert!(!m.is_a(ivec2(0, 1)));
        assert!(m.is_a(ivec2(1, 1)));
    }

    #[test]
    fn checker_handles_negative_coordinates() {
        let m = PatternMask::Checker { size: 2 };
        // 負座標でも周期が崩れない
        assert_eq!(m.is_a(ivec2(-2, 0)), m.is_a(ivec2(2, 0)));
        assert_eq!(m.is_a(ivec2(-1, -1)), m.is_a(ivec2(3, 3)));
    }

    #[test]
    fn bayer_matrix_is_a_permutation_of_0_to_n2() {
        for side in [2u32, 4, 8] {
            let mut seen: Vec<u32> = (0..side)
                .flat_map(|y| (0..side).map(move |x| bayer_value(side, x, y)))
                .collect();
            seen.sort_unstable();
            let expect: Vec<u32> = (0..side * side).collect();
            assert_eq!(seen, expect, "{side}x{side} Bayer 行列");
        }
    }

    #[test]
    fn bayer_classic_4x4_top_left_quadrant() {
        // 標準の 4x4 Bayer 行列の 1 行目は 0, 8, 2, 10
        let row: Vec<u32> = (0..4).map(|x| bayer_value(4, x, 0)).collect();
        assert_eq!(row, vec![0, 8, 2, 10]);
    }

    #[test]
    fn ink_index_always_writes() {
        assert_eq!(Ink::Index(7).resolve(ivec2(3, 4)), Some(7));
    }

    #[test]
    fn stamp_skips_transparent_pixels() {
        let stamp = IndexedCanvas::from_pixels(2, 1, vec![0, 5])
            .unwrap()
            .with_transparent(Some(0));
        let ink = Ink::Stamp(stamp);
        assert_eq!(ink.resolve(ivec2(0, 0)), None);
        assert_eq!(ink.resolve(ivec2(1, 0)), Some(5));
        assert_eq!(
            ink.resolve(ivec2(3, 0)),
            Some(5),
            "スタンプは周期的に繰り返す"
        );
    }

    #[test]
    fn brush_offsets_have_expected_counts() {
        assert_eq!(Brush::Pixel.offsets().len(), 1);
        assert_eq!(Brush::Square { size: 3 }.offsets().len(), 9);
        assert!(Brush::Circle { radius: 1 }.offsets().contains(&ivec2(0, 1)));
        assert!(!Brush::Circle { radius: 1 }.offsets().contains(&ivec2(1, 1)));
    }

    #[test]
    fn brush_offsets_are_deterministic() {
        assert_eq!(
            Brush::Square { size: 2 }.offsets(),
            Brush::Square { size: 2 }.offsets()
        );
    }
}
