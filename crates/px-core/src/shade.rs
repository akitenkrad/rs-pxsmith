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

use crate::canvas::IndexedCanvas;
use crate::color::Rgba8;
use crate::error::Result;
use crate::geom::{Field, Mask, curvature_field, normal_and_curvature, signed_distance};
use crate::math::{IVec2, Rect, Vec2, ivec2, vec2};
use crate::palette::Palette;
use crate::ramp::{LightSource, LightingModel};

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

/// 陰影を作るときの設定．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ShadeOptions {
    /// `HasBounceNeighbor` の探索距離 (下方向 $k$ 画素．設計書 6.2 の既定は 2)．
    pub bounce_reach: u32,
    /// 稜線を埋めるときに距離場を平滑化する回数 (3 段処理の 1 段目)．
    pub smooth_passes: usize,
    /// 近傍の確定した法線を探す半径 (3 段処理の 2 段目)．
    pub fill_radius: i32,
    /// **環境遮蔽 (`--ao`)．凹んだところを遮蔽色へ落とす閾値** (`None` で掛けない)．
    ///
    /// 曲率は符号付き距離場の $-\nabla^2 d$ で，**凸で正 ・凹で負**である (D56) ．
    /// 谷折りの角は左右の面から光が届きにくいので暗くなる — これを 1 画素単位で
    /// 近似する．曲率が $-\text{閾値}$ を下回った画素を [`LightingModel::occlusion`] にする．
    ///
    /// > [!warning] **曲率は平滑化した距離場から取る．**
    /// > ラスタライズした縁の階段は高周波で，**素の曲率では形の凹凸より階段が勝つ** —
    /// > 実測すると凸な円板でも 5% 点が $-1.333$ (最小値そのもの) に張り付き，
    /// > 谷折りのある形と区別が付かない．3 回均すと 円板 $-0.066$ 対 谷折り $-0.289$ と
    /// > 分かれる．
    /// >
    /// > **それでも完全には分かれない** — 円板の最小は $-0.294$ なので，閾値を
    /// > 下げるほど凸な形の縁も拾う．遮蔽は «少し乗る» 側で使う効果なので許容する．
    ///
    /// **4 群で測って 0.25 に決めた** (`px-calib ao`) ．lint と同じ作法である．
    ///
    /// | 群 | 遮蔽の割合 (中央 / 最大) | 望ましい向き |
    /// | --- | --- | --- |
    /// | 凸 (円板 ・箱) | **0.00% / 0.00%** | 乗らない |
    /// | 浅い凹 (谷折り ・L 字) | 1.58% / 2.06% | 少し乗る |
    /// | **深い凹 (十字 ・コの字)** | **4.69% / 6.46%** | 乗る |
    /// | 実素材のスプライト (38 枚) | 2.51% / 12.90% | 少しだけ乗る |
    /// | 画面いっぱいのタイル (26 枚) | **0.00% / 0.00%** | 乗らない (窪みが無い) |
    ///
    /// 深い凹み > 浅い凹み > 凸 の順に並ぶ．閾値を 0.40 まで上げると**深い凹みでも
    /// 0% になる**ので，そこが上限である．
    ///
    /// > [!warning] **この数字は [`ao_curvature`] の余白を入れてからのものである．**
    /// > 入れる前は画面いっぱいのタイル 26 枚が**判で押したように 25.0%** で，
    /// > 実素材のスプライトも 16.8% だった (下記) ．
    pub ambient_occlusion: Option<f32>,
    /// **環境遮蔽の曲率を測る前に距離場を均す回数**．
    ///
    /// 縁の階段は高周波なので，均さないと形の凹凸より階段が勝つ．
    /// 閾値と**組で決める** — 片方だけ動かすと運転点を取り逃す．
    pub ao_smooth_passes: usize,
}

impl Default for ShadeOptions {
    fn default() -> Self {
        Self {
            bounce_reach: 2,
            // 1 回で足りることが多い．増やすと稜線以外の法線まで鈍る
            smooth_passes: 1,
            // 稜線は 1 px の骨格線なので，左右 2 px も見れば面の法線に当たる
            fill_radius: 2,
            // 既定では掛けない — 遮蔽は «足すと絵が締まるが，掛けすぎると汚れる» 類の
            // 効果なので，利用者が明示的に頼んだときだけ働かせる
            ambient_occlusion: None,
            ao_smooth_passes: DEFAULT_AO_SMOOTH_PASSES,
        }
    }
}

