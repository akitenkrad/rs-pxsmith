//! G1 — 輪郭チェーン追跡 (設計書 2.4)．
//!
//! `smooth` / `aa` / `outline` / `anim subpixel` と lint 4・8・20・23・25 が
//! この上に乗る．
//!
//! # 型を分ける理由 (D57)
//!
//! 輪郭全体は [`Contour`] (`closed` を持つ)，単調区間は [`Chain`] (`tangent` を
//! 1 つ持つ)．単調区間が閉じることはないので，1 つの型に両方を同居させると
//! `closed` が常に偽になる．
//!
//! # 向きの規約
//!
//! 画像座標 (y が下向き) で，**外側の輪郭は時計回り，穴の輪郭は反時計回り**に
//! 並ぶ．Moore 近傍追跡を時計回りに回すことから自然に決まる向きである．

use std::collections::BTreeSet;

use crate::math::{IVec2, Vec2, ivec2};

use super::mask::Mask;
use super::regions::label_mask;

/// 8 近傍の向き．時計回りに並べる (画像座標なので y は下向き)．
pub const DIRS8: [IVec2; 8] = [
    ivec2(1, 0),
    ivec2(1, 1),
    ivec2(0, 1),
    ivec2(-1, 1),
    ivec2(-1, 0),
    ivec2(-1, -1),
    ivec2(0, -1),
    ivec2(1, -1),
];

fn dir_index(from: IVec2, to: IVec2) -> Option<usize> {
    let d = to - from;
    DIRS8.iter().position(|&x| x == d)
}

/// 輪郭全体．閉じた経路をなす画素の列．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contour {
    pts: Vec<IVec2>,
    closed: bool,
    hole: bool,
}

impl Contour {
    pub fn points(&self) -> &[IVec2] {
        &self.pts
    }

    pub fn len(&self) -> usize {
        self.pts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pts.is_empty()
    }

    /// 閉じているか．追跡で得た輪郭は必ず閉じている．
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// 穴の輪郭か．
    pub fn is_hole(&self) -> bool {
        self.hole
    }

    /// 符号付き面積の 2 倍 (靴紐公式)．外側は正，穴は負になる．
    ///
    /// 画像座標では y が下向きなので，時計回りが正になる．
    pub fn signed_area2(&self) -> i64 {
        let n = self.pts.len();
        if n < 3 {
            return 0;
        }
        let mut acc = 0i64;
        for i in 0..n {
            let a = self.pts[i];
            let b = self.pts[(i + 1) % n];
            acc += a.x as i64 * b.y as i64 - b.x as i64 * a.y as i64;
        }
        acc
    }
}

/// 単調区間．向きが変わらない画素の並びと，その接線．
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    pts: Vec<IVec2>,
    tangent: Vec2,
}

impl Chain {
    pub fn points(&self) -> &[IVec2] {
        &self.pts
    }

    pub fn len(&self) -> usize {
        self.pts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pts.is_empty()
    }

    /// 形が向いている方向 (D38)．**動きの方向とは独立**である．
    pub fn tangent(&self) -> Vec2 {
        self.tangent
    }

    /// 端から端への変位．
    pub fn displacement(&self) -> IVec2 {
        match (self.pts.first(), self.pts.last()) {
            (Some(a), Some(b)) => *b - *a,
            _ => IVec2::ZERO,
        }
    }

    /// 主軸が水平か (|dx| >= |dy|)．ラン長解析はこの軸に沿って数える．
    pub fn is_horizontal(&self) -> bool {
        let d = self.displacement();
        d.x.abs() >= d.y.abs()
    }
}

