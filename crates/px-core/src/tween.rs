//! 中割り (設計書 6.9)．
//!
//! **形状のみ補間し，色は補間しない．** 陰影は各中間フレームで導出し直す
//! (`px anim tween` が [`crate::shade`] を呼ぶ) ．ここは形だけを引き受ける．
//!
//! # 補間した符号付き距離場の 0 等高線を取る
//!
//! $d = (1 - t)\,d_A + t\,d_B$ を作って $d > 0$ を新しいシルエットとする．
//! 1-Lipschitz 関数の凸結合は 1-Lipschitz なので，補間した場も距離場としての
//! 性質を保つ．
//!
//! ## 結果は必ず $A \cap B$ と $A \cup B$ の間にある
//!
//! **これは測定ではなく代数である．** [`signed_distance`] は内側が正 ・外側が負で，
//! 0 を取る画素は無い (最小の歩みが 1.0) ．
//!
//! | 画素 | $d_A$ | $d_B$ | $(1-t) d_A + t d_B$ |
//! | --- | --- | --- | --- |
//! | $A \cap B$ | 正 | 正 | **必ず正** — 結果に入る |
//! | $A \cup B$ の外 | 負 | 負 | **必ず負** — 結果に入らない |
//!
//! $t \in (0, 1)$ なら係数は両方とも正なので，符号が揃っていれば和の符号も揃う．
//! したがって $A \cap B \subseteq R \subseteq A \cup B$ である．**トポロジーが
//! 変わらないことまでは言えない** — 途中で千切れることはある (設計書 6.9 が
//! «オイラー標数の変化を lint で検出する» と言っているのはこのため) ．
//!
//! ## 動きに直交する向きに痩せる — これも代数である (R11)
//!
//! 半径 $r$ の円板を距離 $D$ だけ平行移動した 2 枚を $t = 1/2$ で補間すると，
//! 0 等高線は $\lvert x - c_A \rvert + \lvert x - c_B \rvert = 2r$，すなわち
//! **焦点を 2 つの中心に持つ楕円**になる．長半径は $r$ のままだが短半径は
//! $\sqrt{r^2 - (D/2)^2}$ に縮む．
//!
//! | $D / r$ | 短半径 / $r$ | 面積比 |
//! | --- | --- | --- |
//! | 0.5 | 0.97 | 0.97 |
//! | 1.0 | 0.87 | 0.87 |
//! | 1.5 | 0.66 | 0.66 |
//! | **2.0** | **0** | **0 — 消える** |
//!
//! **$D \geq 2r$ で中割りは空になる．** これが実装計画書のリスク R11
//! («SDF 補間が実用にならない») の中身である — 平行移動そのものを SDF に
//! 解かせているのが原因なので，[`TweenAlign::Centroid`] で**移動ぶんを先に
//! 取り除いてから形だけを補間する**道を用意した．どちらが良いかは
//! `px-calib tween` で真値のある場面 (平行移動) を作って測ってある．
//!
//! # 余白は «距離場のため» ではなく «重心を戻すため» に要る
//!
//! [`signed_distance`] は**画像の外を «背景» として扱う**ので，縁の近くでは距離が
//! 縁までの距離で頭打ちになる．D91 が `--ao` で踏んだ穴なので，ここでも余白が
//! 要るだろうと考えて掃いた — **測ったら «要らない» という結果だった**．
//!
//! | 群 | 余白 0 と余白 64 の差 |
//! | --- | --- |
//! | 場のまま ・縁に余地のある絵 66 枚 | **0 画素** |
//! | 場のまま ・**縁に接する絵** 66 枚 | **0 画素** |
//! | 重心を取り除く ・縁に接する絵 66 枚 | **17 画素 (5 枚)** |
//!
//! **場をそのまま補間するなら余白は 1 画素も要らない．これは代数である** —
//! 頭打ちが起きるのは «形より縁の方が近い» 画素，すなわち両方の形の外側であり，
//! そこでは $d_A$ も $d_B$ も負なので**補間しても負のままである**．
//! 0 等高線は動かしようがない．
//!
//! 差が出たのは重心を取り除く側だけで，理由も別である — **ずらした先が画布から
//! はみ出して切れていた**．必要な余白は «ずらし量» そのものなので，
//! 校正するものではなく[**重心の差から計算する**](TweenOptions::margin)．

