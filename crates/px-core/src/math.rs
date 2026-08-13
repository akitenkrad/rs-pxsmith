//! 座標系とクリッピング (設計書 3.5)．
//!
//! 負座標・はみ出しは [`clip_pair`] で一度だけ解決し，各アルゴリズムには渡さない．

use std::ops::{Add, Mul, Sub};

/// 整数座標．cel オフセットなど負値を取る量に使う (D6)．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IVec2 {
    pub x: i32,
    pub y: i32,
}

/// 非負整数座標．キャンバスの大きさなどに使う (D6)．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UVec2 {
    pub x: u32,
    pub y: u32,
}

/// 実数ベクトル．法線・接線・光源方向に使う．
#[derive(Copy, Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

/// 実数矩形．面光源の範囲などに使う (設計書 3.3)．
#[derive(Copy, Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn center(self) -> Vec2 {
        vec2(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

/// 整数矩形．左上が `(x, y)`，大きさが `w * h`．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// [`IVec2`] の短縮構築子．
pub const fn ivec2(x: i32, y: i32) -> IVec2 {
    IVec2 { x, y }
}

/// [`UVec2`] の短縮構築子．
pub const fn uvec2(x: u32, y: u32) -> UVec2 {
    UVec2 { x, y }
}

/// [`Vec2`] の短縮構築子．
pub const fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

impl IVec2 {
    pub const ZERO: Self = ivec2(0, 0);

    /// マンハッタン距離．pixel-perfect 補正の角判定に使う (設計書 6.3)．
    pub fn manhattan(self, other: Self) -> u32 {
        self.x.abs_diff(other.x) + self.y.abs_diff(other.y)
    }

    /// チェビシェフ距離．8 近傍の判定に使う．
    pub fn chebyshev(self, other: Self) -> u32 {
        self.x.abs_diff(other.x).max(self.y.abs_diff(other.y))
    }

    pub fn as_vec2(self) -> Vec2 {
        vec2(self.x as f32, self.y as f32)
    }
}

impl UVec2 {
    pub const ZERO: Self = uvec2(0, 0);

    pub fn as_ivec2(self) -> IVec2 {
        ivec2(self.x as i32, self.y as i32)
    }

    /// 面積．`u32` 同士の積が溢れないよう `usize` で返す．
    pub fn area(self) -> usize {
        self.x as usize * self.y as usize
    }
}

impl Vec2 {
    pub const ZERO: Self = vec2(0.0, 0.0);

    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// 長さ 0 のときは `None`．稜線の縮退判定に使う (設計書 6.2)．
    pub fn normalize(self) -> Option<Self> {
        let len = self.length();
        if len < f32::EPSILON {
            None
        } else {
            Some(vec2(self.x / len, self.y / len))
        }
    }

    /// 接線を符号量子化した単位ステップ (D38)．
    ///
    /// 各成分の符号を取るので，軸平行・対角のいずれでも 1 画素の移動になる．
    pub fn unit_step(self) -> IVec2 {
        fn sign(v: f32) -> i32 {
            if v > f32::EPSILON {
                1
            } else if v < -f32::EPSILON {
                -1
            } else {
                0
            }
        }
        ivec2(sign(self.x), sign(self.y))
    }
}

impl Add for IVec2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        ivec2(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for IVec2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        ivec2(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Add for Vec2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        vec2(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Vec2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        vec2(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        vec2(self.x * rhs, self.y * rhs)
    }
}

impl IRect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }

    pub fn right(self) -> i64 {
        self.x as i64 + self.w as i64
    }

    pub fn bottom(self) -> i64 {
        self.y as i64 + self.h as i64
    }

    pub fn contains(self, p: IVec2) -> bool {
        (p.x as i64) >= self.x as i64
            && (p.x as i64) < self.right()
            && (p.y as i64) >= self.y as i64
            && (p.y as i64) < self.bottom()
    }

    /// 左上から右下へ走査する座標列．
    pub fn iter(self) -> impl Iterator<Item = IVec2> {
        let (x0, y0, w, h) = (self.x, self.y, self.w, self.h);
        (0..h).flat_map(move |dy| (0..w).map(move |dx| ivec2(x0 + dx as i32, y0 + dy as i32)))
    }

    /// 2 つの矩形を含む最小の矩形．
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = self.right().max(other.right());
        let y1 = self.bottom().max(other.bottom());
        Self::new(x0, y0, (x1 - x0 as i64) as u32, (y1 - y0 as i64) as u32)
    }
}

