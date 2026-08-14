//! 拡縮と回転 (`px scale` / `px rotate`．設計書 5 章 ・D18)．
//!
//! **下地を生成する機能であり完成品を出すものではない** (設計書 1.3) ．参考書籍も
//! «ツールに頼りすぎず最後は手動で調整する» と述べている．
//!
//! # 標本を選ぶ道具である — 色は作らない
//!
//! 拡縮も回転も «出力の画素が，入力のどこを見るか» を決める仕事に落ちる．
//! **どの流儀でも «入力に在る添字» をそのまま置く** — 混ぜない (D94) ．
//! インデックスカラーで混ぜたら，パレットに無い色を作ることになる (D2 ・D4) ．
//!
//! これは移植元の cleanEdge も同じである — シェーダの戻り値はすべて標本した
//! 近傍の色そのもので，`mix` は 1 度も現れない．**だから添字のまま実装できる**．
//!
//! # 画布は広げる
//!
//! 回転すると外接矩形が必ず大きくなるので，元の画布に収めると切れる．
//! D93 (`compose`) ・D123 (`squash`) と同じ答えだが，**理由は測定ではなく幾何**
//! である — 90 度の倍数以外では外接矩形が必ず伸びる．
//!
//! # 真値のある場面
//!
//! | 場面 | 真値 |
//! | --- | --- |
//! | 90 度の倍数の回転 | **厳密**．添字の置き換えだけで書ける |
//! | 整数倍の拡大 | **厳密**．1 画素が $k \times k$ になる |
//! | $\theta$ 回して $-\theta$ 戻す | 元に戻るはず (往復) |
//! | 対照 | **`nearest`** — これに勝てない流儀は要らない |

use crate::canvas::IndexedCanvas;
use crate::error::{CoreError, Result};
use crate::math::{IRect, Vec2};
use crate::palette::Palette;

/// 拡縮 ・回転の流儀．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ResampleAlgo {
    /// 最近傍．**90 度の倍数と整数倍では厳密**である．
    #[default]
    Nearest,
    /// cleanEdge (torcado, MIT) の移植．**斜めの縁を作り直す**．
    CleanEdge,
}

impl ResampleAlgo {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::CleanEdge => "cleanedge",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "nearest" => Some(Self::Nearest),
            "cleanedge" => Some(Self::CleanEdge),
            _ => None,
        }
    }

    pub const ALL: [Self; 2] = [Self::Nearest, Self::CleanEdge];
}

/// 設定．
#[derive(Copy, Clone, Debug)]
pub struct ResampleOptions {
    pub algo: ResampleAlgo,
    /// 画布を広げるか．**既定は広げる** — 回転すると外接矩形が必ず伸びる．
    pub grow: bool,
}

impl Default for ResampleOptions {
    fn default() -> Self {
        Self {
            algo: ResampleAlgo::default(),
            grow: true,
        }
    }
}

/// 結果の素性．
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResampleReport {
    pub algo_name: &'static str,
    /// 画布の寸法 (前, 後)．
    pub size: ((u32, u32), (u32, u32)),
    /// 不透明だった画素の数 (前, 後)．
    pub opaque: (usize, usize),
    /// **画布からはみ出て捨てた画素の数．広げていれば 0 のはず**．
    pub clipped: usize,
    /// **作った色の数．常に 0 でなければならない** (D94)．
    pub created_colours: usize,
    /// **透明の宣言が無いので実色で埋めた画素の数．**
    ///
    /// 広げた画布のうち標本が当たらなかったところは «何も無い» で埋めたいが，
    /// **透明添字を持たない絵にはその «何も無い» が無い** — 埋め草の添字 0 は
    /// 実色である (D109 «透明添字はパレットのアルファではない» の裏側) ．
    ///
    /// 黙って埋めると **«不透明な画素が 1024 から 1936 に増えた»** とだけ見え，
    /// 増えた 912 が絵なのか埋め草なのか読めない．D92 ・D107 のとおり
    /// **処方せず数えて報告する**．
    pub filled_opaque: usize,
}

impl ResampleReport {
    /// 90 度の倍数か整数倍のように，**厳密に書けた**か．
    pub fn is_exact(&self) -> bool {
        self.clipped == 0 && self.created_colours == 0
    }
}