use crate::error::{CoreError, Result};
use crate::geom::{Mask, signed_distance};
use crate::math::{IVec2, ivec2};

/// 補間する前に位置を揃えるか．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TweenAlign {
    /// 設計書 6.9 のまま — 場をそのまま補間する．
    ///
    /// **平行移動が入っていると動きに直交する向きに痩せる** (モジュールの説明) ．
    None,
    /// **重心を合わせてから形を補間し，重心だけを直線で動かす．**
    ///
    /// 平行移動だけの動きなら 2 枚は重ね合わせで一致するので，中割りは
    /// «元の形を $t$ ぶん動かしたもの» そのものになる．
    #[default]
    Centroid,
}

impl TweenAlign {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Centroid => "centroid",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "centroid" => Some(Self::Centroid),
            _ => None,
        }
    }
}

/// 中割りの設定．
#[derive(Copy, Clone, Debug, Default)]
pub struct TweenOptions {
    /// **上乗せする余白．既定は 0 で，普通は触らなくてよい．**
    ///
    /// 実際に使う余白は `max(margin, 重心の差)` である — 距離場のために余白が
    /// 要るという見立ては測ったら外れており (モジュールの説明) ，要るのは
    /// **重心を戻したときに画布から出ないだけの幅**だけである．
    /// ここは «測るために掃く» ための口として残してある．
    pub margin: u32,
    /// 補間する前に位置を揃えるか．
    pub align: TweenAlign,
}

/// 2 つのマスクを共通の画布へ載せる (設計書 6.9 の `align_to_common_bbox`)．
///
/// **`zip` による切り詰めを禁止する** — 寸法の違う 2 枚をそのまま `zip` すると
/// 短い方に合わせて黙って切れる．大きい方に合わせて広げ，外側へ `margin` を付ける．
///
/// 返すのは (載せた $A$，載せた $B$，共通の画布における元の原点) ．
pub fn align_to_common_canvas(a: &Mask, b: &Mask, margin: u32) -> (Mask, Mask, IVec2) {
    let w = a.width().max(b.width()) + margin * 2;
    let h = a.height().max(b.height()) + margin * 2;
    let origin = ivec2(margin as i32, margin as i32);
    (place(a, w, h, origin), place(b, w, h, origin), origin)
}

fn place(src: &Mask, w: u32, h: u32, origin: IVec2) -> Mask {
    let mut out = Mask::new(w, h);
    for p in src.iter_set() {
        out.set(p + origin, true);
    }
    out
}

/// 中割りしたシルエット 1 枚．
#[derive(Clone, Debug)]
pub struct Tweened {
    pub mask: Mask,
    /// $t$ (0 が $A$，1 が $B$)．
    pub t: f32,
    /// 4 連結の成分数 ($A$ ・$B$ ・結果) ．**トポロジーが変わったかを見る口**である．
    pub components: (usize, usize, usize),
    /// 穴の数 ($A$ ・$B$ ・結果) ．8 連結の背景成分から外側の 1 つを引いたもの．
    pub holes: (usize, usize, usize),
    /// 重心を揃えるために取り除いた平行移動 ([`TweenAlign::Centroid`] のときだけ)．
    pub shift: IVec2,
}

impl Tweened {
    /// オイラー標数 (成分数 − 穴の数) が両端のどちらとも違うか．
    ///
    /// 設計書 6.9 は «トポロジー変化は扱えない» と言う — **扱えないことを
    /// 黙るのではなく数えて報告する**．
    pub fn topology_changed(&self) -> bool {
        let chi = |(c, h): (usize, usize)| c as i64 - h as i64;
        let r = chi((self.components.2, self.holes.2));
        r != chi((self.components.0, self.holes.0)) && r != chi((self.components.1, self.holes.1))
    }
}

