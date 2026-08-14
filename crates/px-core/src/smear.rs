//! おばけ (設計書 6.11 の smear．D43)．
//!
//! 高速移動で 2 枚のキーフレームが離れているとき，**その間を繋ぐ伸びた形**を作る．
//! 1 コマだけ表示して «速すぎて残像に見える» を作る技法である．
//!
//! # 設計書の擬似コードは符号が逆である
//!
//! 6.11 の擬似コードは $d \gets \min_k \big((1-k/n) d_A + (k/n) d_B\big)$ と書き，
//! これを «下側包絡» と呼ぶ．**これは外側が正の距離場を前提にした式である．**
//! こちらの [`signed_distance`](crate::geom::signed_distance) は
//! **内側が正**なので (G2 の規約) ，そのまま書くと
//! **繋ぐどころか共通部分だけが残る**．
//!
//! | 取るもの | 外側正の規約 | **こちらの規約 (内側正)** |
//! | --- | --- | --- |
//! | 掃引 (和) | $\min_k$ | **$\max_k$** |
//! | 共通部分 | $\max_k$ | $\min_k$ |
//!
//! **これは代数であって測定ではない．** 符号を取り違えても «形は出る» ので
//! 気付きにくい (D56 が AA の明暗について言っているのと同じ穴) ．
//! 掃引が両端を含むことを試験で縛ってある．
//!
//! # 掃引は中割りの和集合そのものである
//!
//! $\max_k f_k > 0 \iff \exists k,\ f_k > 0$ なので，**しきい値を取ってから
//! 和を取る**のと**場の包絡を取ってからしきい値を取る**のは同じ集合になる．
//! したがってここは [`tween_mask`] を $k/n$ で呼んで和を取るだけでよい —
//! **形の作り方を 2 か所に書かない** (D110 と同じ理由) ．
//!
//! # 設計書の掃引は，設計書自身が «採らない» と言う union とまったく同じ集合である
//!
//! 6.11 は «union は 2 形状が離れていると 2 つの塊が残るだけで繋がらない» と言い，
//! だから掃引を採ると書いている．**測ったら掃引も 1 件残らず同じ結果だった．**
//!
//! | ずらし | 件数 | union が繋がる | **掃引 (設計書のまま)** | **重心を取り除いた掃引** |
//! | --- | --- | --- | --- | --- |
//! | 4 | 64 | 61 | **61** | **64** |
//! | 8 | 64 | 61 | **61** | **64** |
//! | 16 | 64 | 57 | **57** | **64** |
//! | 24 | 64 | 25 | **25** | **64** |
//! | 32 | 64 | 12 | **12** | **64** |
//!
//! **これは偶然ではなく代数である．** 6.9 の包含定理より
//! $R_t \subseteq A \cup B$ が $t \in [0,1]$ のすべてで成り立ち，掃引は
//! $t = 0$ と $t = 1$ を含むので $A \cup B \subseteq \bigcup_t R_t$ である．
//! 両側から挟まれるので
//!
//! $$ \bigcup_{t \in [0,1]} R_t = A \cup B $$
//!
//! — **掃引そのままでは，設計書が否定した union 以上のものは絶対に作れない．**
//! R11 (中割りが平行移動で痩せる) とまったく同じ原因で，直し方も同じである．
//! **重心を先に取り除くと，掃引は «同じ形を経路に沿って掃いたもの» になり，
//! ずらし 32 画素でも 64 / 64 件が繋がる．**
//!
//! 設計書が正しかったのは**刻み幅**の方である．
//!
//! | 標本の数 (ずらし 32) | 切れた枚数 |
//! | --- | --- |
//! | 1 (両端だけ = union) | 52 / 64 |
//! | 2 | 7 / 64 |
//! | 4 ・8 ・16 | 3 / 64 |
//! | **変位から決める (= 32．1 画素に 1 標本)** | **0 / 64** |
//!
//! 標本 16 (1 歩 2 画素) でも 3 枚が切れる — «$\Delta t \lVert 変位 \rVert
//! \lesssim 1$» はちょうどの条件であって，余裕のある目安ではない．
//! 測る口は `px-calib smear`．

use crate::error::{CoreError, Result};
use crate::geom::{Mask, label_mask};
use crate::math::IVec2;
use crate::tween::{TweenAlign, TweenOptions, centroid, tween_mask};

/// 伸びた形の作り方．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SmearMethod {
    /// **設計書 6.11 の掃引**．補間した距離場の包絡を取る (こちらの規約では $\max$)．
    #[default]
    Sweep,
    /// 2 形状の和集合だけ．**設計書が «採らない» と言う方**で，測る口として残す．
    Union,
}

impl SmearMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sweep => "sweep",
            Self::Union => "union",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "sweep" => Some(Self::Sweep),
            "union" => Some(Self::Union),
            _ => None,
        }
    }
}

