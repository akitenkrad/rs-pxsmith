//! 投影ガイドグリッド (`px guide --projection`．設計書 6.13)．
//!
//! **下地の下地である** — 投影して描く前に，どこへ線を置けばよいかの当たりを引く．
//!
//! # 格子は投影の行列から引く
//!
//! ガイドの 2 本の線族は，**入力の $x$ 軸と $y$ 軸が倒れた先**そのものである．
//! だから[`crate::project::matrix`]の 2 列をそのまま使う — **投影とガイドで
//! «どちらへ倒すか» の決め方が 2 つあってはいけない** (D110) ．
//!
//! # 格子の刻みは整数で持つ
//!
//! 行列の値は $1/\sqrt 2 = 0.707$ のような無理数なので，そのまま倍して座標に
//! すると格子が画素に乗らない．**段は走り : 上がりの整数比**なのだから，
//! 刻みも整数の組で持てばよい (等角なら $(2, 1)$) ．[`GuideOptions::cell`] は
//! その**整数の刻みを何回繰り返すか**である．
//!
//! # チェス盤状の塗り分け
//!
//! 6.13 は «交点のドット連結を避けてチェス盤状の塗り分けを使う» と言う．
//! **2 色で塗り分けられること自体が数え上げで確かめられる** — 格子は平行四辺形の
//! タイル貼りなので $(i + j) \bmod 2$ が常に隣どうしで違う (D102 «47 も 5 も 20 も
//! 数え上げ» と同じ側) ．[`GuideReport::same_colour_adjacent`] が 0 でなければ
//! 塗り分けが壊れている．

use std::collections::BTreeMap;

use crate::canvas::IndexedCanvas;
use crate::color::Rgba8;
use crate::error::{CoreError, Result};
use crate::math::{IVec2, UVec2, ivec2, line};
use crate::palette::Palette;
use crate::project::{Facing, Projection, SourcePlane, Step};

/// ガイドの添字 (パレットの並びでもある)．
pub const IDX_TRANSPARENT: u8 = 0;
pub const IDX_LINE: u8 = 1;
pub const IDX_CHECKER_A: u8 = 2;
pub const IDX_CHECKER_B: u8 = 3;

/// 設定．
#[derive(Copy, Clone, Debug)]
pub struct GuideOptions {
    pub projection: Projection,
    pub plane: SourcePlane,
    pub facing: Facing,
    /// 段．`None` なら [`Projection::default_step`]．
    pub step: Option<Step>,
    /// **整数の刻みを何回繰り返すか**．等角の刻みは $(2, 1)$ なので
    /// `cell = 16` なら 1 升が 32 x 16 画素になる．
    pub cell: u32,
    pub size: UVec2,
    /// 升をチェス盤状に塗り分ける (設計書 6.13)．
    pub checker: bool,
}

impl GuideOptions {
    pub fn step(&self) -> Step {
        self.step.unwrap_or_else(|| self.projection.default_step())
    }
}

/// 結果の素性．
#[derive(Clone, Debug)]
pub struct GuideReport {
    pub projection: &'static str,
    pub plane: &'static str,
    pub facing: &'static str,
    pub step: Step,
    /// 格子の刻み (画素)．
    pub basis: (IVec2, IVec2),
    /// 1 升の外接矩形 (画素)．
    pub cell_size: UVec2,
    /// 引いた線の画素数．
    pub line_pixels: usize,
    /// 画布に載った升の数．
    pub cells: usize,
    /// **またがりを数えるのに使った正方形タイルの一辺**．
    ///
    /// «何枚にまたがるか» はタイルの大きさで変わるので，**枚数だけ出しては
    /// 読めない** (D100 «ルール 7 が掛かるかはタイルの大きさで決まる» と同じ) ．
    pub tile: u32,
    /// **升が正方形タイル何枚にまたがるか** (枚数 → 升の数)．
    ///
    /// 設計書 6.13 の «2 枚 / 4 枚でまたがる» を数える先である．
    pub tile_span: BTreeMap<usize, usize>,
    /// **辺を接する升が同じ色になった数．塗り分けたなら 0 のはず**．
    pub same_colour_adjacent: usize,
}