/// 形だけを補間する (設計書 6.9)．
///
/// `t` は $[0, 1]$．外挿は `px anim extrapolate` の仕事なので受け付けない．
/// 出力の画布は 2 枚の大きい方に揃える (余白は捨てる) ．
pub fn tween_mask(a: &Mask, b: &Mask, t: f32, opts: &TweenOptions) -> Result<Tweened> {
    if !(0.0..=1.0).contains(&t) || !t.is_finite() {
        return Err(CoreError::TweenBadT { t });
    }
    if a.is_empty() || b.is_empty() {
        return Err(CoreError::TweenEmptyMask);
    }
    // **画布の外へ出たら落とす．** $R \subseteq A \cup B$ は代数から出るので，
    // 出たということは符号の規約か戻し方が壊れている
    let (mask, shift, _) = blend(a, b, t, opts, Clip::Forbid)?;

    Ok(Tweened {
        components: (
            count_components(a),
            count_components(b),
            count_components(&mask),
        ),
        holes: (count_holes(a), count_holes(b), count_holes(&mask)),
        mask,
        t,
        shift,
    })
}

/// 画布からはみ出した画素をどう扱うか．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Clip {
    /// 落とす — 包含が代数から出る側 ([`tween_mask`])．
    Forbid,
    /// 数えて捨てる — 包含が成り立たない側 ([`extrapolate_mask`])．
    Count,
}

/// $(1 - t)\,d_A + t\,d_B$ の 0 等高線を取る．**$t$ の範囲は見ない**．
///
/// 返すのは (画布に載せた形，取り除いた移動，画布の外へ落ちた画素の数) ．
fn blend(
    a: &Mask,
    b: &Mask,
    t: f32,
    opts: &TweenOptions,
    clip: Clip,
) -> Result<(Mask, IVec2, usize)> {
    // **重心の差は «形の違い» ではないので，先に取り除く** (R11．モジュールの説明) ．
    // 取り除いた移動は最後に $t$ 倍して戻す — 場に解かせると痩せる
    let shift = match opts.align {
        TweenAlign::None => ivec2(0, 0),
        TweenAlign::Centroid => centroid(b) - centroid(a),
    };
    // **余白はずらし量から計算する．** 足りないと «ずらした先» が画布の外へ落ちて
    // 黙って消える (実測で 66 枚中 5 枚 ・17 画素が消えていた) ．
    // 外挿では戻す量が $\lvert t \rvert$ 倍になるので，そのぶん広げる
    let reach = t.abs().max(1.0);
    let need = ((shift.x.abs().max(shift.y.abs()) as f32) * reach).ceil() as u32;
    let (pa, mut pb, origin) = align_to_common_canvas(a, b, opts.margin.max(need));

    if shift != ivec2(0, 0) {
        pb = translate(&pb, ivec2(-shift.x, -shift.y));
    }

    let da = signed_distance(&pa);
    let db = signed_distance(&pb);

    let mut padded = Mask::new(pa.width(), pa.height());
    for p in pa.bounds().iter() {
        let (x, y) = (da.copied(p).unwrap_or(0.0), db.copied(p).unwrap_or(0.0));
        if (1.0 - t) * x + t * y > 0.0 {
            padded.set(p, true);
        }
    }
    if shift != ivec2(0, 0) {
        let back = ivec2(
            (shift.x as f32 * t).round() as i32,
            (shift.y as f32 * t).round() as i32,
        );
        padded = translate(&padded, back);
    }

    // **画布は 2 枚の大きい方．** アニメーションのフレームは寸法が揃っていなければ
    // ならないので，外挿でどれだけ伸びても画布は広げない — 代わりに**切れた画素を
    // 数える** (黙って消さない)
    let w = a.width().max(b.width());
    let h = a.height().max(b.height());
    let mut mask = Mask::new(w, h);
    let mut clipped = 0usize;
    for p in padded.iter_set() {
        let q = p - origin;
        if q.x < 0 || q.y < 0 || q.x >= w as i32 || q.y >= h as i32 {
            if clip == Clip::Forbid {
                return Err(CoreError::TweenEscapedCanvas { x: q.x, y: q.y });
            }
            clipped += 1;
            continue;
        }
        mask.set(q, true);
    }
    Ok((mask, shift, clipped))
}

/// 予備動作かオーバーシュートか (設計書 6.11．`ExtrapolateKind`)．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ExtrapolateKind {
    /// **予備動作** — 動き出す前に逆へ振る．$t = -\text{amount}$ で $A$ の手前に置く．
    Anticipation,
    /// **オーバーシュート** — 行き過ぎてから戻る．$t = 1 + \text{amount}$．
    Overshoot,
}

