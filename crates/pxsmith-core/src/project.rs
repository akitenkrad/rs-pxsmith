//! 投影変換 (`pxsmith project`．設計書 6.13)．
//!
//! **下地を生成する機能であり完成品を出すものではない** (設計書 1.3) ．参考書籍も
//! 投影は手で描き直す前の «当たり» として扱っている．
//!
//! # 投影は «入力のどの軸をどこへ倒すか» でしかない
//!
//! だから[`crate::resample::affine`]に行列を渡すだけで済む．**標本の取り方 ・
//! 画布の広げ方 ・流儀は書き直さない** — 回転の実装が 2 つになった時点で必ず
//! 食い違う (D110 ・D118) ．
//!
//! # 設計書 6.13 の 2 手順は同じ変換ではない
//!
//! 6.13 は «45 度回転 → 高さ 1/2» **または** «幅 0.866 → 垂直 −30 度歪み» と
//! 書くが，並べると別物である．
//!
//! | 手順 | $x$ 軸の行き先 | $y$ 軸の行き先 | 垂直線 |
//! | --- | --- | --- | --- |
//! | 45 度回転 → 高さ 1/2 | (0.707, 0.354) = **2:1** | (−0.707, 0.354) | **倒れる** |
//! | 幅 0.866 → −30 度歪み | (0.866, 0.500) = **1.73:1** | (0, 1) | **立ったまま** |
//!
//! 前者は**真上から見た絵**を床面へ倒す変換，後者は**横から見た絵**を側面へ倒す
//! 変換であって，適用先が違う．だから «または» ではなく [`SourcePlane`] として
//! **どちらの面を写すのかを宣言させる** — 絵からは決まらない (D89) ．
//!
//! # 30 度は格子に乗らない．2:1 に直した
//!
//! 6.13 の表自身が «等角投影 = 2:1 = 26.57 度 (正確な 30 度は引けないため代用)»
//! と書いているのに，手順の方は $\tan 30° = 0.577$ を使っている — **同じ節の中で
//! 食い違っている**．採るのは表の側である．2:1 は走り 2 ・上がり 1 の整数比なので
//! 段が揃うが，$\tan 30°$ は無理数なので段が揃わない (D105 と同じ «測定ではなく
//! 代数» の側) ．
//!
//! # 歪める向きは宣言させる
//!
//! 6.13 は «歪める方向はオブジェクトが向いている方向に合わせる (逆にしない)» と
//! 言うが，**どちらを向いているかは絵からは決まらない**．推測して外れると静かに
//! 壊れる (D111) ので [`Facing`] を必須にした．

use crate::canvas::IndexedCanvas;
use crate::error::{CoreError, Result};
use crate::palette::Palette;
use crate::resample::{ResampleOptions, ResampleReport, affine};

/// 受ける軸の段 — 走り : 上がり．**整数比なので格子に乗る**．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub run: u32,
    pub rise: u32,
}

impl Step {
    pub const TWO_TO_ONE: Self = Self { run: 2, rise: 1 };
    pub const ONE_TO_ONE: Self = Self { run: 1, rise: 1 };

    /// `走り:上がり` を読む．
    pub fn parse(spec: &str) -> Result<Self> {
        let bad = || CoreError::ProjectBadStep {
            spec: spec.to_string(),
        };
        let (a, b) = spec.split_once(':').ok_or_else(bad)?;
        let run: u32 = a.trim().parse().map_err(|_| bad())?;
        let rise: u32 = b.trim().parse().map_err(|_| bad())?;
        if run == 0 || rise == 0 {
            return Err(bad());
        }
        Ok(Self { run, rise })
    }

    /// 上がり / 走り．
    pub fn slope(self) -> f32 {
        self.rise as f32 / self.run as f32
    }

    /// 受ける軸が水平から何度倒れるか．
    pub fn degrees(self) -> f32 {
        self.slope().atan().to_degrees()
    }

