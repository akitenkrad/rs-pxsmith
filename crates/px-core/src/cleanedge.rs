//! cleanEdge の移植 (`px scale --algo cleanedge` / `px rotate --algo cleanedge`)．
//!
//! 移植元は **torcado の cleanEdge シェーダ (MIT)** [^cleanedge]．
//! 著作権表示は `NOTICE` に置いてある．
//!
//! # なぜ添字のまま書けるのか
//!
//! **シェーダの戻り値はすべて «標本した近傍の色» そのものである** — `mix` が
//! 1 度も現れない．したがって出力は必ず入力に在る色であり，**添字の選択**として
//! 書き直せる (D94 «並べ替えるだけの道具は色を作らない») ．インデックスカラーで
//! 混ぜたらパレットに無い色ができるので (D2 ・D4) ，この性質が無ければ
//! そもそも採用できなかった．
//!
//! 色の比較 (`similar` ・`higher` ・色距離) にはパレットの RGBA が要るので，
//! パレットを一緒に渡す．
//!
//! # 仕様は引いた．書いていないものは書いていないと言う
//!
//! `SLOPE` (2:1 の傾斜) と `CLEANUP` (傾斜の遷移の手当て) は**どちらも有効**の
//! ままにしてある — シェーダの既定と同じである．`highestColor` ・
//! `similarThreshold` ・`lineWidth` も既定を引いている．
//!
//! > [!note] **画布の外は縁を伸ばす．**
//! > シェーダは `texture` の clamp-to-edge を前提にしている．スプライトの縁は
//! > ふつう透明なので «外は透明» と同じ結果になるが，**前提が違えば結果も違う**
//! > ので引いた側に合わせてある．
//!
//! [^cleanedge]: <https://torcado.com/cleanEdge/> ／ シェーダ本体は
//!   <https://gist.github.com/torcado194/e2794f5a4b22049ac0a41f972d14c329>

use crate::canvas::IndexedCanvas;
use crate::color::Rgba8;
use crate::math::Vec2;
use crate::palette::Palette;

/// 2:1 の傾斜を使うか (シェーダの `SLOPE`)．
const SLOPE: bool = true;
/// 傾斜の遷移を手当てするか (シェーダの `CLEANUP`)．
const CLEANUP: bool = true;
/// 優先度の基準色 (シェーダの `highestColor`)．明るい方が手前に来る．
const HIGHEST_COLOUR: [f32; 3] = [1.0, 1.0, 1.0];
/// 同じ色とみなす閾値 (シェーダの `similarThreshold`)．**0 は «厳密に同じ»**．
const SIMILAR_THRESHOLD: f32 = 0.0;
/// 線の太さ (シェーダの `lineWidth`)．
const LINE_WIDTH: f32 = 1.0;

#[derive(Copy, Clone, PartialEq)]
struct Col {
    index: u8,
    rgba: [f32; 4],
}

fn col_of(canvas: &IndexedCanvas, palette: &Palette, x: i32, y: i32) -> Col {
    // **画布の外は縁を伸ばす** (clamp-to-edge)
    let cx = x.clamp(0, canvas.width() as i32 - 1);
    let cy = y.clamp(0, canvas.height() as i32 - 1);
    let index = canvas.get(cx, cy).unwrap_or(0);
    let c = palette.get(index).unwrap_or(Rgba8::TRANSPARENT);
    Col {
        index,
        rgba: [
            c.r as f32 / 255.0,
            c.g as f32 / 255.0,
            c.b as f32 / 255.0,
            c.a as f32 / 255.0,
        ],
    }
}

fn dist4(a: [f32; 4], b: [f32; 4]) -> f32 {
    let mut s = 0.0;
    for k in 0..4 {
        s += (a[k] - b[k]) * (a[k] - b[k]);
    }
    s.sqrt()
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let mut s = 0.0;
    for k in 0..3 {
        s += (a[k] - b[k]) * (a[k] - b[k]);
    }
    s.sqrt()
}

