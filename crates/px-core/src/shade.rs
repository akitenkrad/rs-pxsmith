//! 陰影導出の画素あたりの規則 (設計書 6.2)．
//!
//! 疑似法線 $n$ と光源から，**まずどのランプを引くかを決め，次にその中で添字を決める**
//! 2 段構成である．距離場から $n$ を作る側と，稜線の NaN を埋める側は
//! [`crate::geom`] と `px shade` が持つ — ここは «$n$ が与えられたときに何色になるか»
//! だけを引き受ける．
//!
//! > [!warning] **分岐ごとに $t$ を $[0, 1]$ へ再マップする**
//! > 同じ照度をそのまま使うと，影ランプの上半分が到達不能になる — 影に落ちる画素は
//! > $\langle n, \ell \rangle \le 0$ なので，正規化しなければ $t$ が常に下半分に
//! > 収まってしまう．**ランプを 5 段用意して 3 段しか使わない**のは陰影の階調を
//! > 捨てているのと同じである．
//!
//! 照度はすべて**解析解**で求める (設計書 6.2) ．ランプ段数 $\lvert R \rvert$ は
//! 4 〜 6 なので許容誤差は $1/(2 \lvert R \rvert) \approx 8 \sim 12\%$ であり，
//! 多点サンプリングの精度は丸めで消える．

use crate::math::{Rect, Vec2};
use crate::ramp::LightSource;

/// どのランプを引くか (設計書 6.2)．
///
/// 設計書は $\mathrm{surface}(p)$ と書くが，**この名前は [`crate::frame::Surface`]
/// (レイヤの実体) が使っている**ので，[`crate::ramp::LightingModel`] の «3 ランプ» に
/// 合わせて `Lamp` と呼ぶ．指しているものは同じである．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Lamp {
    /// 光面．$\langle n, \ell \rangle > 0$．
    Key,
    /// 影面．
    Shadow,
    /// 反射光．影の側で，かつ下方向に反射面がある画素．
    Bounce,
}

/// 引くランプと，その中の位置 $t \in [0, 1]$．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Shading {
    pub lamp: Lamp,
    /// **分岐ごとに正規化済み**の位置．そのまま段数へ掛けてよい．
    pub t: f32,
}

impl Shading {
    /// 段数 $m$ のランプでの添字 $i = \lfloor \mathrm{clamp}(t, 0, 1) \cdot (m - 1) \rfloor$．
    ///
    /// $t = 1$ でちょうど最終段に着く．段数 0 のランプは呼び出し側の誤りなので 0 を返す．
    ///
    /// > [!warning] **上端は丸めてから床を取る．**
    /// > 正対する面の $\langle n, \ell \rangle$ は単位ベクトルどうしの内積なので，
    /// > 浮動小数では 0.99999994 になる．そのまま $\lfloor t (m - 1) \rfloor$ を取ると
    /// > **最終段に 1 つ届かない** (5 段のランプで 3 段目に落ちる) ．
    /// > 「光へ正対した面が最も明るい段になる」は陰影の根幹なので，ここだけ閉じる．
    pub fn index(self, steps: usize) -> usize {
        if steps <= 1 {
            return 0;
        }
        let t = if self.t >= 1.0 - 1e-5 {
            1.0
        } else {
            self.t.clamp(0.0, 1.0)
        };
        ((t * (steps - 1) as f32).floor() as usize).min(steps - 1)
    }
}

