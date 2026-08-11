//! ランプ生成と照明モデル (設計書 3.3 / 6.6，D48)．
//!
//! # ランプ生成の 4 つの既定 (D48)
//!
//! | 項目 | 既定 |
//! | --- | --- |
//! | 色相ずらし | **明→黄，暗→紫** |
//! | 彩度カーブ | 5 種．既定は中央で最大 (明度に対し非単調) |
//! | 純黒回避 | 最暗色に明度と彩度を残す |
//! | 端点共有 | 複数ランプで最暗色を揃えられる |
//!
//! 彩度を明度に対し単調にしないのが要点である．中間で最大になる形が自然に見え，
//! 単調なランプは lint ルール 5 で検出される．

use crate::color::{Oklab, Rgba8, oklab_of};
use crate::error::Result;
use crate::math::{Rect, Vec2};
use crate::palette::{ChromaCurve, Palette, Ramp};
use crate::quantize::oklab_to_rgba;

/// 明るい側が寄っていく色相 (OKLab の度)．黄．
pub const HUE_LIGHT: f32 = 110.0;
/// 暗い側が寄っていく色相 (OKLab の度)．紫．
pub const HUE_DARK: f32 = 310.0;

/// ランプ生成の指定．
#[derive(Copy, Clone, Debug)]
pub struct RampSpec {
    /// 固有色．ここを中心に明暗へ広げる．
    pub base: Rgba8,
    /// 段数．4〜6 が実用域 (設計書 6.2)．
    pub steps: u8,
    pub chroma_curve: ChromaCurve,
    /// 色相をずらす量 (度)．0 でずらさない．
    pub hue_shift: f32,
    /// 明度の下限と上限．
    pub lightness: (f32, f32),
    /// 純黒を避ける (lint ルール 18)．
    pub avoid_pure_black: bool,
}

impl Default for RampSpec {
    fn default() -> Self {
        Self {
            base: Rgba8::rgb(0xb1, 0x3e, 0x53),
            steps: 5,
            chroma_curve: ChromaCurve::PeakMiddle,
            hue_shift: 25.0,
            lightness: (0.20, 0.90),
            avoid_pure_black: true,
        }
    }
}

/// 純黒とみなす明度の下限．これを下回らせない．
const MIN_LIGHTNESS: f32 = 0.12;
/// 純黒回避のために最低限残す彩度．
const MIN_CHROMA: f32 = 0.015;
/// 影面の色相を光面からこれだけは離す．
///
/// 離さないと**影が光と同一色相の明度違いだけ**になり，lint ルール 6 (単色影) に
/// 掛かる．
///
/// 35 度としてあるのは，暗く彩度の低い色では **8 ビットへ丸めるだけで色相が
/// 数度動く**ためである．20 度程度の分離だと，丸めた後に光と影がほとんど同じ
/// 色相になってしまう．
const MIN_HUE_SEPARATION: f32 = 35.0;

/// 影面が光面から空の色へ寄る割合．
///
/// 影面は「空・環境光の色」なので，固有色の近くに留めず目標色相へ大きく寄せる．
const SHADOW_HUE_FRACTION: f32 = 0.55;

/// `from` から `to` への符号付きの弧 (度)．短い方を返す．
fn signed_arc(from: f32, to: f32) -> f32 {
    let mut d = (to - from).rem_euclid(360.0);
    if d > 180.0 {
        d -= 360.0;
    }
    d
}

/// OKLab を極座標 (明度・彩度・色相) へ．
fn to_lch(lab: Oklab) -> (f32, f32, f32) {
    let c = (lab.a * lab.a + lab.b * lab.b).sqrt();
    let h = lab.b.atan2(lab.a).to_degrees().rem_euclid(360.0);
    (lab.l, c, h)
}

fn from_lch(l: f32, c: f32, h: f32) -> Oklab {
    let r = h.to_radians();
    Oklab::new(l, c * r.cos(), c * r.sin())
}