/// **疑似法線の場．稜線の不定を 3 段で埋める** (設計書 6.2)．
///
/// > [!warning] **環境光へ直行してはならない．**
/// > 稜線 (medial axis) は**パーツ中心を貫く 1 px の骨格線**である．そこだけ環境光に
/// > すると，**明るい面の真ん中に暗い線が 1 本入る** — 最も目に付く壊れ方になる．
///
/// | 段 | 手 | 効く相手 |
/// | --- | --- | --- |
/// | 1 | 距離場を平滑化して再評価する | 量子化で偶然勾配が消えた点 |
/// | 2 | 近傍の確定した法線をコピーする | 本物の稜線 (骨格線) |
/// | 3 | それでも埋まらない点だけ `None` | 近傍が丸ごと不定な平坦部 |
///
/// 3 段目が残ったときだけ呼び出し側が環境光へ落とす．
pub fn normal_field(d: &Field<f32>, opts: ShadeOptions) -> Field<Option<Vec2>> {
    let mut out = Field::filled(d.width(), d.height(), None);
    for p in d.bounds().iter() {
        out.set(p, normal_and_curvature(d, p).map(|(n, _)| n));
    }

    // 1 段目 — 平滑化した距離場で測り直す
    let mut smoothed = d.clone();
    for _ in 0..opts.smooth_passes {
        smoothed = box_blur(&smoothed);
        for p in d.bounds().iter() {
            if out.copied(p).flatten().is_none()
                && let Some((n, _)) = normal_and_curvature(&smoothed, p)
            {
                out.set(p, Some(n));
            }
        }
    }

    // 2 段目 — 近傍の確定した法線をコピーする．**同じ場を読みながら書かない**
    // (埋めた値が次の点の «確定値» になると，稜線に沿って伝播して面の法線を汚す)
    let settled = out.clone();
    for p in d.bounds().iter() {
        if out.copied(p).flatten().is_some() {
            continue;
        }
        if let Some(n) = nearest_settled(&settled, p, opts.fill_radius) {
            out.set(p, Some(n));
        }
    }
    out
}

