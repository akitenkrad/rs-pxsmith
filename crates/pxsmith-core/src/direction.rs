//! 方向展開 (`pxsmith direction`．設計書 4.3 ・L2 «8 方向»)．
//!
//! 実装計画書は «反転 + 陰影再導出で 8 方向生成» と言い，設計書 4.3 は «自動ミラーは
//! 陰影を持つ素材では光源方向を反転させる．**自動ミラーで生成したタイルには lint
//! ルール 7 を blocking で適用する**» と言う．**測ってから 3 つ決めた**
//! (`pxsmith-calib direction`) ．
//!
//! # 反転で作れるのは 3 方向だけである
//!
//! 左右反転で移り合うのは `NE`↔`NW` ・`E`↔`W` ・`SE`↔`SW` の 3 組だけで，
//! `N` と `S` は軸の上にあるので相手がいない．**回転では作れない** (向きが変われば
//! 見えている面が変わる) ので，**8 方向は «描いた 5 枚 + 反転した 3 枚» で埋まる**．
//! 描いていない方向は埋めずに «描いていない» と報告する．
//!
//! # 反転が «矛盾» になるかは光源の横成分だけで決まる
//!
//! 反転で裏返るのは明度勾配の $x$ 成分だけなので，宣言した光源 $\ell$ に対する
//! 一致度は反転後 $1 - 2\ell_x^2$ になる (実素材 60 枚で誤差 $2 \times 10^{-4}$) ．
//! ルール 7 の閾値 $0.55$ と突き合わせると，**鳴るのは
//! $\lvert \ell_x \rvert > \sqrt{(1 - 0.55)/2} = 0.474$ のときに限る**．
//!
//! | | 値 |
//! | --- | --- |
//! | 実測で鳴った 28 枚の $\lvert \ell_x \rvert$ の最小 | 0.509 |
//! | 鳴らなかった 32 枚の $\lvert \ell_x \rvert$ の最大 | 0.471 |
//!
//! **これは校正の対象ではない — 閾値から代数的に決まる** (D92 と同じ性質) ．
//! そして $\ell_x = 0$ (真上からの光) で鳴らないのは**見逃しではなく正しい** —
//! 真上から照らされた絵は，左右反転しても真上から照らされたままである．
//!
//! # 再導出は既定にしない
//!
//! | 群 | 反転しただけ (鳴る) | 反転 + 再導出 (鳴る) | 再導出が書き換えた画素 |
//! | --- | --- | --- | --- |
//! | `pxsmith shade` の出力 | **244 / 244** | **0 / 244** | 中央 62.5% |
//! | 実素材 (手描き) | 28 / 60 | **0 / 60** | **中央 100%** |
//!
//! 再導出は必ず直すが，**手描きの絵は 1 画素残らず書き換わる** — シルエットだけ
//! 使って塗り直すのだから当然である．`pxsmith shade` で作った素材には正しい操作だが，
//! 手で描いた絵に既定で掛けてよいものではない．**既定は [`ExpandMode::Mirror`] とし，
//! 再導出は明示して選ぶ．**

use std::collections::BTreeMap;

use crate::canvas::IndexedCanvas;
use crate::color::Rgba8;
use crate::error::{CoreError, Result};
use crate::frame::{Frame, Surface};
use crate::geom::Mask;
use crate::palette::{ChromaCurve, Palette};
use crate::ramp::{LightPreset, LightSource, build_lighting};
use crate::shade::{ShadeOptions, shade_to_canvas};

/// 8 方向．画面の上を `N` とする．
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Direction {
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
    NW,
}

pub const ALL: [Direction; 8] = [
    Direction::N,
    Direction::NE,
    Direction::E,
    Direction::SE,
    Direction::S,
    Direction::SW,
    Direction::W,
    Direction::NW,
];

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::N => "n",
            Self::NE => "ne",
            Self::E => "e",
            Self::SE => "se",
            Self::S => "s",
            Self::SW => "sw",
            Self::W => "w",
            Self::NW => "nw",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        ALL.into_iter()
            .find(|d| d.as_str() == s.to_ascii_lowercase())
    }

    /// 左右反転で移る先．**`N` と `S` は自分自身** (軸の上なので相手がいない)．
    pub fn mirrored(self) -> Self {
        match self {
            Self::N => Self::N,
            Self::NE => Self::NW,
            Self::E => Self::W,
            Self::SE => Self::SW,
            Self::S => Self::S,
            Self::SW => Self::SE,
            Self::W => Self::E,
            Self::NW => Self::NE,
        }
    }

    /// 反転で作れる方向か (自分自身に移るものは作れない)．
    pub fn is_mirror_derivable(self) -> bool {
        self.mirrored() != self
    }
}