/// 光源から面への «面から光源へ向かう単位ベクトル» $\ell$ と，素の照度．
///
/// 返り値の照度は光源型ごとに意味が違う (設計書 6.2 の表) ．**向きだけが要る分岐と
/// 強さが要る分岐があるので，両方返す**．
///
/// | 光源型 | 素の照度 |
/// | --- | --- |
/// | Directional | $\langle n, \ell \rangle$ |
/// | Point | $I / r^2$ (可視半球でクリップ) |
/// | Line ・Area | $I (\sin \theta_2 - \sin \theta_1)$ |
///
/// 面の位置 `p` は画素の中心とする．`Ambient` は向きを持たないので `None`．
pub fn incidence(source: LightSource, p: Vec2, n: Vec2) -> Option<(Vec2, f32)> {
    match source {
        // `dir` は光源から面へ向かう向きなので，$\ell$ はその逆である
        LightSource::Directional { dir } => {
            let l = (dir * -1.0).normalize()?;
            Some((l, n.dot(l)))
        }
        LightSource::Point { pos, intensity } => {
            let d = pos - p;
            let r2 = d.dot(d);
            let l = d.normalize()?;
            // **可視半球でクリップする．** 背面を照らす光は届かない
            let cos = n.dot(l);
            let e = if cos > 0.0 && r2 > f32::EPSILON {
                intensity / r2
            } else {
                0.0
            };
            Some((l, e))
        }
        LightSource::Line { a, b, intensity } => {
            let l = (segment_midpoint(a, b) - p).normalize()?;
            Some((l, segment_illuminance(a, b, p, n, intensity)))
        }
        LightSource::Area { rect, intensity } => {
            let l = (rect.center() - p).normalize()?;
            // 面光源は «画素から見た矩形の張り» を線光源 2 本ぶんとして扱う —
            // 立体角の解析解は 2 次元では線分の張る角に落ちる
            let (top, bottom) = area_edges(rect);
            let e = segment_illuminance(top.0, top.1, p, n, intensity)
                .max(segment_illuminance(bottom.0, bottom.1, p, n, intensity));
            Some((l, e))
        }
        LightSource::Ambient => None,
    }
}

/// 線分光源の照度 $I (\sin \theta_2 - \sin \theta_1)$ (設計書 6.2)．
///
/// $\theta$ は面の法線を基準にした角である．$E(p) = \int L \cos\theta \, d\omega
/// = I \int_{\theta_1}^{\theta_2} \cos\theta \, d\theta$ の解析解であり，
/// **多点サンプリングと違って光源の長さに依らず $O(1)$ である**．
fn segment_illuminance(a: Vec2, b: Vec2, p: Vec2, n: Vec2, intensity: f32) -> f32 {
    let (Some(da), Some(db)) = ((a - p).normalize(), (b - p).normalize()) else {
        return 0.0;
    };
    // 法線を基準にした角 (接線方向の符号で向きを付ける)
    let tangent = Vec2 { x: -n.y, y: n.x };
    let angle = |d: Vec2| d.dot(tangent).atan2(d.dot(n));
    let (t1, t2) = {
        let (x, y) = (angle(da), angle(db));
        if x <= y { (x, y) } else { (y, x) }
    };
    // 可視半球の外は寄与しない
    let half = std::f32::consts::FRAC_PI_2;
    let (t1, t2) = (t1.clamp(-half, half), t2.clamp(-half, half));
    (intensity * (t2.sin() - t1.sin())).max(0.0)
}

fn segment_midpoint(a: Vec2, b: Vec2) -> Vec2 {
    Vec2 {
        x: (a.x + b.x) * 0.5,
        y: (a.y + b.y) * 0.5,
    }
}

/// 矩形を «上辺» と «下辺» の 2 本の線分にする．
fn area_edges(rect: Rect) -> ((Vec2, Vec2), (Vec2, Vec2)) {
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.w, rect.y + rect.h);
    (
        (Vec2 { x: x0, y: y0 }, Vec2 { x: x1, y: y0 }),
        (Vec2 { x: x0, y: y1 }, Vec2 { x: x1, y: y1 }),
    )
}

