//! 残像 (`pxsmith anim afterimage`)．
//!
//! **設計書はこの機能を «残像» の 1 語でしか決めていない** (5 章の表に基盤の
//! 欄が «—» で載っているだけである) ．したがってここは «設計書を実装した» のでは
//! なく，**他の判断から引ける形に落としたもの**である．引いた先を明記する．
//!
//! | 決めたこと | どこから引いたか |
//! | --- | --- |
//! | **ランプの宣言を要求する** | D89 ・D118 — 絵だけから «どの色がどのランプの何段目か» は決まらない |
//! | **色を作らない** | D94 — 並べ替えるだけの道具は色を作ってはいけない |
//! | **端の埋め方を選ばせる** | D95 — `wrap` は**ループのときだけ正しい**ので既定にしない |
//! | **効かなかったことを黙らない** | D77 ・D118 — «鳴らない» と «効いていない» を分ける |
//!
//! # 残像は «前のコマの絵をランプの下へ落として後ろに敷く» ものである
//!
//! $k$ コマ前の画素は `ramp.step(index, -k * step)` へ落として置く．
//! **現在の絵が不透明な画素には置かない** — 残像は後ろに出るものだからである．
//!
//! ここには 3 通りの «置けない» があり，**混ぜずに数える**．
//!
//! | 置けなかった理由 | 意味 |
//! | --- | --- |
//! | `covered` | 現在の絵に隠れた．**動きが遅いとこれが全部になる** |
//! | `not_in_ramp` | 添字が宣言したランプに無い．落とす先が決まらない |
//! | `saturated` | ランプの端に着いた．**それ以上暗い色は無い** (作らない) |
//!
//! `covered` が全部になるのは失敗ではなく，**残像が要らない場面だという結果**で
//! ある．道具は黙らずにそう言う．
//!
//! # 残像が «見える» のは動きが大きいときだけである (`pxsmith-calib afterimage`)
//!
//! 実素材 64 枚を `pxsmith shade` に描かせ (ランプが宣言されている状態を作るため) ，
//! 1 コマあたりのずらしを変えて 3 コマの列を作った．
//!
//! | 1 コマのずらし | 長さ 1 で見えた枚数 | 見えた画素 (中央) | 隠れた画素 (中央) |
//! | --- | --- | --- | --- |
//! | 1 画素 | **9 / 64** | **0** | 664 |
//! | 2 画素 | 64 / 64 | 2 | 612 |
//! | 4 画素 | 64 / 64 | 12 | 506 |
//! | 8 画素 | 64 / 64 | 58 | 304 |
//!
//! **1 画素の動きでは 64 枚中 55 枚が 1 画素も見えない．** 前のコマが現在の絵に
//! すっぽり隠れるからで，道具の誤りではない — だから [`AfterimageReport::invisible`]
//! を返して**そう言う**．
//!
//! **3 コマの列では長さ 2 と 3 が同じ結果になる** ([`TrailEdge::None`] が端で
//! 止まるため) ．長さを伸ばしても列が短ければ効かない．
//!
//! ランプの端に着いて置けなかった画素は，ずらし 8 画素 ・長さ 2 で中央 32 —
//! **段数が足りなければ «もっと暗い色» を作らずに数える**．

use crate::canvas::IndexedCanvas;
use crate::error::{CoreError, Result};
use crate::frame::{Frame, Layer, Surface};
use crate::math::{IVec2, ivec2};
use crate::palette::Ramp;

/// 列の先頭で «前のコマ» が無いときどうするか．
///
/// **既定は `None`．** `Wrap` は D95 と同じ性質の判断で，**ループするアニメの
/// ときだけ正しい** — 1 度きりの動きに巻き付けると，最初のコマに «来ていない
/// 未来» の残像が出る．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TrailEdge {
    /// 前のコマが無いぶんは残像を出さない．**何も仮定しない**．
    #[default]
    None,
    /// 列を巡回させる．**ループのときだけ正しい**．
    Wrap,
}

impl TrailEdge {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Wrap => "wrap",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "wrap" => Some(Self::Wrap),
            _ => None,
        }
    }
}