/// 出力の画素から入力の座標への写像．
#[derive(Copy, Clone, Debug)]
struct Mapping {
    /// 出力の画布．
    out: IRect,
    /// 出力の画素の中心 → 入力の座標．
    matrix: [f32; 4],
    offset: Vec2,
}

impl Mapping {
    fn source_of(&self, x: i32, y: i32) -> Vec2 {
        // 画素の中心で測る
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        Vec2 {
            x: self.matrix[0] * px + self.matrix[1] * py + self.offset.x,
            y: self.matrix[2] * px + self.matrix[3] * py + self.offset.y,
        }
    }
}

/// **前向きの 2x2 行列から逆写像を組む．**
///
/// 回転も投影も «入力の軸をどこへ倒すか» を決めるだけなので，違うのは行列だけ
/// である．**流儀 ・画布の広げ方 ・標本の取り方を 1 か所に集める** — 同じ仕事の
/// 実装を 2 つ作ると必ず食い違う (D110 ・D118) ．
///
/// `forward` は `[a, b, c, d]` で $(x, y) \mapsto (a x + b y,\; c x + d y)$．
/// 退化した行列 (行列式 0) は写せないので `None` を返す．
fn mapping_from_forward(canvas: &IndexedCanvas, forward: [f32; 4], grow: bool) -> Option<Mapping> {
    let (w, h) = (canvas.width() as f32, canvas.height() as f32);
    let [a, b, c, d] = forward;
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() < 1e-6 {
        return None;
    }

    // 出力の画布 — 4 隅を写して外接矩形を取る
    let (ow, oh) = if grow {
        let (mx, my) = (w / 2.0, h / 2.0);
        let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
        for (x, y) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
            let (dx, dy) = (x - mx, y - my);
            let (rx, ry) = (a * dx + b * dy, c * dx + d * dy);
            lo = (lo.0.min(rx), lo.1.min(ry));
            hi = (hi.0.max(rx), hi.1.max(ry));
        }
        (
            (hi.0 - lo.0).ceil().max(1.0) as u32,
            (hi.1 - lo.1).ceil().max(1.0) as u32,
        )
    } else {
        (canvas.width(), canvas.height())
    };

    // 逆写像 (出力 → 入力) を持ち，出力の中心を入力の中心へ戻す
    let inv = [d / det, -b / det, -c / det, a / det];
    let (ocx, ocy) = (ow as f32 / 2.0, oh as f32 / 2.0);
    let offset = Vec2 {
        x: w / 2.0 - (inv[0] * ocx + inv[1] * ocy),
        y: h / 2.0 - (inv[2] * ocx + inv[3] * ocy),
    };
    Some(Mapping {
        out: IRect::new(0, 0, ow, oh),
        matrix: inv,
        offset,
    })
}

fn rotation_mapping(canvas: &IndexedCanvas, degrees: f32, grow: bool) -> Mapping {
    let (s, c) = degrees.to_radians().sin_cos();
    // 回転は退化しないので必ず取れる
    mapping_from_forward(canvas, [c, -s, s, c], grow).expect("回転行列は退化しない")
}

/// **任意の 2x2 行列で写す** — `px project` (設計書 6.13) が使う口．
///
/// `forward` は `[a, b, c, d]` で $(x, y) \mapsto (a x + b y,\; c x + d y)$．
/// 回転と同じ標本を通るので，**投影のために標本を書き直してはいけない**．
pub fn affine(
    canvas: &IndexedCanvas,
    palette: &Palette,
    forward: [f32; 4],
    opts: &ResampleOptions,
) -> Result<(IndexedCanvas, ResampleReport)> {
    let mapping = mapping_from_forward(canvas, forward, opts.grow)
        .ok_or(CoreError::ResampleDegenerate { matrix: forward })?;
    Ok(resample(canvas, palette, &mapping, opts))
}

fn scale_mapping(canvas: &IndexedCanvas, factor: f32) -> Mapping {
    let ow = ((canvas.width() as f32 * factor).round() as u32).max(1);
    let oh = ((canvas.height() as f32 * factor).round() as u32).max(1);
    Mapping {
        out: IRect::new(0, 0, ow, oh),
        matrix: [1.0 / factor, 0.0, 0.0, 1.0 / factor],
        offset: Vec2 { x: 0.0, y: 0.0 },
    }
}