/// 転送元と転送先の重なりを一度だけ解決する (設計書 3.5)．
///
/// `at` は転送元の原点を転送先のどこへ置くかを表す．重なりが無ければ `None`．
/// 返り値は `(転送先座標での矩形, 転送元座標での矩形)` で，大きさは必ず等しい．
pub fn clip_pair(dst: UVec2, src: UVec2, at: IVec2) -> Option<(IRect, IRect)> {
    let (dw, dh) = (dst.x as i64, dst.y as i64);
    let (sw, sh) = (src.x as i64, src.y as i64);
    let (ax, ay) = (at.x as i64, at.y as i64);

    let x0 = ax.max(0);
    let y0 = ay.max(0);
    let x1 = (ax + sw).min(dw);
    let y1 = (ay + sh).min(dh);

    if x1 <= x0 || y1 <= y0 {
        return None;
    }

    let w = (x1 - x0) as u32;
    let h = (y1 - y0) as u32;
    let dst_rect = IRect::new(x0 as i32, y0 as i32, w, h);
    let src_rect = IRect::new((x0 - ax) as i32, (y0 - ay) as i32, w, h);
    Some((dst_rect, src_rect))
}

/// Bresenham の直線 (両端を含む)．ストロークの点列を 8 連結で繋ぐのに使う．
pub fn line(a: IVec2, b: IVec2) -> Vec<IVec2> {
    let dx = (b.x - a.x).abs();
    let dy = -(b.y - a.y).abs();
    let sx = if a.x < b.x { 1 } else { -1 };
    let sy = if a.y < b.y { 1 } else { -1 };
    let mut err = dx + dy;
    let mut p = a;
    let mut out = Vec::new();
    loop {
        out.push(p);
        if p == b {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            p.x += sx;
        }
        if e2 <= dx {
            err += dx;
            p.y += sy;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_pair_full_overlap() {
        let (d, s) = clip_pair(uvec2(16, 16), uvec2(4, 4), ivec2(2, 3)).unwrap();
        assert_eq!(d, IRect::new(2, 3, 4, 4));
        assert_eq!(s, IRect::new(0, 0, 4, 4));
    }

    #[test]
    fn clip_pair_negative_origin() {
        let (d, s) = clip_pair(uvec2(16, 16), uvec2(8, 8), ivec2(-3, -5)).unwrap();
        assert_eq!(d, IRect::new(0, 0, 5, 3));
        assert_eq!(s, IRect::new(3, 5, 5, 3));
    }

    #[test]
    fn clip_pair_partially_past_far_edge() {
        let (d, s) = clip_pair(uvec2(10, 10), uvec2(6, 6), ivec2(7, 8)).unwrap();
        assert_eq!(d, IRect::new(7, 8, 3, 2));
        assert_eq!(s, IRect::new(0, 0, 3, 2));
    }

    #[test]
    fn clip_pair_no_overlap() {
        assert!(clip_pair(uvec2(8, 8), uvec2(4, 4), ivec2(8, 0)).is_none());
        assert!(clip_pair(uvec2(8, 8), uvec2(4, 4), ivec2(0, -4)).is_none());
    }

    #[test]
    fn clip_pair_does_not_overflow() {
        assert!(clip_pair(uvec2(8, 8), uvec2(4, 4), ivec2(i32::MAX, 0)).is_none());
        assert!(clip_pair(uvec2(8, 8), uvec2(4, 4), ivec2(i32::MIN, 0)).is_none());
    }

    #[test]
    fn clip_pair_rect_sizes_always_match() {
        for ax in -6i32..12 {
            for ay in -6i32..12 {
                if let Some((d, s)) = clip_pair(uvec2(8, 8), uvec2(5, 5), ivec2(ax, ay)) {
                    assert_eq!((d.w, d.h), (s.w, s.h));
                }
            }
        }
    }

    #[test]
    fn unit_step_quantizes_diagonal_tangent() {
        assert_eq!(vec2(0.707, 0.707).unit_step(), ivec2(1, 1));
        assert_eq!(vec2(-0.1, 0.0).unit_step(), ivec2(-1, 0));
        assert_eq!(Vec2::ZERO.unit_step(), IVec2::ZERO);
    }

    #[test]
    fn line_is_eight_connected() {
        let pts = line(ivec2(0, 0), ivec2(5, 3));
        assert_eq!(pts.first(), Some(&ivec2(0, 0)));
        assert_eq!(pts.last(), Some(&ivec2(5, 3)));
        for w in pts.windows(2) {
            assert_eq!(w[0].chebyshev(w[1]), 1);
        }
    }
}