/// おばけの設定．
#[derive(Copy, Clone, Debug, Default)]
pub struct SmearOptions {
    pub method: SmearMethod,
    /// 掃引する前に重心を合わせるか．**既定は中割りと同じ `centroid`** (D114)．
    pub align: TweenAlign,
    /// 標本の数．`None` なら**重心変位から決める** (設計書 6.11: 1 画素ごとに 1 標本)．
    ///
    /// **書き写す既定値ではない** — 変位で決まる量なので，指定しないのが普通である．
    pub samples: Option<u32>,
}

/// 伸びた形 1 枚と，その素性．
#[derive(Clone, Debug)]
pub struct Smear {
    pub mask: Mask,
    /// 実際に取った標本の数 ($k = 0 \ldots n$ なので枚数は $n + 1$)．
    pub samples: u32,
    /// 重心の変位 (画素)．標本の数はここから決まる．
    pub displacement: f32,
    /// 4 連結の成分数 ($A$ ・$B$ ・結果)．
    pub components: (usize, usize, usize),
}

impl Smear {
    /// 伸びた形が 1 つに繋がっているか．
    ///
    /// **おばけの存在理由がこれである** — 繋がらないなら 2 枚を並べたのと同じで，
    /// 設計書が «union は採らない» と言う理由がそのまま自分に返ってくる．
    pub fn connects(&self) -> bool {
        self.components.2 == 1
    }
}

/// 伸びた形を作る (設計書 6.11)．
///
/// 両端を必ず含む — 掃引は $k = 0$ と $k = n$ を含むので $A \cup B \subseteq R$ が
/// 代数から出る．**破れていたら落とす**のではなく，`covers_ends` を試験で縛ってある．
pub fn smear_mask(a: &Mask, b: &Mask, opts: &SmearOptions) -> Result<Smear> {
    if a.is_empty() || b.is_empty() {
        return Err(CoreError::TweenEmptyMask);
    }
    let delta = centroid(b) - centroid(a);
    let displacement = ((delta.x * delta.x + delta.y * delta.y) as f32).sqrt();

    // **1 画素ごとに 1 標本** (設計書 6.11) — 刻みが粗いと掃引の途中が抜ける．
    // 変位が 0 でも両端は要るので下限は 1
    let samples = match opts.samples {
        Some(0) => return Err(CoreError::SmearNoSamples),
        Some(n) => n,
        None => (displacement.ceil() as u32).max(1),
    };

    let w = a.width().max(b.width());
    let h = a.height().max(b.height());
    let mut mask = Mask::new(w, h);

    match opts.method {
        SmearMethod::Union => {
            for p in a.iter_set() {
                mask.set(p, true);
            }
            for p in b.iter_set() {
                mask.set(p, true);
            }
        }
        SmearMethod::Sweep => {
            let topts = TweenOptions {
                margin: 0,
                align: opts.align,
            };
            for k in 0..=samples {
                let t = k as f32 / samples as f32;
                for p in tween_mask(a, b, t, &topts)?.mask.iter_set() {
                    mask.set(p, true);
                }
            }
        }
    }

    Ok(Smear {
        components: (
            label_mask(a, false).len(),
            label_mask(b, false).len(),
            label_mask(&mask, false).len(),
        ),
        mask,
        samples,
        displacement,
    })
}

/// $A \cup B$ が結果に含まれているか (掃引の代数の確認)．
pub fn covers_ends(a: &Mask, b: &Mask, out: &Mask) -> bool {
    a.iter_set().chain(b.iter_set()).all(|p| out.get(p))
}