/// 陰影を導出し直すときの指定．
#[derive(Clone, Debug)]
pub struct ReshadeSpec {
    /// 固有色．**シルエットだけ使って塗り直すので，元の絵の色は残らない**．
    pub base: Rgba8,
    pub preset: LightPreset,
    pub steps: u8,
    pub curve: ChromaCurve,
    pub shade: ShadeOptions,
}

impl ReshadeSpec {
    pub fn new(base: Rgba8, preset: LightPreset) -> Self {
        Self {
            base,
            preset,
            steps: 5,
            curve: ChromaCurve::PeakMiddle,
            shade: ShadeOptions::default(),
        }
    }
}

/// 反転したものをどう仕上げるか．
#[derive(Clone, Debug, Default)]
pub enum ExpandMode {
    /// 反転だけ．**既定．** 陰影を持たない素材はこれで足りる (設計書 4.3) ．
    ///
    /// 陰影を持つ素材では光源と矛盾しうるので，**呼ぶ側がルール 7 を blocking で
    /// 掛けること**．矛盾するのは $\lvert \ell_x \rvert > 0.474$ のときに限る．
    #[default]
    Mirror,
    /// 反転したシルエットへ陰影を導出し直す．
    ///
    /// **元の絵の画素は残らない** (実素材で中央 100% が書き換わる) ．
    /// `pxsmith shade` で作った素材のための道具である．
    Reshade(Box<ReshadeSpec>),
}

#[derive(Clone, Debug, Default)]
pub struct ExpandOptions {
    pub mode: ExpandMode,
}

/// 作った方向 1 つぶんの報告．
#[derive(Clone, Debug)]
pub struct Generated {
    pub direction: Direction,
    /// どの方向から作ったか．
    pub from: Direction,
    /// 陰影を導出し直したか．
    pub reshaded: bool,
    /// 再導出が «反転しただけの絵» から書き換えた不透明画素の割合 (0.0 〜 1.0)．
    pub rewritten: f32,
}

#[derive(Clone, Debug)]
pub struct ExpandReport {
    /// 元から描いてあった方向．
    pub drawn: Vec<Direction>,
    /// 反転で作った方向．**ここにルール 7 を掛ける** (設計書 4.3)．
    pub generated: Vec<Generated>,
    /// 埋まらなかった方向．**反転では作れないので «描いていない» と言う**．
    pub missing: Vec<Direction>,
}

impl ExpandReport {
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }
}

/// 左右反転．**補間は挟まない** — 添字をそのまま写すので色もシルエットも変わらない．
pub fn mirror_canvas(canvas: &IndexedCanvas) -> IndexedCanvas {
    let (w, h) = (canvas.width(), canvas.height());
    let mut out = canvas.clone();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let v = canvas.get(w as i32 - 1 - x, y).expect("範囲内");
            out.set(x, y, v);
        }
    }
    out
}

/// フレームを左右反転する．
pub fn mirror_frame(frame: &Frame) -> Result<Frame> {
    let mut out = frame.clone();
    for layer in &mut out.layers {
        let Some(canvas) = layer.surface.as_indexed() else {
            return Err(CoreError::NotIndexed {
                name: layer.meta.name.clone(),
            });
        };
        layer.surface = Surface::Indexed(mirror_canvas(canvas));
    }
    Ok(out)
}

fn mask_of(canvas: &IndexedCanvas) -> Mask {
    let mut m = Mask::new(canvas.width(), canvas.height());
    for p in canvas.bounds().iter() {
        if !canvas.is_transparent_at(p) {
            m.set(p, true);
        }
    }
    m
}

/// 反転したフレームのシルエットへ陰影を導出し直す．
///
/// 返すのは (フレーム，書き換えた不透明画素の割合)．
fn reshade_frame(frame: &Frame, spec: &ReshadeSpec, light: LightSource) -> Result<(Frame, f32)> {
    let (ramp_palette, model) = build_lighting(spec.base, spec.preset, spec.steps, spec.curve)?;

    let mut out = frame.clone();
    let (mut opaque, mut changed) = (0usize, 0usize);
    let mut palette: Option<Palette> = None;

    for layer in &mut out.layers {
        let Some(canvas) = layer.surface.as_indexed() else {
            return Err(CoreError::NotIndexed {
                name: layer.meta.name.clone(),
            });
        };
        let before = canvas.clone();
        let (shaded, shaded_palette) =
            shade_to_canvas(&mask_of(&before), light, &model, &ramp_palette, spec.shade)?;
        for p in before.bounds().iter() {
            if before.is_transparent_at(p) {
                continue;
            }
            opaque += 1;
            let was = before.get_at(p).and_then(|i| frame.palette.get(i));
            let now = shaded.get_at(p).and_then(|i| shaded_palette.get(i));
            if was != now {
                changed += 1;
            }
        }
        layer.surface = Surface::Indexed(shaded);
        palette = Some(shaded_palette);
    }

    if let Some(p) = palette {
        out.palette = p;
    }
    Ok((
        out,
        if opaque == 0 {
            0.0
        } else {
            changed as f32 / opaque as f32
        },
    ))
}

