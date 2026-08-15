//! G2 — 距離場と曲率 (設計書 2.4)．
//!
//! `shade` / `aa` / `--ao` / `anim tween` / `anim smear` と lint 7・13・22 が
//! この上に乗る．
//!
//! # 符号の規約
//!
//! | 量 | 規約 |
//! | --- | --- |
//! | 符号付き距離 | **内側が正，外側が負**．境界のすぐ内側が $+0.5$ 付近 |
//! | 曲率 | **正が凸 (外向き)，負が凹 (内向き)** (D56) |
//!
//! 曲率の符号は 6.5 節の `select_aa_index` が `curvature > 0.0` を凸として扱う
//! ことに合わせてある．**取り違えると AA の明暗が丸ごと反転するが lint は通る**
//! (ルール 14 は中間色の比率しか見ない) ため，規約として固定し試験で縛る．

use crate::math::{IVec2, Vec2, ivec2, vec2};

use super::mask::{Field, Mask};

/// chamfer 距離の重み．3-4 近似は 2 パスで済み，誤差は 8% 程度に収まる．
const ORTHO: f32 = 3.0;
const DIAG: f32 = 4.0;
const SCALE: f32 = 3.0;

/// 符号付き距離場 (設計書 2.4 の G2)．
///
/// chamfer 3-4 の 2 パス近似．**内側が正，外側が負**．
pub fn signed_distance(mask: &Mask) -> Field<f32> {
    let inside = chamfer(mask, true);
    let outside = chamfer(mask, false);
    let mut out = Field::filled(mask.width(), mask.height(), 0.0f32);
    for p in mask.bounds().iter() {
        let v = if mask.get(p) {
            inside.copied(p).unwrap_or(0.0)
        } else {
            -outside.copied(p).unwrap_or(0.0)
        };
        out.set(p, v);
    }
    out
}

/// `target` の画素から，そうでない最も近い画素までの距離．
///
/// 画像の外は「`target` でない」として扱う．端に接する形が無限に遠いと
/// みなされないようにするためである．
fn chamfer(mask: &Mask, target: bool) -> Field<f32> {
    let (w, h) = (mask.width() as i32, mask.height() as i32);
    let mut d = Field::filled(mask.width(), mask.height(), f32::INFINITY);
    for p in mask.bounds().iter() {
        if mask.get(p) != target {
            d.set(p, 0.0);
        }
    }

    let forward = [
        (ivec2(-1, -1), DIAG),
        (ivec2(0, -1), ORTHO),
        (ivec2(1, -1), DIAG),
        (ivec2(-1, 0), ORTHO),
    ];
    let backward = [
        (ivec2(1, 1), DIAG),
        (ivec2(0, 1), ORTHO),
        (ivec2(-1, 1), DIAG),
        (ivec2(1, 0), ORTHO),
    ];

    for y in 0..h {
        for x in 0..w {
            relax(&mut d, ivec2(x, y), &forward);
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            relax(&mut d, ivec2(x, y), &backward);
        }
    }

    for v in d.data_mut() {
        if v.is_finite() {
            *v /= SCALE;
        }
    }
    d
}

fn relax(d: &mut Field<f32>, p: IVec2, offsets: &[(IVec2, f32)]) {
    let mut best = d.copied(p).unwrap_or(f32::INFINITY);
    for &(o, cost) in offsets {
        // 画像の外は距離 0 の背景として扱う (端に接する形が無限遠にならないように)
        let neighbor = d.copied(p + o).unwrap_or(0.0);
        best = best.min(neighbor + cost);
    }
    d.set(p, best);
}

