//! 形だけを表す独立型と，2 次元の値配列 (設計書 2.4)．
//!
//! [`Mask`] を [`IndexedCanvas`] と結合させないのが規約である (D55)．G1・G2 は
//! 形だけの純関数なので，結合させると `geom` にパレットが入り込み，「`pxsmith-core` は
//! I/O に依存しない純関数の集合」という方針が内側から崩れる．

use crate::canvas::IndexedCanvas;
use crate::error::{CoreError, Result};
use crate::math::{IRect, IVec2, UVec2, ivec2, uvec2};

/// 形を表すビットマスク．
#[derive(Clone, PartialEq, Eq)]
pub struct Mask {
    w: u32,
    h: u32,
    bits: Vec<u64>,
}

impl std::fmt::Debug for Mask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Mask({}x{}, {} 画素)", self.w, self.h, self.count())
    }
}

impl Mask {
    pub fn new(w: u32, h: u32) -> Self {
        let len = uvec2(w, h).area().div_ceil(64);
        Self {
            w,
            h,
            bits: vec![0; len],
        }
    }

    /// 真偽の列から作る．
    pub fn from_bools(w: u32, h: u32, values: &[bool]) -> Result<Self> {
        let expected = uvec2(w, h).area();
        if values.len() != expected {
            return Err(CoreError::PixelCountMismatch {
                width: w,
                height: h,
                expected,
                actual: values.len(),
            });
        }
        let mut m = Self::new(w, h);
        for (i, &v) in values.iter().enumerate() {
            if v {
                m.set_index(i, true);
            }
        }
        Ok(m)
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    pub fn size(&self) -> UVec2 {
        uvec2(self.w, self.h)
    }

    pub fn bounds(&self) -> IRect {
        IRect::new(0, 0, self.w, self.h)
    }

    fn index_of(&self, p: IVec2) -> Option<usize> {
        if p.x < 0 || p.y < 0 || p.x as i64 >= self.w as i64 || p.y as i64 >= self.h as i64 {
            None
        } else {
            Some(p.y as usize * self.w as usize + p.x as usize)
        }
    }

    fn set_index(&mut self, i: usize, value: bool) {
        let (word, bit) = (i / 64, i % 64);
        if value {
            self.bits[word] |= 1 << bit;
        } else {
            self.bits[word] &= !(1 << bit);
        }
    }

    /// **範囲外は偽**．端の判定を呼び出し側に書かせないための規約である．
    /// 輪郭追跡は画像の外を「背景」として扱えないと，端に接する形で破綻する．
    pub fn get(&self, p: IVec2) -> bool {
        match self.index_of(p) {
            Some(i) => self.bits[i / 64] & (1 << (i % 64)) != 0,
            None => false,
        }
    }

    pub fn set(&mut self, p: IVec2, value: bool) -> bool {
        match self.index_of(p) {
            Some(i) => {
                self.set_index(i, value);
                true
            }
            None => false,
        }
    }

    /// 立っているビットの数．
    pub fn count(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|&w| w == 0)
    }

    /// 立っている画素を走査する (上から下，左から右)．
    pub fn iter_set(&self) -> impl Iterator<Item = IVec2> + '_ {
        self.bounds().iter().filter(|p| self.get(*p))
    }

    /// 最初に立っている画素 (上から下，左から右)．
    pub fn first_set(&self) -> Option<IVec2> {
        self.iter_set().next()
    }

    /// 反転したマスク．
    pub fn inverted(&self) -> Self {
        let mut out = Self::new(self.w, self.h);
        for p in self.bounds().iter() {
            out.set(p, !self.get(p));
        }
        out
    }

    /// 立っている画素を囲う最小の矩形．
    pub fn bbox(&self) -> Option<IRect> {
        let (mut x0, mut y0) = (i32::MAX, i32::MAX);
        let (mut x1, mut y1) = (i32::MIN, i32::MIN);
        for p in self.iter_set() {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        (x0 <= x1).then(|| IRect::new(x0, y0, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32))
    }

    /// 立っていて，かつ 4 近傍に立っていない画素がある画素．
    ///
    /// 画像の外は背景として数えるので，端に接する形でも境界になる．
    pub fn is_boundary(&self, p: IVec2) -> bool {
        if !self.get(p) {
            return false;
        }
        [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)]
            .iter()
            .any(|&d| !self.get(p + d))
    }
}

impl IndexedCanvas {
    /// 指定した添字の画素だけを立てたマスクを作る (D55)．
    pub fn mask_of(&self, index: u8) -> Mask {
        let mut m = Mask::new(self.width(), self.height());
        for p in self.bounds().iter() {
            if self.get_at(p) == Some(index) {
                m.set(p, true);
            }
        }
        m
    }

    /// 透明でない画素を立てたマスク (シルエット)．
    ///
    /// 透明添字を持たない場合は全画素が立つ．
    pub fn silhouette(&self) -> Mask {
        let mut m = Mask::new(self.width(), self.height());
        for p in self.bounds().iter() {
            if !self.is_transparent_at(p) && self.get_at(p).is_some() {
                m.set(p, true);
            }
        }
        m
    }
}

/// 2 次元の値配列．距離場・局所推定結果の共通の器．
#[derive(Clone, Debug, PartialEq)]
pub struct Field<T> {
    w: u32,
    h: u32,
    data: Vec<T>,
}

impl<T: Clone> Field<T> {
    pub fn filled(w: u32, h: u32, value: T) -> Self {
        Self {
            w,
            h,
            data: vec![value; uvec2(w, h).area()],
        }
    }
}