/// 重心の変位 (画素)．
pub fn displacement_of(a: &Mask, b: &Mask) -> f32 {
    let d: IVec2 = centroid(b) - centroid(a);
    ((d.x * d.x + d.y * d.y) as f32).sqrt()
}

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

    /// **壊れると: «繋ぐ» はずの掃引が «共通部分だけ» になる．**
    ///
    /// 設計書の擬似コードは外側正の距離場を前提に $\min$ と書いている．
    /// こちらは内側正なので $\max$ でなければならない — **符号を取り違えても
    /// 形は出る**ので，両端を含むことで縛る．
    #[test]
    fn the_sweep_contains_both_ends_because_it_takes_the_upper_envelope() {
        let a = disc(48, 24, 12.0, 12.0, 6.0);
        let b = disc(48, 24, 34.0, 12.0, 6.0);
        let out = smear_mask(&a, &b, &SmearOptions::default()).expect("掃引");
        assert!(
            covers_ends(&a, &b, &out.mask),
            "両端を含まない — 符号が逆になっている"
        );
        // 共通部分は空なので，min を取っていたら結果も空になる
        assert!(out.mask.count() > a.count() + b.count());
    }

    /// **壊れると: 速い動きでちょうどおばけが要る場面で繋がらなくなる．**
    ///
    /// 設計書は «union は 2 塊が残る» と言うが，**掃引をそのまま (重心を残したまま)
    /// 当てても同じことが起きる** — $D \geq 2r$ で中間の 0 等高線が消えるからで，
    /// R11 と同じ代数である．重心を先に取り除くと繋がる．
    #[test]
    fn a_fast_move_only_connects_once_the_centroid_is_taken_out() {
        // 半径 6 の円板を 22 画素動かす — $D / r = 3.7$ で，中割りは空になる領域
        let a = disc(56, 24, 12.0, 12.0, 6.0);
        let b = disc(56, 24, 34.0, 12.0, 6.0);

        let union = smear_mask(
            &a,
            &b,
            &SmearOptions {
                method: SmearMethod::Union,
                ..Default::default()
            },
        )
        .expect("和集合");
        assert_eq!(union.components.2, 2, "和集合は 2 塊のまま");

        let plain = smear_mask(
            &a,
            &b,
            &SmearOptions {
                align: TweenAlign::None,
                ..Default::default()
            },
        )
        .expect("掃引");
        assert_eq!(
            plain.components.2, 2,
            "場のままの掃引も 2 塊 — 中間の 0 等高線が消えるので union と変わらない"
        );

        let aligned = smear_mask(&a, &b, &SmearOptions::default()).expect("掃引");
        assert!(aligned.connects(), "重心を取り除けば繋がる");
    }

    /// **壊れると: «掃引は union より良い» という前提で重心を取り除かなくなる．**
    ///
    /// 6.9 の包含定理から $\bigcup_t R_t = A \cup B$ が出るので，
    /// **場をそのまま掃引しても union と 1 画素も違わない**．代数なので
    /// 実素材ではなく «必ずそうなる» ことを縛る．
    #[test]
    fn the_plain_sweep_is_exactly_the_union_it_was_supposed_to_beat() {
        for (cx, r) in [(20.0f32, 6.0f32), (30.0, 9.0), (26.0, 4.0)] {
            let a = disc(64, 32, 12.0, 16.0, r);
            let b = disc(64, 32, 12.0 + cx, 16.0, r);
            let plain = smear_mask(
                &a,
                &b,
                &SmearOptions {
                    align: TweenAlign::None,
                    ..Default::default()
                },
            )
            .expect("掃引");
            let union = smear_mask(
                &a,
                &b,
                &SmearOptions {
                    method: SmearMethod::Union,
                    ..Default::default()
                },
            )
            .expect("和集合");
            let differing = plain
                .mask
                .bounds()
                .iter()
                .filter(|p| plain.mask.get(*p) != union.mask.get(*p))
                .count();
            assert_eq!(
                differing, 0,
                "ずらし {cx} ・半径 {r} で {differing} 画素違う"
            );
        }
    }

    /// **壊れると: 刻みが粗いまま掃引して «数珠状» の形を出す．**
    ///
    /// 設計書 6.11 は $\Delta t \cdot \lVert \text{変位} \rVert \lesssim 1$ を
    /// 求める．標本を減らすと本当に切れることを固定する．
    #[test]
    fn too_few_samples_break_the_sweep_into_beads() {
        let a = disc(64, 24, 10.0, 12.0, 4.0);
        let b = disc(64, 24, 52.0, 12.0, 4.0);
        // 変位 42 画素．標本 4 では 1 歩 10.5 画素で，直径 8 の円板は届かない
        let coarse = smear_mask(
            &a,
            &b,
            &SmearOptions {
                samples: Some(4),
                ..Default::default()
            },
        )
        .expect("掃引");
        assert!(coarse.components.2 > 1, "粗い刻みでは切れるはず");

        let auto = smear_mask(&a, &b, &SmearOptions::default()).expect("掃引");
        assert_eq!(auto.samples, 42, "標本の数は変位から決まる");
        assert!(auto.connects());
    }

    /// **壊れると: 標本 0 で «両端だけ» を静かに返す．**
    #[test]
    fn zero_samples_is_an_error_rather_than_a_silent_pair() {
        let a = disc(24, 24, 8.0, 12.0, 4.0);
        assert!(matches!(
            smear_mask(
                &a,
                &a,
                &SmearOptions {
                    samples: Some(0),
                    ..Default::default()
                }
            ),
            Err(CoreError::SmearNoSamples)
        ));
    }

    /// **壊れると: 動いていない 2 枚で標本 0 になり掃引が回らない．**
    #[test]
    fn a_still_pair_still_takes_one_step() {
        let a = disc(24, 24, 12.0, 12.0, 5.0);
        let out = smear_mask(&a, &a, &SmearOptions::default()).expect("掃引");
        assert_eq!(out.samples, 1);
        assert_eq!(out.displacement, 0.0);
        assert_eq!(out.mask.count(), a.count(), "動いていないなら形も動かない");
    }
}