/// 描いた方向から 8 方向を埋める．
///
/// **反転で作れるのは `NE` ・`E` ・`SE` とその鏡像の 3 組だけである．**
/// `N` ・`S` は軸の上にあるので相手がいない — 描いてなければ埋まらない．
///
/// 既に描いてある方向は**上書きしない** (手で描いたものが正である) ．
pub fn expand(
    drawn: &BTreeMap<Direction, Vec<Frame>>,
    opts: &ExpandOptions,
) -> Result<(BTreeMap<Direction, Vec<Frame>>, ExpandReport)> {
    if drawn.is_empty() {
        return Err(CoreError::DirectionNothingDrawn);
    }

    let mut out: BTreeMap<Direction, Vec<Frame>> = drawn.clone();
    let mut generated: Vec<Generated> = Vec::new();

    // **決定論的な順に回す** (設計書 6.15 規則 1)．`ALL` は宣言順で固定してある
    for target in ALL {
        if out.contains_key(&target) {
            continue;
        }
        let source = target.mirrored();
        if source == target {
            continue; // 軸の上 — 反転では作れない
        }
        let Some(frames) = drawn.get(&source) else {
            continue;
        };

        let mut made: Vec<Frame> = Vec::with_capacity(frames.len());
        let mut rewritten = 0.0f32;
        for frame in frames {
            let flipped = mirror_frame(frame)?;
            match &opts.mode {
                ExpandMode::Mirror => made.push(flipped),
                ExpandMode::Reshade(spec) => {
                    let light = spec.preset.default_source();
                    let (f, r) = reshade_frame(&flipped, spec, light)?;
                    rewritten = rewritten.max(r);
                    made.push(f);
                }
            }
        }

        generated.push(Generated {
            direction: target,
            from: source,
            reshaded: matches!(opts.mode, ExpandMode::Reshade(_)),
            rewritten,
        });
        out.insert(target, made);
    }

    let missing: Vec<Direction> = ALL.into_iter().filter(|d| !out.contains_key(d)).collect();
    Ok((
        out,
        ExpandReport {
            drawn: drawn.keys().copied().collect(),
            generated,
            missing,
        },
    ))
}