/// 残像の設定．
#[derive(Copy, Clone, Debug)]
pub struct AfterimageOptions {
    /// 何コマ前まで残すか．
    pub trail: u32,
    /// 1 コマ古くなるごとにランプを何段落とすか．
    pub step: u32,
    pub edge: TrailEdge,
}

/// 残像の既定の長さ．**測って決めた数ではなく，見せ方の選択である**．
pub const DEFAULT_TRAIL: u32 = 2;
/// 1 コマあたりに落とす段数の既定．
pub const DEFAULT_STEP: u32 = 1;

impl Default for AfterimageOptions {
    fn default() -> Self {
        Self {
            trail: DEFAULT_TRAIL,
            step: DEFAULT_STEP,
            edge: TrailEdge::None,
        }
    }
}

/// 残像を敷いた結果の素性．
#[derive(Clone, Debug, Default)]
pub struct AfterimageReport {
    /// 実際に置いた画素の数．
    pub drawn: usize,
    /// 現在の絵に隠れて置かなかった画素の数．
    pub covered: usize,
    /// 宣言したランプに無い添字だったので置けなかった画素の数．
    pub not_in_ramp: usize,
    /// ランプの端に着いて，それ以上落とせなかった画素の数．
    pub saturated: usize,
    /// フレームごとに置いた画素の数．
    pub per_frame: Vec<usize>,
}

impl AfterimageReport {
    /// **1 画素も見えなかったか．**
    ///
    /// 動きが遅いと前のコマは現在の絵にすっぽり隠れる — 失敗ではなく
    /// «残像が要らない場面だ» という結果なので，**黙らずに言う**．
    pub fn invisible(&self) -> bool {
        self.drawn == 0
    }
}

/// 残像を敷く．
///
/// 元の絵は 1 画素も書き換えない — **残像は現在の絵が透明な画素にだけ置く**．
pub fn afterimage(
    frames: &[Frame],
    ramp: &Ramp,
    opts: &AfterimageOptions,
) -> Result<(Vec<Frame>, AfterimageReport)> {
    if frames.len() < 2 {
        return Err(CoreError::AfterimageTooFewFrames {
            frames: frames.len(),
        });
    }
    if ramp.len() < 2 {
        return Err(CoreError::AnimNoRamp);
    }
    if opts.trail == 0 {
        return Err(CoreError::AfterimageNoTrail);
    }

    let n = frames.len();
    let mut report = AfterimageReport::default();
    let mut out: Vec<Frame> = Vec::with_capacity(n);

    for i in 0..n {
        let mut next = frames[i].clone();
        let occluded = opaque_mask(&frames[i]);
        let mut painted = 0usize;

        // **古い方から置く．** 新しい残像で上書きされる順にしないと，
        // 重なった画素が «より暗い方» になって順序が逆に見える
        for age in (1..=opts.trail as usize).rev() {
            let Some(src) = source_frame(frames, i, age, opts.edge) else {
                continue;
            };
            let delta = -((age as u32 * opts.step) as i32);
            for (p, index) in opaque_pixels(src) {
                if occluded.contains(&(p.x, p.y)) {
                    report.covered += 1;
                    continue;
                }
                if ramp.position_of(index).is_none() {
                    report.not_in_ramp += 1;
                    continue;
                }
                let faded = ramp.step(index, delta);
                if faded == index {
                    // 既に端にいる — **暗い色を作ってまで置かない** (D94)
                    report.saturated += 1;
                    continue;
                }
                if put(&mut next, p, faded) {
                    painted += 1;
                }
            }
        }
        report.drawn += painted;
        report.per_frame.push(painted);
        out.push(next);
    }
    Ok((out, report))
}

/// $age$ コマ前のフレーム．端は [`TrailEdge`] に従う．
fn source_frame(frames: &[Frame], at: usize, age: usize, edge: TrailEdge) -> Option<&Frame> {
    if at >= age {
        return Some(&frames[at - age]);
    }
    match edge {
        TrailEdge::None => None,
        TrailEdge::Wrap => frames.get((at + frames.len() - age % frames.len()) % frames.len()),
    }
}