    pub fn label(self) -> String {
        format!("{}:{}", self.run, self.rise)
    }
}

/// 投影の種類 (設計書 6.13 の表)．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Projection {
    /// 等角投影．**段は 2:1 に固定** (26.57 度)．
    Iso,
    /// 45 度二等角投影．**段は 1:1 に固定**．
    Dimetric45,
    /// 斜投影．**縮まない (純粋な歪み)**．段は 1:1 と 2:1 から選ぶ．
    Oblique,
}

impl Projection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Iso => "iso",
            Self::Dimetric45 => "dimetric45",
            Self::Oblique => "oblique",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "iso" => Some(Self::Iso),
            "dimetric45" => Some(Self::Dimetric45),
            "oblique" => Some(Self::Oblique),
            _ => None,
        }
    }

    /// 表が定める段．
    pub fn default_step(self) -> Step {
        match self {
            Self::Iso | Self::Oblique => Step::TWO_TO_ONE,
            Self::Dimetric45 => Step::ONE_TO_ONE,
        }
    }

    /// **段を選べるのは斜投影だけである．**
    ///
    /// 6.13 の表で 2 通りの比が挙がっているのは斜投影の行だけで，等角と 45 度
    /// 二等角は 1 つずつしか書かれていない．**名前が比を決めているので，別の比を
    /// 与えたらそれはもうその投影ではない．**
    pub fn step_is_free(self) -> bool {
        matches!(self, Self::Oblique)
    }

    pub const ALL: [Self; 3] = [Self::Iso, Self::Dimetric45, Self::Oblique];
}

/// 入力がどの面を描いた絵か．**絵からは決まらないので宣言させる** (D89)．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SourcePlane {
    /// 真上から見た絵．**2 軸とも倒れる** (設計書 6.13 の «45 度回転 → 高さ 1/2»)．
    Top,
    /// 横から見た絵．**垂直線は立ったまま** (同 «幅を縮めて垂直に歪める»)．
    Side,
}

impl SourcePlane {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Side => "side",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "top" => Some(Self::Top),
            "side" => Some(Self::Side),
            _ => None,
        }
    }

    pub const ALL: [Self; 2] = [Self::Top, Self::Side];
}

/// 歪める向き．**絵からは決まらないので宣言させる** (設計書 6.13 «逆にしない»)．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Facing {
    Right,
    Left,
}

impl Facing {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Left => "left",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "right" => Some(Self::Right),
            "left" => Some(Self::Left),
            _ => None,
        }
    }

    /// 上がりの符号．
    fn sign(self) -> f32 {
        match self {
            Self::Right => 1.0,
            Self::Left => -1.0,
        }
    }

    pub const ALL: [Self; 2] = [Self::Right, Self::Left];
}

/// 設定．
#[derive(Copy, Clone, Debug)]
pub struct ProjectOptions {
    pub projection: Projection,
    pub plane: SourcePlane,
    pub facing: Facing,
    /// 段．`None` なら [`Projection::default_step`]．
    pub step: Option<Step>,
    pub resample: ResampleOptions,
}

impl ProjectOptions {
    pub fn step(&self) -> Step {
        self.step.unwrap_or_else(|| self.projection.default_step())
    }
}

/// 結果の素性．
#[derive(Clone, Debug)]
pub struct ProjectReport {
    pub projection: &'static str,
    pub plane: &'static str,
    pub facing: &'static str,
    pub step: Step,
    /// 前向きの 2x2 行列．
    pub matrix: [f32; 4],
    /// 受ける軸が水平から倒れた角度．
    pub degrees: f32,
    /// **垂直線が立ったままか** (横から見た絵を写すときは立っているはず)．
    pub keeps_vertical: bool,
    /// **面積が変わらないか** (斜投影は純粋な歪みなので変わらないはず)．
    pub area_ratio: f32,
    pub resample: ResampleReport,
}