impl ExtrapolateKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anticipation => "anticipation",
            Self::Overshoot => "overshoot",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "anticipation" => Some(Self::Anticipation),
            "overshoot" => Some(Self::Overshoot),
            _ => None,
        }
    }

    /// 振り幅を $t$ に写す．**符号反転と超過は同じ式の両側である**．
    pub fn t_for(self, amount: f32) -> f32 {
        match self {
            Self::Anticipation => -amount,
            Self::Overshoot => 1.0 + amount,
        }
    }
}

/// 外挿した 1 枚．
#[derive(Clone, Debug)]
pub struct Extrapolated {
    pub mask: Mask,
    pub t: f32,
    pub kind: ExtrapolateKind,
    /// 4 連結の成分数 ($A$ ・$B$ ・結果) ．
    pub components: (usize, usize, usize),
    /// 穴の数 ($A$ ・$B$ ・結果) ．
    pub holes: (usize, usize, usize),
    /// 取り除いた平行移動．
    pub shift: IVec2,
    /// **画布の外へ出て切れた画素の数**．外挿では包含が成り立たないので起こりうる．
    pub clipped: usize,
}

/// 2 枚のキーフレームの外側へ形を外挿する (設計書 6.11)．
///
/// > [!warning] **包含定理は成り立たない．**
/// > $t \in (0, 1)$ の中割りは $A \cap B \subseteq R \subseteq A \cup B$ が
/// > 代数から出るが (6.9 の説明) ，**外挿では係数の片方が負になる**ので符号の
/// > 議論が丸ごと崩れる．したがって
/// >
/// > - 結果が 2 枚の外接矩形の外へ出ることがある → **切れた画素を数えて返す**
/// > - トポロジーも面積も保証しない → **成分数と穴の数を返す**
///
/// `amount` の既定は決め打ちにしない — 何画素動くかは 2 枚の変位で決まるので，
/// 呼ぶ側が測って決める．
///
/// # 平行移動なら真値があるので測れる (`px-calib extrapolate`)
///
/// $A$ を $d$ 動かした $B$ に対し，$t$ の外挿の真値は «$A$ を $t d$ 動かした絵» で
/// ある．実素材 64 枚 x ずらし 2 通りで測った (真値との IoU の中央値) ．
///
/// | 種類 | 振り幅 | 場のまま | **重心を取り除く** | 端のキーをそのまま (対照) |
/// | --- | --- | --- | --- | --- |
/// | 予備動作 | 0.25 | 0.803 | **1.000** | 0.725 |
/// | 予備動作 | 0.50 | 0.681 | **1.000** | 0.533 |
/// | 予備動作 | 1.00 | 0.475 | **1.000** | 0.271 |
/// | オーバーシュート | 0.50 | 0.684 | **1.000** | 0.533 |
/// | オーバーシュート | 1.00 | 0.483 | **1.000** | 0.271 |
///
/// **重心を取り除けば 768 件すべてで真値と画素単位一致する** (D114 と同じ結果) ．
/// 心配していた «係数が負になるので暴れる» は起きなかった — **切れた件数 0 ・
/// トポロジーが変わった件数 0**．平行移動だけの動きでは $d_A = d_B$ なので，
/// 係数が何であれ場が変わらないからである (これも代数) ．
///
/// **形が違う 2 枚では保証は無い．** その場合に暴れるかどうかは測っていない —
/// 真値の作りようが無いためで，`clipped` と成分数を返して呼ぶ側に渡す．
pub fn extrapolate_mask(
    a: &Mask,
    b: &Mask,
    kind: ExtrapolateKind,
    amount: f32,
    opts: &TweenOptions,
) -> Result<Extrapolated> {
    if !amount.is_finite() || amount < 0.0 {
        return Err(CoreError::ExtrapolateBadAmount { amount });
    }
    if a.is_empty() || b.is_empty() {
        return Err(CoreError::TweenEmptyMask);
    }
    let t = kind.t_for(amount);
    let (mask, shift, clipped) = blend(a, b, t, opts, Clip::Count)?;
    Ok(Extrapolated {
        components: (
            count_components(a),
            count_components(b),
            count_components(&mask),
        ),
        holes: (count_holes(a), count_holes(b), count_holes(&mask)),
        mask,
        t,
        kind,
        shift,
        clipped,
    })
}