/// sRGB の色域に収まっているか．
fn in_gamut(lab: Oklab) -> bool {
    use ::palette::{FromColor, Srgb};
    let c = Srgb::from_color(::palette::Oklab::new(lab.l, lab.a, lab.b));
    const EPS: f32 = 1e-4;
    [c.red, c.green, c.blue]
        .iter()
        .all(|v| *v >= -EPS && *v <= 1.0 + EPS)
}

/// **色相を保ったまま**彩度を色域内へ落とす．
///
/// そのまま `Srgb` へ変換して成分を切り詰めると，切り詰めた成分だけが動くので
/// **色相がずれる**．影面の色相を光面から離した設計 (lint ルール 6 の回避) が，
/// 変換の最後で崩れてしまう．二分探索で入る彩度を探し，色相と明度は動かさない．
pub fn clip_to_gamut(l: f32, c: f32, h: f32) -> Oklab {
    let full = from_lch(l, c, h);
    if in_gamut(full) {
        return full;
    }
    let (mut lo, mut hi) = (0.0f32, c);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if in_gamut(from_lch(l, mid, h)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    from_lch(l, lo, h)
}

/// `from` から `to` へ短い方の弧を `amount` 度だけ回す．
fn rotate_towards(from: f32, to: f32, amount: f32) -> f32 {
    let mut diff = (to - from).rem_euclid(360.0);
    if diff > 180.0 {
        diff -= 360.0;
    }
    let step = amount.min(diff.abs()) * diff.signum();
    (from + step).rem_euclid(360.0)
}

/// 彩度カーブの倍率．$t$ は暗 (0) から明 (1)．
pub fn chroma_factor(curve: ChromaCurve, t: f32, steps: u8) -> f32 {
    match curve {
        ChromaCurve::Uniform => 1.0,
        // 中央で最大．端でも 0 にはしない — 0 にすると端が灰色になる
        ChromaCurve::PeakMiddle => 0.55 + 0.45 * (std::f32::consts::PI * t).sin(),
        ChromaCurve::ShadowHeavy => 1.0 - 0.45 * t,
        ChromaCurve::LightHeavy => 0.55 + 0.45 * t,
        ChromaCurve::SingleAccent(step) => {
            let n = steps.max(1) as f32;
            let target = (step as f32 + 0.5) / n;
            if (t - target).abs() < 0.5 / n {
                1.0
            } else {
                0.5
            }
        }
    }
}

/// ランプの色を作る．暗い順に並ぶ．
pub fn generate_ramp(spec: &RampSpec) -> Vec<Rgba8> {
    let steps = spec.steps.max(2);
    let (_, base_c, base_h) = to_lch(oklab_of(spec.base));
    let (mut lo, hi) = spec.lightness;
    if spec.avoid_pure_black {
        lo = lo.max(MIN_LIGHTNESS);
    }

    (0..steps)
        .map(|i| {
            let t = i as f32 / (steps - 1) as f32;
            let l = lo + (hi - lo) * t;

            // 明るい側は黄へ，暗い側は紫へ (D48)．中央では動かさない
            let target = if t >= 0.5 { HUE_LIGHT } else { HUE_DARK };
            let amount = spec.hue_shift * (2.0 * t - 1.0).abs();
            let h = rotate_towards(base_h, target, amount);

            let mut c = base_c * chroma_factor(spec.chroma_curve, t, steps);
            if spec.avoid_pure_black && i == 0 {
                c = c.max(MIN_CHROMA);
            }
            oklab_to_rgba(clip_to_gamut(l, c, h))
        })
        .collect()
}

/// 光源の型 (設計書 3.3)．
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum LightSource {
    Point {
        pos: Vec2,
        intensity: f32,
    },
    Line {
        a: Vec2,
        b: Vec2,
        intensity: f32,
    },
    Area {
        rect: Rect,
        intensity: f32,
    },
    /// `dir` は**光源から面へ向かう方向**．陰影計算で使う $\ell$ はその逆向き．
    Directional {
        dir: Vec2,
    },
    Ambient,
}

/// 照明モデル (設計書 3.3)．光面・影面・反射光の 3 ランプを持つ．
#[derive(Clone, Debug, PartialEq)]
pub struct LightingModel {
    /// 光面 (光源色)．
    pub key: Ramp,
    /// 影面 (空・環境光の色)．**`key` と色相が異なる** — 同一色相の明度違いだけだと
    /// lint ルール 6 (単色影) に掛かる．
    pub shadow: Ramp,
    /// 反射光．
    pub bounce: Ramp,
    /// 接地・遮蔽部の最暗色．
    pub occlusion: u8,
}

/// 光源のプリセット (設計書 3.3)．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LightPreset {
    /// 晴天 — 強い直射光，影は空の青．
    Clear,
    /// 曇天 — 拡散光，明暗差が小さい．
    Overcast,
    /// 夕方 — 光が橙，影が紫へ深く寄る．
    Sunset,
    /// 夜 (点光源) — 光が暖色，影がほぼ無彩色の暗色．
    Night,
    /// 月光 — 光が青白く弱い．
    Moonlight,
}

