//! パレットとランプ (設計書 3.2)．
//!
//! 不変条件は 3 つある．
//!
//! 1. アルファは 2 値のみ (D4)．
//! 2. `lab` は `entries` から導出されるキャッシュであり，個別に書き換える API を公開しない．
//! 3. 明度順への正規化は**作業層でのみ**行い，`order_inv` を保持して
//!    `merge_back` で添字を戻す (D50)．保持層は並べ替えない．
//!
//! 3 が重要である．保持層で並べ替えると全画素の添字が変わり，`.aseprite` の
//! バイト一致往復が成立しない．

use crate::canvas::IndexedCanvas;
use crate::color::{Oklab, Rgba8, oklab_of};
use crate::error::{CoreError, Result};

/// 彩度カーブのプリセット (D48)．
///
/// 明度に対して彩度を単調にしないのが既定である．中間で最大になる形が自然に見える．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ChromaCurve {
    /// 全て同じ彩度．
    Uniform,
    /// 中央で最大 (既定)．
    #[default]
    PeakMiddle,
    /// 影の彩度を上げ明色を下げる．
    ShadowHeavy,
    /// 影の彩度を下げ明色を上げる．
    LightHeavy,
    /// 指定した段だけ大きく上げる．
    SingleAccent(u8),
}

/// 色傾斜 (ランプ)．`entries` はパレット添字の列で，明るい順ではなく段の順に並ぶ．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ramp {
    entries: Vec<u8>,
    chroma_curve: ChromaCurve,
}

impl Ramp {
    pub fn new(entries: Vec<u8>, chroma_curve: ChromaCurve) -> Self {
        Self {
            entries,
            chroma_curve,
        }
    }

    pub fn entries(&self) -> &[u8] {
        &self.entries
    }

    pub fn chroma_curve(&self) -> ChromaCurve {
        self.chroma_curve
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// ランプ内での段の位置．含まれなければ `None`．
    pub fn position_of(&self, index: u8) -> Option<usize> {
        self.entries.iter().position(|&e| e == index)
    }

    /// ランプ上を `delta` 段だけ移動した添字を返す．
    ///
    /// 端は飽和させる．`base` がランプに含まれなければ `base` をそのまま返す．
    /// AA の色決定 (設計書 6.5) とサブピクセル (6.10) が共通で使う．
    pub fn step(&self, base: u8, delta: i32) -> u8 {
        let Some(pos) = self.position_of(base) else {
            return base;
        };
        let last = self.entries.len() as i32 - 1;
        let next = (pos as i32 + delta).clamp(0, last.max(0));
        self.entries[next as usize]
    }

    fn remap(&mut self, map: &[u8]) {
        for e in &mut self.entries {
            if let Some(&n) = map.get(*e as usize) {
                *e = n;
            }
        }
    }
}

/// パレット．
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    entries: Vec<Rgba8>,
    lab: Vec<Oklab>,
    ramps: Vec<Ramp>,
    locked: Vec<bool>,
    order_inv: Option<Vec<u8>>,
}

impl Palette {
    /// 色数の上限．添字が `u8` 固定であることによる (D2)．
    pub const MAX_COLORS: usize = 256;

    /// 色列からパレットを作る．アルファが 2 値でなければエラー．
    pub fn new(entries: Vec<Rgba8>) -> Result<Self> {
        if entries.len() > Self::MAX_COLORS {
            return Err(CoreError::PaletteTooLarge(entries.len()));
        }
        for (index, c) in entries.iter().enumerate() {
            if !c.has_binary_alpha() {
                return Err(CoreError::NonBinaryAlpha { index, alpha: c.a });
            }
        }
        let lab = entries.iter().map(|&c| oklab_of(c)).collect();
        let locked = vec![false; entries.len()];
        Ok(Self {
            entries,
            lab,
            ramps: Vec::new(),
            locked,
            order_inv: None,
        })
    }

    pub fn entries(&self) -> &[Rgba8] {
        &self.entries
    }

