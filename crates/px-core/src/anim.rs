//! タイミングと周期アニメーション (設計書 6.11 ・6.12)．
//!
//! # 表示時間はコマ打ちと FPS から出す (D40)
//!
//! 設計書 6.11 の表は $\mathrm{round}(\text{コマ打ち} \times 1000 / \mathrm{FPS})$
//! そのものである — 24 ・30 ・60 FPS の 12 マスすべてが一致することを試験で確かめた．
//! **表を書き写さず式から引く** (D111 と同じ作法) ．
//!
//! ## 未決事項 #5 (60 FPS の 1 コマを 17 ms とするか 16 ms とするか) を測って閉じた
//!
//! 設計書 付録 B は «表記ではなく仕様の選択» として M4 送りにしていた．
//! **`px validate` の «表示時間は表示周期の倍数か» (D40) に掛けて決めた．**
//!
//! | | gb (59.73 Hz) | nes ・snes (60.0988 Hz) | gba (59.7275 Hz) |
//! | --- | --- | --- | --- |
//! | **四捨五入 (17 ・33 ・50 ・67)** | **違反 0** | **違反 0** | **違反 0** |
//! | 16 ms 固定 (16 ・32 ・48 ・64) | 違反 3 | 違反 3 | 違反 3 |
//!
//! **1 コマだけ見ると 16 ms でも通る** (ずれ 0.64 〜 0.74 ms) ．落ちるのは 2 コマ
//! 以降で，16 ms の誤差 0.67 ms がコマ打ちのぶんだけ積もる (2 コマで 1.3 ms ・
//! 4 コマで 2.6 〜 3.0 ms) ．**1 コマだけ測っていたら逆の答えを選んでいた．**
//!
//! 採るのは**四捨五入 (60 FPS の 1 コマ = 17 ms)** である．
//!
//! > [!note] pico8 (30 Hz) では 17 ms と 50 ms が違反になる．
//! > これは表の誤りではなく «60 FPS で作った絵を 30 Hz の機械へ出している»
//! > という事実であり，`px validate` はそれを言うべきである．
//!
//! # 周期アニメは «何を変調するか x どう変調するか» (6.12 ・D24)
//!
//! $4 \times 4 = 16$ 通り．**うち 12 通りを書き，4 通り ([`ModTarget::Rotate`])
//! は書いていない** — 回転は `px rotate` (設計書 6.13) の仕事で，まだ無い．
//! ここで別の回転を書くと**回転の実装が 2 つになる** (D110 が «正規出力が 2 つ
//! あるのは正規出力が無いのと同じ» と言ったのと同じ形の誤り) ．D92 の作法どおり
//! **書いた分だけ書き，残りは «書いていない» と報告する**．
//!
//! ## フレームは 3 枚から (D44)
//!
//! 設計書は «2 枚では軌跡が表現できず切り替わるだけになる» を理由に挙げる．
//! **正弦波では，それより強いことが代数から言える** — $n = 2$ で
//! $\sin(2\pi k/2)$ を採ると $k = 0, 1$ のどちらも $0$ になり，
//! **振幅がいくつでも 1 画素も動かない**．試験で縛ってある．

use crate::canvas::IndexedCanvas;
use crate::error::{CoreError, Result};
use crate::frame::{Frame, Layer, Surface};
use crate::math::{IVec2, ivec2};
use crate::palette::Ramp;

// ---------------------------------------------------------------- タイミング

/// コマ打ちと FPS から表示時間を出す (設計書 6.11 の表)．
///
/// **丸めは逆数の四捨五入で統一する．** 表の 12 マスはこの式から出る．
pub fn duration_ms(fps: f32, hold: u32) -> Result<u32> {
    if !(fps.is_finite() && fps > 0.0) {
        return Err(CoreError::AnimBadFps { fps });
    }
    if hold == 0 {
        return Err(CoreError::AnimBadHold);
    }
    Ok((hold as f32 * 1000.0 / fps).round().max(1.0) as u32)
}

/// イージングの結果．
#[derive(Clone, Debug)]
pub struct EaseReport {
    pub fps: f32,
    /// フレームごとの (コマ打ち，表示時間)．
    pub holds: Vec<(u32, u32)>,
    /// 合計の表示時間 (ミリ秒)．
    pub total_ms: u32,
}