impl LightPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Overcast => "overcast",
            Self::Sunset => "sunset",
            Self::Night => "night",
            Self::Moonlight => "moonlight",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "clear" => Some(Self::Clear),
            "overcast" => Some(Self::Overcast),
            "sunset" => Some(Self::Sunset),
            "night" => Some(Self::Night),
            "moonlight" => Some(Self::Moonlight),
            _ => None,
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Clear,
        Self::Overcast,
        Self::Sunset,
        Self::Night,
        Self::Moonlight,
    ];

    /// `(光の色相, 影の色相, 光の彩度倍率, 影の彩度倍率, 明暗の幅)`．
    fn profile(self) -> (f32, f32, f32, f32, (f32, f32)) {
        match self {
            Self::Clear => (95.0, 260.0, 1.0, 0.9, (0.22, 0.92)),
            // 明暗差が小さいのが曇天の要点
            Self::Overcast => (100.0, 250.0, 0.6, 0.5, (0.38, 0.78)),
            Self::Sunset => (55.0, 300.0, 1.3, 1.1, (0.18, 0.88)),
            Self::Night => (70.0, 280.0, 0.9, 0.4, (0.14, 0.70)),
            Self::Moonlight => (240.0, 270.0, 0.5, 0.35, (0.16, 0.72)),
        }
    }

    /// 既定の光源の向き．
    pub fn default_source(self) -> LightSource {
        match self {
            // 点光源の夜だけは位置を持つ光源にする
            Self::Night => LightSource::Point {
                pos: Vec2 { x: 0.0, y: -8.0 },
                intensity: 1.0,
            },
            _ => LightSource::Directional {
                dir: Vec2 { x: -0.6, y: 0.8 },
            },
        }
    }
}

