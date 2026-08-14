//! トポロジー — 成分数 ・穴の数 ・オイラー標数 (G5)．
//!
//! `px anim tween` (6.9) と `px lint` のルール 22 が同じ量を見るので，
//! **数え方は 1 か所に置く** — 正規の数え方が 2 つあるのは無いのと同じである
//! (D110) ．
//!
//! # 前景は 4 連結，背景は 8 連結
//!
//! **両方を同じ連結性で数えるとオイラー標数が市松模様で破綻する**．
//! 前景を 4 連結で数えるなら背景は 8 連結で数え，画布の縁に接する背景成分を
//! «外側» として穴から除く．

use crate::geom::mask::Mask;
use crate::geom::regions::label_mask;

/// 4 連結の成分数．
pub fn components(mask: &Mask) -> usize {
    label_mask(mask, false).len()
}

/// 穴の数．**背景を 8 連結で数え，画布の縁に触れる成分を除く**．
pub fn holes(mask: &Mask) -> usize {
    let bg = mask.inverted();
    let (w, h) = (mask.width() as i32, mask.height() as i32);
    label_mask(&bg, true)
        .components()
        .iter()
        .filter(|pts| {
            !pts.iter()
                .any(|p| p.x == 0 || p.y == 0 || p.x == w - 1 || p.y == h - 1)
        })
        .count()
}

/// オイラー標数 $\chi = (\text{成分数}) - (\text{穴の数})$．
///
/// **閾値の要らない量である** — 数え上げなので校正の対象ではない (D92)．
pub fn euler_characteristic(mask: &Mask) -> i64 {
    components(mask) as i64 - holes(mask) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::ivec2;

    fn ring(w: u32, h: u32) -> Mask {
        let mut m = Mask::new(w, h);
        for p in m.bounds().iter() {
            let inner = p.x > 1 && p.y > 1 && p.x < w as i32 - 2 && p.y < h as i32 - 2;
            let outer = p.x > 0 && p.y > 0 && p.x < w as i32 - 1 && p.y < h as i32 - 1;
            if outer && !inner {
                m.set(p, true);
            }
        }
        m
    }

    /// **壊れると: 穴のある形と無い形が同じ標数になる．**
    #[test]
    fn a_ring_has_one_component_and_one_hole() {
        let m = ring(9, 9);
        assert_eq!(components(&m), 1);
        assert_eq!(holes(&m), 1);
        assert_eq!(euler_characteristic(&m), 0);
    }

    /// **壊れると: 画布の外側を穴と数える．**
    #[test]
    fn the_outside_is_not_a_hole() {
        let mut m = Mask::new(5, 5);
        m.set(ivec2(2, 2), true);
        assert_eq!(holes(&m), 0);
        assert_eq!(euler_characteristic(&m), 1);
    }

    /// **壊れると: 市松模様で標数が破綻する** (前景も背景も 4 連結で数えた場合)．
    #[test]
    fn a_checkerboard_does_not_break_the_characteristic() {
        let mut m = Mask::new(7, 7);
        for p in m.bounds().iter() {
            if (p.x + p.y) % 2 == 0 {
                m.set(p, true);
            }
        }
        // 背景を 8 連結で数えるので «穴» は生まれない
        assert_eq!(holes(&m), 0);
        assert_eq!(euler_characteristic(&m), components(&m) as i64);
    }
}