/// **どのランプを引き，その中のどこに落ちるかを決める** (設計書 6.2)．
///
/// `has_bounce_neighbor` は «下方向 $k$ 画素以内にシルエット境界があるか»
/// (既定 $k = 2$) ．呼び出し側が距離場から求めて渡す．
///
/// $$ \mathrm{surface}(p) = \begin{cases}
///   \text{key} & \langle n, \ell \rangle > 0 \\
///   \text{bounce} & \langle n, \ell \rangle \le 0 \land \mathrm{HasBounceNeighbor}(p) \\
///   \text{shadow} & \text{otherwise}
/// \end{cases} $$
///
/// $t$ は**分岐ごとに** $[0, 1]$ へ移す．
///
/// | 分岐 | $t$ | 意味 |
/// | --- | --- | --- |
/// | key | $\langle n, \ell \rangle \in (0, 1]$ | 正対するほど明るい |
/// | shadow | $1 + \langle n, \ell \rangle \in [0, 1)$ | 斜めの面ほど明るい端に寄る |
/// | bounce | 反射面までの距離で減衰 | 端ほど明るい |
pub fn shade(
    source: LightSource,
    p: Vec2,
    n: Vec2,
    bounce_distance: Option<f32>,
    bounce_range: f32,
) -> Shading {
    let Some((l, raw)) = incidence(source, p, n) else {
        // 環境光は向きを持たない．**影ランプの中央に置く** — 明暗を作らない光なので，
        // 端に寄せると «向きのある光» のふりをすることになる
        return Shading {
            lamp: Lamp::Shadow,
            t: 0.5,
        };
    };

    let cos = n.dot(l);
    if cos > 0.0 {
        // 光面．強さのある光源では素の照度で位置を決める (遠いほど暗い)
        let t = match source {
            LightSource::Directional { .. } => cos,
            _ => raw.min(1.0),
        };
        return Shading { lamp: Lamp::Key, t };
    }

    match bounce_distance.filter(|_| bounce_range > 0.0) {
        // 反射光．**反射面に近いほど明るい**ので距離で減衰させて反転する
        Some(d) if d <= bounce_range => Shading {
            lamp: Lamp::Bounce,
            t: (1.0 - d / bounce_range).clamp(0.0, 1.0),
        },
        _ => Shading {
            lamp: Lamp::Shadow,
            // $1 + \langle n, \ell \rangle$．正対の裏 (-1) で 0 ・稜線 (0) で 1 に近づく
            t: (1.0 + cos).clamp(0.0, 1.0 - f32::EPSILON),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::vec2;

    fn dir(x: f32, y: f32) -> LightSource {
        LightSource::Directional { dir: vec2(x, y) }
    }

    /// 光へ正対する面は光ランプの最終段に着く．
    #[test]
    fn a_face_turned_to_the_light_reaches_the_end_of_the_key_ramp() {
        // 光は右下へ進む → $\ell$ は左上向き．法線も左上なら正対する
        let s = shade(
            dir(1.0, 1.0),
            vec2(0.0, 0.0),
            vec2(-1.0, -1.0).normalize().unwrap(),
            None,
            2.0,
        );
        assert_eq!(s.lamp, Lamp::Key);
        assert!((s.t - 1.0).abs() < 1e-5, "t {}", s.t);
        assert_eq!(s.index(5), 4);
    }

    /// **分岐ごとの正規化が無いと影ランプの上半分が使われない．**
    ///
    /// 影に落ちる画素は $\langle n, \ell \rangle \le 0$ なので，素の照度をそのまま
    /// 使うと $t$ が下半分に張り付く．正規化して初めて全段に届く．
    #[test]
    fn the_shadow_branch_is_remapped_so_the_whole_ramp_is_reachable() {
        let l = dir(1.0, 0.0); // $\ell$ は $(-1, 0)$
        // 完全な裏面 → 影ランプの先頭
        let back = shade(l, vec2(0.0, 0.0), vec2(1.0, 0.0), None, 0.0);
        assert_eq!(back.lamp, Lamp::Shadow);
        assert!(back.t < 1e-5, "t {}", back.t);
        // 稜線に近い面 (ほぼ直交) → 影ランプの終わり側
        let grazing = shade(
            l,
            vec2(0.0, 0.0),
            vec2(0.02, 1.0).normalize().unwrap(),
            None,
            0.0,
        );
        assert_eq!(grazing.lamp, Lamp::Shadow);
        assert!(grazing.t > 0.9, "t {}", grazing.t);
        // **正規化前の素の照度なら，どちらも 0 以下で区別が付かない**
        let (_, raw_back) = incidence(l, vec2(0.0, 0.0), vec2(1.0, 0.0)).unwrap();
        let (_, raw_grazing) =
            incidence(l, vec2(0.0, 0.0), vec2(0.02, 1.0).normalize().unwrap()).unwrap();
        assert!(raw_back <= 0.0 && raw_grazing <= 0.0);
    }

    /// 反射面が下にある影の画素だけが反射光を引く．
    #[test]
    fn only_a_shadowed_pixel_near_a_bouncing_surface_uses_the_bounce_ramp() {
        let l = dir(1.0, 0.0);
        let n = vec2(1.0, 0.0); // 裏面
        assert_eq!(
            shade(l, vec2(0.0, 0.0), n, Some(1.0), 2.0).lamp,
            Lamp::Bounce
        );
        // 反射面が遠ければ影のまま
        assert_eq!(
            shade(l, vec2(0.0, 0.0), n, Some(5.0), 2.0).lamp,
            Lamp::Shadow
        );
        // 光面はそもそも反射光を引かない
        assert_eq!(
            shade(l, vec2(0.0, 0.0), vec2(-1.0, 0.0), Some(1.0), 2.0).lamp,
            Lamp::Key
        );
    }

    /// 反射光は**反射面に近いほど明るい** (端ほど明るい)．
    #[test]
    fn the_bounce_ramp_is_brightest_next_to_the_bouncing_surface() {
        let l = dir(1.0, 0.0);
        let n = vec2(1.0, 0.0);
        let near = shade(l, vec2(0.0, 0.0), n, Some(0.0), 2.0);
        let far = shade(l, vec2(0.0, 0.0), n, Some(1.9), 2.0);
        assert!(near.t > far.t, "近 {} 遠 {}", near.t, far.t);
    }

    /// 点光源は $I / r^2$ で減衰し，**背面には届かない**．
    #[test]
    fn a_point_light_falls_off_with_the_square_of_the_distance() {
        let n = vec2(0.0, -1.0);
        let near = LightSource::Point {
            pos: vec2(0.0, -2.0),
            intensity: 4.0,
        };
        let far = LightSource::Point {
            pos: vec2(0.0, -4.0),
            intensity: 4.0,
        };
        let (_, e_near) = incidence(near, vec2(0.0, 0.0), n).unwrap();
        let (_, e_far) = incidence(far, vec2(0.0, 0.0), n).unwrap();
        assert!((e_near - 1.0).abs() < 1e-5, "近 {e_near}");
        assert!((e_far - 0.25).abs() < 1e-5, "遠 {e_far}");
        // 可視半球の外 (法線が逆) では 0
        let (_, e_back) = incidence(near, vec2(0.0, 0.0), vec2(0.0, 1.0)).unwrap();
        assert_eq!(e_back, 0.0);
    }

    /// 線光源の照度は $I (\sin\theta_2 - \sin\theta_1)$ で，**広く張るほど明るい**．
    #[test]
    fn a_line_light_grows_with_the_angle_it_spans() {
        let n = vec2(0.0, -1.0);
        let narrow = LightSource::Line {
            a: vec2(-0.5, -4.0),
            b: vec2(0.5, -4.0),
            intensity: 1.0,
        };
        let wide = LightSource::Line {
            a: vec2(-4.0, -4.0),
            b: vec2(4.0, -4.0),
            intensity: 1.0,
        };
        let (_, e_narrow) = incidence(narrow, vec2(0.0, 0.0), n).unwrap();
        let (_, e_wide) = incidence(wide, vec2(0.0, 0.0), n).unwrap();
        assert!(e_wide > e_narrow, "広 {e_wide} 狭 {e_narrow}");
        // 真上に無限に広い光源でも 2 を超えない ($\sin$ の差の上限)
        assert!(e_wide <= 2.0 + 1e-5, "{e_wide}");
    }

    /// 環境光は向きを持たないので**影ランプの中央**に置く．
    #[test]
    fn ambient_light_has_no_direction_and_sits_in_the_middle() {
        let s = shade(
            LightSource::Ambient,
            vec2(0.0, 0.0),
            vec2(0.0, -1.0),
            None,
            0.0,
        );
        assert_eq!(s.lamp, Lamp::Shadow);
        assert!((s.t - 0.5).abs() < 1e-6);
        assert!(incidence(LightSource::Ambient, vec2(0.0, 0.0), vec2(0.0, -1.0)).is_none());
    }

    /// 添字は $\lfloor t (m - 1) \rfloor$ で，**$t = 1$ がちょうど最終段**である．
    #[test]
    fn the_index_spans_the_whole_ramp() {
        let at = |t: f32| Shading { lamp: Lamp::Key, t };
        assert_eq!(at(0.0).index(5), 0);
        assert_eq!(at(0.5).index(5), 2);
        assert_eq!(at(1.0).index(5), 4);
        // 範囲外は閉じる
        assert_eq!(at(-1.0).index(5), 0);
        assert_eq!(at(2.0).index(5), 4);
        // 1 段のランプは常に 0
        assert_eq!(at(1.0).index(1), 0);
    }
}
