//! 潰しと伸ばし (設計書 6.11．D41)．
//!
//! **制約は体積保存** — 縦に潰したぶんだけ横に広げ，$h \times w$ を保つ．
//! lint ルール 27 «体積不保存» が見るのもこの量である．
//!
//! # 実数では保存できるが，画素では保存できない
//!
//! $h' = h / k$ ・$w' = w k$ なら $h' w' = h w$ がちょうど成り立つ．
//! **しかし画素は整数なので，どちらも丸めた時点で積はずれる．**
//! ずれを «無い» ことにしないために，2 通り用意して測ってある．
//!
//! | 決め方 | 高さ | 幅 |
//! | --- | --- | --- |
//! | [`VolumeRule::Independent`] | $\mathrm{round}(h / k)$ | $\mathrm{round}(w k)$ |
//! | [`VolumeRule::Derived`] | $\mathrm{round}(h / k)$ | $\mathrm{round}(h w / h')$ |
//!
//! `Derived` は**幅を体積の式から引く** — 頼んだ倍率ではなく保存の方を優先する．
//! 実素材 35 枚 x 4 段階を測ると，**`Derived` の方が常に良い**．
//!
//! | 決め方 | 件数 | 矩形の面積の誤差 (中央) | 最悪 |
//! | --- | --- | --- | --- |
//! | `Independent` | 140 | 1.562% | 6.7% |
//! | **`Derived` (採用)** | 140 | **1.042%** | **5.0%** |
//!
//! **どちらも 0 にはならない．** 高さを 1 画素単位でしか動かせない以上，
//! 丸めの残りは原理的に消せない — «体積保存» は制約であって，**満たしきれる
//! 保証ではない**．ルール 27 が advisory なのはこのためだと読める．
//!
//! # 体積は 2 つあるが，測ったら同じように動いた
//!
//! 設計書もルール 27 も «$h \times w$» と言うので**外接矩形の面積**が体積である．
//! しかし絵として動くのは**不透明な画素の数**の方なので，両方返す．
//!
//! 最初に測ったときは画素数の誤差が **19.0%** と出て «2 つの体積は別物» に
//! 見えたが，**それは拡縮のせいではなく画布で切れていたせい**だった
//! (D104 ««測れない» の理由も分ける» と同じ取り違え) ．分けて測ると
//!
//! | | 画素数の誤差 (中央) |
//! | --- | --- |
//! | 画布そのまま (140 通り中 **136 通りで切れる**) | 19.0% |
//! | **画布を広げる** | **1.6%** |
//!
//! ——**拡縮そのものは 1.6% しか動かさない**．[`SquashReport::resample_error`]
//! が «切れたぶんを除いた» 方で，`pixel_error` が «切れたぶんも含む» 方である．
//!
//! # 色は作らない
//!
//! 拡縮は**最近傍のみ**である．線形補間を入れるとパレットに無い色が生まれ，
//! D94 «並べ替えるだけの道具は色を作らない» が破れる．潰しも同じ性質の道具なので，
//! 使った添字の集合が増えないことを試験で縛ってある．

use std::collections::BTreeSet;

use crate::canvas::IndexedCanvas;
use crate::error::{CoreError, Result};
use crate::math::{IRect, IVec2, ivec2};

/// 潰した形をどこで固定するか．
///
/// **既定は `Bottom`** — 潰れる物は地面に着いたまま潰れる．中心で固定すると
/// 潰した瞬間に足が浮く．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SquashAnchor {
    Top,
    Center,
    #[default]
    Bottom,
}

impl SquashAnchor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Center => "center",
            Self::Bottom => "bottom",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "top" => Some(Self::Top),
            "center" => Some(Self::Center),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }
}

/// もう一方の辺をどう決めるか．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum VolumeRule {
    /// 倍率をそのまま両辺に当てて，別々に丸める．
    Independent,
    /// **幅を $h w / h'$ から引く** — 頼んだ倍率より体積保存を優先する．
    #[default]
    Derived,
}