/// シルエットの重心を最も近い整数画素へ丸めたもの．
///
/// **画素は動かせないので整数で持つ．** 半端なぶんは «動かない» 側に倒れるが，
/// 中割りは 1 画素の位置ではなく形を作る道具なので，そこを副画素で追うのは
/// `px anim subpixel` の仕事である．
///
/// `smear` ・`extrapolate` も同じ重心を使う — **重心の取り方を 2 か所に
/// 書かない** (D110 と同じ理由) ．
pub fn centroid(mask: &Mask) -> IVec2 {
    let n = mask.count().max(1) as f32;
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    for p in mask.iter_set() {
        sx += p.x as f32;
        sy += p.y as f32;
    }
    ivec2((sx / n).round() as i32, (sy / n).round() as i32)
}

fn translate(mask: &Mask, d: IVec2) -> Mask {
    let mut out = Mask::new(mask.width(), mask.height());
    for p in mask.iter_set() {
        out.set(p + d, true);
    }
    out
}

/// `steps` 枚の中割りを作る．$t = k / (steps + 1)$ で，**両端は含まない**．
pub fn tween_series(a: &Mask, b: &Mask, steps: u32, opts: &TweenOptions) -> Result<Vec<Tweened>> {
    if steps == 0 {
        return Err(CoreError::TweenNoSteps);
    }
    (1..=steps)
        .map(|k| tween_mask(a, b, k as f32 / (steps + 1) as f32, opts))
        .collect()
}