/// フレーム列に表示時間を付ける (設計書 6.11 «イージング = `duration_ms` 配列») ．
///
/// `holds` が 1 つならすべてのフレームに同じコマ打ちを掛ける．複数なら
/// **フレーム数とちょうど一致していなければならない** — 足りないぶんを黙って
/// 埋めると «指定したつもりの無いコマ打ち» が入る．
pub fn ease(frames: &mut [Frame], fps: f32, holds: &[u32]) -> Result<EaseReport> {
    if frames.is_empty() {
        return Err(CoreError::AnimNoFrames);
    }
    if holds.is_empty() {
        return Err(CoreError::AnimBadHold);
    }
    if holds.len() != 1 && holds.len() != frames.len() {
        return Err(CoreError::AnimHoldCountMismatch {
            holds: holds.len(),
            frames: frames.len(),
        });
    }

    let mut out = Vec::with_capacity(frames.len());
    for (i, frame) in frames.iter_mut().enumerate() {
        let hold = if holds.len() == 1 { holds[0] } else { holds[i] };
        let ms = duration_ms(fps, hold)?;
        frame.duration_ms = ms;
        out.push((hold, ms));
    }
    let total = out.iter().map(|(_, ms)| ms).sum();
    Ok(EaseReport {
        fps,
        holds: out,
        total_ms: total,
    })
}

// ------------------------------------------------------------ 周期アニメーション

/// 何を変調するか (設計書 3.6 の `ModTarget`)．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ModTarget {
    /// 宣言されたランプの上を上下する (蛍光灯のちらつきなど)．
    Ramp,
    /// 画布ごと平行移動する (揺れ)．
    Offset,
    /// シルエットを膨らませたり縮めたりする (波紋)．
    Mask,
    /// 回転．**書いていない** (モジュールの説明)．
    Rotate,
}

impl ModTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ramp => "ramp",
            Self::Offset => "offset",
            Self::Mask => "mask",
            Self::Rotate => "rotate",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ramp" => Some(Self::Ramp),
            "offset" => Some(Self::Offset),
            "mask" => Some(Self::Mask),
            "rotate" => Some(Self::Rotate),
            _ => None,
        }
    }

    pub const ALL: &'static [Self] = &[Self::Ramp, Self::Offset, Self::Mask, Self::Rotate];
}

/// どう変調するか (設計書 3.6 の `Wave`)．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    /// 半周ごとに $\pm 1$．**0 を取らない**ので $n = 2$ でも動く．
    Square,
    /// フレームごとの一様乱数 ($[-1, 1]$)．
    Noise,
    /// フレームごとに $\pm 1$．蛍光灯のちらつき．
    RandomBlink,
}

impl Wave {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Square => "square",
            Self::Noise => "noise",
            Self::RandomBlink => "random-blink",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "sine" => Some(Self::Sine),
            "square" => Some(Self::Square),
            "noise" => Some(Self::Noise),
            "random-blink" | "blink" => Some(Self::RandomBlink),
            _ => None,
        }
    }

    pub const ALL: &'static [Self] = &[Self::Sine, Self::Square, Self::Noise, Self::RandomBlink];

    /// 第 `k` 番目のフレームでの値 ($[-1, 1]$)．
    ///
    /// **周期の中で閉じている** — `k` は `0..n` の範囲でしか引かないので，
    /// 乱数系も含めて «最後の次が最初» になる．
    pub fn at(self, k: u32, n: u32, phase: f32, seed: u64) -> f32 {
        let u = k as f32 / n.max(1) as f32 + phase;
        match self {
            Self::Sine => (std::f32::consts::TAU * u).sin(),
            Self::Square => {
                if u.rem_euclid(1.0) < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Noise => {
                let shift = (phase * n as f32).round() as i64;
                let at = (k as i64 + shift).rem_euclid(n.max(1) as i64) as u64;
                unit(hash(seed, at)) * 2.0 - 1.0
            }
            Self::RandomBlink => {
                let shift = (phase * n as f32).round() as i64;
                let at = (k as i64 + shift).rem_euclid(n.max(1) as i64) as u64;
                if hash(seed, at) & 1 == 0 { -1.0 } else { 1.0 }
            }
        }
    }
}