impl<T> Field<T> {
    pub fn from_data(w: u32, h: u32, data: Vec<T>) -> Result<Self> {
        let expected = uvec2(w, h).area();
        if data.len() != expected {
            return Err(CoreError::PixelCountMismatch {
                width: w,
                height: h,
                expected,
                actual: data.len(),
            });
        }
        Ok(Self { w, h, data })
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    pub fn size(&self) -> UVec2 {
        uvec2(self.w, self.h)
    }

    pub fn bounds(&self) -> IRect {
        IRect::new(0, 0, self.w, self.h)
    }

    pub fn data(&self) -> &[T] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [T] {
        &mut self.data
    }

    fn index_of(&self, p: IVec2) -> Option<usize> {
        if p.x < 0 || p.y < 0 || p.x as i64 >= self.w as i64 || p.y as i64 >= self.h as i64 {
            None
        } else {
            Some(p.y as usize * self.w as usize + p.x as usize)
        }
    }

    pub fn get(&self, p: IVec2) -> Option<&T> {
        self.index_of(p).map(|i| &self.data[i])
    }

    pub fn get_mut(&mut self, p: IVec2) -> Option<&mut T> {
        self.index_of(p).map(|i| &mut self.data[i])
    }

    pub fn set(&mut self, p: IVec2, value: T) -> bool {
        match self.index_of(p) {
            Some(i) => {
                self.data[i] = value;
                true
            }
            None => false,
        }
    }
}

impl<T: Copy> Field<T> {
    pub fn copied(&self, p: IVec2) -> Option<T> {
        self.get(p).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plus() -> Mask {
        // 3x3 の十字
        Mask::from_bools(
            3,
            3,
            &[false, true, false, true, true, true, false, true, false],
        )
        .unwrap()
    }

    #[test]
    fn out_of_bounds_reads_as_background() {
        let m = plus();
        assert!(!m.get(ivec2(-1, 0)));
        assert!(!m.get(ivec2(3, 0)));
        assert!(!m.get(ivec2(0, -1)));
        assert!(m.get(ivec2(1, 1)));
    }

    #[test]
    fn count_and_bbox() {
        let m = plus();
        assert_eq!(m.count(), 5);
        assert_eq!(m.bbox(), Some(IRect::new(0, 0, 3, 3)));
        assert!(!m.is_empty());
        assert!(Mask::new(4, 4).is_empty());
    }

    #[test]
    fn first_set_scans_row_major() {
        assert_eq!(plus().first_set(), Some(ivec2(1, 0)));
    }

    #[test]
    fn inversion_is_an_involution() {
        let m = plus();
        assert_eq!(m.inverted().inverted(), m);
        assert_eq!(m.inverted().count(), 9 - 5);
    }

    #[test]
    fn the_arms_of_a_plus_are_boundary_but_the_centre_is_not() {
        let m = plus();
        // 十字の中心は 4 近傍が全て前景なので境界ではない
        assert!(!m.is_boundary(ivec2(1, 1)));
        for p in [ivec2(1, 0), ivec2(0, 1), ivec2(2, 1), ivec2(1, 2)] {
            assert!(m.is_boundary(p), "{p:?}");
        }
    }

    #[test]
    fn interior_pixels_are_not_boundary() {
        let mut m = Mask::new(5, 5);
        for p in IRect::new(1, 1, 3, 3).iter() {
            m.set(p, true);
        }
        assert!(!m.is_boundary(ivec2(2, 2)));
        assert!(m.is_boundary(ivec2(1, 1)));
    }

    #[test]
    fn shape_touching_the_edge_is_still_boundary() {
        let mut m = Mask::new(3, 3);
        for p in m.bounds().iter() {
            m.set(p, true);
        }
        // 全面が埋まっていても画像の外は背景なので，縁は境界になる
        assert!(m.is_boundary(ivec2(0, 0)));
        assert!(!m.is_boundary(ivec2(1, 1)));
    }

    #[test]
    fn mask_of_projects_a_single_index() {
        let c = IndexedCanvas::from_pixels(2, 2, vec![0, 1, 1, 2]).unwrap();
        assert_eq!(c.mask_of(1).count(), 2);
        assert!(c.mask_of(1).get(ivec2(1, 0)));
        assert!(!c.mask_of(1).get(ivec2(0, 0)));
    }

    #[test]
    fn silhouette_excludes_the_transparent_index() {
        let c = IndexedCanvas::from_pixels(2, 2, vec![0, 1, 1, 2])
            .unwrap()
            .with_transparent(Some(0));
        assert_eq!(c.silhouette().count(), 3);
    }

    #[test]
    fn bits_survive_a_size_that_is_not_a_multiple_of_64() {
        let mut m = Mask::new(10, 10);
        m.set(ivec2(9, 9), true);
        assert!(m.get(ivec2(9, 9)));
        assert_eq!(m.count(), 1);
    }

    #[test]
    fn field_rejects_length_mismatch() {
        assert!(Field::from_data(2, 2, vec![0u8; 3]).is_err());
        assert!(Field::from_data(2, 2, vec![0u8; 4]).is_ok());
    }

    #[test]
    fn field_get_is_bounds_checked() {
        let f = Field::filled(2, 2, 7u8);
        assert_eq!(f.copied(ivec2(1, 1)), Some(7));
        assert_eq!(f.copied(ivec2(2, 0)), None);
        assert_eq!(f.copied(ivec2(-1, 0)), None);
    }
}