/// マスクの輪郭をすべて追跡する．
///
/// 連結成分 (8 連結) ごとに外側の輪郭を，囲まれた背景 (4 連結) ごとに穴の輪郭を返す．
pub fn trace_contours(mask: &Mask) -> Vec<Contour> {
    let mut out = Vec::new();

    // 外側の輪郭 — 前景の連結成分ごとに，最初の画素から追う
    let fg = label_mask(mask, true);
    for component in fg.components() {
        let Some(start) = component.first() else {
            continue;
        };
        // 走査順の最初の画素なので，左隣は必ず背景 (画像外も背景)
        if let Some(pts) = trace_from(mask, *start, ivec2(start.x - 1, start.y)) {
            out.push(Contour {
                pts,
                closed: true,
                hole: false,
            });
        }
    }

    // 穴の輪郭 — 画像の縁に届かない背景の連結成分が穴である
    let bg = label_mask(&mask.inverted(), true);
    for component in bg.components() {
        let touches_edge = component.iter().any(|p| {
            p.x == 0
                || p.y == 0
                || p.x + 1 == mask.width() as i32
                || p.y + 1 == mask.height() as i32
        });
        if touches_edge {
            continue;
        }
        let Some(hole_start) = component.first() else {
            continue;
        };
        // 穴の最上・最左の画素の真上は必ず前景 (囲まれているため)
        let start = ivec2(hole_start.x, hole_start.y - 1);
        if !mask.get(start) {
            continue;
        }
        if let Some(pts) = trace_from(mask, start, *hole_start) {
            out.push(Contour {
                pts,
                closed: true,
                hole: true,
            });
        }
    }

    out
}

/// Moore 近傍追跡．`backtrack` は `start` に隣接する背景画素．
///
/// Jacob の停止条件 (同じ画素へ同じ向きから入ったら終わり) を使う．単純に
/// 「開始点へ戻ったら終わり」にすると，8 の字に接する形で 1 周目の途中で止まる．
fn trace_from(mask: &Mask, start: IVec2, backtrack: IVec2) -> Option<Vec<IVec2>> {
    if !mask.get(start) {
        return None;
    }
    let mut pts = vec![start];
    let (mut b, mut c) = (start, backtrack);
    let (first_b, first_c) = (b, c);

    // 1 画素だけの成分は追跡の輪ができないので早く返す
    if DIRS8.iter().all(|&d| !mask.get(start + d)) {
        return Some(pts);
    }

    // 上限は「全画素を 8 方向ぶん通る」で十分に余裕がある
    let limit = mask.size().area().saturating_mul(8).max(16);
    for _ in 0..limit {
        let d0 = dir_index(b, c)?;
        let mut next = None;
        for k in 1..=8 {
            let d = (d0 + k) % 8;
            let q = b + DIRS8[d];
            if mask.get(q) {
                // 時計回りに 1 つ手前が新しい背景側
                next = Some((q, b + DIRS8[(d + 7) % 8]));
                break;
            }
        }
        let (nb, nc) = next?;
        if nb == first_b && nc == first_c {
            return Some(pts);
        }
        pts.push(nb);
        b = nb;
        c = nc;
    }
    Some(pts)
}

/// 輪郭を単調区間へ分ける．
///
/// 「単調」とは，進む向きの符号 (dx, dy それぞれ) が変わらないことをいう．
/// 6.4 節のラン長解析と 6.10 節の接線シフトはどちらもこの出力 1 本を単位に動く．
pub fn split_monotone(contour: &Contour) -> Vec<Chain> {
    let pts = contour.points();
    if pts.len() < 2 {
        return pts
            .iter()
            .map(|&p| Chain {
                pts: vec![p],
                tangent: Vec2::ZERO,
            })
            .collect();
    }

    let mut walk: Vec<IVec2> = pts.to_vec();
    if contour.is_closed() {
        // 閉じた輪郭は最後に始点へ戻る
        walk.push(pts[0]);
    }
    let mut out = split_points(&walk);

    // 走査の開始点が辺の途中だと，そこで 1 本の辺が 2 本に割れてしまう．
    // 閉じた輪郭では末尾と先頭が繋がるので，繋げられるなら繋ぐ．
    if contour.is_closed() && out.len() >= 2 {
        let last = out.last().expect("2 本以上ある").points().to_vec();
        let first = out.first().expect("2 本以上ある").points().to_vec();
        let mut joined = last;
        joined.extend_from_slice(&first[1..]);
        if split_points(&joined).len() == 1 {
            let merged = make_chain(joined);
            out.pop();
            out[0] = merged;
        }
    }
    out
}