impl VolumeRule {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Independent => "independent",
            Self::Derived => "derived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "independent" => Some(Self::Independent),
            "derived" => Some(Self::Derived),
            _ => None,
        }
    }
}

/// 潰しの設定．
#[derive(Copy, Clone, Debug)]
pub struct SquashOptions {
    pub anchor: SquashAnchor,
    pub rule: VolumeRule,
    /// **収まらないぶんだけ画布を広げるか．既定は広げる．**
    ///
    /// 実素材 35 枚を掃くと，広げない側は**140 通りのうち 136 通りで切れる**
    /// (中央 34 画素 ・最悪は画素の 3 割) ．体積を保つということは片方の辺が
    /// 必ず伸びるということなので，**元の画布に収まる方が例外である**．
    ///
    /// D93 (`compose` は画布を広げる) と同じ答えになるが，**理由は違う** —
    /// あちらは «縁に余白のある絵が 38 枚中 5 枚しかない» という測定で，
    /// こちらは «体積保存が片方の辺を必ず伸ばす» という代数である．
    pub grow: bool,
}

impl Default for SquashOptions {
    fn default() -> Self {
        Self {
            anchor: SquashAnchor::default(),
            rule: VolumeRule::default(),
            grow: true,
        }
    }
}

/// 潰した結果の素性．
#[derive(Clone, Debug)]
pub struct SquashReport {
    /// 元の外接矩形 (幅, 高さ)．
    pub from: (u32, u32),
    /// 潰した後の外接矩形 (幅, 高さ)．
    pub to: (u32, u32),
    /// 外接矩形の面積 (前, 後)．**設計書とルール 27 が «体積» と呼ぶ量**．
    pub bbox_volume: (u32, u32),
    /// 不透明な画素の数 (前, 後)．**絵として動く量**．
    pub pixels: (usize, usize),
    /// 置く前 (拡縮した直後) の不透明な画素の数．
    ///
    /// **«拡縮で動いたぶん» と «画布で切れたぶん» を分けるため**にある
    /// (D104 ««測れない» の理由も分ける» と同じ) ．
    pub scaled_pixels: usize,
    /// 画布の寸法 (前, 後)．広げたなら後の方が大きい．
    pub canvas_size: ((u32, u32), (u32, u32)),
    /// 画布を広げたときに原点がずれた量．**他のフレームも同じだけずらす必要がある**．
    pub origin_shift: IVec2,
    /// 画布に収まらず切れた画素の数．
    pub clipped: usize,
    /// 使った添字の数 (前, 後)．**増えていたら色を作っている**．
    pub colors: (usize, usize),
}

impl SquashReport {
    /// 外接矩形の面積の相対誤差．**丸めのぶんは必ず残る**ので 0 にはならない．
    pub fn volume_error(&self) -> f32 {
        let (before, after) = self.bbox_volume;
        if before == 0 {
            return 0.0;
        }
        (after as f32 - before as f32).abs() / before as f32
    }

    /// 不透明な画素の数の相対誤差．
    pub fn pixel_error(&self) -> f32 {
        let (before, after) = self.pixels;
        if before == 0 {
            return 0.0;
        }
        (after as f32 - before as f32).abs() / before as f32
    }

    /// **拡縮そのものが動かした量** (画布で切れたぶんを除く)．
    pub fn resample_error(&self) -> f32 {
        let before = self.pixels.0;
        if before == 0 {
            return 0.0;
        }
        (self.scaled_pixels as f32 - before as f32).abs() / before as f32
    }
}