/// 格子の刻みを**整数の組**で組む．
///
/// [`crate::project::matrix`] の 2 列と同じ向きだが，無理数を通さない．
fn basis(
    projection: Projection,
    plane: SourcePlane,
    facing: Facing,
    step: Step,
    cell: i32,
) -> (IVec2, IVec2) {
    let sign = match facing {
        Facing::Right => 1,
        Facing::Left => -1,
    };
    let (run, rise) = (step.run as i32, step.rise as i32);
    match (projection, plane) {
        // 斜投影を真上から見た絵に掛けると，横幅はそのままで奥行きが倒れる
        (Projection::Oblique, SourcePlane::Top) => {
            (ivec2(cell * run, 0), ivec2(sign * cell * run, cell * rise))
        }
        // 横から見た絵 — 受ける軸だけ倒れ，縦は立ったまま
        (_, SourcePlane::Side) => (ivec2(cell * run, sign * cell * rise), ivec2(0, cell * run)),
        // 真上から見た絵 — 2 軸とも倒れる
        (_, SourcePlane::Top) => (
            ivec2(cell * run, sign * cell * rise),
            ivec2(-sign * cell * run, cell * rise),
        ),
    }
}

/// 画素がどの升に属するかを返す (格子の逆写像)．
fn cell_of(p: IVec2, u: IVec2, v: IVec2, det: i64) -> (i64, i64) {
    let (px, py) = (p.x as i64, p.y as i64);
    let a = px * v.y as i64 - py * v.x as i64;
    let b = u.x as i64 * py - u.y as i64 * px;
    (a.div_euclid(det), b.div_euclid(det))
}