/// 距離場の勾配 (疑似法線) と曲率を返す．
///
/// 勾配の長さが 0 に近い点 (稜線 = medial axis) では `None` を返す．
/// **ここで環境光へ直行してはならない** — 稜線はパーツ中心を貫く 1px の骨格線
/// なので，明るい面の中央に暗い線が入る (設計書 6.2)．
pub fn normal_and_curvature(d: &Field<f32>, p: IVec2) -> Option<(Vec2, f32)> {
    let gx = (sample(d, p + ivec2(1, 0)) - sample(d, p + ivec2(-1, 0))) * 0.5;
    let gy = (sample(d, p + ivec2(0, 1)) - sample(d, p + ivec2(0, -1))) * 0.5;
    let grad = vec2(gx, gy);
    let normal = grad.normalize()?;

    // 符号付き距離場では曲率 ~ -Laplacian(d)．凸な形 (円板) で正になる (D56)
    let laplacian = sample(d, p + ivec2(1, 0))
        + sample(d, p + ivec2(-1, 0))
        + sample(d, p + ivec2(0, 1))
        + sample(d, p + ivec2(0, -1))
        - 4.0 * sample(d, p);
    Some((normal, -laplacian))
}

/// 範囲外は最も近い縁の値を複製する (Neumann 境界)．
///
/// 0 で埋めると画像の縁に人工的な段差ができ，そこだけ曲率が跳ね上がる．
fn sample(d: &Field<f32>, p: IVec2) -> f32 {
    let x = p.x.clamp(0, d.width() as i32 - 1);
    let y = p.y.clamp(0, d.height() as i32 - 1);
    d.copied(ivec2(x, y)).unwrap_or(0.0)
}

/// 曲率だけを場として求める．
pub fn curvature_field(d: &Field<f32>) -> Field<f32> {
    let mut out = Field::filled(d.width(), d.height(), 0.0f32);
    for p in d.bounds().iter() {
        if let Some((_, k)) = normal_and_curvature(d, p) {
            out.set(p, k);
        }
    }
    out
}