// 数え方は `geom::topology` に 1 つだけ置く (D110) — `px lint` のルール 22 が
// 同じ量を見るためである
use crate::geom::topology::{components as count_components, holes as count_holes};

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(w: u32, h: u32, cx: f32, cy: f32, r: f32) -> Mask {
        let mut m = Mask::new(w, h);
        for p in m.bounds().iter() {
            let (dx, dy) = (p.x as f32 - cx, p.y as f32 - cy);
            if dx * dx + dy * dy <= r * r {
                m.set(p, true);
            }
        }
        m
    }

    fn shifted(m: &Mask, d: IVec2) -> Mask {
        let mut out = Mask::new(m.width(), m.height());
        for p in m.iter_set() {
            out.set(p + d, true);
        }
        out
    }

    /// **壊れると: 中割りの列の両端が元のキーフレームと違う絵になる．**
    ///
    /// $t = 0$ と $t = 1$ は補間ではなく元の場そのものなので，**画素単位で一致
    /// しなければならない**．ここがずれると «キーフレームが動く» という一番
    /// 気付きにくい壊れ方をする．
    #[test]
    fn the_ends_reproduce_the_key_frames_exactly() {
        let a = disc(40, 40, 14.0, 20.0, 8.0);
        let b = disc(40, 40, 26.0, 20.0, 8.0);
        let opts = TweenOptions::default();
        assert_eq!(tween_mask(&a, &b, 0.0, &opts).expect("t=0").mask, a);
        assert_eq!(tween_mask(&a, &b, 1.0, &opts).expect("t=1").mask, b);
    }

    /// **壊れると: 中割りが 2 つの形のどちらにも無いところへ広がる．**
    ///
    /// $A \cap B \subseteq R \subseteq A \cup B$ は代数から出る (モジュールの説明) ．
    /// 破れているなら符号の規約か余白の切り方が壊れている．
    #[test]
    fn the_result_lies_between_the_intersection_and_the_union() {
        let a = disc(48, 48, 14.0, 24.0, 9.0);
        let b = disc(48, 48, 34.0, 24.0, 6.0);
        // **包含は `none` のときの性質である** — `centroid` は間の位置へ戻すので，
        // 和集合からはみ出しうる (それが痩せを避けるための仕掛けである)
        let opts = TweenOptions {
            align: TweenAlign::None,
            ..TweenOptions::default()
        };
        for k in 1..10 {
            let t = k as f32 / 10.0;
            let r = tween_mask(&a, &b, t, &opts).expect("中割り").mask;
            for p in r.bounds().iter() {
                if a.get(p) && b.get(p) {
                    assert!(r.get(p), "t={t} で共通部分が落ちた {p:?}");
                }
                if !a.get(p) && !b.get(p) {
                    assert!(!r.get(p), "t={t} で和集合の外に立った {p:?}");
                }
            }
        }
    }

    /// **壊れると: 寸法の違う 2 枚を黙って切り詰める** (設計書 6.9 の禁止事項) ．
    #[test]
    fn two_canvases_of_different_size_are_padded_not_truncated() {
        let a = disc(20, 20, 10.0, 10.0, 6.0);
        let b = disc(32, 24, 16.0, 12.0, 6.0);
        let out = tween_mask(&a, &b, 0.5, &TweenOptions::default()).expect("中割り");
        assert_eq!(out.mask.width(), 32);
        assert_eq!(out.mask.height(), 24);
        // 端は元の絵をそのまま (大きい画布へ載せた形で) 返す
        let back = tween_mask(&a, &b, 1.0, &TweenOptions::default()).expect("t=1");
        assert_eq!(back.mask.count(), b.count());
    }

    fn iou(x: &Mask, y: &Mask) -> f32 {
        let inter = x.iter_set().filter(|p| y.get(*p)).count();
        let union = x.count() + y.count() - inter;
        if union == 0 {
            return 1.0;
        }
        inter as f32 / union as f32
    }

    /// **壊れると: 平行移動の中割りが真値からずれる．**
    ///
    /// 真値のある場面である — 円板を 12 画素動かしたなら，中割りは 6 画素
    /// 動かした円板でなければならない．**`centroid` はこれを当てるための道具**で，
    /// `none` は当てられない (下の試験が «なぜ当てられないか» を固定する) ．
    #[test]
    fn translating_a_shape_lands_on_the_true_middle_when_the_centroid_is_taken_out() {
        let a = disc(64, 40, 18.0, 20.0, 8.0);
        let b = shifted(&a, ivec2(12, 0));
        let truth = shifted(&a, ivec2(6, 0));
        let r = tween_mask(&a, &b, 0.5, &TweenOptions::default()).expect("中割り");
        assert_eq!(r.shift, ivec2(12, 0), "取り除いた移動が違う");
        assert_eq!(r.mask, truth, "真値と 1 画素でも違う");
        assert!(iou(&r.mask, &truth) > 0.99);
    }

    /// **壊れると: «SDF 補間は平行移動で痩せる» という前提が変わったのに気付けない．**
    ///
    /// 2 つの円板の補間場の 0 等高線は**焦点を 2 つの中心に持つ楕円**であって
    /// 円板ではない (モジュールの説明) ．短半径は $\sqrt{r^2 - (D/2)^2}$ まで縮み，
    /// $D \geq 2r$ では**空になる**．R11 の中身なので数で縛る．
    #[test]
    fn the_plain_field_thins_across_the_motion_and_vanishes_at_twice_the_radius() {
        let opts = TweenOptions {
            align: TweenAlign::None,
            ..TweenOptions::default()
        };
        let a = disc(64, 40, 18.0, 20.0, 8.0);

        // D = 12 (= 1.5 r) — 楕円の面積比は 0.66 なので真値との IoU も 0.66 付近
        let b = shifted(&a, ivec2(12, 0));
        let truth = shifted(&a, ivec2(6, 0));
        let r = tween_mask(&a, &b, 0.5, &opts).expect("中割り");
        let got = iou(&r.mask, &truth);
        assert!(
            (0.55..0.75).contains(&got),
            "痩せ方が変わった (IoU {got:.3})"
        );
        // 楕円は真値の円板に含まれるので，取りこぼしはあっても «はみ出し» は無い
        assert!(r.mask.iter_set().all(|p| truth.get(p)), "楕円がはみ出した");

        // **面積は単調に痩せ，$D = 2r$ の少し先で空になる．**
        // 実測 (r = 8 の円板 197 画素) — 比は上の表の予測とよく合う
        let expected = [(0, 197), (4, 183), (8, 165), (12, 127), (16, 15), (18, 0)];
        for (d, want) in expected {
            let far = shifted(&a, ivec2(d, 0));
            let got = tween_mask(&a, &far, 0.5, &opts)
                .expect("中割り")
                .mask
                .count();
            assert_eq!(got, want, "D={d} の面積が変わった");
        }
    }

    /// **壊れると: «余白が要らない» という代数が崩れたのに気付けない．**
    ///
    /// 距離が縁で頭打ちになるのは両方の形の外側だけで，そこは $d_A$ ・$d_B$ とも
    /// 負なので 0 等高線は動かない．**縁に接する形**で確かめる (余地のある形で
    /// 測ると当たり前に一致してしまい，何も見ていないことになる) ．
    #[test]
    fn the_margin_does_not_move_the_zero_contour() {
        // 左上の隅にめり込ませて縁に接する形を作る
        let a = disc(32, 32, 0.0, 16.0, 10.0);
        let b = disc(32, 32, 8.0, 16.0, 10.0);
        let mk = |margin| TweenOptions {
            margin,
            align: TweenAlign::None,
        };
        let small = tween_mask(&a, &b, 0.5, &mk(0)).expect("余白 0");
        let big = tween_mask(&a, &b, 0.5, &mk(64)).expect("余白 64");
        assert_eq!(small.mask, big.mask, "余白で 0 等高線が動いた");
    }

    /// **壊れると: 重心を戻した先が画布からはみ出して黙って消える．**
    ///
    /// 実測で 66 枚中 5 枚 ・17 画素がこれで消えていた．余白は掃いて決める量では
    /// なく**ずらし量そのもの**なので，指定が 0 でも足りるだけ取る．
    #[test]
    fn the_centroid_shift_gets_the_room_it_needs_even_with_no_margin() {
        let a = disc(48, 24, 8.0, 12.0, 6.0);
        let b = disc(48, 24, 40.0, 12.0, 6.0);
        let mk = |margin| TweenOptions {
            margin,
            align: TweenAlign::Centroid,
        };
        let small = tween_mask(&a, &b, 0.5, &mk(0)).expect("余白 0");
        let big = tween_mask(&a, &b, 0.5, &mk(64)).expect("余白 64");
        assert_eq!(small.mask, big.mask, "余白 0 で画素が消えた");
        assert_eq!(small.mask.count(), a.count(), "形が痩せた");
    }

    /// **壊れると: 受け付けてはいけない `t` を黙って通す．**
    /// 外挿は `px anim extrapolate` の仕事である．
    #[test]
    fn t_outside_the_unit_interval_is_an_error() {
        let a = disc(20, 20, 10.0, 10.0, 5.0);
        let b = disc(20, 20, 12.0, 10.0, 5.0);
        let opts = TweenOptions::default();
        assert!(tween_mask(&a, &b, -0.1, &opts).is_err());
        assert!(tween_mask(&a, &b, 1.1, &opts).is_err());
        assert!(tween_mask(&a, &b, f32::NAN, &opts).is_err());
        assert!(tween_series(&a, &b, 0, &opts).is_err());
    }

    /// **壊れると: 穴の数え方が連結性の取り違えで狂う．**
    ///
    /// 前景 4 連結 ・背景 8 連結で数える．外側の背景は穴ではない．
    #[test]
    fn a_ring_has_one_hole_and_a_disc_has_none() {
        let outer = disc(40, 40, 20.0, 20.0, 12.0);
        let inner = disc(40, 40, 20.0, 20.0, 5.0);
        let mut ring = outer.clone();
        for p in inner.iter_set() {
            ring.set(p, false);
        }
        assert_eq!(count_holes(&outer), 0);
        assert_eq!(count_holes(&ring), 1);
        assert_eq!(count_components(&ring), 1);
    }

    /// **壊れると: 中割りの列が両端を含んでしまい，同じ絵が 2 枚並ぶ．**
    #[test]
    fn the_series_leaves_out_both_ends() {
        let a = disc(40, 40, 14.0, 20.0, 7.0);
        let b = disc(40, 40, 26.0, 20.0, 7.0);
        let out = tween_series(&a, &b, 3, &TweenOptions::default()).expect("3 枚");
        let ts: Vec<f32> = out.iter().map(|x| x.t).collect();
        assert_eq!(ts, vec![0.25, 0.5, 0.75]);
        assert!(out.iter().all(|x| x.mask != a && x.mask != b));
    }
}