fn similar(a: Col, b: Col) -> bool {
    (a.rgba[3] == 0.0 && b.rgba[3] == 0.0) || dist4(a.rgba, b.rgba) <= SIMILAR_THRESHOLD
}

fn similar3(a: Col, b: Col, c: Col) -> bool {
    similar(a, b) && similar(b, c)
}

fn similar4(a: Col, b: Col, c: Col, d: Col) -> bool {
    similar(a, b) && similar(b, c) && similar(c, d)
}

/// 優先度．**似ていれば «高い» ではない**．
fn higher(this: Col, other: Col) -> bool {
    if similar(this, other) {
        return false;
    }
    if this.rgba[3] == other.rgba[3] {
        let rgb = |c: Col| [c.rgba[0], c.rgba[1], c.rgba[2]];
        dist3(rgb(this), HIGHEST_COLOUR) < dist3(rgb(other), HIGHEST_COLOUR)
    } else {
        this.rgba[3] > other.rgba[3]
    }
}

/// 色距離．
fn cd(a: Col, b: Col) -> f32 {
    dist4(a.rgba, b.rgba)
}

fn dist_to_line(test: Vec2, p1: Vec2, p2: Vec2, dir: Vec2) -> f32 {
    let line = Vec2 {
        x: p2.x - p1.x,
        y: p2.y - p1.y,
    };
    let perp = Vec2 {
        x: line.y,
        y: -line.x,
    };
    let to_p1 = Vec2 {
        x: p1.x - test.x,
        y: p1.y - test.y,
    };
    let len = (perp.x * perp.x + perp.y * perp.y).sqrt();
    if len == 0.0 {
        return 0.0;
    }
    let n = Vec2 {
        x: perp.x / len,
        y: perp.y / len,
    };
    let sign = if perp.x * dir.x + perp.y * dir.y > 0.0 {
        1.0
    } else {
        -1.0
    };
    sign * (n.x * to_p1.x + n.y * to_p1.y)
}

/// `center + vec2(a, b) * pointDir`．
fn at(a: f32, b: f32, dir: Vec2) -> Vec2 {
    Vec2 {
        x: 0.5 + a * dir.x,
        y: 0.5 + b * dir.y,
    }
}