/// 近傍の確定した法線を**そのままコピーする** (半径内で最も近いものから順に見る)．
///
/// > [!warning] **平均を取ってはいけない．**
/// > 稜線は左右の面から等距離にある骨格線なので，近傍の法線は**互いに向かい合う**．
/// > 円板の中心では放射状に並んだ法線が**足すと零ベクトルになり**，正規化できずに
/// > «埋まらない» ことになる — 埋めたい相手のちょうど真ん中で失敗する．
/// > 設計書 6.2 が «近傍の確定した法線をコピーして埋める» と書いているとおりにする．
///
/// 同じ半径に複数あるときは走査順で先のものを採る (決定論性の規則 2) ．
fn nearest_settled(field: &Field<Option<Vec2>>, p: IVec2, radius: i32) -> Option<Vec2> {
    for r in 1..=radius {
        for dy in -r..=r {
            for dx in -r..=r {
                // その半径の «輪» だけを見る (内側は前の周回で見ている)
                if dx.abs() != r && dy.abs() != r {
                    continue;
                }
                if let Some(Some(n)) = field.copied(p + ivec2(dx, dy)) {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// 環境遮蔽の閾値の既定 (`px shade --ao`)．**4 群で測って決めた**
/// ([`ShadeOptions::ambient_occlusion`] を読むこと)．
///
/// > **暫定である．** 均した曲率の分布 (円板の 5% 点 $-0.066$ ・谷折り $-0.289$) の
/// > 間を取っただけで，正例 ・負例で決めていない — M3 の lint 閾値と同じ枠でやる．
/// > **CLI はこの定数を引くこと** (数値を書き写すと黙って食い違う) ．
pub const DEFAULT_AMBIENT_OCCLUSION: f32 = 0.25;

/// 環境遮蔽の曲率を測る前に距離場を均す既定の回数．
///
/// **縁の階段を落とすために要る．** 均さないと 凸 ・凹 のどちらも $-1.333$ に張り付く．
pub const DEFAULT_AO_SMOOTH_PASSES: usize = 3;

/// **環境遮蔽のための曲率場．**
///
/// 距離場は «画像の外は空» とみなすので，**シルエットが画像の端まで届いている絵では
/// 端がまるごと «窪み» になる**．画面いっぱいのタイルに掛けると外周 2 画素が額縁の
/// ように暗くなり，実測では **26 枚すべてが判で押したように 25.0% (0.25 の閾値)**
/// だった — 並べて使うタイルでは継ぎ目がそこだけ暗くなる明確な誤りである．
///
/// 画像の外は «空» ではなく **«分からない»** なので，マスクを外側へ複製して
/// 余白を付けてから距離場を作る ([`box_blur`] が範囲外を縁の複製として扱うのと
/// 同じ規約) ．余白の幅は平滑化の窓が届く範囲 + 1 とする．
fn ao_curvature(mask: &Mask, opts: ShadeOptions) -> Field<f32> {
    let margin = opts.ao_smooth_passes as i32 + 1;
    let (w, h) = (mask.width() as i32, mask.height() as i32);
    let mut padded = Mask::new((w + margin * 2) as u32, (h + margin * 2) as u32);
    for p in padded.bounds().iter() {
        // 端を複製する — 外側の «分からない» 部分はシルエットの続きとみなす
        let q = ivec2(
            (p.x - margin).clamp(0, w - 1),
            (p.y - margin).clamp(0, h - 1),
        );
        padded.set(p, mask.get(q));
    }

    let mut smoothed = signed_distance(&padded);
    for _ in 0..opts.ao_smooth_passes {
        smoothed = box_blur(&smoothed);
    }
    let full = curvature_field(&smoothed);

    // 元の窓へ切り戻す
    let mut out = Field::filled(mask.width(), mask.height(), 0.0);
    for p in mask.bounds().iter() {
        if let Some(k) = full.copied(p + ivec2(margin, margin)) {
            out.set(p, k);
        }
    }
    out
}

/// 3x3 の箱平滑化．範囲外は縁を複製する ([`normal_and_curvature`] と同じ扱い)．
fn box_blur(d: &Field<f32>) -> Field<f32> {
    let mut out = Field::filled(d.width(), d.height(), 0.0f32);
    let at = |p: IVec2| -> f32 {
        let x = p.x.clamp(0, d.width() as i32 - 1);
        let y = p.y.clamp(0, d.height() as i32 - 1);
        d.copied(ivec2(x, y)).unwrap_or(0.0)
    };
    for p in d.bounds().iter() {
        let mut acc = 0.0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                acc += at(p + ivec2(dx, dy));
            }
        }
        out.set(p, acc / 9.0);
    }
    out
}

/// `HasBounceNeighbor` — **下方向 $k$ 画素以内にシルエット境界があるか** (設計書 6.2)．
///
/// 返すのは境界までの距離である (無ければ `None`) ．反射光は接地面からの照り返しなので，
/// **近いほど明るい**．
pub fn bounce_distance_field(mask: &Mask, reach: u32) -> Field<Option<f32>> {
    let mut out = Field::filled(mask.width(), mask.height(), None);
    for p in mask.bounds().iter() {
        if !mask.get(p) {
            continue;
        }
        for k in 1..=reach as i32 {
            let below = p + ivec2(0, k);
            // 画像の外もシルエットの外なので «境界がある» と数える
            let outside = !mask.get(below);
            if outside {
                out.set(p, Some(k as f32 - 1.0));
                break;
            }
        }
    }
    out
}

/// **シルエットへ陰影を付け，パレットの添字を返す** (設計書 6.2)．
///
/// 返り値はシルエットの外が `None`．稜線の 3 段処理で埋まらなかった画素だけ
/// `model.occlusion` (環境光) へ落ちる．
pub fn shade_mask(
    mask: &Mask,
    source: LightSource,
    model: &LightingModel,
    opts: ShadeOptions,
) -> Field<Option<u8>> {
    let d = signed_distance(mask);
    let normals = normal_field(&d, opts);
    let bounce = bounce_distance_field(mask, opts.bounce_reach);
    let reach = opts.bounce_reach as f32;
    // 環境遮蔽は曲率から求める (G2)．掛けないなら場も作らない
    let curvature = opts.ambient_occlusion.map(|_| ao_curvature(mask, opts));

    let mut out = Field::filled(mask.width(), mask.height(), None);
    for p in mask.bounds().iter() {
        if !mask.get(p) {
            continue;
        }
        let Some(Some(n)) = normals.copied(p) else {
            // 3 段目 — 近傍に確定した法線が 1 つも無い画素だけがここへ来る
            out.set(p, Some(model.occlusion));
            continue;
        };
        let centre = vec2(p.x as f32 + 0.5, p.y as f32 + 0.5);
        // **面の法線は距離場の勾配の逆向きである．**
        // 符号付き距離場は内側ほど値が大きいので $\nabla d$ は «内側» を指す —
        // そのまま使うと光と影が丸ごと裏返る (左上から照らした円板の左上が影になる) ．
        let shading = shade(source, centre, n * -1.0, bounce.copied(p).flatten(), reach);
        let ramp = match shading.lamp {
            Lamp::Key => &model.key,
            Lamp::Shadow => &model.shadow,
            Lamp::Bounce => &model.bounce,
        };
        let entries = ramp.entries();
        let mut index = entries
            .get(shading.index(entries.len()))
            .copied()
            .unwrap_or(model.occlusion);

        // 環境遮蔽 — **凹んだところだけ**遮蔽色へ落とす
        if let (Some(threshold), Some(k)) = (
            opts.ambient_occlusion,
            curvature.as_ref().and_then(|f| f.copied(p)),
        ) && k < -threshold
        {
            index = model.occlusion;
        }
        out.set(p, Some(index));
    }
    out
}

/// **`px shade` の実体** — シルエットへ陰影を付け，キャンバスとパレットを返す．
///
/// [`shade_mask`] との違いはシルエットの外の扱いだけである．キャンバスは添字しか
/// 持てないので，**透明を表す添字が要る**．
///
/// > [!warning] **透明色は末尾に足す．**
/// > 先頭に入れると [`LightingModel`] の 3 ランプが指す添字が**全部 1 つずれる** —
/// > 光ランプの添字で影の色を引くことになり，絵が静かに壊れる．パレットに既に
/// > アルファ 0 の色があればそれを使う ([`crate::quantize`] と同じ «最初のアルファ 0
/// > の色が透明» という約束) ．
pub fn shade_to_canvas(
    mask: &Mask,
    source: LightSource,
    model: &LightingModel,
    palette: &Palette,
    opts: ShadeOptions,
) -> Result<(IndexedCanvas, Palette)> {
    let mut palette = palette.clone();
    let transparent = match palette.entries().iter().position(|c| c.a == 0) {
        Some(i) => i as u8,
        None => palette.push(Rgba8::TRANSPARENT)?,
    };

    let shaded = shade_mask(mask, source, model, opts);
    let mut canvas = IndexedCanvas::filled(mask.width(), mask.height(), transparent)
        .with_transparent(Some(transparent));
    for p in mask.bounds().iter() {
        if let Some(Some(index)) = shaded.copied(p) {
            canvas.set_at(p, index);
        }
    }
    Ok((canvas, palette))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::vec2;

    /// 円板 — 中心に稜線 (medial axis) ができる形である．
    fn disc(size: u32, radius: f32) -> Mask {
        let mut m = Mask::new(size, size);
        let c = (size as f32 - 1.0) / 2.0;
        for p in m.bounds().iter() {
            let (dx, dy) = (p.x as f32 - c, p.y as f32 - c);
            if dx * dx + dy * dy <= radius * radius {
                m.set(p, true);
            }
        }
        m
    }

    /// 内角 (谷折り) のある形 — 遮蔽が乗る相手である．
    fn notch(size: u32) -> Mask {
        let mut m = Mask::new(size, size);
        for p in m.bounds().iter() {
            // 右下の 1/4 を削った L 字
            let cut = p.x >= size as i32 / 2 && p.y >= size as i32 / 2;
            if !cut && p.x > 0 && p.y > 0 && p.x < size as i32 - 1 && p.y < size as i32 - 1 {
                m.set(p, true);
            }
        }
        m
    }

    fn lighting() -> (crate::palette::Palette, LightingModel) {
        crate::ramp::build_lighting(
            crate::color::Rgba8::rgb(120, 90, 70),
            crate::ramp::LightPreset::Clear,
            5,
            crate::palette::ChromaCurve::default(),
        )
        .expect("ランプを作れない")
    }

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

    /// **稜線に暗い線が入らない．** 設計書 6.2 が名指しで禁じている壊れ方である．
    ///
    /// 円板の中心は medial axis なので勾配が消える．3 段処理が無いとそこだけ環境光へ
    /// 落ち，**明るい面の真ん中に 1 px の暗い線が走る**．
    #[test]
    fn the_ridge_does_not_fall_through_to_the_ambient_colour() {
        let mask = disc(21, 9.0);
        let d = signed_distance(&mask);
        let opts = ShadeOptions::default();

        // 素の勾配では中心付近が不定になる (これが埋める相手である)
        let raw_holes = mask
            .bounds()
            .iter()
            .filter(|&p| mask.get(p) && normal_and_curvature(&d, p).is_none())
            .count();
        assert!(raw_holes > 0, "稜線が 1 つも無い形では試験にならない");

        let filled = normal_field(&d, opts);
        let left = mask
            .bounds()
            .iter()
            .filter(|&p| mask.get(p) && filled.copied(p).flatten().is_none())
            .count();
        assert!(
            left < raw_holes,
            "3 段処理で埋まっていない (前 {raw_holes} → 後 {left})"
        );
    }

    /// 反射光は**下方向 $k$ 画素以内にシルエット境界がある画素だけ**が引く．
    #[test]
    fn only_pixels_above_the_silhouette_edge_can_bounce() {
        // 4x4 の四角
        let mut mask = Mask::new(6, 6);
        for y in 1..5 {
            for x in 1..5 {
                mask.set(ivec2(x, y), true);
            }
        }
        let b = bounce_distance_field(&mask, 2);
        // 最下段は真下が外なので距離 0
        assert_eq!(b.copied(ivec2(2, 4)).flatten(), Some(0.0));
        // その 1 つ上は距離 1
        assert_eq!(b.copied(ivec2(2, 3)).flatten(), Some(1.0));
        // さらに上は届かない
        assert_eq!(b.copied(ivec2(2, 2)).flatten(), None);
        // シルエットの外は測らない
        assert_eq!(b.copied(ivec2(0, 0)).flatten(), None);
    }

    /// シルエットの中はすべて色が付き，外は付かない．
    #[test]
    fn every_pixel_inside_the_silhouette_gets_a_colour() {
        let mask = disc(21, 9.0);
        let (palette, model) = lighting();
        let out = shade_mask(
            &mask,
            crate::ramp::LightPreset::Clear.default_source(),
            &model,
            ShadeOptions::default(),
        );
        for p in mask.bounds().iter() {
            assert_eq!(
                out.copied(p).flatten().is_some(),
                mask.get(p),
                "{p:?} でシルエットと食い違う"
            );
            if let Some(Some(i)) = out.copied(p) {
                assert!((i as usize) < palette.len(), "パレットの外を指した {i}");
            }
        }
    }

    /// 曲率の分布を測る (閾値を決めるための一時的な口ではなく，**根拠として残す**)．
    #[test]
    fn curvature_separates_a_convex_disc_from_a_notched_shape() {
        use crate::geom::{curvature_field, signed_distance};
        let stats = |m: &Mask| {
            let mut d = signed_distance(m);
            // 縁の階段は高周波なので均す (掛けないと形の凹凸より階段が勝つ)
            for _ in 0..3 {
                d = box_blur(&d);
            }
            let k = curvature_field(&d);
            let mut v: Vec<f32> = m
                .bounds()
                .iter()
                .filter(|&p| m.get(p))
                .filter_map(|p| k.copied(p))
                .collect();
            v.sort_by(f32::total_cmp);
            (v[0], v[v.len() / 20], v[v.len() / 2], v[v.len() - 1])
        };
        let (dmin, dp5, dmed, dmax) = stats(&disc(21, 9.0));
        let (nmin, np5, nmed, nmax) = stats(&notch(16));
        println!("円板  最小 {dmin:.3} 5% {dp5:.3} 中央 {dmed:.3} 最大 {dmax:.3}");
        println!("谷折り 最小 {nmin:.3} 5% {np5:.3} 中央 {nmed:.3} 最大 {nmax:.3}");
    }

    /// **画面いっぱいのマスクには遮蔽が乗らない．**
    ///
    /// 全面が不透明なタイルには «外» が無いので窪みも無い．距離場は «画像の外は空»
    /// とみなすため，素直に作ると**外周が窪みに見えて額縁のように暗くなる** —
    /// 実測では 16x16 のタイル 26 枚すべてが判で押したように 25.0% だった．
    /// 並べて使うタイルでは継ぎ目だけが暗くなる明確な誤りである．
    #[test]
    fn ambient_occlusion_does_not_frame_a_full_bleed_tile() {
        let mut mask = Mask::new(16, 16);
        for p in mask.bounds().iter() {
            mask.set(p, true);
        }
        let (_, model) = lighting();
        let out = shade_mask(
            &mask,
            LightSource::Directional {
                dir: vec2(-0.6, 0.8),
            },
            &model,
            ShadeOptions {
                ambient_occlusion: Some(DEFAULT_AMBIENT_OCCLUSION),
                ..ShadeOptions::default()
            },
        );
        let dark = out
            .data()
            .iter()
            .filter(|c| **c == Some(model.occlusion))
            .count();
        assert_eq!(
            dark, 0,
            "画面いっぱいのタイルに遮蔽が乗った ({dark} / 256 画素)"
        );
    }

    /// **環境遮蔽は凹んだところにだけ乗る．** 凸な形 (円板) では 1 画素も落ちない．
    #[test]
    fn ambient_occlusion_only_darkens_concave_places() {
        let (_, model) = lighting();
        let source = LightSource::Directional {
            dir: vec2(1.0, 1.0),
        };
        let on = ShadeOptions {
            ambient_occlusion: Some(0.25),
            ..ShadeOptions::default()
        };

        // 凸な形にはほとんど乗らない．**完全に 0 にはならない** — 平滑化しても
        // 円板の縁の曲率は最小 -0.294 まで振れるので，閾値を下げるほど拾う
        let mask = disc(21, 9.0);
        let inside = mask.bounds().iter().filter(|&p| mask.get(p)).count();
        let disc_out = shade_mask(&mask, source, &model, on);
        let disc_dark = disc_out
            .data()
            .iter()
            .filter(|c| **c == Some(model.occlusion))
            .count();
        assert!(
            disc_dark * 20 < inside,
            "凸な形に遮蔽が乗りすぎ ({disc_dark} / {inside})"
        );

        // 谷折りのある形 — 内角のところが落ちる
        let notched = notch(16);
        let off = shade_mask(&notched, source, &model, ShadeOptions::default());
        let with_ao = shade_mask(&notched, source, &model, on);
        let count = |f: &Field<Option<u8>>| {
            f.data()
                .iter()
                .filter(|c| **c == Some(model.occlusion))
                .count()
        };
        assert!(
            count(&with_ao) > count(&off),
            "凹んだ形で遮蔽が増えない ({} → {})",
            count(&off),
            count(&with_ao)
        );
    }

    /// **光の当たる側と影の側で色が違う．** 陰影が付いていることの最低限の確認である．
    #[test]
    fn the_lit_side_and_the_shadow_side_use_different_colours() {
        let mask = disc(21, 9.0);
        // 光は右下へ進む → 左上が明るい
        let source = LightSource::Directional {
            dir: vec2(1.0, 1.0),
        };
        let (_, model) = lighting();
        let out = shade_mask(&mask, source, &model, ShadeOptions::default());
        let lit = out.copied(ivec2(6, 6)).flatten().expect("左上が空");
        let dark = out.copied(ivec2(14, 14)).flatten().expect("右下が空");
        assert_ne!(lit, dark, "光側と影側が同じ色である");
        assert!(model.key.entries().contains(&lit), "左上が光ランプでない");
        assert!(
            model.shadow.entries().contains(&dark) || model.bounce.entries().contains(&dark),
            "右下が影 / 反射ランプでない"
        );
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