    /// `entries` から導出されたキャッシュ．書き換える API は無い (不変条件 2)．
    pub fn lab(&self) -> &[Oklab] {
        &self.lab
    }

    pub fn ramps(&self) -> &[Ramp] {
        &self.ramps
    }

    pub fn locked(&self) -> &[bool] {
        &self.locked
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, index: u8) -> Option<Rgba8> {
        self.entries.get(index as usize).copied()
    }

    pub fn lab_of(&self, index: u8) -> Option<Oklab> {
        self.lab.get(index as usize).copied()
    }

    pub fn is_locked(&self, index: u8) -> bool {
        self.locked.get(index as usize).copied().unwrap_or(false)
    }

    pub fn set_locked(&mut self, index: u8, locked: bool) {
        if let Some(slot) = self.locked.get_mut(index as usize) {
            *slot = locked;
        }
    }

    /// 1 色を差し替える．`lab` を同時に更新するので不変条件 2 が保たれる．
    pub fn set_entry(&mut self, index: u8, color: Rgba8) -> Result<()> {
        if !color.has_binary_alpha() {
            return Err(CoreError::NonBinaryAlpha {
                index: index as usize,
                alpha: color.a,
            });
        }
        let i = index as usize;
        if i >= self.entries.len() {
            return Err(CoreError::PaletteTooLarge(i + 1));
        }
        self.entries[i] = color;
        self.lab[i] = oklab_of(color);
        Ok(())
    }

    pub fn push(&mut self, color: Rgba8) -> Result<u8> {
        if self.entries.len() >= Self::MAX_COLORS {
            return Err(CoreError::PaletteTooLarge(self.entries.len() + 1));
        }
        if !color.has_binary_alpha() {
            return Err(CoreError::NonBinaryAlpha {
                index: self.entries.len(),
                alpha: color.a,
            });
        }
        let index = self.entries.len() as u8;
        self.entries.push(color);
        self.lab.push(oklab_of(color));
        self.locked.push(false);
        Ok(index)
    }

    pub fn add_ramp(&mut self, ramp: Ramp) {
        self.ramps.push(ramp);
    }

    /// 色列をまとめて差し替える．色数が変わらなければランプと施錠フラグを保つ．
    ///
    /// 正規化済みのパレットには適用できない．`order_inv` が指す元の並びとの対応が
    /// 失われ，`merge_back` で添字を戻せなくなるためである (D50)．
    pub fn replace_entries(&mut self, entries: Vec<Rgba8>) -> Result<()> {
        if self.order_inv.is_some() {
            return Err(CoreError::AlreadyNormalized);
        }
        let same_len = entries.len() == self.entries.len();
        let mut next = Self::new(entries)?;
        if same_len {
            next.ramps = std::mem::take(&mut self.ramps);
            next.locked = std::mem::take(&mut self.locked);
        }
        *self = next;
        Ok(())
    }