/// 進む向きの軸．
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Axis {
    Horizontal,
    Vertical,
    Diagonal,
}

fn axis_of(step: IVec2) -> Axis {
    match (step.x != 0, step.y != 0) {
        (true, true) => Axis::Diagonal,
        (_, false) => Axis::Horizontal,
        (false, _) => Axis::Vertical,
    }
}

/// 開いた点列を単調区間へ分ける．
///
/// 区切るのは 2 つの場合である．
///
/// 1. **符号の反転** — どちらかの軸で進む向きが逆になった．
/// 2. **主軸の切り替わり** — 主軸に直交する歩みが 2 歩続いた．
///
/// 2 が要る．階段は主軸方向の歩みの合間に直交する 1 歩が挟まる形なので，
/// 「向きが変わったら区切る」では階段が刻まれてラン長解析が成立しない．一方で
/// L 字の角は直交する歩みが 2 歩続くので，ここで切らないと**上辺と右辺が 1 本の
/// チェーンに混ざり，ラン長列が意味を失う**．
fn split_points(pts: &[IVec2]) -> Vec<Chain> {
    if pts.len() < 2 {
        return pts
            .iter()
            .map(|&p| Chain {
                pts: vec![p],
                tangent: Vec2::ZERO,
            })
            .collect();
    }

    let mut out = Vec::new();
    let mut current = vec![pts[0]];
    let (mut sx, mut sy) = (0i32, 0i32);
    let mut major: Option<Axis> = None;
    let mut minor_run = 0usize;

    for i in 0..pts.len() - 1 {
        let step = pts[i + 1] - pts[i];
        if step == IVec2::ZERO {
            continue;
        }
        let (dx, dy) = (step.x.signum(), step.y.signum());
        let axis = axis_of(step);

        let reversal = (dx != 0 && sx != 0 && dx != sx) || (dy != 0 && sy != 0 && dy != sy);
        let is_minor = matches!(major, Some(m) if axis != Axis::Diagonal && axis != m);
        let turn = is_minor && minor_run >= 1;

        if reversal {
            out.push(make_chain(std::mem::take(&mut current)));
            current.push(pts[i]);
            sx = 0;
            sy = 0;
            major = None;
            minor_run = 0;
        } else if turn {
            // 角は 1 歩前 — 直交する歩みが始まった点にある
            let last = current.pop().expect("少なくとも 1 点ある");
            let corner = *current.last().expect("角の点が残っている");
            out.push(make_chain(std::mem::take(&mut current)));
            current.push(corner);
            current.push(last);
            sx = 0;
            sy = 0;
            major = None;
            minor_run = 0;
        }

        if dx != 0 {
            sx = dx;
        }
        if dy != 0 {
            sy = dy;
        }
        match major {
            None if axis != Axis::Diagonal => major = Some(axis),
            _ => {}
        }
        minor_run = if matches!(major, Some(m) if axis != Axis::Diagonal && axis != m) {
            minor_run + 1
        } else {
            0
        };
        current.push(pts[i + 1]);
    }
    if current.len() > 1 {
        out.push(make_chain(current));
    }
    out
}

fn make_chain(pts: Vec<IVec2>) -> Chain {
    let tangent = match (pts.first(), pts.last()) {
        (Some(a), Some(b)) => (*b - *a).as_vec2().normalize().unwrap_or(Vec2::ZERO),
        _ => Vec2::ZERO,
    };
    Chain { pts, tangent }
}