/// 体積を保ったまま潰す / 伸ばす (設計書 6.11)．
///
/// `amount` は**縦の倍率から 1 を引いたもの** — $+0.25$ で高さ 1.25 倍 (伸ばし) ，
/// $-0.25$ で 0.75 倍 (潰し) ．横はその逆数側に動く．
pub fn squash(
    canvas: &IndexedCanvas,
    amount: f32,
    opts: &SquashOptions,
) -> Result<(IndexedCanvas, SquashReport)> {
    let k = 1.0 + amount;
    if !k.is_finite() || k <= 0.0 {
        return Err(CoreError::SquashBadAmount { amount });
    }
    // **透明の宣言が無い絵は «全部が絵» である．**
    //
    // [`IndexedCanvas::opaque_bbox`] は透明添字が無いと `None` を返すので，
    // そのまま «空» と読むと**背景タイルのように隙間の無い絵をすべて拒む**．
    // 実素材 66 枚のうち何枚かがこれで，端から端まで通す試験が捕まえた．
    let bbox = match canvas.opaque_bbox() {
        Some(b) => b,
        None if canvas.transparent().is_none() && canvas.width() > 0 && canvas.height() > 0 => {
            IRect::new(0, 0, canvas.width(), canvas.height())
        }
        None => return Err(CoreError::SquashEmptyCanvas),
    };
    let (w, h) = (bbox.w, bbox.h);

    // **高さを先に決め，幅は規則で決める．** どちらを先に決めるかで結果が変わるので，
    // «縦に潰す» 側を主にする (潰しは縦の動きだから)
    let nh = ((h as f32 * k).round() as i64).max(1) as u32;
    let nw = match opts.rule {
        VolumeRule::Independent => (((w as f32 / k).round() as i64).max(1)) as u32,
        VolumeRule::Derived => {
            let volume = (w as u64) * (h as u64);
            ((volume as f32 / nh as f32).round() as i64).max(1) as u32
        }
    };

    // 最近傍で拡縮する — **色は作らない** (D94)
    let transparent = canvas.transparent();
    let fill = transparent.unwrap_or(0);
    let mut scaled = IndexedCanvas::filled(nw, nh, fill).with_transparent(transparent);
    for y in 0..nh as i32 {
        let sy = bbox.y + ((y as f32 + 0.5) * h as f32 / nh as f32).floor() as i32;
        for x in 0..nw as i32 {
            let sx = bbox.x + ((x as f32 + 0.5) * w as f32 / nw as f32).floor() as i32;
            if let Some(index) =
                canvas.get(sx.min(bbox.x + w as i32 - 1), sy.min(bbox.y + h as i32 - 1))
            {
                scaled.set(x, y, index);
            }
        }
    }

    // 置き直す．**横は中心，縦は錨で合わせる**
    let mut at = anchor_origin(bbox, nw, nh, opts.anchor);
    // **収まらないぶんだけ画布を広げる．** 体積を保つなら片方の辺は必ず伸びるので，
    // 元の画布に収まる方が例外である
    let (cw, ch) = (canvas.width() as i32, canvas.height() as i32);
    let (mut ow, mut oh) = (canvas.width(), canvas.height());
    let mut origin_shift = ivec2(0, 0);
    if opts.grow {
        let (x0, y0) = (at.x.min(0), at.y.min(0));
        let (x1, y1) = ((at.x + nw as i32).max(cw), (at.y + nh as i32).max(ch));
        ow = (x1 - x0) as u32;
        oh = (y1 - y0) as u32;
        origin_shift = ivec2(-x0, -y0);
        at = at + origin_shift;
    }

    let mut out = IndexedCanvas::filled(ow, oh, fill).with_transparent(transparent);
    let mut clipped = 0usize;
    for y in 0..nh as i32 {
        for x in 0..nw as i32 {
            let p = ivec2(x, y);
            if scaled.is_transparent_at(p) {
                continue;
            }
            let q = at + p;
            let index = scaled.get_at(p).unwrap_or(fill);
            if !out.set_at(q, index) {
                clipped += 1;
            }
        }
    }

    let report = SquashReport {
        from: (w, h),
        to: (nw, nh),
        bbox_volume: (w * h, nw * nh),
        pixels: (opaque_count(canvas), opaque_count(&out)),
        scaled_pixels: opaque_count(&scaled),
        canvas_size: ((canvas.width(), canvas.height()), (ow, oh)),
        origin_shift,
        clipped,
        colors: (used_indices(canvas).len(), used_indices(&out).len()),
    };
    Ok((out, report))
}