    /// 複数のキャンバスから使われている色を集めてパレットを作る
    /// (`pxsmith palette extract`)．
    ///
    /// 走査順ではなく**色の値で並べる**．同じ絵の集合なら，与える順番を変えても
    /// 同じパレットになる (設計書 6.15 規則 1)．
    pub fn extract_from<'a>(
        sources: impl IntoIterator<Item = (&'a IndexedCanvas, &'a Palette)>,
    ) -> Result<Self> {
        let mut colors: Vec<Rgba8> = Vec::new();
        for (canvas, palette) in sources {
            let mut used: Vec<u8> = canvas.pixels().to_vec();
            used.sort_unstable();
            used.dedup();
            for i in used {
                if let Some(c) = palette.get(i) {
                    colors.push(c);
                }
            }
        }
        colors.sort_unstable_by_key(|c| c.sort_key());
        colors.dedup();
        Self::new(colors)
    }

    /// 複数のパレットを 1 つに束ねる (`pxsmith palette merge`)．
    ///
    /// `share_endpoints` が真なら，**各パレットの最暗色と最明色を寄せて共有する**
    /// (D48)．色数の制約が厳しい場面で端点を揃えると，ランプ間で色を使い回せる．
    /// 寄せる判定は OKLab 距離が `threshold` 以下かどうかで行う．
    pub fn merged(palettes: &[Palette], share_endpoints: bool, threshold: f32) -> Result<Self> {
        let mut colors: Vec<Rgba8> = Vec::new();
        let mut endpoints: Vec<Rgba8> = Vec::new();

        for p in palettes {
            let mut opaque: Vec<(f32, Rgba8)> = p
                .entries()
                .iter()
                .zip(p.lab())
                .filter(|(c, _)| c.a != 0)
                .map(|(c, l)| (l.l, *c))
                .collect();
            opaque.sort_by(|a, b| {
                a.0.total_cmp(&b.0)
                    .then(a.1.sort_key().cmp(&b.1.sort_key()))
            });
            if share_endpoints && opaque.len() >= 2 {
                endpoints.push(opaque[0].1);
                endpoints.push(opaque[opaque.len() - 1].1);
            }
            colors.extend(p.entries().iter().copied());
        }

        if share_endpoints {
            // 近い端点どうしを代表色へ寄せる
            let mut representatives: Vec<Rgba8> = Vec::new();
            for e in &endpoints {
                let lab = oklab_of(*e);
                match representatives.iter().find(|r| {
                    crate::color::distance_sq(lab, oklab_of(**r), 1.0).sqrt() <= threshold
                }) {
                    Some(_) => {}
                    None => representatives.push(*e),
                }
            }
            for c in &mut colors {
                let lab = oklab_of(*c);
                if let Some(r) = representatives.iter().find(|r| {
                    **r != *c
                        && endpoints.contains(c)
                        && crate::color::distance_sq(lab, oklab_of(**r), 1.0).sqrt() <= threshold
                }) {
                    *c = *r;
                }
            }
        }

        colors.sort_unstable_by_key(|c| c.sort_key());
        colors.dedup();
        Self::new(colors)
    }

    /// 明度順へ正規化済みかどうか．
    pub fn is_normalized(&self) -> bool {
        self.order_inv.is_some()
    }

    /// 正規化前の添字への逆置換表．`order_inv[new] = old`．
    pub fn order_inv(&self) -> Option<&[u8]> {
        self.order_inv.as_deref()
    }

    /// 知覚明度の昇順へ並べ替える (D50)．返り値は前方向の置換表 `map[old] = new` で，
    /// [`IndexedCanvas::remap`](crate::canvas::IndexedCanvas::remap) にそのまま渡せる．
    ///
    /// 同じ明度の色が複数あっても並びが揺れないよう，明度が等しい場合は元の添字順で
    /// 決める (設計書 6.15 規則 2)．
    pub fn normalize_by_lightness(&mut self) -> Result<Vec<u8>> {
        if self.order_inv.is_some() {
            return Err(CoreError::AlreadyNormalized);
        }
        let n = self.entries.len();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            self.lab[a]
                .l
                .total_cmp(&self.lab[b].l)
                .then_with(|| a.cmp(&b))
        });

        // order[new] = old
        let mut fwd = vec![0u8; n];
        for (new, &old) in order.iter().enumerate() {
            fwd[old] = new as u8;
        }
        self.apply_permutation(&order, &fwd);
        self.order_inv = Some(order.iter().map(|&o| o as u8).collect());
        Ok(fwd)
    }

    /// 正規化前の並びへ戻す (D50)．返り値は前方向の置換表 `map[current] = original`．
    pub fn denormalize(&mut self) -> Result<Vec<u8>> {
        let n = self.entries.len();
        let len = self
            .order_inv
            .as_ref()
            .ok_or(CoreError::NotNormalized)?
            .len();
        if len != n {
            return Err(CoreError::BadPermutation {
                expected: n,
                actual: len,
            });
        }
        let order_inv = self.order_inv.take().expect("直前に存在を確認している");
        // order_inv[new] = old なので，戻すときは「新しい並びの new 番目に old が来る」
        // という向きで読み替える．
        let mut order = vec![0usize; n];
        for (new, &old) in order_inv.iter().enumerate() {
            order[old as usize] = new;
        }
        let fwd: Vec<u8> = order_inv.clone();
        self.apply_permutation(&order, &fwd);
        Ok(fwd)
    }

    /// `order[new] = old` で並べ替え，ランプの参照を `fwd[old] = new` で張り替える．
    fn apply_permutation(&mut self, order: &[usize], fwd: &[u8]) {
        self.entries = order.iter().map(|&o| self.entries[o]).collect();
        self.lab = order.iter().map(|&o| self.lab[o]).collect();
        self.locked = order.iter().map(|&o| self.locked[o]).collect();
        for ramp in &mut self.ramps {
            ramp.remap(fwd);
        }
    }

    /// 与えた色に最も近い添字を返す (設計書 6.6)．同点は添字の小さい方．
    pub fn nearest(&self, color: Rgba8, w_l: f32) -> Option<u8> {
        let target = oklab_of(color);
        let mut best: Option<(f32, u8)> = None;
        for (i, &lab) in self.lab.iter().enumerate() {
            // 透明色は色距離の対象にしない．アルファが 2 値なので判定は等値で足りる．
            if self.entries[i].a == 0 {
                continue;
            }
            let d = crate::color::distance_sq(target, lab, w_l);
            match best {
                Some((bd, _)) if bd <= d => {}
                _ => best = Some((d, i as u8)),
            }
        }
        best.map(|(_, i)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::IndexedCanvas;

    fn unsorted() -> Palette {
        // 明度がばらばらな 5 色
        Palette::new(vec![
            Rgba8::rgb(255, 255, 255),
            Rgba8::rgb(0, 0, 0),
            Rgba8::rgb(128, 0, 0),
            Rgba8::rgb(200, 200, 200),
            Rgba8::rgb(40, 40, 40),
        ])
        .unwrap()
    }

    #[test]
    fn rejects_partial_alpha() {
        let e = Palette::new(vec![Rgba8::new(1, 2, 3, 128)]).unwrap_err();
        assert!(matches!(
            e,
            CoreError::NonBinaryAlpha {
                index: 0,
                alpha: 128
            }
        ));
    }

    #[test]
    fn accepts_both_binary_alphas() {
        assert!(Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(1, 2, 3)]).is_ok());
    }

    #[test]
    fn lab_stays_in_sync_with_entries() {
        let mut p = unsorted();
        p.set_entry(0, Rgba8::rgb(10, 20, 30)).unwrap();
        assert_eq!(p.lab_of(0).unwrap(), oklab_of(Rgba8::rgb(10, 20, 30)));
        let pushed = p.push(Rgba8::rgb(9, 9, 9)).unwrap();
        assert_eq!(p.lab_of(pushed).unwrap(), oklab_of(Rgba8::rgb(9, 9, 9)));
        assert_eq!(p.lab().len(), p.entries().len());
    }

    #[test]
    fn normalize_orders_by_perceptual_lightness() {
        let mut p = unsorted();
        p.normalize_by_lightness().unwrap();
        let ls: Vec<f32> = p.lab().iter().map(|l| l.l).collect();
        for w in ls.windows(2) {
            assert!(w[0] <= w[1], "明度昇順のはず: {ls:?}");
        }
    }

    #[test]
    fn normalize_is_rejected_twice() {
        let mut p = unsorted();
        p.normalize_by_lightness().unwrap();
        assert!(matches!(
            p.normalize_by_lightness().unwrap_err(),
            CoreError::AlreadyNormalized
        ));
    }

    #[test]
    fn denormalize_requires_normalized_palette() {
        let mut p = unsorted();
        assert!(matches!(
            p.denormalize().unwrap_err(),
            CoreError::NotNormalized
        ));
    }

    /// M0 の中核 — 作業層での明度順正規化と逆置換の往復．
    #[test]
    fn normalize_denormalize_round_trip_restores_palette_and_canvas() {
        let original = unsorted();
        let canvas0 = IndexedCanvas::from_pixels(3, 2, vec![0, 1, 2, 3, 4, 0])
            .unwrap()
            .with_transparent(Some(1));

        let mut p = original.clone();
        let mut c = canvas0.clone();

        let fwd = p.normalize_by_lightness().unwrap();
        c.remap(&fwd).unwrap();
        assert!(p.is_normalized());
        assert_ne!(p.entries(), original.entries(), "並びが変わっているはず");

        let back = p.denormalize().unwrap();
        c.remap(&back).unwrap();

        assert!(!p.is_normalized());
        assert_eq!(p.entries(), original.entries());
        assert_eq!(p.lab(), original.lab());
        assert_eq!(c, canvas0);
    }

    /// 正規化の前後で「画素が指す色」が変わらないこと．
    #[test]
    fn normalization_preserves_resolved_colors() {
        let mut p = unsorted();
        let mut c = IndexedCanvas::from_pixels(5, 1, vec![0, 1, 2, 3, 4]).unwrap();
        let before: Vec<Rgba8> = c.pixels().iter().map(|&i| p.get(i).unwrap()).collect();

        let fwd = p.normalize_by_lightness().unwrap();
        c.remap(&fwd).unwrap();

        let after: Vec<Rgba8> = c.pixels().iter().map(|&i| p.get(i).unwrap()).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn normalization_remaps_ramp_entries() {
        let mut p = unsorted();
        p.add_ramp(Ramp::new(vec![1, 4, 2, 3, 0], ChromaCurve::PeakMiddle));
        let colors_before: Vec<Rgba8> = p.ramps()[0]
            .entries()
            .iter()
            .map(|&i| p.get(i).unwrap())
            .collect();

        p.normalize_by_lightness().unwrap();

        let colors_after: Vec<Rgba8> = p.ramps()[0]
            .entries()
            .iter()
            .map(|&i| p.get(i).unwrap())
            .collect();
        assert_eq!(
            colors_before, colors_after,
            "ランプが指す色は変わらないはず"
        );
    }

    #[test]
    fn normalization_moves_locked_flags_with_their_colors() {
        let mut p = unsorted();
        p.set_locked(2, true);
        let locked_color = p.get(2).unwrap();
        let fwd = p.normalize_by_lightness().unwrap();
        assert!(p.is_locked(fwd[2]));
        assert_eq!(p.get(fwd[2]).unwrap(), locked_color);
        assert_eq!(p.locked().iter().filter(|&&b| b).count(), 1);
    }

    #[test]
    fn ties_in_lightness_keep_original_order() {
        // 明度が完全に等しい 2 色 (同じ灰色) を含む
        let mut p = Palette::new(vec![
            Rgba8::rgb(128, 128, 128),
            Rgba8::rgb(0, 0, 0),
            Rgba8::rgb(128, 128, 128),
        ])
        .unwrap();
        let fwd = p.normalize_by_lightness().unwrap();
        // 黒が先頭へ，同明度の 2 色は元の添字順を保つ
        assert_eq!(fwd, vec![1, 0, 2]);
    }

    #[test]
    fn ramp_step_saturates_at_both_ends() {
        let r = Ramp::new(vec![3, 5, 7], ChromaCurve::Uniform);
        assert_eq!(r.step(5, 1), 7);
        assert_eq!(r.step(5, -1), 3);
        assert_eq!(r.step(7, 1), 7);
        assert_eq!(r.step(3, -1), 3);
        assert_eq!(r.step(99, 1), 99, "ランプ外の添字はそのまま");
    }

    #[test]
    fn nearest_skips_transparent_entries_and_breaks_ties_by_index() {
        let p = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0, 0, 0),
            Rgba8::rgb(0, 0, 0),
            Rgba8::rgb(255, 255, 255),
        ])
        .unwrap();
        assert_eq!(p.nearest(Rgba8::rgb(4, 4, 4), 1.0), Some(1));
        assert_eq!(p.nearest(Rgba8::rgb(250, 250, 250), 1.0), Some(3));
    }
}