/// 固有色とプリセットから 3 ランプぶんのパレットと照明モデルを作る．
///
/// 返り値のパレットには occlusion を含む全ての色が入り，`LightingModel` の
/// ランプはその添字を指す．
pub fn build_lighting(
    base: Rgba8,
    preset: LightPreset,
    steps: u8,
    curve: ChromaCurve,
) -> Result<(Palette, LightingModel)> {
    let (key_hue, shadow_hue, key_chroma, shadow_chroma, lightness) = preset.profile();
    let (_, base_c, base_h) = to_lch(oklab_of(base));

    // 光面の色相は固有色から光源色へ寄せる
    let key_hue_at = |t: f32| rotate_towards(base_h, key_hue, 30.0 * (0.5 + 0.5 * t));
    // 影面は**光面を基準に**回す．固有色を基準にすると，光源色と空の色が同じ側に
    // あるときに光と影が同じ角度へ収束してしまう
    let shadow_hue_at = |t: f32| {
        let k = key_hue_at(t);
        let arc = signed_arc(k, shadow_hue).abs();
        let amount = (arc * SHADOW_HUE_FRACTION).max(MIN_HUE_SEPARATION).min(arc);
        rotate_towards(k, shadow_hue, amount)
    };
    // 反射光は影と光の中間へ
    let bounce_hue_at = |t: f32| {
        let s = shadow_hue_at(t);
        rotate_towards(s, key_hue_at(t), signed_arc(s, key_hue_at(t)).abs() * 0.4)
    };

    let ramp_colors =
        |hue_at: &dyn Fn(f32) -> f32, chroma_scale: f32, range: (f32, f32)| -> Vec<Rgba8> {
            let steps = steps.max(2);
            (0..steps)
                .map(|i| {
                    let t = i as f32 / (steps - 1) as f32;
                    let l = range.0 + (range.1 - range.0) * t;
                    let mut c = base_c * chroma_scale * chroma_factor(curve, t, steps);
                    if i == 0 {
                        c = c.max(MIN_CHROMA);
                    }
                    oklab_to_rgba(clip_to_gamut(l.max(MIN_LIGHTNESS), c, hue_at(t)))
                })
                .collect()
        };

    let key = ramp_colors(&key_hue_at, key_chroma, lightness);
    // 影は光より暗い側へ寄せる
    let shadow = ramp_colors(
        &shadow_hue_at,
        shadow_chroma,
        (
            lightness.0 * 0.7,
            lightness.0 + (lightness.1 - lightness.0) * 0.45,
        ),
    );
    let bounce = ramp_colors(
        &bounce_hue_at,
        shadow_chroma * 1.2,
        (
            lightness.0 * 0.85,
            lightness.0 + (lightness.1 - lightness.0) * 0.35,
        ),
    );
    let occlusion_color = oklab_to_rgba(clip_to_gamut(
        (lightness.0 * 0.55).max(MIN_LIGHTNESS * 0.8),
        (base_c * shadow_chroma * 0.6).max(MIN_CHROMA),
        shadow_hue_at(0.0),
    ));

    let mut entries = Vec::new();
    let push_range = |colors: &[Rgba8], entries: &mut Vec<Rgba8>| -> Vec<u8> {
        colors
            .iter()
            .map(|c| {
                let at = entries.len() as u8;
                entries.push(*c);
                at
            })
            .collect()
    };
    let key_idx = push_range(&key, &mut entries);
    let shadow_idx = push_range(&shadow, &mut entries);
    let bounce_idx = push_range(&bounce, &mut entries);
    let occlusion = entries.len() as u8;
    entries.push(occlusion_color);

    let mut palette = Palette::new(entries)?;
    let model = LightingModel {
        key: Ramp::new(key_idx, curve),
        shadow: Ramp::new(shadow_idx, curve),
        bounce: Ramp::new(bounce_idx, curve),
        occlusion,
    };
    palette.add_ramp(model.key.clone());
    palette.add_ramp(model.shadow.clone());
    palette.add_ramp(model.bounce.clone());
    Ok((palette, model))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lightness_of(c: Rgba8) -> f32 {
        oklab_of(c).l
    }

    fn chroma_of(c: Rgba8) -> f32 {
        oklab_of(c).chroma()
    }

    fn hue_of(c: Rgba8) -> f32 {
        to_lch(oklab_of(c)).2
    }

    #[test]
    fn a_ramp_is_monotone_in_lightness() {
        let colors = generate_ramp(&RampSpec::default());
        for w in colors.windows(2) {
            assert!(
                lightness_of(w[0]) < lightness_of(w[1]),
                "明度が単調でない: {:?}",
                colors
            );
        }
    }

    #[test]
    fn a_ramp_has_the_requested_number_of_steps() {
        for steps in [2u8, 4, 5, 6, 8] {
            let spec = RampSpec {
                steps,
                ..RampSpec::default()
            };
            assert_eq!(generate_ramp(&spec).len(), steps as usize);
        }
    }

    /// D48 の要点 — 明→黄・暗→紫．
    #[test]
    fn hue_shifts_towards_yellow_when_light_and_purple_when_dark() {
        let spec = RampSpec {
            // 青系を基準にすると，両端がどちらへ動いたか分かりやすい
            base: Rgba8::rgb(0x3b, 0x5d, 0xc9),
            steps: 5,
            hue_shift: 40.0,
            ..RampSpec::default()
        };
        let colors = generate_ramp(&spec);
        let base_h = hue_of(spec.base);
        let dark_h = hue_of(colors[0]);
        let light_h = hue_of(*colors.last().unwrap());

        let towards = |from: f32, to: f32| {
            let mut d = (to - from).rem_euclid(360.0);
            if d > 180.0 {
                d -= 360.0;
            }
            d
        };
        assert!(
            towards(base_h, light_h).signum() == towards(base_h, HUE_LIGHT).signum(),
            "明側が黄へ寄っていない (基準 {base_h:.0}° -> {light_h:.0}°)"
        );
        assert!(
            towards(base_h, dark_h).signum() == towards(base_h, HUE_DARK).signum(),
            "暗側が紫へ寄っていない (基準 {base_h:.0}° -> {dark_h:.0}°)"
        );
    }

    /// lint ルール 5 が見るのは「彩度が明度に対し単調」かどうか．
    #[test]
    fn the_default_chroma_curve_is_not_monotone() {
        let colors = generate_ramp(&RampSpec::default());
        let chromas: Vec<f32> = colors.iter().map(|c| chroma_of(*c)).collect();
        let rising = chromas.windows(2).all(|w| w[0] <= w[1]);
        let falling = chromas.windows(2).all(|w| w[0] >= w[1]);
        assert!(
            !rising && !falling,
            "既定の彩度カーブが単調になっている: {chromas:?}"
        );
    }

    #[test]
    fn uniform_curve_keeps_the_chroma_factor_flat() {
        let factors: Vec<f32> = (0..5)
            .map(|i| chroma_factor(ChromaCurve::Uniform, i as f32 / 4.0, 5))
            .collect();
        assert!(
            factors.iter().all(|f| (*f - 1.0).abs() < 1e-6),
            "{factors:?}"
        );
    }

    #[test]
    fn gamut_clipping_preserves_hue() {
        // 色域から大きくはみ出す彩度を与えても色相が動かないこと
        for h in [0.0f32, 55.0, 110.0, 200.0, 300.0] {
            for l in [0.15f32, 0.5, 0.9] {
                let got = clip_to_gamut(l, 0.9, h);
                let (_, c, got_h) = to_lch(got);
                if c < 1e-4 {
                    continue;
                }
                let diff = (got_h - h).abs().min(360.0 - (got_h - h).abs());
                assert!(diff < 1.0, "L={l} h={h}° が {got_h:.1}° へずれた");
            }
        }
    }

    /// 見た目の彩度は sRGB の色域に切り詰められる．明暗の端では落ちるのが正しい．
    #[test]
    fn uniform_curve_is_flat_within_the_srgb_gamut() {
        let spec = RampSpec {
            chroma_curve: ChromaCurve::Uniform,
            hue_shift: 0.0,
            // 端を避けて色域に余裕のある範囲で見る
            lightness: (0.45, 0.65),
            ..RampSpec::default()
        };
        let chromas: Vec<f32> = generate_ramp(&spec).iter().map(|c| chroma_of(*c)).collect();
        let spread = chromas.iter().fold(f32::MIN, |a, b| a.max(*b))
            - chromas.iter().fold(f32::MAX, |a, b| a.min(*b));
        assert!(spread < 0.02, "色域の内側でも彩度が揺れている: {chromas:?}");
    }

    #[test]
    fn shadow_heavy_and_light_heavy_are_opposites() {
        let dark = chroma_factor(ChromaCurve::ShadowHeavy, 0.0, 5);
        let light = chroma_factor(ChromaCurve::ShadowHeavy, 1.0, 5);
        assert!(dark > light, "ShadowHeavy が影を濃くしていない");
        let dark = chroma_factor(ChromaCurve::LightHeavy, 0.0, 5);
        let light = chroma_factor(ChromaCurve::LightHeavy, 1.0, 5);
        assert!(dark < light, "LightHeavy が明色を濃くしていない");
    }

    #[test]
    fn single_accent_lifts_exactly_one_step() {
        let n = 5u8;
        let lifted: Vec<usize> = (0..n)
            .filter(|i| {
                let t = *i as f32 / (n - 1) as f32;
                chroma_factor(ChromaCurve::SingleAccent(2), t, n) > 0.9
            })
            .map(|i| i as usize)
            .collect();
        assert_eq!(lifted.len(), 1, "1 段だけ上がるはず: {lifted:?}");
    }

    /// lint ルール 18 — 純黒 ($L \approx 0$ かつ彩度 0) を使わない．
    #[test]
    fn pure_black_is_avoided() {
        let spec = RampSpec {
            lightness: (0.0, 0.9),
            avoid_pure_black: true,
            ..RampSpec::default()
        };
        let darkest = generate_ramp(&spec)[0];
        assert!(lightness_of(darkest) > 0.05, "最暗色が黒すぎる");
        assert!(chroma_of(darkest) > 0.005, "最暗色に彩度が無い");
        assert_ne!(darkest, Rgba8::rgb(0, 0, 0));
    }

    #[test]
    fn pure_black_can_be_allowed_explicitly() {
        let spec = RampSpec {
            lightness: (0.0, 0.9),
            avoid_pure_black: false,
            chroma_curve: ChromaCurve::Uniform,
            hue_shift: 0.0,
            base: Rgba8::rgb(128, 128, 128),
            ..RampSpec::default()
        };
        assert!(lightness_of(generate_ramp(&spec)[0]) < 0.05);
    }

    #[test]
    fn every_preset_builds_a_usable_lighting_model() {
        for preset in LightPreset::ALL {
            let (palette, model) = build_lighting(
                Rgba8::rgb(0xb1, 0x3e, 0x53),
                preset,
                5,
                ChromaCurve::PeakMiddle,
            )
            .unwrap_or_else(|e| panic!("{} で失敗: {e}", preset.as_str()));

            assert_eq!(model.key.len(), 5);
            assert_eq!(model.shadow.len(), 5);
            assert_eq!(model.bounce.len(), 5);
            for r in [&model.key, &model.shadow, &model.bounce] {
                for &i in r.entries() {
                    assert!(
                        palette.get(i).is_some(),
                        "{}: 添字 {i} が範囲外",
                        preset.as_str()
                    );
                }
            }
            assert!(palette.get(model.occlusion).is_some());
        }
    }

    /// lint ルール 6 — 影面が光面と同一色相の明度違いだけになっていないこと．
    #[test]
    fn shadow_hue_differs_from_key_hue() {
        for preset in LightPreset::ALL {
            let (palette, model) = build_lighting(
                Rgba8::rgb(0x38, 0xb7, 0x64),
                preset,
                5,
                ChromaCurve::PeakMiddle,
            )
            .unwrap();
            let key_h = hue_of(palette.get(model.key.entries()[2]).unwrap());
            let shadow_h = hue_of(palette.get(model.shadow.entries()[2]).unwrap());
            let diff = (key_h - shadow_h)
                .abs()
                .min(360.0 - (key_h - shadow_h).abs());
            assert!(
                diff > 10.0,
                "{}: 影と光の色相が近すぎる ({key_h:.0}° と {shadow_h:.0}°)",
                preset.as_str()
            );
        }
    }

    #[test]
    fn overcast_has_the_smallest_lightness_range() {
        let range = |p: LightPreset| {
            let (_, _, _, _, l) = p.profile();
            l.1 - l.0
        };
        for other in [LightPreset::Clear, LightPreset::Sunset] {
            assert!(
                range(LightPreset::Overcast) < range(other),
                "曇天の明暗差が {} より大きい",
                other.as_str()
            );
        }
    }

    #[test]
    fn night_uses_a_point_light() {
        assert!(matches!(
            LightPreset::Night.default_source(),
            LightSource::Point { .. }
        ));
        assert!(matches!(
            LightPreset::Clear.default_source(),
            LightSource::Directional { .. }
        ));
    }

    #[test]
    fn preset_names_round_trip() {
        for p in LightPreset::ALL {
            assert_eq!(LightPreset::parse(p.as_str()), Some(p));
        }
        assert_eq!(LightPreset::parse("noon"), None);
    }

    #[test]
    fn rotate_towards_takes_the_short_arc() {
        assert!((rotate_towards(350.0, 10.0, 5.0) - 355.0).abs() < 1e-3);
        assert!((rotate_towards(10.0, 350.0, 5.0) - 5.0).abs() < 1e-3);
        // 行き過ぎない
        assert!((rotate_towards(0.0, 10.0, 90.0) - 10.0).abs() < 1e-3);
    }
}