/// フレームの不透明な画素の位置の集合．
fn opaque_mask(frame: &Frame) -> std::collections::BTreeSet<(i32, i32)> {
    let mut set = std::collections::BTreeSet::new();
    for layer in &frame.layers {
        let Some(c) = layer.surface.as_indexed() else {
            continue;
        };
        for y in 0..c.height() as i32 {
            for x in 0..c.width() as i32 {
                if !c.is_transparent_at(ivec2(x, y)) {
                    set.insert((x, y));
                }
            }
        }
    }
    set
}

/// フレームの不透明な画素とその添字．**上のレイヤが勝つ**．
fn opaque_pixels(frame: &Frame) -> Vec<(IVec2, u8)> {
    let mut seen: std::collections::BTreeMap<(i32, i32), u8> = Default::default();
    for layer in &frame.layers {
        let Some(c) = layer.surface.as_indexed() else {
            continue;
        };
        for y in 0..c.height() as i32 {
            for x in 0..c.width() as i32 {
                let p = ivec2(x, y);
                if !c.is_transparent_at(p) {
                    seen.insert((x, y), c.get_at(p).unwrap_or(0));
                }
            }
        }
    }
    seen.into_iter()
        .map(|((x, y), i)| (ivec2(x, y), i))
        .collect()
}

/// 一番下の指標付きレイヤへ置く (残像は後ろに出る)．
fn put(frame: &mut Frame, p: IVec2, index: u8) -> bool {
    for layer in &mut frame.layers {
        if let Surface::Indexed(c) = &mut layer.surface {
            return c.set_at(p, index);
        }
    }
    false
}