/// 稜線 (medial axis) の候補．勾配がほぼ消える点．
pub fn ridge_mask(d: &Field<f32>) -> Mask {
    let mut out = Mask::new(d.width(), d.height());
    for p in d.bounds().iter() {
        let inside = d.copied(p).unwrap_or(0.0) > 0.0;
        if inside && normal_and_curvature(d, p).is_none() {
            out.set(p, true);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::IRect;

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

    fn filled_rect(w: u32, h: u32, r: IRect) -> Mask {
        let mut m = Mask::new(w, h);
        for p in r.iter() {
            m.set(p, true);
        }
        m
    }

    #[test]
    fn inside_is_positive_and_outside_is_negative() {
        let m = filled_rect(9, 9, IRect::new(2, 2, 5, 5));
        let d = signed_distance(&m);
        assert!(d.copied(ivec2(4, 4)).unwrap() > 0.0, "中心は正");
        assert!(d.copied(ivec2(0, 0)).unwrap() < 0.0, "外は負");
    }

    #[test]
    fn distance_grows_towards_the_centre() {
        let m = filled_rect(11, 11, IRect::new(1, 1, 9, 9));
        let d = signed_distance(&m);
        let edge = d.copied(ivec2(1, 5)).unwrap();
        let mid = d.copied(ivec2(3, 5)).unwrap();
        let centre = d.copied(ivec2(5, 5)).unwrap();
        assert!(edge < mid && mid < centre, "{edge} < {mid} < {centre}");
    }

    #[test]
    fn boundary_pixels_are_about_one_from_the_outside() {
        let m = filled_rect(9, 9, IRect::new(2, 2, 5, 5));
        let d = signed_distance(&m);
        // 境界のすぐ内側は 1 画素ぶん
        assert!((d.copied(ivec2(2, 4)).unwrap() - 1.0).abs() < 0.35);
        // すぐ外側は -1 画素ぶん
        assert!((d.copied(ivec2(1, 4)).unwrap() + 1.0).abs() < 0.35);
    }

    #[test]
    fn chamfer_approximates_euclidean_distance() {
        let m = filled_rect(21, 21, IRect::new(0, 0, 21, 21));
        let d = signed_distance(&m);
        // (10,10) から縁までは 11 画素 (外を 0 とするため縁 +1)
        let got = d.copied(ivec2(10, 10)).unwrap();
        assert!((got - 11.0).abs() / 11.0 < 0.12, "誤差が大きい: {got}");
    }

    #[test]
    fn a_shape_touching_the_edge_is_not_infinitely_far() {
        let m = filled_rect(9, 9, IRect::new(0, 0, 9, 9));
        let d = signed_distance(&m);
        for p in d.bounds().iter() {
            assert!(d.copied(p).unwrap().is_finite(), "{p:?} が無限大");
        }
    }

    /// D56 の規約を縛る — 凸が正．取り違えると AA の明暗が反転するが lint は通る．
    #[test]
    fn convex_shapes_have_positive_curvature() {
        let m = disc(21, 8.0);
        let d = signed_distance(&m);
        // 円板の境界のすぐ内側を何点か見る
        let mut samples = Vec::new();
        for p in [ivec2(10, 3), ivec2(3, 10), ivec2(17, 10), ivec2(10, 17)] {
            if let Some((_, k)) = normal_and_curvature(&d, p) {
                samples.push(k);
            }
        }
        assert!(!samples.is_empty(), "曲率が取れる点が無い");
        let mean: f32 = samples.iter().sum::<f32>() / samples.len() as f32;
        assert!(mean > 0.0, "凸なのに曲率が正でない: {samples:?}");
    }

    /// 凹んだ角では負になること．
    #[test]
    fn concave_corners_have_negative_curvature() {
        // 大きな四角から 1 隅を削って凹角を作る
        let mut m = filled_rect(15, 15, IRect::new(2, 2, 11, 11));
        for p in IRect::new(2, 2, 5, 5).iter() {
            m.set(p, false);
        }
        let d = signed_distance(&m);
        // 凹角の内側 (削った角の対角)
        let k = normal_and_curvature(&d, ivec2(7, 7)).map(|(_, k)| k);
        assert!(k.is_some_and(|k| k < 0.0), "凹角が負でない: {k:?}");
    }

    #[test]
    fn the_normal_points_towards_the_inside() {
        let m = filled_rect(11, 11, IRect::new(3, 3, 5, 5));
        let d = signed_distance(&m);
        // 左の辺の上では，内向き (+x) を向くはず (距離が増える向き)
        let (n, _) = normal_and_curvature(&d, ivec2(3, 5)).unwrap();
        assert!(n.x > 0.5, "法線が内側を向いていない: {n:?}");
        // 右の辺では -x
        let (n, _) = normal_and_curvature(&d, ivec2(7, 5)).unwrap();
        assert!(n.x < -0.5, "法線が内側を向いていない: {n:?}");
    }

    #[test]
    fn the_ridge_of_a_thin_bar_is_detected() {
        // 幅 3 の棒．中央の列が稜線になる
        let m = filled_rect(11, 5, IRect::new(0, 1, 11, 3));
        let d = signed_distance(&m);
        let ridge = ridge_mask(&d);
        assert!(!ridge.is_empty(), "稜線が 1 つも見つからない");
        for p in ridge.iter_set() {
            assert_eq!(p.y, 2, "稜線が中央の行でない: {p:?}");
        }
    }

    #[test]
    fn curvature_field_covers_every_pixel() {
        let m = disc(15, 5.0);
        let d = signed_distance(&m);
        let k = curvature_field(&d);
        assert_eq!(k.size(), d.size());
    }

    #[test]
    fn the_image_edge_does_not_create_fake_curvature() {
        // 全面が前景．縁で曲率が跳ね上がっていないこと
        let m = filled_rect(9, 9, IRect::new(0, 0, 9, 9));
        let d = signed_distance(&m);
        let k = curvature_field(&d);
        let corner = k.copied(ivec2(0, 0)).unwrap().abs();
        let centre = k.copied(ivec2(4, 4)).unwrap().abs();
        assert!(
            corner <= centre + 1.0,
            "縁だけ曲率が跳ねている ({corner} と {centre})"
        );
    }

    #[test]
    fn an_empty_mask_is_negative_everywhere() {
        let m = Mask::new(5, 5);
        let d = signed_distance(&m);
        for p in d.bounds().iter() {
            assert!(d.copied(p).unwrap() <= 0.0);
        }
    }
}