/// 1 つの «切り口» を判定する (シェーダの `sliceDist`)．
///
/// 引数の並びは移植元のままである — 呼ぶ側が近傍を入れ替えて 3 回呼ぶ．
#[allow(clippy::too_many_arguments)]
fn slice(
    point: Vec2,
    main_dir: Vec2,
    point_dir: Vec2,
    ub: Col,
    u: Col,
    uf: Col,
    uff: Col,
    b: Col,
    c: Col,
    f: Col,
    ff: Col,
    db: Col,
    d: Col,
    df: Col,
    dff: Col,
    ddb: Col,
    dd: Col,
    ddf: Col,
) -> Option<Col> {
    let (min_width, max_width) = if SLOPE { (0.45, 1.142) } else { (0.0, 1.4) };
    let line_width = LINE_WIDTH.clamp(min_width, max_width);
    // 点を折り返す
    let point = Vec2 {
        x: main_dir.x * (point.x - 0.5) + 0.5,
        y: main_dir.y * (point.y - 0.5) + 0.5,
    };

    // 縁の検出
    let dist_against = 4.0 * cd(f, d) + cd(uf, c) + cd(c, db) + cd(ff, df) + cd(df, dd);
    let dist_towards = 4.0 * cd(c, df) + cd(u, f) + cd(f, dff) + cd(b, d) + cd(d, ddf);
    let mut should_slice =
        dist_against < dist_towards || ((dist_against < dist_towards + 0.001) && !higher(c, f));
    if similar4(f, d, b, u) && similar4(uf, df, db, ub) && !similar(c, f) {
        // 市松の場合
        should_slice = false;
    }
    if !should_slice {
        return None;
    }

    let mut dist;
    let mut flip = false;
    let pd = point_dir;

    if SLOPE && similar3(f, d, db) && !similar3(f, d, b) && !similar(uf, db) {
        // 浅い 2:1 の傾斜 (下)
        if similar(c, df) && higher(c, f) {
            // 1 画素幅の斜線．折り返さない
        } else {
            if higher(c, f) {
                flip = true;
            }
            if similar(u, f) && !similar(c, df) && !higher(c, u) {
                flip = true;
            }
        }
        dist = if flip {
            line_width
                - dist_to_line(
                    point,
                    at(1.5, -1.0, pd),
                    at(-0.5, 0.0, pd),
                    Vec2 { x: -pd.x, y: -pd.y },
                )
        } else {
            dist_to_line(point, at(1.5, 0.0, pd), at(-0.5, 1.0, pd), pd)
        };
        if CLEANUP
            && !flip
            && similar(c, uf)
            && !(similar3(c, uf, uff) && !similar3(c, uf, ff) && !similar(d, uff))
        {
            let d2 = dist_to_line(point, at(2.0, -1.0, pd), at(0.0, 1.0, pd), pd);
            dist = dist.min(d2);
        }
        dist -= line_width / 2.0;
        return (dist <= 0.0).then(|| if cd(c, f) <= cd(c, d) { f } else { d });
    }

    if SLOPE && similar3(uf, f, d) && !similar3(u, f, d) && !similar(uf, db) {
        // 急な 2:1 の傾斜 (前)
        if similar(c, df) && higher(c, d) {
        } else {
            if higher(c, d) {
                flip = true;
            }
            if similar(b, d) && !similar(c, df) && !higher(c, d) {
                flip = true;
            }
        }
        dist = if flip {
            line_width
                - dist_to_line(
                    point,
                    at(0.0, -0.5, pd),
                    at(-1.0, 1.5, pd),
                    Vec2 { x: -pd.x, y: -pd.y },
                )
        } else {
            dist_to_line(point, at(1.0, -0.5, pd), at(0.0, 1.5, pd), pd)
        };
        if CLEANUP
            && !flip
            && similar(c, db)
            && !(similar3(c, db, ddb) && !similar3(c, db, dd) && !similar(f, ddb))
        {
            let d2 = dist_to_line(point, at(1.0, 0.0, pd), at(-1.0, 2.0, pd), pd);
            dist = dist.min(d2);
        }
        dist -= line_width / 2.0;
        return (dist <= 0.0).then(|| if cd(c, f) <= cd(c, d) { f } else { d });
    }

    if similar(f, d) {
        // 45 度の斜線
        if similar(c, df) && higher(c, f) {
            if !similar(c, dd) && !similar(c, ff) {
                flip = true;
            }
        } else {
            if higher(c, f) {
                flip = true;
            }
            if !similar(c, b) && similar4(b, f, d, u) {
                flip = true;
            }
        }
        if ((similar(f, db) && similar3(u, f, df)) || (similar(uf, d) && similar3(b, d, df)))
            && !similar(c, df)
        {
            flip = true;
        }
        dist = if flip {
            line_width
                - dist_to_line(
                    point,
                    at(1.0, -1.0, pd),
                    at(-1.0, 1.0, pd),
                    Vec2 { x: -pd.x, y: -pd.y },
                )
        } else {
            dist_to_line(point, at(1.0, 0.0, pd), at(0.0, 1.0, pd), pd)
        };
        if SLOPE && CLEANUP && !flip {
            if similar3(c, uf, uff) && !similar3(c, uf, ff) && !similar(d, uff) {
                let d2 = dist_to_line(point, at(1.5, 0.0, pd), at(-0.5, 1.0, pd), pd);
                dist = dist.max(d2);
            }
            if similar3(ddb, db, c) && !similar3(dd, db, c) && !similar(ddb, f) {
                let d2 = dist_to_line(point, at(1.0, -0.5, pd), at(0.0, 1.5, pd), pd);
                dist = dist.max(d2);
            }
        }
        dist -= line_width / 2.0;
        return (dist <= 0.0).then(|| if cd(c, f) <= cd(c, d) { f } else { d });
    }

    if SLOPE && similar3(ff, df, d) && !similar3(ff, df, c) && !similar(uff, d) {
        // 浅い傾斜の遠い角
        if similar(f, dff) && higher(f, ff) {
        } else {
            if higher(f, ff) {
                flip = true;
            }
            if similar(uf, ff) && !similar(f, dff) && !higher(f, uf) {
                flip = true;
            }
        }
        dist = if flip {
            line_width
                - dist_to_line(
                    point,
                    at(2.5, -1.0, pd),
                    at(0.5, 0.0, pd),
                    Vec2 { x: -pd.x, y: -pd.y },
                )
        } else {
            dist_to_line(point, at(2.5, 0.0, pd), at(0.5, 1.0, pd), pd)
        };
        dist -= line_width / 2.0;
        return (dist <= 0.0).then(|| if cd(f, ff) <= cd(f, df) { ff } else { df });
    }

    if SLOPE && similar3(f, df, dd) && !similar3(c, df, dd) && !similar(f, ddb) {
        // 急な傾斜の遠い角
        if similar(d, ddf) && higher(d, dd) {
        } else {
            if higher(d, dd) {
                flip = true;
            }
            if similar(db, dd) && !similar(d, ddf) && !higher(d, dd) {
                flip = true;
            }
        }
        dist = if flip {
            line_width
                - dist_to_line(
                    point,
                    at(0.0, 0.5, pd),
                    at(-1.0, 2.5, pd),
                    Vec2 { x: -pd.x, y: -pd.y },
                )
        } else {
            dist_to_line(point, at(1.0, 0.5, pd), at(0.0, 2.5, pd), pd)
        };
        dist -= line_width / 2.0;
        return (dist <= 0.0).then(|| if cd(d, df) <= cd(d, dd) { df } else { dd });
    }

    None
}