/// 全色境界を追跡する (D33)．
///
/// **輪郭線ではなく全色境界が対象である**．線を一度も引かなくても異なる色の面が
/// 接するところに線が生まれ，陰影の境界にジャギーがあってはならない．
pub fn trace_color_boundaries(canvas: &crate::canvas::IndexedCanvas) -> Vec<(u8, Vec<Contour>)> {
    let mut indices: BTreeSet<u8> = canvas.pixels().iter().copied().collect();
    if let Some(t) = canvas.transparent() {
        indices.remove(&t);
    }
    indices
        .into_iter()
        .map(|index| (index, trace_contours(&canvas.mask_of(index))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::IRect;

    fn rect_mask(w: u32, h: u32, r: IRect) -> Mask {
        let mut m = Mask::new(w, h);
        for p in r.iter() {
            m.set(p, true);
        }
        m
    }

    #[test]
    fn single_pixel_has_a_one_point_contour() {
        let m = rect_mask(3, 3, IRect::new(1, 1, 1, 1));
        let c = trace_contours(&m);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].points(), &[ivec2(1, 1)]);
        assert!(c[0].is_closed());
        assert!(!c[0].is_hole());
    }

    #[test]
    fn square_contour_visits_every_edge_pixel_once() {
        let m = rect_mask(5, 5, IRect::new(1, 1, 3, 3));
        let c = trace_contours(&m);
        assert_eq!(c.len(), 1);
        // 3x3 の外周は 8 画素
        assert_eq!(c[0].len(), 8, "{:?}", c[0].points());
        let unique: BTreeSet<_> = c[0].points().iter().collect();
        assert_eq!(unique.len(), 8, "同じ画素を 2 回通っている");
        assert!(!c[0].points().contains(&ivec2(2, 2)), "内部は輪郭でない");
    }

    /// プロパティテスト — 閉曲線の一貫性 (M1a の完了条件)．
    #[test]
    fn traced_contours_are_consistent_closed_curves() {
        let shapes: Vec<Mask> = vec![
            rect_mask(8, 8, IRect::new(1, 1, 5, 4)),
            rect_mask(8, 8, IRect::new(0, 0, 8, 8)),
            rect_mask(9, 9, IRect::new(2, 2, 1, 5)),
            plus_mask(),
            donut_mask(),
            diagonal_mask(),
        ];
        for (i, m) in shapes.iter().enumerate() {
            for c in trace_contours(m) {
                assert!(!c.is_empty(), "形 {i}: 空の輪郭");
                assert!(c.is_closed(), "形 {i}: 閉じていない");
                // 1. すべての点がマスク上にある
                for p in c.points() {
                    assert!(m.get(*p), "形 {i}: 輪郭が背景を通っている {p:?}");
                }
                // 2. すべての点が境界画素である
                for p in c.points() {
                    assert!(m.is_boundary(*p), "形 {i}: 内部を通っている {p:?}");
                }
                // 3. 隣り合う点は 8 近傍で隣接する (末尾と先頭も含む)
                let n = c.len();
                if n > 1 {
                    for k in 0..n {
                        let a = c.points()[k];
                        let b = c.points()[(k + 1) % n];
                        assert_eq!(a.chebyshev(b), 1, "形 {i}: 途切れている {a:?} -> {b:?}");
                    }
                }
            }
        }
    }

    fn plus_mask() -> Mask {
        let mut m = Mask::new(7, 7);
        for p in IRect::new(2, 0, 3, 7).iter() {
            m.set(p, true);
        }
        for p in IRect::new(0, 2, 7, 3).iter() {
            m.set(p, true);
        }
        m
    }

    fn donut_mask() -> Mask {
        let mut m = rect_mask(7, 7, IRect::new(1, 1, 5, 5));
        for p in IRect::new(3, 3, 1, 1).iter() {
            m.set(p, false);
        }
        m
    }

    fn diagonal_mask() -> Mask {
        let mut m = Mask::new(8, 8);
        for i in 0..6 {
            m.set(ivec2(i + 1, i + 1), true);
            m.set(ivec2(i + 1, i), true);
        }
        m
    }

    #[test]
    fn a_hole_produces_a_second_contour() {
        let contours = trace_contours(&donut_mask());
        assert_eq!(contours.len(), 2, "外周と穴で 2 本のはず");
        assert_eq!(contours.iter().filter(|c| c.is_hole()).count(), 1);
    }

    #[test]
    fn outer_and_hole_contours_wind_in_opposite_directions() {
        let contours = trace_contours(&donut_mask());
        let outer = contours.iter().find(|c| !c.is_hole()).unwrap();
        let hole = contours.iter().find(|c| c.is_hole()).unwrap();
        assert!(outer.signed_area2() > 0, "外周は時計回り (正) のはず");
        assert!(hole.signed_area2() < 0, "穴は反時計回り (負) のはず");
    }

    #[test]
    fn separate_components_get_separate_contours() {
        let mut m = Mask::new(9, 4);
        for p in IRect::new(0, 0, 3, 3).iter() {
            m.set(p, true);
        }
        for p in IRect::new(6, 0, 3, 3).iter() {
            m.set(p, true);
        }
        assert_eq!(trace_contours(&m).len(), 2);
    }

    #[test]
    fn split_monotone_breaks_at_direction_reversals() {
        let m = rect_mask(5, 5, IRect::new(1, 1, 3, 3));
        let contour = &trace_contours(&m)[0];
        let chains = split_monotone(contour);
        assert!(chains.len() >= 4, "正方形は 4 辺に分かれるはず: {chains:?}");
        for chain in &chains {
            let d = chain.displacement();
            // 各区間の内部で符号が反転していないこと
            let mut sx = 0;
            let mut sy = 0;
            for w in chain.points().windows(2) {
                let step = w[1] - w[0];
                if step.x != 0 {
                    assert!(sx == 0 || sx == step.x.signum(), "x の符号が反転している");
                    sx = step.x.signum();
                }
                if step.y != 0 {
                    assert!(sy == 0 || sy == step.y.signum(), "y の符号が反転している");
                    sy = step.y.signum();
                }
            }
            assert!(d != IVec2::ZERO || chain.len() == 1);
        }
    }

    #[test]
    fn tangent_points_along_the_chain() {
        let m = diagonal_mask();
        let contour = &trace_contours(&m)[0];
        for chain in split_monotone(contour) {
            if chain.len() < 2 {
                continue;
            }
            let d = chain.displacement().as_vec2();
            let t = chain.tangent();
            // 接線は端から端への向きと同じ側を向く
            assert!(t.dot(d) > 0.0, "接線が逆を向いている: {t:?} と {d:?}");
            assert!((t.length() - 1.0).abs() < 1e-4, "接線が単位ベクトルでない");
        }
    }

    #[test]
    fn horizontal_chains_are_detected() {
        let mut m = Mask::new(10, 3);
        for p in IRect::new(0, 1, 10, 1).iter() {
            m.set(p, true);
        }
        let contour = &trace_contours(&m)[0];
        let chains = split_monotone(contour);
        assert!(chains.iter().any(|c| c.is_horizontal() && c.len() >= 5));
    }

    #[test]
    fn colour_boundaries_are_traced_per_index() {
        let c = crate::canvas::IndexedCanvas::from_pixels(4, 2, vec![0, 1, 1, 2, 0, 1, 1, 2])
            .unwrap()
            .with_transparent(Some(0));
        let boundaries = trace_color_boundaries(&c);
        // 透明の 0 を除いた 1 と 2
        assert_eq!(boundaries.len(), 2);
        assert_eq!(boundaries[0].0, 1);
        assert_eq!(boundaries[1].0, 2);
        assert!(!boundaries[0].1.is_empty());
    }
}