/// 前向きの 2x2 行列を組む．
///
/// `[a, b, c, d]` で $(x, y) \mapsto (a x + b y,\; c x + d y)$．
pub fn matrix(projection: Projection, plane: SourcePlane, facing: Facing, step: Step) -> [f32; 4] {
    let sign = facing.sign();
    let slope = step.slope();
    match (projection, plane) {
        // 斜投影は**縮まない** — 純粋な歪みなので行列式は 1 である
        (Projection::Oblique, SourcePlane::Side) => [1.0, 0.0, sign * slope, 1.0],
        (Projection::Oblique, SourcePlane::Top) => [1.0, sign / slope, 0.0, 1.0],

        // 真上から見た絵 — 45 度回して上がり / 走りだけ縦を潰す (6.13 の手順 1)
        (_, SourcePlane::Top) => {
            const C: f32 = std::f32::consts::FRAC_1_SQRT_2;
            [C, -sign * C, sign * C * slope, C * slope]
        }

        // 横から見た絵 — 幅を縮めて垂直に歪める (6.13 の手順 2)．
        // **縮める量は段から引く** — 表が 2:1 と言っているのに手順が tan 30 度を
        // 使っているのは節の中の食い違いで，採るのは表の側である
        (_, SourcePlane::Side) => {
            let hyp = (step.run as f32).hypot(step.rise as f32);
            let (cos, sin) = (step.run as f32 / hyp, step.rise as f32 / hyp);
            [cos, 0.0, sign * sin, 1.0]
        }
    }
}

/// 投影する．
pub fn project(
    canvas: &IndexedCanvas,
    palette: &Palette,
    opts: &ProjectOptions,
) -> Result<(IndexedCanvas, ProjectReport)> {
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

    let m = matrix(opts.projection, opts.plane, opts.facing, step);
    let (out, resample) = affine(canvas, palette, m, &opts.resample)?;

    // (0, 1) の行き先が (0, *) なら垂直線は立ったままである
    let keeps_vertical = m[1].abs() < 1e-4;
    let area_ratio = (m[0] * m[3] - m[1] * m[2]).abs();
    // **受ける軸は投影によって違う．**
    //
    // 斜投影を真上から見た絵に掛けるときだけ，横幅はそのままで **奥行き ($y$ 軸)
    // が倒れる** — ここで $x$ 軸を見ると «0 度» と出て，倒していないように読める
    // (D104 «測れない の理由も分ける» と同じ形で，**見る先を間違えると数え上げが
    // 嘘をつく**) ．
    let recede = if matches!(
        (opts.projection, opts.plane),
        (Projection::Oblique, SourcePlane::Top)
    ) {
        (m[1], m[3])
    } else {
        (m[0], m[2])
    };
    let degrees = if recede.0.abs() < 1e-6 {
        90.0
    } else {
        (recede.1 / recede.0).abs().atan().to_degrees()
    };

    Ok((
        out,
        ProjectReport {
            projection: opts.projection.as_str(),
            plane: opts.plane.as_str(),
            facing: opts.facing.as_str(),
            step,
            matrix: m,
            degrees,
            keeps_vertical,
            area_ratio,
            resample,
        },
    ))
}

/// **設計書 6.13 が «または» で並べた 2 手順を，そのまま行列にする．**
///
/// 測る口 (`pxsmith-calib project`) が «同じ変換なのか» を確かめるために使う．
/// **実装は使わない** — 使う行列は [`matrix`] の方である．
pub mod as_written {
    /// «45 度回転 → 高さ 1/2»．
    pub fn rotate_then_halve() -> [f32; 4] {
        const C: f32 = std::f32::consts::FRAC_1_SQRT_2;
        [C, -C, C * 0.5, C * 0.5]
    }