/// 回転する．**角度は度**．
pub fn rotate(
    canvas: &IndexedCanvas,
    palette: &Palette,
    degrees: f32,
    opts: &ResampleOptions,
) -> Result<(IndexedCanvas, ResampleReport)> {
    if !degrees.is_finite() {
        return Err(CoreError::ResampleBadAngle { degrees });
    }
    // **90 度の倍数は厳密に書ける．** 標本の丸めに任せると 1 画素ずれることがある
    let quarter = degrees / 90.0;
    if (quarter - quarter.round()).abs() < 1e-4 {
        return Ok(rotate_quarter(canvas, quarter.round() as i32));
    }
    let mapping = rotation_mapping(canvas, degrees, opts.grow);
    Ok(resample(canvas, palette, &mapping, opts))
}

/// 拡縮する．
pub fn scale(
    canvas: &IndexedCanvas,
    palette: &Palette,
    factor: f32,
    opts: &ResampleOptions,
) -> Result<(IndexedCanvas, ResampleReport)> {
    if !factor.is_finite() || factor <= 0.0 {
        return Err(CoreError::ResampleBadFactor { factor });
    }
    let mapping = scale_mapping(canvas, factor);
    Ok(resample(canvas, palette, &mapping, opts))
}

/// 90 度の倍数の回転．**添字の置き換えだけなので厳密である**．
fn rotate_quarter(canvas: &IndexedCanvas, quarters: i32) -> (IndexedCanvas, ResampleReport) {
    let q = quarters.rem_euclid(4);
    let (w, h) = (canvas.width(), canvas.height());
    let (ow, oh) = if q % 2 == 0 { (w, h) } else { (h, w) };
    let fill = canvas.transparent().unwrap_or(0);
    let mut out = IndexedCanvas::filled(ow, oh, fill);
    out.set_transparent(canvas.transparent());

    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let Some(i) = canvas.get(x, y) else { continue };
            let (nx, ny) = match q {
                0 => (x, y),
                1 => (h as i32 - 1 - y, x),
                2 => (w as i32 - 1 - x, h as i32 - 1 - y),
                _ => (y, w as i32 - 1 - x),
            };
            out.set(nx, ny, i);
        }
    }
    let report = ResampleReport {
        algo_name: "quarter",
        size: ((w, h), (ow, oh)),
        opaque: (opaque_count(canvas), opaque_count(&out)),
        clipped: 0,
        created_colours: 0,
        // 4 分の 1 回転は画布が伸びず全画素が書き換わるので埋め草は出ない
        filled_opaque: 0,
    };
    (out, report)
}

fn opaque_count(canvas: &IndexedCanvas) -> usize {
    let t = canvas.transparent();
    canvas.pixels().iter().filter(|i| t != Some(**i)).count()
}

fn resample(
    canvas: &IndexedCanvas,
    palette: &Palette,
    mapping: &Mapping,
    opts: &ResampleOptions,
) -> (IndexedCanvas, ResampleReport) {
    let fill = canvas.transparent().unwrap_or(0);
    let mut out = IndexedCanvas::filled(mapping.out.w, mapping.out.h, fill);
    out.set_transparent(canvas.transparent());

    let mut unsampled = 0usize;
    for y in 0..mapping.out.h as i32 {
        for x in 0..mapping.out.w as i32 {
            let p = mapping.source_of(x, y);
            let index = match opts.algo {
                ResampleAlgo::Nearest => sample_nearest(canvas, p),
                ResampleAlgo::CleanEdge => crate::cleanedge::sample(canvas, palette, p),
            };
            match index {
                Some(i) => {
                    out.set(x, y, i);
                }
                None => unsampled += 1,
            }
        }
    }

    // **入力の不透明な画素のうち，出力のどこにも現れなかったものを数える**
    let clipped = count_clipped(canvas, mapping);
    // 透明添字を持たない絵では，標本の当たらなかったところが実色で埋まる
    let filled_opaque = if canvas.transparent().is_some() {
        0
    } else {
        unsampled
    };
    let report = ResampleReport {
        algo_name: opts.algo.as_str(),
        size: (
            (canvas.width(), canvas.height()),
            (mapping.out.w, mapping.out.h),
        ),
        opaque: (opaque_count(canvas), opaque_count(&out)),
        clipped,
        created_colours: 0,
        filled_opaque,
    };
    (out, report)
}