/// splitmix64 を状態なしの混ぜ合わせとして使う (`px-calib` の `rng` と同じ構成) ．
fn hash(seed: u64, k: u64) -> u64 {
    let mut z = seed
        .wrapping_add(k.wrapping_mul(0x9e37_79b9_7f4a_7c15))
        .wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn unit(v: u64) -> f32 {
    (v >> 40) as f32 / (1u64 << 24) as f32
}

/// 周期アニメの仕様 (設計書 3.6 の `CycleSpec`)．
#[derive(Clone, Debug)]
pub struct CycleSpec {
    pub target: ModTarget,
    pub wave: Wave,
    /// フレーム数．**既定 3** (D44)．
    pub frames: u32,
    pub amplitude: f32,
    pub phase: f32,
    /// **必須．** ノイズ系を含むので，無いと決定論性が崩れる (設計書 6.12) ．
    pub seed: u64,
    /// 平行移動の向き．**設計書は決めていないので既定を横にした** (SWAY = 揺れ) ．
    pub direction: IVec2,
}

impl Default for CycleSpec {
    fn default() -> Self {
        Self {
            target: ModTarget::Offset,
            wave: Wave::Sine,
            frames: DEFAULT_FRAMES,
            amplitude: 1.0,
            phase: 0.0,
            seed: 0,
            direction: ivec2(1, 0),
        }
    }
}

/// 周期アニメの最小フレーム数 (D44)．
pub const DEFAULT_FRAMES: u32 = 3;
pub const MIN_FRAMES: u32 = 3;

impl CycleSpec {
    /// 蛍光灯 (設計書 6.12)．
    pub fn flicker(seed: u64) -> Self {
        Self {
            target: ModTarget::Ramp,
            wave: Wave::RandomBlink,
            seed,
            ..Self::default()
        }
    }

    /// 揺れ．
    pub fn sway(seed: u64) -> Self {
        Self {
            target: ModTarget::Offset,
            wave: Wave::Sine,
            seed,
            ..Self::default()
        }
    }

    /// 換気扇．**書いていない** — 回転は `px rotate` の仕事である．
    pub fn rotate(seed: u64) -> Self {
        Self {
            target: ModTarget::Rotate,
            wave: Wave::Sine,
            seed,
            ..Self::default()
        }
    }

    /// 波紋．
    pub fn ripple(seed: u64) -> Self {
        Self {
            target: ModTarget::Mask,
            wave: Wave::Sine,
            seed,
            ..Self::default()
        }
    }

    pub fn preset(name: &str, seed: u64) -> Option<Self> {
        match name {
            "flicker" => Some(Self::flicker(seed)),
            "sway" => Some(Self::sway(seed)),
            "rotate" => Some(Self::rotate(seed)),
            "ripple" => Some(Self::ripple(seed)),
            _ => None,
        }
    }

    pub const PRESETS: &'static [&'static str] = &["flicker", "sway", "rotate", "ripple"];
}

/// 周期アニメの結果．
#[derive(Clone, Debug)]
pub struct CycleReport {
    /// フレームごとの (波の値，実際に適用した段数 / 画素数)．
    pub steps: Vec<(f32, i32)>,
    /// **1 枚も動かなかったか．** 振幅が小さすぎる ・波が 0 に潰れている，など．
    pub all_still: bool,
    /// 逆再生を足したか (D44 の `--reverse-derive`)．
    pub reversed: usize,
}