    /// «幅 cos 30 度 → 垂直 −30 度歪み»．
    pub fn squash_then_shear() -> [f32; 4] {
        let cos30 = (30.0f32).to_radians().cos();
        let tan30 = (30.0f32).to_radians().tan();
        [cos30, 0.0, cos30 * tan30, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;

    fn pal() -> Palette {
        Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0x1a, 0x1c, 0x2c),
            Rgba8::rgb(0xb1, 0x3e, 0x53),
        ])
        .unwrap()
    }

    fn art() -> IndexedCanvas {
        let mut c = IndexedCanvas::filled(8, 8, 0);
        c.set_transparent(Some(0));
        for y in 2..6i32 {
            for x in 2..6i32 {
                c.set(x, y, 1);
            }
        }
        c
    }

    fn opts(projection: Projection, plane: SourcePlane) -> ProjectOptions {
        ProjectOptions {
            projection,
            plane,
            facing: Facing::Right,
            step: None,
            resample: ResampleOptions::default(),
        }
    }

    /// **壊れると: 設計書の 2 手順が «同じもの» という誤解に戻る．**
    ///
    /// 6.13 は «または» で並べるが，垂直線の扱いも段も違う．
    #[test]
    fn the_two_procedures_in_the_design_are_not_the_same_transform() {
        let a = as_written::rotate_then_halve();
        let b = as_written::squash_then_shear();
        assert!(
            (a[1] - b[1]).abs() > 0.5,
            "y 軸の行き先が同じになっている: {a:?} 対 {b:?}"
        );
        // 回して潰す方は垂直線が倒れる．幅を縮めて歪める方は立ったまま
        assert!(a[1].abs() > 0.5, "手順 1 で垂直線が立っている");
        assert!(b[1].abs() < 1e-6, "手順 2 で垂直線が倒れている");
        // 受ける軸の段も違う (2:1 対 tan 30 度)
        let slope_a = a[2] / a[0];
        let slope_b = b[2] / b[0];
        assert!(
            (slope_a - 0.5).abs() < 1e-4,
            "手順 1 は 2:1 のはず: {slope_a}"
        );
        assert!(
            (slope_b - (30.0f32).to_radians().tan()).abs() < 1e-4,
            "手順 2 は tan 30 度のはず: {slope_b}"
        );
    }

    /// **壊れると: 斜投影を真上から見た絵に掛けたとき «0 度» と報告する．**
    ///
    /// 倒れているのは奥行き ($y$ 軸) なので，$x$ 軸を見ると倒していないように
    /// 読める — 見る先を間違えると数え上げが嘘をつく (D104 と同じ形)．
    #[test]
    fn an_oblique_top_view_reports_the_angle_of_the_axis_that_actually_recedes() {
        let (_, r) = project(&art(), &pal(), &opts(Projection::Oblique, SourcePlane::Top)).unwrap();
        assert!(
            (r.degrees - 26.565).abs() < 0.01,
            "奥行きの角度が {} 度と出た (x 軸を見ていないか)",
            r.degrees
        );
    }

    /// **壊れると: 等角の段が 2:1 でなくなる (26.57 度から外れる)．**
    #[test]
    fn iso_recedes_at_two_to_one() {
        assert!((Step::TWO_TO_ONE.degrees() - 26.565).abs() < 0.01);
        for plane in SourcePlane::ALL {
            let m = matrix(Projection::Iso, plane, Facing::Right, Step::TWO_TO_ONE);
            let slope = m[2] / m[0];
            assert!(
                (slope - 0.5).abs() < 1e-4,
                "{plane:?} の段が 2:1 でない: {slope}"
            );
        }
    }

    /// **壊れると: 横から見た絵を写したのに垂直線が倒れる．**
    #[test]
    fn projecting_a_side_view_keeps_vertical_lines_vertical() {
        for projection in Projection::ALL {
            let (_, r) = project(&art(), &pal(), &opts(projection, SourcePlane::Side)).unwrap();
            assert!(r.keeps_vertical, "{} で垂直線が倒れた", r.projection);
        }
    }

    /// **壊れると: 真上から見た絵を写したのに 2 軸とも倒れない．**
    #[test]
    fn projecting_a_top_view_tilts_both_axes() {
        let (_, r) = project(&art(), &pal(), &opts(Projection::Iso, SourcePlane::Top)).unwrap();
        assert!(!r.keeps_vertical, "y 軸が倒れていない");
        assert!(r.matrix[1].abs() > 0.5);
    }

    /// **壊れると: 斜投影が縮む (斜投影は純粋な歪みである)．**
    #[test]
    fn an_oblique_projection_preserves_area() {
        for plane in SourcePlane::ALL {
            let (_, r) = project(&art(), &pal(), &opts(Projection::Oblique, plane)).unwrap();
            assert!(
                (r.area_ratio - 1.0).abs() < 1e-4,
                "{plane:?} で面積が {} 倍になった",
                r.area_ratio
            );
        }
    }

    /// **壊れると: 向きが効かない (どちらを向いても同じ絵が出る)．**
    #[test]
    fn facing_flips_the_direction_of_the_shear() {
        let mut right = opts(Projection::Iso, SourcePlane::Side);
        let mut left = right;
        right.facing = Facing::Right;
        left.facing = Facing::Left;
        let (a, _) = project(&art(), &pal(), &right).unwrap();
        let (b, _) = project(&art(), &pal(), &left).unwrap();
        assert_ne!(a.pixels(), b.pixels(), "向きを変えても同じ絵が出た");
        assert_eq!(a.size(), b.size(), "向きで画布の大きさが変わるのはおかしい");
    }

    /// **壊れると: 名前が段を決めていることが崩れる．**
    #[test]
    fn only_an_oblique_projection_lets_you_choose_the_step() {
        let mut o = opts(Projection::Iso, SourcePlane::Side);
        o.step = Some(Step::ONE_TO_ONE);
        assert!(project(&art(), &pal(), &o).is_err(), "iso が段を受け入れた");

        let mut o = opts(Projection::Oblique, SourcePlane::Side);
        o.step = Some(Step::ONE_TO_ONE);
        assert!(project(&art(), &pal(), &o).is_ok(), "oblique が段を拒んだ");
    }

    /// **壊れると: 投影が色を作る (D94 の不変条件)．**
    #[test]
    fn projecting_never_creates_an_index_that_was_not_there() {
        for projection in Projection::ALL {
            for plane in SourcePlane::ALL {
                let (out, _) = project(&art(), &pal(), &opts(projection, plane)).unwrap();
                for i in out.pixels() {
                    assert!(
                        art().pixels().contains(i),
                        "{}/{plane:?}: 入力に無い添字 {i} が出た",
                        projection.as_str()
                    );
                }
            }
        }
    }

    /// **壊れると: 画布を広げず絵が切れる．**
    #[test]
    fn projecting_grows_the_canvas_so_nothing_is_clipped() {
        let mut solid = IndexedCanvas::filled(16, 16, 1);
        solid.set_transparent(Some(0));
        for projection in Projection::ALL {
            for plane in SourcePlane::ALL {
                let (_, r) = project(&solid, &pal(), &opts(projection, plane)).unwrap();
                assert_eq!(
                    r.resample.clipped,
                    0,
                    "{}/{plane:?} で {} 画素が切れた",
                    projection.as_str(),
                    r.resample.clipped
                );
            }
        }
    }

    /// **壊れると: 段の読み取りが緩くなる (0 や負の段を通す)．**
    #[test]
    fn a_step_must_be_two_positive_integers() {
        assert_eq!(Step::parse("2:1").unwrap(), Step::TWO_TO_ONE);
        assert_eq!(Step::parse(" 1 : 1 ").unwrap(), Step::ONE_TO_ONE);
        for bad in ["2", "2:0", "0:1", "a:1", "2:1:1", "-2:1", ""] {
            assert!(Step::parse(bad).is_err(), "'{bad}' を通した");
        }
    }
}