/// 残像だけを別のレイヤに分けたい場合に使う，空の «残像» レイヤ．
///
/// **既定では使わない** — L0 (`.px.toml`) は 1 ファイル 1 レイヤなので，
/// 分けると保持層で落ちる (D81 ・D119 と同じ形の穴) ．
pub fn empty_trail_layer(frame: &Frame, transparent: u8) -> Layer {
    Layer::new(
        crate::frame::LayerMeta::named("afterimage"),
        Surface::Indexed(
            IndexedCanvas::filled(frame.size.x, frame.size.y, transparent)
                .with_transparent(Some(transparent)),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;
    use crate::frame::LayerMeta;
    use crate::palette::{ChromaCurve, Palette};

    /// 5 段のランプ (添字 1 が最も暗い) を持つパレット．
    fn palette() -> Palette {
        let mut colors = vec![Rgba8::TRANSPARENT];
        for i in 0..5u8 {
            let v = 40 + i * 40;
            colors.push(Rgba8::new(v, v, v, 255));
        }
        Palette::new(colors).expect("パレット")
    }

    fn ramp() -> Ramp {
        Ramp::new(vec![1, 2, 3, 4, 5], ChromaCurve::PeakMiddle)
    }

    /// 幅 4 のベタ塗りを `x` に置いた 24x8 のフレーム．
    fn blob(x: i32, index: u8) -> Frame {
        let mut c = IndexedCanvas::filled(24, 8, 0).with_transparent(Some(0));
        for dy in 0..4i32 {
            for dx in 0..4i32 {
                c.set(x + dx, 2 + dy, index);
            }
        }
        let mut f = Frame::new(crate::math::uvec2(24, 8), palette());
        f.layers
            .push(Layer::new(LayerMeta::named("art"), Surface::Indexed(c)));
        f
    }

    /// **壊れると: 残像が現在の絵を上書きし，主体の色が変わる．**
    #[test]
    fn the_trail_never_touches_the_current_drawing() {
        let frames = vec![blob(0, 5), blob(6, 5), blob(12, 5)];
        let (out, _) = afterimage(&frames, &ramp(), &AfterimageOptions::default()).expect("残像");
        for (before, after) in frames.iter().zip(&out) {
            let (b, a) = (
                before.layers[0].surface.as_indexed().expect("指標"),
                after.layers[0].surface.as_indexed().expect("指標"),
            );
            for y in 0..8i32 {
                for x in 0..24i32 {
                    let p = ivec2(x, y);
                    if !b.is_transparent_at(p) {
                        assert_eq!(b.get_at(p), a.get_at(p), "({x},{y}) の主体が変わった");
                    }
                }
            }
        }
    }

    /// **壊れると: 残像が «無い色» を要求してパレットが増える (D94)．**
    ///
    /// ランプの端に着いたら**それ以上暗くせず数える** — 作らない．
    #[test]
    fn a_trail_that_runs_off_the_ramp_is_counted_instead_of_invented() {
        // 添字 1 はランプの最下段なので，1 段でも落とせない
        let frames = vec![blob(0, 1), blob(6, 1), blob(12, 1)];
        let (out, r) = afterimage(&frames, &ramp(), &AfterimageOptions::default()).expect("残像");
        assert!(r.saturated > 0, "端に着いたことを数えていない");
        assert_eq!(r.drawn, 0);
        assert!(r.invisible());
        for f in &out {
            let c = f.layers[0].surface.as_indexed().expect("指標");
            let used: std::collections::BTreeSet<u8> =
                c.pixels().iter().copied().filter(|&i| i != 0).collect();
            assert_eq!(used, [1u8].into_iter().collect(), "色が増えた: {used:?}");
        }
    }

    /// **壊れると: 動いていない列に «見えない残像» を置いて，効いたと報告する．**
    ///
    /// 前のコマが現在の絵にすっぽり隠れるなら残像は 1 画素も見えない．
    /// これは失敗ではなく結果なので，`invisible()` で言う．
    #[test]
    fn a_still_sequence_reports_that_nothing_is_visible() {
        let frames = vec![blob(4, 5), blob(4, 5), blob(4, 5)];
        let (_, r) = afterimage(&frames, &ramp(), &AfterimageOptions::default()).expect("残像");
        assert!(r.invisible());
        assert!(r.covered > 0, "隠れたことを数えていない");
        assert_eq!(r.not_in_ramp, 0);
    }

    /// **壊れると: ループでない列の 1 コマ目に «未来» の残像が出る (D95)．**
    #[test]
    fn the_first_frame_has_no_trail_unless_the_sequence_is_declared_a_loop() {
        let frames = vec![blob(0, 5), blob(6, 5), blob(12, 5)];
        let (_, plain) = afterimage(&frames, &ramp(), &AfterimageOptions::default()).expect("残像");
        assert_eq!(plain.per_frame[0], 0, "1 コマ目に残像が出ている");

        let (_, wrapped) = afterimage(
            &frames,
            &ramp(),
            &AfterimageOptions {
                edge: TrailEdge::Wrap,
                ..Default::default()
            },
        )
        .expect("残像");
        assert!(wrapped.per_frame[0] > 0, "巻き付けても出ていない");
    }

    /// **壊れると: 古い残像が新しい残像の上に出て，濃さの順が逆に見える．**
    #[test]
    fn the_newest_trail_wins_where_two_of_them_overlap() {
        // 2 画素ずつ動かすと，1 コマ前と 2 コマ前が重なる
        let frames = vec![blob(0, 5), blob(2, 5), blob(4, 5), blob(6, 5)];
        let (out, _) = afterimage(&frames, &ramp(), &AfterimageOptions::default()).expect("残像");
        let c = out[3].layers[0].surface.as_indexed().expect("指標");
        // x = 4 は 1 コマ前 (添字 4) と 2 コマ前 (添字 3) の両方に含まれる
        assert_eq!(c.get(4, 3), Some(4), "1 コマ前が勝っていない");
        assert_eq!(c.get(2, 3), Some(3), "2 コマ前が出ていない");
    }

    /// **壊れると: 1 枚だけ ・残像 0 コマを黙って受け取る．**
    #[test]
    fn a_single_frame_or_a_zero_trail_is_an_error() {
        assert!(matches!(
            afterimage(&[blob(0, 5)], &ramp(), &AfterimageOptions::default()),
            Err(CoreError::AfterimageTooFewFrames { .. })
        ));
        assert!(matches!(
            afterimage(
                &[blob(0, 5), blob(4, 5)],
                &ramp(),
                &AfterimageOptions {
                    trail: 0,
                    ..Default::default()
                }
            ),
            Err(CoreError::AfterimageNoTrail)
        ));
    }
}