/// 1 枚の絵から周期アニメを作る (設計書 6.12)．
///
/// `ramp` は [`ModTarget::Ramp`] のときだけ使う．**宣言が要る** — 絵だけから
/// «どの色がどのランプの何段目か» は決まらないので，推定して当てるのは同語反復に
/// なる (ルール 7 が光源の宣言を要求するのと同じ理由，D89) ．
pub fn cycle(
    frame: &Frame,
    spec: &CycleSpec,
    ramp: Option<&Ramp>,
) -> Result<(Vec<Frame>, CycleReport)> {
    if spec.frames < MIN_FRAMES {
        return Err(CoreError::AnimTooFewFrames {
            frames: spec.frames,
            min: MIN_FRAMES,
        });
    }
    if spec.target == ModTarget::Rotate {
        return Err(CoreError::AnimRotateNotWritten);
    }
    if spec.target == ModTarget::Ramp && ramp.is_none_or(|r| r.len() < 2) {
        return Err(CoreError::AnimNoRamp);
    }

    let mut out = Vec::with_capacity(spec.frames as usize);
    let mut steps = Vec::with_capacity(spec.frames as usize);
    for k in 0..spec.frames {
        let value = spec.wave.at(k, spec.frames, spec.phase, spec.seed);
        let step = (value * spec.amplitude).round() as i32;
        steps.push((value, step));

        let mut next = frame.clone();
        for layer in &mut next.layers {
            let Some(canvas) = layer.surface.as_indexed() else {
                return Err(CoreError::NotIndexed {
                    name: layer.meta.name.clone(),
                });
            };
            let moved = match spec.target {
                ModTarget::Ramp => shift_ramp(canvas, ramp.expect("上で確かめた"), step),
                ModTarget::Offset => offset(canvas, spec.direction, step),
                ModTarget::Mask => grow(canvas, step),
                ModTarget::Rotate => unreachable!("上で落としている"),
            };
            *layer = Layer::new(layer.meta.clone(), Surface::Indexed(moved));
        }
        out.push(next);
    }

    let all_still = steps.iter().all(|(_, s)| *s == 0);
    Ok((
        out,
        CycleReport {
            steps,
            all_still,
            reversed: 0,
        },
    ))
}

/// 逆再生を後ろへ足す (D44 の `--reverse-derive`)．
///
/// **両端は重ねない** — 3 枚 `[a, b, c]` から `[a, b, c, b]` を作る．`c` と `a` を
/// 2 度出すと，そこだけ 2 倍の時間だけ止まって見える．
pub fn reverse_derive(frames: &[Frame]) -> Vec<Frame> {
    let mut out = frames.to_vec();
    if frames.len() < 3 {
        return out;
    }
    for f in frames.iter().skip(1).rev().skip(1) {
        out.push(f.clone());
    }
    out
}

/// ランプの上を `step` 段だけ動かす．**ランプに無い色は動かさない**．
fn shift_ramp(canvas: &IndexedCanvas, ramp: &Ramp, step: i32) -> IndexedCanvas {
    let mut out = canvas.clone();
    if step == 0 {
        return out;
    }
    let mut map: Vec<u8> = (0..=255u8).collect();
    for e in ramp.entries() {
        map[*e as usize] = ramp.step(*e, step);
    }
    for v in out.pixels_mut() {
        *v = map[*v as usize];
    }
    out
}

/// 画布ごと平行移動する．外へ出たぶんは切れる (画布は動かさない)．
fn offset(canvas: &IndexedCanvas, direction: IVec2, step: i32) -> IndexedCanvas {
    let transparent = canvas.transparent().unwrap_or(0);
    let mut out = IndexedCanvas::filled(canvas.width(), canvas.height(), transparent)
        .with_transparent(Some(transparent));
    if step == 0 {
        return canvas.clone();
    }
    out.blit(canvas, ivec2(direction.x * step, direction.y * step), false);
    out
}