/// 錨に従って置く位置を決める．
fn anchor_origin(bbox: IRect, nw: u32, nh: u32, anchor: SquashAnchor) -> IVec2 {
    let x = bbox.x + (bbox.w as i32 - nw as i32) / 2;
    let y = match anchor {
        SquashAnchor::Top => bbox.y,
        SquashAnchor::Center => bbox.y + (bbox.h as i32 - nh as i32) / 2,
        SquashAnchor::Bottom => bbox.y + bbox.h as i32 - nh as i32,
    };
    ivec2(x, y)
}

fn opaque_count(canvas: &IndexedCanvas) -> usize {
    canvas
        .pixels()
        .iter()
        .filter(|&&i| canvas.transparent() != Some(i))
        .count()
}

fn used_indices(canvas: &IndexedCanvas) -> BTreeSet<u8> {
    canvas
        .pixels()
        .iter()
        .copied()
        .filter(|&i| canvas.transparent() != Some(i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 幅 `w` ・高さ `h` のベタ塗りを 32x32 の画布の中央に置く．
    fn block(w: u32, h: u32) -> IndexedCanvas {
        let mut c = IndexedCanvas::filled(32, 32, 0).with_transparent(Some(0));
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                c.set(4 + x, 4 + y, 1);
            }
        }
        c
    }

    /// **壊れると: 潰しが体積を変え，ルール 27 が自分の道具に鳴る．**
    ///
    /// 実数では保存できるが画素では丸めが残る — **`Derived` は丸めのぶんまでで
    /// 収まる**ことを縛る．
    #[test]
    fn the_derived_rule_keeps_the_bbox_volume_within_the_rounding() {
        for (w, h) in [(16u32, 16u32), (12, 20), (7, 13), (24, 9)] {
            for amount in [-0.5f32, -0.25, 0.25, 0.5] {
                let (_, r) = squash(&block(w, h), amount, &SquashOptions::default()).expect("潰し");
                let (before, after) = r.bbox_volume;
                // 高さは 1 画素単位でしか動かせないので，誤差の上限は «幅 1 本ぶん»
                let slack = r.to.0.max(r.to.1);
                assert!(
                    after.abs_diff(before) <= slack,
                    "{w}x{h} amount={amount}: {before} -> {after} (許容 {slack})"
                );
            }
        }
    }

    /// **壊れると: 拡縮が中間色を作り，パレット逸脱が出る (D94)．**
    #[test]
    fn scaling_never_invents_an_index() {
        let mut c = IndexedCanvas::filled(32, 32, 0).with_transparent(Some(0));
        for y in 0..20i32 {
            for x in 0..14i32 {
                c.set(6 + x, 6 + y, 1 + ((x + y) % 3) as u8);
            }
        }
        let before = used_indices(&c);
        for amount in [-0.4f32, -0.2, 0.2, 0.4] {
            let (out, r) = squash(&c, amount, &SquashOptions::default()).expect("潰し");
            let after = used_indices(&out);
            assert!(
                after.is_subset(&before),
                "amount={amount}: 添字が増えた {after:?} ⊄ {before:?}"
            );
            assert!(r.colors.1 <= r.colors.0);
        }
    }

    /// **壊れると: 潰した物の足が浮く．**
    #[test]
    fn the_bottom_anchor_keeps_the_feet_on_the_ground() {
        let c = block(16, 16);
        let floor = c.opaque_bbox().expect("矩形");
        let bottom = floor.y + floor.h as i32 - 1;
        let (out, _) = squash(&c, -0.5, &SquashOptions::default()).expect("潰し");
        let after = out.opaque_bbox().expect("矩形");
        assert_eq!(after.y + after.h as i32 - 1, bottom, "底が動いた");

        let (out, _) = squash(
            &c,
            -0.5,
            &SquashOptions {
                anchor: SquashAnchor::Top,
                ..Default::default()
            },
        )
        .expect("潰し");
        assert_eq!(out.opaque_bbox().expect("矩形").y, floor.y);
    }

    /// **壊れると: 画布で切れたぶんを «拡縮が動かした» と読み違える (D104)．**
    ///
    /// 最初に測ったときは画素数の誤差が 19% と出たが，**19% のほとんどは画布で
    /// 切れた画素**だった．2 つを分けて持つことを縛る．
    #[test]
    fn clipping_and_resampling_are_counted_separately() {
        // 画布いっぱいの円板を潰すと，広げなければ必ず切れる
        let mut c = IndexedCanvas::filled(32, 32, 0).with_transparent(Some(0));
        for y in 0..32i32 {
            for x in 0..32i32 {
                let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
                if dx * dx + dy * dy <= 15.5 * 15.5 {
                    c.set(x, y, 1);
                }
            }
        }
        let tight = SquashOptions {
            grow: false,
            ..Default::default()
        };
        let (_, cut) = squash(&c, -0.4, &tight).expect("潰し");
        assert!(cut.clipped > 0, "切れていない — 試験が意味を失っている");
        assert!(
            cut.pixel_error() > cut.resample_error(),
            "切れたぶんが «拡縮の誤差» に混ざっている ({:?})",
            (cut.pixel_error(), cut.resample_error())
        );

        let (_, grown) = squash(&c, -0.4, &SquashOptions::default()).expect("潰し");
        assert_eq!(grown.clipped, 0, "広げても切れた");
        assert!(
            grown.canvas_size.1.0 > grown.canvas_size.0.0,
            "広がっていない"
        );
        // 広げれば «拡縮が動かした量» がそのまま画素数の誤差になる
        assert!((grown.pixel_error() - grown.resample_error()).abs() < 1e-6);
    }

    /// **壊れると: 2 つの «体積» を取り違えて «保存できている» と報告する．**
    ///
    /// 外接矩形の面積は保存しても，**不透明な画素の数は最近傍のぶんだけ動く**
    /// (実素材で中央 1.6%) ．両方返すのはこのためである．
    #[test]
    fn the_two_volumes_do_not_move_together() {
        // 円板は外接矩形を埋めないので，2 つの量の動きがはっきり分かれる
        let mut c = IndexedCanvas::filled(48, 48, 0).with_transparent(Some(0));
        for y in 0..48i32 {
            for x in 0..48i32 {
                let (dx, dy) = (x as f32 - 23.5, y as f32 - 23.5);
                if dx * dx + dy * dy <= 15.0 * 15.0 {
                    c.set(x, y, 1);
                }
            }
        }
        let (_, r) = squash(&c, -0.4, &SquashOptions::default()).expect("潰し");
        assert!(r.volume_error() < 0.02, "矩形の面積 {:?}", r.bbox_volume);
        assert!(
            r.pixel_error() > 0.0,
            "画素の数まで一致するなら試験が意味を失っている {:?}",
            r.pixels
        );
    }

    /// **壊れると: 空の画布を潰して «成功» を返す．**
    #[test]
    fn an_empty_canvas_is_an_error() {
        let c = IndexedCanvas::filled(8, 8, 0).with_transparent(Some(0));
        assert!(matches!(
            squash(&c, 0.5, &SquashOptions::default()),
            Err(CoreError::SquashEmptyCanvas)
        ));
        assert!(matches!(
            squash(&block(4, 4), -1.0, &SquashOptions::default()),
            Err(CoreError::SquashBadAmount { .. })
        ));
    }
}