/// 出力の画布に載らなかった入力の画素を数える．
///
/// > [!warning] **«標本されなかった画素» を数えてはいけない．**
/// > 等倍の回転では出力の画素数が入力と同じなので，切れていなくても
/// > 標本の当たらない入力画素が出る (最初これで «広げても切れる» と出た) ．
/// > **前向きに写して画布の外に出たものだけ**を数える．
fn count_clipped(canvas: &IndexedCanvas, mapping: &Mapping) -> usize {
    // 逆写像 (出力 → 入力) の逆を取って前向きに写す
    let [a, b, c, d] = mapping.matrix;
    let det = a * d - b * c;
    if det.abs() < f32::EPSILON {
        return 0;
    }
    let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
    let t = canvas.transparent();
    let mut clipped = 0usize;
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            if canvas.get(x, y).is_some_and(|i| t == Some(i)) {
                continue;
            }
            let (sx, sy) = (
                x as f32 + 0.5 - mapping.offset.x,
                y as f32 + 0.5 - mapping.offset.y,
            );
            let (ox, oy) = (ia * sx + ib * sy, ic * sx + id * sy);
            if ox < 0.0 || oy < 0.0 || ox >= mapping.out.w as f32 || oy >= mapping.out.h as f32 {
                clipped += 1;
            }
        }
    }
    clipped
}