/// ガイドを引く．
pub fn guide(opts: &GuideOptions) -> Result<(IndexedCanvas, Palette, GuideReport)> {
    if opts.cell == 0 || opts.size.x == 0 || opts.size.y == 0 {
        return Err(CoreError::GuideBadSize {
            cell: opts.cell,
            width: opts.size.x,
            height: opts.size.y,
        });
    }
    let step = opts.step();
    if opts.step.is_some()
        && !opts.projection.step_is_free()
        && step != opts.projection.default_step()
    {
        return Err(CoreError::ProjectBadStep {
            spec: format!(
                "{} は段が {} に決まっている (段を選べるのは oblique だけ)",
                opts.projection.as_str(),
                opts.projection.default_step().label()
            ),
        });
    }

    let (u, v) = basis(
        opts.projection,
        opts.plane,
        opts.facing,
        step,
        opts.cell as i32,
    );
    let det = u.x as i64 * v.y as i64 - u.y as i64 * v.x as i64;
    if det == 0 {
        return Err(CoreError::ResampleDegenerate {
            matrix: [u.x as f32, v.x as f32, u.y as f32, v.y as f32],
        });
    }

    let (w, h) = (opts.size.x as i32, opts.size.y as i32);
    let mut canvas = IndexedCanvas::filled(opts.size.x, opts.size.y, IDX_TRANSPARENT);
    canvas.set_transparent(Some(IDX_TRANSPARENT));

    // 塗り分けは線より先 — 線が上に乗る
    let mut cells: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    for y in 0..h {
        for x in 0..w {
            let (i, j) = cell_of(ivec2(x, y), u, v, det);
            *cells.entry((i, j)).or_default() += 1;
            if opts.checker {
                let index = if (i + j).rem_euclid(2) == 0 {
                    IDX_CHECKER_A
                } else {
                    IDX_CHECKER_B
                };
                canvas.set(x, y, index);
            }
        }
    }

    // 4 隅がどの升に来るかから，引くべき格子点の範囲を取る
    let corners = [ivec2(0, 0), ivec2(w, 0), ivec2(0, h), ivec2(w, h)];
    let (mut lo, mut hi) = ((i64::MAX, i64::MAX), (i64::MIN, i64::MIN));
    for c in corners {
        let (i, j) = cell_of(c, u, v, det);
        lo = (lo.0.min(i), lo.1.min(j));
        hi = (hi.0.max(i), hi.1.max(j));
    }
    // 端の升も閉じるので 1 つ広げる
    let (i0, i1) = (lo.0 - 1, hi.0 + 1);
    let (j0, j1) = (lo.1 - 1, hi.1 + 1);

    let point = |i: i64, j: i64| {
        ivec2(
            (i * u.x as i64 + j * v.x as i64) as i32,
            (i * u.y as i64 + j * v.y as i64) as i32,
        )
    };

    let mut line_pixels = 0usize;
    for j in j0..=j1 {
        for i in i0..=i1 {
            for to in [point(i + 1, j), point(i, j + 1)] {
                for p in line(point(i, j), to) {
                    if canvas.set_at(p, IDX_LINE) {
                        line_pixels += 1;
                    }
                }
            }
        }
    }

    // **升が正方形タイル何枚にまたがるか** — 設計書 6.13 の «2 枚 / 4 枚»
    let tile = (opts.cell as i32 * step.run as i32).max(1);
    let mut tile_span: BTreeMap<usize, usize> = BTreeMap::new();
    let mut spans: BTreeMap<(i64, i64), std::collections::BTreeSet<(i32, i32)>> = BTreeMap::new();
    for y in 0..h {
        for x in 0..w {
            let key = cell_of(ivec2(x, y), u, v, det);
            spans
                .entry(key)
                .or_default()
                .insert((x.div_euclid(tile), y.div_euclid(tile)));
        }
    }
    // **画布の縁で切れた升は数えない** — またぐ枚数が «切れたから» 減るのは
    // 主張の反証にならない (D104 «測れない の理由も分ける»)
    let full = |key: &(i64, i64)| -> bool {
        let n = cells.get(key).copied().unwrap_or(0);
        n as i64 == det.abs()
    };
    let mut counted = 0usize;
    for (key, tiles) in &spans {
        if !full(key) {
            continue;
        }
        counted += 1;
        *tile_span.entry(tiles.len()).or_default() += 1;
    }

    // **塗り分けが本当に «チェス盤» になっているか**を数える
    let mut same_colour_adjacent = 0usize;
    if opts.checker {
        for (i, j) in spans.keys().copied() {
            for (di, dj) in [(1i64, 0i64), (0, 1)] {
                let other = (i + di, j + dj);
                if !spans.contains_key(&other) {
                    continue;
                }
                let a = (i + j).rem_euclid(2);
                let b = (other.0 + other.1).rem_euclid(2);
                if a == b {
                    same_colour_adjacent += 1;
                }
            }
        }
    }

    let palette = Palette::new(vec![
        Rgba8::TRANSPARENT,
        Rgba8::rgb(0xf4, 0xf4, 0xf4),
        Rgba8::rgb(0x33, 0x3c, 0x57),
        Rgba8::rgb(0x1a, 0x1c, 0x2c),
    ])?;

    let cell_size = UVec2 {
        x: (u.x.abs() + v.x.abs()).unsigned_abs(),
        y: (u.y.abs() + v.y.abs()).unsigned_abs(),
    };

    Ok((
        canvas,
        palette,
        GuideReport {
            projection: opts.projection.as_str(),
            plane: opts.plane.as_str(),
            facing: opts.facing.as_str(),
            step,
            basis: (u, v),
            tile: tile as u32,
            cell_size,
            line_pixels,
            cells: counted,
            tile_span,
            same_colour_adjacent,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::uvec2;

    fn opts(projection: Projection, plane: SourcePlane) -> GuideOptions {
        GuideOptions {
            projection,
            plane,
            facing: Facing::Right,
            step: None,
            cell: 8,
            size: uvec2(128, 128),
            checker: true,
        }
    }

    /// **壊れると: 格子が画素に乗らなくなる (刻みが整数でない)．**
    #[test]
    fn the_lattice_step_is_an_integer_pair() {
        let (u, v) = basis(
            Projection::Iso,
            SourcePlane::Top,
            Facing::Right,
            Step::TWO_TO_ONE,
            8,
        );
        assert_eq!(u, ivec2(16, 8), "等角の刻みは (2, 1) の 8 倍のはず");
        assert_eq!(v, ivec2(-16, 8));
    }

    /// **壊れると: ガイドの向きが投影とずれる．**
    ///
    /// ガイドの線族は投影が倒した軸そのものなので，**行列の 2 列と符号も
    /// 傾きも一致していなければならない** (決め方が 2 つあってはいけない．D110)．
    #[test]
    fn the_guide_follows_the_same_axes_as_the_projection() {
        for projection in Projection::ALL {
            for plane in SourcePlane::ALL {
                for facing in Facing::ALL {
                    let step = projection.default_step();
                    let m = crate::project::matrix(projection, plane, facing, step);
                    let (u, v) = basis(projection, plane, facing, step, 8);
                    for (col, axis, name) in [((m[0], m[2]), u, "x"), ((m[1], m[3]), v, "y")] {
                        // 向きが同じなら外積が 0 になる
                        let cross = col.0 * axis.y as f32 - col.1 * axis.x as f32;
                        assert!(
                            cross.abs() < 1e-3,
                            "{}/{plane:?}/{facing:?}: {name} 軸が投影とずれている \
                             (行列 {col:?} 対 刻み {axis:?})",
                            projection.as_str()
                        );
                    }
                }
            }
        }
    }

    /// **壊れると: チェス盤の塗り分けが破れる (隣の升が同じ色になる)．**
    ///
    /// 格子は平行四辺形のタイル貼りなので $(i + j) \bmod 2$ で必ず塗り分く —
    /// 測定ではなく数え上げである．
    #[test]
    fn the_checkerboard_never_puts_the_same_colour_side_by_side() {
        for projection in Projection::ALL {
            for plane in SourcePlane::ALL {
                let (_, _, r) = guide(&opts(projection, plane)).unwrap();
                assert_eq!(
                    r.same_colour_adjacent,
                    0,
                    "{}/{plane:?} で塗り分けが破れた",
                    projection.as_str()
                );
            }
        }
    }

    /// **壊れると: 設計書 6.13 の «2 枚 / 4 枚でまたがる» が崩れる．**
    ///
    /// 数え上げなので校正しない (D92 ・D101 ・D102 と同じ側)．一辺 `cell x 走り`
    /// の正方形タイルに対し，**等角のひし形はちょうど 2 枚か 4 枚にまたがる** —
    /// 幅がタイル 2 枚ぶんで高さが 1 枚ぶんなので，縦にずれた升だけが 4 枚になる．
    ///
    /// **枚数はタイルの大きさで変わる**ので，何を基準に数えたかを併記しないと
    /// 読めない (D100 と同じ) — だから [`GuideReport::tile`] を返している．
    #[test]
    fn an_iso_cell_spans_exactly_two_or_four_square_tiles() {
        let (_, _, r) = guide(&opts(Projection::Iso, SourcePlane::Top)).unwrap();
        assert!(r.cells >= 20, "収まりきった升が足りない: {}", r.cells);
        assert_eq!(r.tile, 16, "タイルの一辺は 刻み x 走り のはず");
        let spans: Vec<usize> = r.tile_span.keys().copied().collect();
        assert_eq!(
            spans,
            vec![2, 4],
            "またがりが 2 枚 / 4 枚以外になった: {spans:?}"
        );
        assert_eq!(
            r.tile_span.values().sum::<usize>(),
            r.cells,
            "数えた升の合計が合わない"
        );
    }

    /// **壊れると: 横から見たガイドの縦線が倒れる．**
    #[test]
    fn a_side_view_guide_keeps_one_family_of_lines_vertical() {
        for projection in Projection::ALL {
            let (_, _, r) = guide(&opts(projection, SourcePlane::Side)).unwrap();
            assert_eq!(
                r.basis.1.x,
                0,
                "{}: 縦の線族が倒れている ({:?})",
                projection.as_str(),
                r.basis.1
            );
        }
    }

    /// **壊れると: ガイドの線が 1 本も引かれない．**
    #[test]
    fn a_guide_actually_draws_lines() {
        let (canvas, _, r) = guide(&opts(Projection::Iso, SourcePlane::Top)).unwrap();
        assert!(r.line_pixels > 0, "線が引かれていない");
        assert!(r.cells > 0, "升が 1 つも載っていない");
        assert!(canvas.pixels().contains(&IDX_LINE), "画布に線の添字が無い");
    }

    /// **壊れると: 塗り分けを頼んでいないのに塗る．**
    #[test]
    fn without_the_checkerboard_only_lines_are_drawn() {
        let mut o = opts(Projection::Iso, SourcePlane::Top);
        o.checker = false;
        let (canvas, _, r) = guide(&o).unwrap();
        assert_eq!(r.same_colour_adjacent, 0, "塗っていないのに数えている");
        assert!(
            !canvas
                .pixels()
                .iter()
                .any(|i| *i == IDX_CHECKER_A || *i == IDX_CHECKER_B),
            "塗り分けを頼んでいないのに塗った"
        );
    }

    /// **壊れると: 潰れた設定を通す．**
    #[test]
    fn a_zero_cell_or_size_is_an_error() {
        for (cell, size) in [(0u32, uvec2(64, 64)), (8, uvec2(0, 64)), (8, uvec2(64, 0))] {
            let mut o = opts(Projection::Iso, SourcePlane::Top);
            o.cell = cell;
            o.size = size;
            assert!(guide(&o).is_err(), "cell {cell} ・寸法 {size:?} を通した");
        }
    }
}