/// 入力の座標 `p` (画素単位) を標本する．**戻るのは入力に在る添字だけ**である．
pub fn sample(canvas: &IndexedCanvas, palette: &Palette, p: Vec2) -> Option<u8> {
    let (tx, ty) = (p.x.floor(), p.y.floor());
    if tx < 0.0 || ty < 0.0 || tx >= canvas.width() as f32 || ty >= canvas.height() as f32 {
        return None;
    }
    let (ix, iy) = (tx as i32, ty as i32);
    let local = Vec2 {
        x: p.x - tx,
        y: p.y - ty,
    };
    // 画素の中のどの象限にいるか
    let pd = Vec2 {
        x: local.x.round() * 2.0 - 1.0,
        y: local.y.round() * 2.0 - 1.0,
    };
    let g = |dx: f32, dy: f32| -> Col {
        col_of(
            canvas,
            palette,
            ix + (dx * pd.x) as i32,
            iy + (dy * pd.y) as i32,
        )
    };

    let (uub, uu, uuf) = (g(-1.0, -2.0), g(0.0, -2.0), g(1.0, -2.0));
    let (ubb, ub, u, uf, uff) = (
        g(-2.0, -2.0),
        g(-1.0, -1.0),
        g(0.0, -1.0),
        g(1.0, -1.0),
        g(2.0, -1.0),
    );
    let (bb, b, c, f, ff) = (
        g(-2.0, 0.0),
        g(-1.0, 0.0),
        g(0.0, 0.0),
        g(1.0, 0.0),
        g(2.0, 0.0),
    );
    let (dbb, db, d, df, dff) = (
        g(-2.0, 1.0),
        g(-1.0, 1.0),
        g(0.0, 1.0),
        g(1.0, 1.0),
        g(2.0, 1.0),
    );
    let (ddb, dd, ddf) = (g(-1.0, 2.0), g(0.0, 2.0), g(1.0, 2.0));

    let mut col = c;
    // 角 ・後ろ ・上の 3 つの切り口 (他の象限からはこの 3 つしか届かない)
    let c_col = slice(
        local,
        Vec2 { x: 1.0, y: 1.0 },
        pd,
        ub,
        u,
        uf,
        uff,
        b,
        c,
        f,
        ff,
        db,
        d,
        df,
        dff,
        ddb,
        dd,
        ddf,
    );
    let b_col = slice(
        local,
        Vec2 { x: -1.0, y: 1.0 },
        pd,
        uf,
        u,
        ub,
        ubb,
        f,
        c,
        b,
        bb,
        df,
        d,
        db,
        dbb,
        ddf,
        dd,
        ddb,
    );
    let u_col = slice(
        local,
        Vec2 { x: 1.0, y: -1.0 },
        pd,
        db,
        d,
        df,
        dff,
        b,
        c,
        f,
        ff,
        ub,
        u,
        uf,
        uff,
        uub,
        uu,
        uuf,
    );

    if let Some(v) = c_col {
        col = v;
    }
    if let Some(v) = b_col {
        col = v;
    }
    if let Some(v) = u_col {
        col = v;
    }
    Some(col.index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::IndexedCanvas;

    fn setup() -> (IndexedCanvas, Palette) {
        // 斜めの階段 — cleanEdge が作り直す対象そのもの
        let palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0x1a, 0x1c, 0x2c),
            Rgba8::rgb(0xf4, 0xf4, 0xf4),
        ])
        .unwrap();
        let mut c = IndexedCanvas::filled(8, 8, 0);
        c.set_transparent(Some(0));
        for y in 0..8i32 {
            for x in 0..8i32 {
                c.set(x, y, if x <= y { 1 } else { 2 });
            }
        }
        (c, palette)
    }

    /// **壊れると: 入力に無い添字を返す (D94 の不変条件)．**
    #[test]
    fn every_sample_is_an_index_that_was_in_the_source() {
        let (c, p) = setup();
        for k in 0..400 {
            let q = Vec2 {
                x: (k % 20) as f32 * 0.4,
                y: (k / 20) as f32 * 0.4,
            };
            if let Some(i) = sample(&c, &p, q) {
                assert!(c.pixels().contains(&i), "入力に無い添字 {i} を返した");
            }
        }
    }

    /// **壊れると: 画素の中心で元の色と違うものを返す (恒等でなくなる)．**
    ///
    /// 標本が画素の中心なら，切り口に掛からない限り元の色が返るはずである．
    #[test]
    fn sampling_a_flat_area_at_its_centre_returns_that_colour() {
        let palette = Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(0x1a, 0x1c, 0x2c)]).unwrap();
        let mut c = IndexedCanvas::filled(8, 8, 1);
        c.set_transparent(Some(0));
        for y in 2..6i32 {
            for x in 2..6i32 {
                assert_eq!(
                    sample(
                        &c,
                        &palette,
                        Vec2 {
                            x: x as f32 + 0.5,
                            y: y as f32 + 0.5
                        }
                    ),
                    Some(1),
                    "平らな面の中心で色が変わった ({x}, {y})"
                );
            }
        }
    }

    /// **壊れると: 画布の外を標本して落ちる．**
    #[test]
    fn sampling_outside_the_canvas_is_none() {
        let (c, p) = setup();
        assert_eq!(sample(&c, &p, Vec2 { x: -0.5, y: 4.0 }), None);
        assert_eq!(sample(&c, &p, Vec2 { x: 4.0, y: 8.5 }), None);
    }

    /// **壊れると: 階段の縁を 1 つも作り直さない (移植が効いていない)．**
    #[test]
    fn the_staircase_edge_is_actually_reshaped() {
        let (c, p) = setup();
        // 画布を細かく標本して，最近傍と違う答えが出る場所を数える
        let mut differs = 0;
        let mut total = 0;
        for gy in 0..64 {
            for gx in 0..64 {
                let q = Vec2 {
                    x: gx as f32 * 0.125,
                    y: gy as f32 * 0.125,
                };
                let nearest = c.get(q.x.floor() as i32, q.y.floor() as i32);
                let got = sample(&c, &p, q);
                if got.is_some() {
                    total += 1;
                    if got != nearest {
                        differs += 1;
                    }
                }
            }
        }
        assert!(
            differs > 0,
            "最近傍とまったく同じなら移植が効いていない ({total} 点を見た)"
        );
        assert!(
            differs * 4 < total,
            "違いが多すぎる — {differs} / {total}．縁だけが変わるはず"
        );
    }
}