/// 反転しても光源と矛盾しない光源かどうか — $\lvert \ell_x \rvert \le$ 境目．
///
/// **これは校正の対象ではない．** ルール 7 の閾値 $\theta$ から
/// $\lvert \ell_x \rvert = \sqrt{(1 - \theta)/2}$ と代数的に決まる (D96) ．
/// 反転で裏返るのは明度勾配の $x$ 成分だけなので，一致度は $1 - 2\ell_x^2$ になる．
pub fn mirror_is_checkable(light: LightSource, agreement_threshold: f32) -> bool {
    let LightSource::Directional { .. } = light else {
        // 点 ・線 ・面の光源ではルール 7 が掛からない (D89)
        return false;
    };
    let lx = crate::outline::light_direction(light).x;
    lx * lx > (1.0 - agreement_threshold) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Layer, LayerMeta};
    use crate::math::{UVec2, Vec2};

    fn frame_of(w: u32, h: u32, data: &[u8], colors: &[Rgba8]) -> Frame {
        let mut entries = vec![Rgba8::TRANSPARENT];
        entries.extend_from_slice(colors);
        let palette = Palette::new(entries).expect("256 色以内");
        let mut f = Frame::new(UVec2 { x: w, y: h }, palette);
        let canvas = IndexedCanvas::from_pixels(w, h, data.to_vec())
            .expect("画素数が合う")
            .with_transparent(Some(0));
        f.layers.push(Layer::new(
            LayerMeta::named("art"),
            Surface::Indexed(canvas),
        ));
        f
    }

    /// 壊れると: 反転で作れない方向を «作った» と言い張る．
    /// `N` と `S` は軸の上なので鏡像が自分自身である．
    #[test]
    fn north_and_south_have_no_mirror_partner() {
        assert_eq!(Direction::N.mirrored(), Direction::N);
        assert_eq!(Direction::S.mirrored(), Direction::S);
        assert!(!Direction::N.is_mirror_derivable());
        assert!(!Direction::S.is_mirror_derivable());
        for d in [Direction::NE, Direction::E, Direction::SE] {
            assert!(d.is_mirror_derivable());
            assert_eq!(d.mirrored().mirrored(), d);
        }
    }

    /// 壊れると: 描いていない方向が黙って欠けたまま «8 方向» を名乗る．
    #[test]
    fn five_drawn_directions_fill_eight_and_three_drawn_do_not() {
        let art = frame_of(2, 1, &[1, 0], &[Rgba8::rgb(9, 9, 9)]);
        let mut drawn = BTreeMap::new();
        for d in [
            Direction::N,
            Direction::NE,
            Direction::E,
            Direction::SE,
            Direction::S,
        ] {
            drawn.insert(d, vec![art.clone()]);
        }
        let (out, report) = expand(&drawn, &ExpandOptions::default()).expect("展開できる");
        assert_eq!(out.len(), 8);
        assert!(report.is_complete());
        assert_eq!(report.generated.len(), 3);

        // N と S を描かなければ 8 方向にはならない — 反転では作れない
        let mut partial = BTreeMap::new();
        for d in [Direction::NE, Direction::E, Direction::SE] {
            partial.insert(d, vec![art.clone()]);
        }
        let (out, report) = expand(&partial, &ExpandOptions::default()).expect("展開できる");
        assert_eq!(out.len(), 6);
        assert_eq!(report.missing, vec![Direction::N, Direction::S]);
    }

    /// 壊れると: 反転が色やシルエットを変える．
    /// 反転は添字を写すだけなので，1 画素も色が変わってはいけない．
    #[test]
    fn mirroring_moves_pixels_without_touching_colours() {
        let art = frame_of(
            3,
            1,
            &[1, 2, 0],
            &[Rgba8::rgb(1, 1, 1), Rgba8::rgb(2, 2, 2)],
        );
        let flipped = mirror_frame(&art).expect("反転できる");
        let a = art.layers[0].surface.as_indexed().expect("添字");
        let b = flipped.layers[0].surface.as_indexed().expect("添字");
        assert_eq!(b.pixels(), &[0, 2, 1]);
        assert_eq!(a.pixels().len(), b.pixels().len());
        assert_eq!(flipped.palette.entries(), art.palette.entries());
        // 2 度反転すれば元に戻る
        let back = mirror_frame(&flipped).expect("反転できる");
        assert_eq!(
            back.layers[0].surface.as_indexed().expect("添字").pixels(),
            a.pixels()
        );
    }

    /// 壊れると: 手で描いた方向が反転で上書きされる．
    #[test]
    fn a_drawn_direction_is_never_overwritten_by_a_mirror() {
        let left = frame_of(2, 1, &[1, 0], &[Rgba8::rgb(1, 1, 1)]);
        let right = frame_of(2, 1, &[0, 2], &[Rgba8::rgb(1, 1, 1), Rgba8::rgb(2, 2, 2)]);
        let mut drawn = BTreeMap::new();
        drawn.insert(Direction::E, vec![left]);
        drawn.insert(Direction::W, vec![right.clone()]);
        let (out, report) = expand(&drawn, &ExpandOptions::default()).expect("展開できる");
        assert!(report.generated.is_empty(), "両方描いてあるので作らない");
        assert_eq!(
            out[&Direction::W][0].layers[0]
                .surface
                .as_indexed()
                .expect("添字")
                .pixels(),
            right.layers[0].surface.as_indexed().expect("添字").pixels()
        );
    }

    /// 壊れると: «反転しても矛盾しない光源» まで検査対象として数え，
    /// 見逃していないものを «見逃し» と報告する．
    ///
    /// 反転で裏返るのは $x$ 成分だけなので，真上からの光では矛盾が起きない．
    #[test]
    fn a_light_from_straight_above_is_not_checkable_by_mirroring() {
        let straight = LightSource::Directional {
            dir: Vec2 { x: 0.0, y: 1.0 },
        };
        assert!(!mirror_is_checkable(straight, 0.55));

        let diagonal = LightSource::Directional {
            dir: Vec2 { x: -0.6, y: 0.8 },
        };
        assert!(mirror_is_checkable(diagonal, 0.55));

        // 境目は閾値から代数的に決まる — 0.474 のすぐ両側で切り替わる
        let just_under = LightSource::Directional {
            dir: Vec2 {
                x: -0.47,
                y: 0.8827,
            },
        };
        let just_over = LightSource::Directional {
            dir: Vec2 {
                x: -0.48,
                y: 0.8773,
            },
        };
        assert!(!mirror_is_checkable(just_under, 0.55));
        assert!(mirror_is_checkable(just_over, 0.55));

        // 点光源には掛からない (D89)
        assert!(!mirror_is_checkable(
            LightSource::Point {
                pos: Vec2 { x: -4.0, y: -4.0 },
                intensity: 1.0,
            },
            0.55
        ));
    }

    /// 壊れると: 何も描いていない入力が «展開できた» ことになる．
    #[test]
    fn expanding_nothing_is_an_error() {
        let empty: BTreeMap<Direction, Vec<Frame>> = BTreeMap::new();
        assert!(expand(&empty, &ExpandOptions::default()).is_err());
    }
}