/// シルエットを `step` 画素だけ膨らませる (負なら縮める)．
///
/// **色は作らない** — 膨らませた画素は隣の不透明な画素の添字をそのまま写す．
/// 走査の向きを固定してあるので決定論的である．
fn grow(canvas: &IndexedCanvas, step: i32) -> IndexedCanvas {
    let mut out = canvas.clone();
    let transparent = canvas.transparent().unwrap_or(0);
    let dirs = [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)];
    for _ in 0..step.abs() {
        let src = out.clone();
        for y in 0..src.height() as i32 {
            for x in 0..src.width() as i32 {
                let p = ivec2(x, y);
                let here = src.get_at(p).unwrap_or(transparent);
                if step > 0 {
                    if here != transparent {
                        continue;
                    }
                    // 走査の向きを固定して «最初に見つけた不透明な隣» を写す
                    if let Some(v) = dirs
                        .iter()
                        .filter_map(|d| src.get_at(p + *d))
                        .find(|v| *v != transparent)
                    {
                        out.set_at(p, v);
                    }
                } else {
                    if here == transparent {
                        continue;
                    }
                    // 画布の外は «透明» として扱う (縁に接する形も縮む)
                    let exposed = dirs
                        .iter()
                        .any(|d| src.get_at(p + *d).unwrap_or(transparent) == transparent);
                    if exposed {
                        out.set_at(p, transparent);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;
    use crate::math::uvec2;
    use crate::palette::{ChromaCurve, Palette};

    /// **壊れると: 設計書 6.11 の表と実際の表示時間がずれる．**
    ///
    /// 表を書き写さず式から引く．12 マスすべてを固定する．
    #[test]
    fn the_timing_table_of_the_design_comes_out_of_the_formula() {
        let table = [
            (24.0, [42, 83, 125, 167]),
            (30.0, [33, 67, 100, 133]),
            (60.0, [17, 33, 50, 67]),
        ];
        for (fps, want) in table {
            for (i, w) in want.iter().enumerate() {
                let hold = i as u32 + 1;
                assert_eq!(
                    duration_ms(fps, hold).expect("引ける"),
                    *w,
                    "{fps} FPS {hold} コマ"
                );
            }
        }
        assert!(duration_ms(0.0, 1).is_err());
        assert!(duration_ms(60.0, 0).is_err());
    }

    /// **壊れると: 60 FPS の 1 コマを 16 ms にした «別の表» が紛れ込む** (未決事項 #5) ．
    ///
    /// 決め手は `px validate` の «表示時間は表示周期の倍数か» である．
    /// **1 コマだけ見ると 16 ms でも通る** ので，2 コマ以降まで見る．
    #[test]
    fn sixty_fps_rounds_to_seventeen_because_the_error_of_sixteen_accumulates() {
        use crate::validate::{Target, validate_frames};
        let ours: Vec<u32> = (1..=4)
            .map(|h| duration_ms(60.0, h).expect("引ける"))
            .collect();
        assert_eq!(ours, vec![17, 33, 50, 67]);

        let canvas = IndexedCanvas::filled(8, 8, 1).with_transparent(Some(0));
        let fired = |ms: u32, target: &Target| {
            let mut f = Frame::new(uvec2(8, 8), palette());
            f.layers.push(Layer::new(
                crate::frame::LayerMeta::named("art"),
                Surface::Indexed(canvas.clone()),
            ));
            f.duration_ms = ms;
            validate_frames(&[f], target)
                .violations
                .iter()
                .any(|v| v.constraint == "frame-ms")
        };
        for name in ["gb", "nes", "snes", "gba"] {
            let target = Target::builtin(name).expect("組み込み");
            assert_eq!(
                ours.iter().filter(|ms| fired(**ms, &target)).count(),
                0,
                "{name} で四捨五入の表が違反になった"
            );
            let sixteen = [16u32, 32, 48, 64];
            assert_eq!(
                sixteen.iter().filter(|ms| fired(**ms, &target)).count(),
                3,
                "{name} で 16 ms 固定の違反数が変わった"
            );
            // **1 コマだけなら 16 ms も通る** — ここだけ見て決めると逆を選ぶ
            assert!(!fired(16, &target), "{name} の 1 コマは 16 ms でも通るはず");
        }
    }

    fn palette() -> Palette {
        Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::new(40, 40, 60, 255),
            Rgba8::new(90, 90, 120, 255),
            Rgba8::new(150, 150, 190, 255),
        ])
        .expect("パレット")
    }

    fn frame_of(canvas: IndexedCanvas) -> Frame {
        let mut f = Frame::new(uvec2(canvas.width(), canvas.height()), palette());
        f.layers.push(Layer::new(
            crate::frame::LayerMeta::named("art"),
            Surface::Indexed(canvas),
        ));
        f
    }

    fn blob() -> IndexedCanvas {
        let mut c = IndexedCanvas::filled(16, 16, 0).with_transparent(Some(0));
        for y in 5..11 {
            for x in 5..11 {
                c.set(x, y, 2);
            }
        }
        c
    }

    /// **壊れると: イージングの指定とフレーム数がずれたまま黙って通る．**
    #[test]
    fn easing_takes_one_hold_for_all_frames_or_exactly_one_each() {
        let mut frames = vec![frame_of(blob()), frame_of(blob()), frame_of(blob())];
        let r = ease(&mut frames, 24.0, &[2]).expect("一律");
        assert!(frames.iter().all(|f| f.duration_ms == 83));
        assert_eq!(r.total_ms, 249);

        let r = ease(&mut frames, 24.0, &[1, 2, 4]).expect("フレームごと");
        assert_eq!(
            frames.iter().map(|f| f.duration_ms).collect::<Vec<_>>(),
            vec![42, 83, 167]
        );
        assert_eq!(r.total_ms, 292);

        // 数が合わないものは黙って埋めない
        assert!(ease(&mut frames, 24.0, &[1, 2]).is_err());
        assert!(ease(&mut [], 24.0, &[1]).is_err());
    }

    /// **壊れると: 正弦波の 2 枚が «動いている» ことになり，D44 の根拠が崩れる．**
    ///
    /// $n = 2$ では $\sin(0) = \sin(\pi) = 0$ なので振幅がいくつでも動かない．
    /// **代数なので数で縛る．**
    #[test]
    fn a_sine_cycle_of_two_frames_cannot_move_at_all() {
        for k in 0..2 {
            assert!(
                Wave::Sine.at(k, 2, 0.0, 0).abs() < 1e-6,
                "k={k} で 0 でない"
            );
        }
        // 3 枚なら動く
        let moved = (0..3)
            .filter(|k| Wave::Sine.at(*k, 3, 0.0, 0).abs() > 0.5)
            .count();
        assert_eq!(moved, 2);
        // 矩形波は 0 を取らないので 2 枚でも «動く» — 理由が波によって違う
        assert_eq!(Wave::Square.at(0, 2, 0.0, 0), 1.0);
        assert_eq!(Wave::Square.at(1, 2, 0.0, 0), -1.0);
        // それでも 2 枚は受け付けない (D44)
        let spec = CycleSpec {
            frames: 2,
            ..CycleSpec::sway(0)
        };
        assert!(matches!(
            cycle(&frame_of(blob()), &spec, None),
            Err(CoreError::AnimTooFewFrames { .. })
        ));
    }

    /// **壊れると: 同じ seed で違う絵が出る (設計書 6.12 が禁じている)．**
    ///
    /// **書いた 12 通りすべて**で確かめる — 1 つだけ見ると乱数系の取りこぼしに
    /// 気付けない．
    #[test]
    fn all_twelve_written_combinations_reproduce_with_the_same_seed() {
        let src = frame_of(blob());
        let ramp = Ramp::new(vec![1, 2, 3], ChromaCurve::PeakMiddle);
        let mut written = 0;
        for target in ModTarget::ALL {
            for wave in Wave::ALL {
                let spec = CycleSpec {
                    target: *target,
                    wave: *wave,
                    frames: 5,
                    amplitude: 2.0,
                    seed: 12345,
                    ..CycleSpec::default()
                };
                let a = cycle(&src, &spec, Some(&ramp));
                let b = cycle(&src, &spec, Some(&ramp));
                if *target == ModTarget::Rotate {
                    assert!(matches!(a, Err(CoreError::AnimRotateNotWritten)));
                    continue;
                }
                let (a, _) = a.expect("作れる");
                let (b, _) = b.expect("作れる");
                assert_eq!(a.len(), 5);
                for (x, y) in a.iter().zip(&b) {
                    assert_eq!(
                        x.layers[0].surface.as_indexed(),
                        y.layers[0].surface.as_indexed(),
                        "{}x{} が同じ seed で揺れた",
                        target.as_str(),
                        wave.as_str()
                    );
                }
                written += 1;
            }
        }
        assert_eq!(written, 12, "書いた通り数が変わった");
    }

    /// **壊れると: 種を変えても同じ絵が出る (乱数が効いていない)．**
    #[test]
    fn the_seed_actually_changes_the_random_waves() {
        let src = frame_of(blob());
        for wave in [Wave::Noise, Wave::RandomBlink] {
            let mk = |seed| CycleSpec {
                wave,
                frames: 8,
                amplitude: 3.0,
                seed,
                ..CycleSpec::sway(seed)
            };
            let (a, _) = cycle(&src, &mk(1), None).expect("作れる");
            let (b, _) = cycle(&src, &mk(2), None).expect("作れる");
            let same = a
                .iter()
                .zip(&b)
                .filter(|(x, y)| {
                    x.layers[0].surface.as_indexed() == y.layers[0].surface.as_indexed()
                })
                .count();
            assert!(same < 8, "{} が種で変わらない", wave.as_str());
        }
    }

    /// **壊れると: 回転が «こっそり別の実装» で書かれる．**
    ///
    /// 回転は `px rotate` (設計書 6.13) の仕事である．**書いていないと言う．**
    #[test]
    fn rotate_is_refused_instead_of_guessed() {
        let spec = CycleSpec::rotate(0);
        assert!(matches!(
            cycle(&frame_of(blob()), &spec, None),
            Err(CoreError::AnimRotateNotWritten)
        ));
    }

    /// **壊れると: ランプの宣言が無いのに «それらしい» 明滅を作る (同語反復)．**
    #[test]
    fn modulating_a_ramp_needs_the_ramp_to_be_declared() {
        let spec = CycleSpec::flicker(7);
        assert!(matches!(
            cycle(&frame_of(blob()), &spec, None),
            Err(CoreError::AnimNoRamp)
        ));
        let ramp = Ramp::new(vec![1, 2, 3], ChromaCurve::PeakMiddle);
        let (frames, _) = cycle(&frame_of(blob()), &spec, Some(&ramp)).expect("作れる");
        assert_eq!(frames.len(), 3);
    }

    /// **壊れると: 変調が色を作る．**
    ///
    /// 3 つとも «既にある添字を置き直す» 操作なので，出てくる添字は元の絵と
    /// ランプの範囲に収まる — 合成の不変条件 (D94) と同じ性質である．
    #[test]
    fn modulation_never_invents_a_colour() {
        let src = frame_of(blob());
        let ramp = Ramp::new(vec![1, 2, 3], ChromaCurve::PeakMiddle);
        let before: std::collections::BTreeSet<u8> = src.layers[0]
            .surface
            .as_indexed()
            .expect("添字")
            .pixels()
            .iter()
            .copied()
            .collect();
        for target in [ModTarget::Ramp, ModTarget::Offset, ModTarget::Mask] {
            let spec = CycleSpec {
                target,
                wave: Wave::Sine,
                frames: 5,
                amplitude: 2.0,
                ..CycleSpec::default()
            };
            let (frames, _) = cycle(&src, &spec, Some(&ramp)).expect("作れる");
            for f in &frames {
                for v in f.layers[0].surface.as_indexed().expect("添字").pixels() {
                    let known = before.contains(v) || ramp.entries().contains(v);
                    assert!(known, "{} が添字 {v} を作った", target.as_str());
                }
            }
        }
    }

    /// **壊れると: 逆再生で両端が 2 度出て，そこだけ止まって見える．**
    #[test]
    fn the_reverse_leaves_out_both_ends() {
        let frames: Vec<Frame> = (0..3).map(|_| frame_of(blob())).collect();
        assert_eq!(reverse_derive(&frames).len(), 4);
        let five: Vec<Frame> = (0..5).map(|_| frame_of(blob())).collect();
        assert_eq!(reverse_derive(&five).len(), 8);
    }

    /// **壊れると: 縮めた形が «縁に接する部分だけ縮まない» という歪み方をする．**
    #[test]
    fn shrinking_treats_the_outside_of_the_canvas_as_transparent() {
        let mut c = IndexedCanvas::filled(8, 8, 0).with_transparent(Some(0));
        for y in 0..8 {
            for x in 0..8 {
                c.set(x, y, 2);
            }
        }
        let out = grow(&c, -1);
        assert_eq!(out.get(0, 0), Some(0), "縁が縮んでいない");
        assert_eq!(out.get(3, 3), Some(2), "内側まで消えた");
    }
}