fn sample_nearest(canvas: &IndexedCanvas, p: Vec2) -> Option<u8> {
    canvas.get(p.x.floor() as i32, p.y.floor() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::IVec2;

    fn pal() -> Palette {
        Palette::new(vec![
            crate::color::Rgba8::TRANSPARENT,
            crate::color::Rgba8::rgb(0x1a, 0x1c, 0x2c),
            crate::color::Rgba8::rgb(0xb1, 0x3e, 0x53),
            crate::color::Rgba8::rgb(0xf4, 0xf4, 0xf4),
        ])
        .unwrap()
    }

    fn art() -> IndexedCanvas {
        // 左上に印を付けた 4x3
        let mut c = IndexedCanvas::filled(4, 3, 0);
        c.set_transparent(Some(0));
        c.set(0, 0, 1);
        c.set(1, 0, 2);
        c.set(0, 1, 3);
        c
    }

    /// **壊れると: 90 度回転が厳密でなくなる (標本の丸めで 1 画素ずれる)．**
    #[test]
    fn a_quarter_turn_is_exact() {
        let a = art();
        let (b, r) = rotate(&a, &pal(), 90.0, &ResampleOptions::default()).unwrap();
        assert_eq!(b.size(), crate::math::uvec2(3, 4));
        assert_eq!(b.get(2, 0), Some(1), "左上の印が右上へ来る");
        assert_eq!(b.get(2, 1), Some(2));
        assert_eq!(b.get(1, 0), Some(3));
        assert!(r.is_exact());
        assert_eq!(r.opaque.0, r.opaque.1, "画素の数は変わらない");
    }

    /// **壊れると: 4 回まわしても元に戻らない．**
    #[test]
    fn four_quarter_turns_return_the_original() {
        let a = art();
        let mut b = a.clone();
        for _ in 0..4 {
            b = rotate(&b, &pal(), 90.0, &ResampleOptions::default())
                .unwrap()
                .0;
        }
        assert_eq!(b.pixels(), a.pixels());
        assert_eq!(b.size(), a.size());
    }

    /// **壊れると: 整数倍の拡大が厳密でなくなる．**
    #[test]
    fn an_integer_upscale_is_exact() {
        let a = art();
        let (b, r) = scale(&a, &pal(), 3.0, &ResampleOptions::default()).unwrap();
        assert_eq!(b.size(), crate::math::uvec2(12, 9));
        for y in 0..3i32 {
            for x in 0..4i32 {
                for dy in 0..3 {
                    for dx in 0..3 {
                        assert_eq!(
                            b.get(x * 3 + dx, y * 3 + dy),
                            a.get(x, y),
                            "({x}, {y}) の 3x3 が揃っていない"
                        );
                    }
                }
            }
        }
        assert!(r.is_exact());
    }

    /// **壊れると: 回転で絵が切れる (画布を広げていない)．**
    #[test]
    fn rotation_grows_the_canvas_so_nothing_is_clipped() {
        let mut a = IndexedCanvas::filled(16, 16, 0);
        a.set_transparent(Some(0));
        for y in 0..16i32 {
            for x in 0..16i32 {
                a.set(x, y, 1);
            }
        }
        let (b, r) = rotate(&a, &pal(), 30.0, &ResampleOptions::default()).unwrap();
        assert!(b.width() > 16 && b.height() > 16, "画布が広がっていない");
        assert_eq!(r.clipped, 0, "切れた画素がある");
    }

    /// **壊れると: 透明の宣言が無い絵を広げたとき，実色で埋めたことを黙る．**
    ///
    /// 端から端まで CLI で通して出た．全面不透明なタイルを回すと
    /// «不透明な画素 1024 -> 1936» とだけ出て，増えた 912 が絵なのか埋め草なのか
    /// 読めなかった (D128 «透明の宣言が無い絵を «空» と読んでいた» の裏側)．
    #[test]
    fn growing_a_canvas_with_no_transparent_index_says_it_filled_with_a_real_colour() {
        // 透明添字を宣言していない全面不透明の絵
        let opaque = IndexedCanvas::filled(16, 16, 1);
        assert_eq!(opaque.transparent(), None);
        let (_, r) = rotate(&opaque, &pal(), 30.0, &ResampleOptions::default()).unwrap();
        assert!(
            r.filled_opaque > 0,
            "広げた画布を実色で埋めたのに 0 と報告した"
        );

        // 透明添字がある絵では埋め草は出ない
        let mut clear = IndexedCanvas::filled(16, 16, 1);
        clear.set_transparent(Some(0));
        let (_, r) = rotate(&clear, &pal(), 30.0, &ResampleOptions::default()).unwrap();
        assert_eq!(r.filled_opaque, 0, "透明で埋められるのに埋め草を数えた");
    }

    /// **壊れると: 広げないと切れることを黙る．**
    #[test]
    fn refusing_to_grow_clips_and_says_so() {
        let mut a = IndexedCanvas::filled(16, 16, 0);
        a.set_transparent(Some(0));
        for y in 0..16i32 {
            for x in 0..16i32 {
                a.set(x, y, 1);
            }
        }
        let (_, r) = rotate(
            &a,
            &pal(),
            30.0,
            &ResampleOptions {
                grow: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(r.clipped > 0, "広げなければ切れるはず");
    }

    /// **壊れると: 色を作る (D94 の不変条件)．**
    #[test]
    fn resampling_never_creates_an_index_that_was_not_there() {
        let a = art();
        for algo in ResampleAlgo::ALL {
            let opts = ResampleOptions {
                algo,
                ..Default::default()
            };
            let (b, r) = rotate(&a, &pal(), 37.0, &opts).unwrap();
            for i in b.pixels() {
                assert!(
                    a.pixels().contains(i),
                    "{}: 入力に無い添字 {i} が出た",
                    algo.as_str()
                );
            }
            assert_eq!(r.created_colours, 0);
        }
    }

    /// **壊れると: 角度が有限でない値を受け取って壊れる．**
    #[test]
    fn a_bad_angle_or_factor_is_an_error() {
        let a = art();
        assert!(rotate(&a, &pal(), f32::NAN, &ResampleOptions::default()).is_err());
        assert!(scale(&a, &pal(), 0.0, &ResampleOptions::default()).is_err());
        assert!(scale(&a, &pal(), -1.0, &ResampleOptions::default()).is_err());
    }

    /// 逆写像が中心を保つこと (回転の基準がずれていないか)．
    #[test]
    fn the_centre_maps_to_the_centre() {
        let a = IndexedCanvas::filled(9, 9, 0);
        let m = rotation_mapping(&a, 45.0, true);
        let p = m.source_of(m.out.w as i32 / 2, m.out.h as i32 / 2);
        let d = IVec2 {
            x: (p.x - 4.5).abs() as i32,
            y: (p.y - 4.5).abs() as i32,
        };
        assert!(d.x <= 1 && d.y <= 1, "中心が {p:?} へずれている");
    }
}
