//! タイル分割と同値判定 (`px tileset extract`．設計書 6.7)．
//!
//! 許可された変換をすべて当てて**バイト列として最小になるもの**を正規形とし，
//! 同点なら**変換 ID の最小値**を採る (決定論性の要件．設計書 6.7 ・6.15 規則 1) ．
//!
//! # 削減率は校正の対象ではない
//!
//! 何割減るかは**入力のタイル並びで決まる**ので，測って閾値を置く類の量ではない．
//! 設計書 6.7 が求めているのは «縮約前後のタイル数と削減率を必ず報告する» ことで，
//! [`ExtractReport`] がそれを持つ．D92 (`px validate`) と同じ «数え上げ» 側である．
//!
//! # 反転で束ねるとルール 7 の相手になる — ただしタイルの大きさで決まる
//!
//! 設計書 6.7 は «反転を有効にした場合，陰影を持つ素材では lint ルール 7 で
//! 検出する» と言う．**掛かるかどうかはタイルの大きさで決まる** — ルール 7 は
//! 4 近傍がすべて不透明な画素でしか勾配を測れず，`shading_min_pixels`
//! (既定 64) に届かないタイルは «測れない» を返す．
//!
//! | 素材 | 勾配を測れる画素 | 64 に届く |
//! | --- | --- | --- |
//! | Kenney の 16x16 タイル 32 枚 | 中央 196 ・最小 64 | **32 / 32** |
//! | Dungeon Crawl の 32x32 32 枚 | 中央 487 | 32 / 32 |
//! | 同じ絵を 16x16 へ切った 128 タイル | 中央 116 | **92 / 128 (72%)** |
//! | **8x8 のタイル** | 上限 $6 \times 6 = 36$ | **構造的に 0** |
//!
//! **8x8 では 1 枚も検査できない．** これは閾値が厳しいのではなく，測る材料が
//! 無いということである — **«鳴らない» と «測れない» を混ぜない** (D77) ．
//! 呼ぶ側は測れなかった枚数を報告に併記すること (D92 の作法) ．

use std::collections::BTreeMap;

use crate::canvas::IndexedCanvas;
use crate::error::{CoreError, Result};
use crate::frame::{TileGrid, TileRef};

/// 同値とみなす範囲 (設計書 6.7)．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum DedupeMode {
    /// 完全一致のみ．**既定** (設計書 6.7 «既定は完全一致のみ») ．
    #[default]
    Exact,
    /// 左右 ・上下 ・その両方の反転まで束ねる．**ルール 7 の相手になる**．
    Flip,
    /// 反転に加えて 90 度回転と対角反転も束ねる (二面体群 $D_4$ の 8 変換)．
    FlipRotate,
}

impl DedupeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Flip => "flip",
            Self::FlipRotate => "flip-rotate",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "exact" => Some(Self::Exact),
            "flip" => Some(Self::Flip),
            "flip-rotate" | "fliprotate" => Some(Self::FlipRotate),
            _ => None,
        }
    }

    /// 許す変換の ID．**並びが決定論性を決める** — 同点では小さい ID を採る．
    fn transforms(self) -> &'static [u8] {
        match self {
            Self::Exact => &[0],
            Self::Flip => &[0, 1, 2, 3],
            Self::FlipRotate => &[0, 1, 2, 3, 4, 5, 6, 7],
        }
    }
}

/// 変換 ID を `(flip_x, flip_y, flip_d)` へ開く．
///
/// ID の並びは **`flip_d` が最上位** — 反転だけの 4 通り (0 〜 3) が先に来るので，
/// [`DedupeMode::Flip`] の許す集合が ID の前半とちょうど一致する．
fn flags_of(id: u8) -> (bool, bool, bool) {
    (id & 1 != 0, id & 2 != 0, id & 4 != 0)
}

fn id_of(flip_x: bool, flip_y: bool, flip_d: bool) -> u8 {
    u8::from(flip_x) | (u8::from(flip_y) << 1) | (u8::from(flip_d) << 2)
}

/// 変換を当てた画素を並べる．
///
/// `dst(x, y) = src(写した先)` で，**対角反転を先に，反転を後に**当てる．
/// 8 通りが二面体群 $D_4$ をちょうど 1 度ずつ与える (試験で固定してある) ．
fn apply(src: &[u8], w: u32, h: u32, id: u8) -> Vec<u8> {
    let (fx, fy, fd) = flags_of(id);
    // 対角反転は縦横を入れ替えるので，出力の大きさが変わる
    let (dw, dh) = if fd { (h, w) } else { (w, h) };
    let mut out = vec![0u8; (dw * dh) as usize];
    for y in 0..dh {
        for x in 0..dw {
            let (mut sx, mut sy) = if fd { (y, x) } else { (x, y) };
            if fx {
                sx = w - 1 - sx;
            }
            if fy {
                sy = h - 1 - sy;
            }
            out[(y * dw + x) as usize] = src[(sy * w + sx) as usize];
        }
    }
    out
}

/// 正規形と，**正規形から元のタイルへ戻す**変換．
///
/// `TileRef` の旗は «置いてあるタイルにこれを当てると絵になる» という向きなので，
/// 返すのは正規化に使った変換ではなく**その逆**である．
fn canonicalize(tile: &[u8], size: u32, mode: DedupeMode) -> (Vec<u8>, u8) {
    let mut best: Option<(Vec<u8>, u8)> = None;
    for &id in mode.transforms() {
        let v = apply(tile, size, size, id);
        // バイト列で最小，同点なら**変換 ID の最小値** (設計書 6.7)
        let better = match &best {
            None => true,
            Some((b, _)) => v < *b,
        };
        if better {
            best = Some((v, id));
        }
    }
    let (canonical, _) = best.expect("変換は 1 つ以上ある");

    // 正規形へ当てると元へ戻る変換を探す．**8 通りしかないので探して確かめる** —
    // 逆元を式で書くと対角反転が絡む組で間違えやすい
    let back = mode
        .transforms()
        .iter()
        .copied()
        .find(|&id| apply(&canonical, size, size, id) == tile)
        .unwrap_or(0);
    (canonical, back)
}

#[derive(Clone, Debug)]
pub struct ExtractOptions {
    /// タイルの一辺．
    pub tile: u32,
    pub mode: DedupeMode,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            tile: 16,
            mode: DedupeMode::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExtractReport {
    pub tile: u32,
    pub mode: DedupeMode,
    /// 切り出したタイルの数 (縮約前)．
    pub before: usize,
    /// 正規形の数 (縮約後)．
    pub after: usize,
    /// 恒等でない向きで置かれた升の数．
    ///
    /// > [!warning] **これは «束ねた枚数» ではない．** 1 度しか現れないタイルでも，
    /// > 正規形 (バイト列最小) が元の向きと違えば恒等でない変換で置かれる．
    /// > 削減率 0% なのにこの数が 101 になって**測定が誤りを暴いた**．
    /// > ルール 7 を掛ける相手は [`Self::mirror_reliant`] の方である．
    pub oriented: usize,
    /// **反転に頼って別々の升を再現しているタイルの数．**
    ///
    /// 同じタイル添字を指す升が**2 通り以上の向き**で使われており，そこに反転が
    /// 含まれるものを数える．設計書 6.7 が «陰影を持つ素材ではルール 7 で検出する»
    /// と言っているのはこの形である — «同じ絵を裏返して別の升に使っている» ので，
    /// 陰影があれば光源と矛盾する．
    ///
    /// **向きが 1 通りしかない群は数えない．** 全部の升が同じ向きなら，それは
    /// «タイルを裏返して保存した» だけで，どの升も描かれたとおりに出る．
    pub mirror_reliant: usize,
}

impl ExtractReport {
    /// 削減率．**校正の対象ではなく報告する量である** (設計書 6.7)．
    pub fn reduction(&self) -> f32 {
        if self.before == 0 {
            0.0
        } else {
            1.0 - self.after as f32 / self.before as f32
        }
    }
}

/// 1 枚の絵をタイルへ切り，同値なものを束ねる．
///
/// **端数は切り捨てない** — 絵の寸法がタイルの倍数でなければ誤りとする．
/// 黙って切ると，切り落とした帯が «そこには何も無かった» ことになる．
pub fn extract(
    canvas: &IndexedCanvas,
    opts: &ExtractOptions,
) -> Result<(Vec<IndexedCanvas>, TileGrid, ExtractReport)> {
    let n = opts.tile;
    if n == 0 {
        return Err(CoreError::TileSizeZero);
    }
    let (w, h) = (canvas.width(), canvas.height());
    if !w.is_multiple_of(n) || !h.is_multiple_of(n) {
        return Err(CoreError::TileSizeMismatch {
            width: w,
            height: h,
            tile: n,
        });
    }
    let (cols, rows) = (w / n, h / n);

    // 正規形 → (添字，正規形の画素)．**走査順で最初に現れたものから番号を振る**
    let mut index: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
    let mut tiles: Vec<IndexedCanvas> = Vec::new();
    let mut refs: Vec<TileRef> = Vec::with_capacity((cols * rows) as usize);
    let mut oriented = 0usize;

    for ty in 0..rows {
        for tx in 0..cols {
            let mut pixels = Vec::with_capacity((n * n) as usize);
            for y in 0..n {
                for x in 0..n {
                    pixels.push(
                        canvas
                            .get((tx * n + x) as i32, (ty * n + y) as i32)
                            .expect("倍数を確かめてある"),
                    );
                }
            }
            let (canonical, back) = canonicalize(&pixels, n, opts.mode);
            let id = match index.get(&canonical) {
                Some(i) => *i,
                None => {
                    let i = tiles.len() as u32;
                    let mut c = IndexedCanvas::from_pixels(n, n, canonical.clone())?;
                    c.set_transparent(canvas.transparent());
                    tiles.push(c);
                    index.insert(canonical, i);
                    i
                }
            };
            if back != 0 {
                oriented += 1;
            }
            let (flip_x, flip_y, flip_d) = flags_of(back);
            refs.push(TileRef {
                id,
                flip_x,
                flip_y,
                flip_d,
            });
        }
    }

    let before = refs.len();
    let grid = TileGrid::from_tiles(cols, rows, refs)?;
    let mirror_reliant = mirror_reliant_cells(&grid).len();
    let report = ExtractReport {
        tile: n,
        mode: opts.mode,
        before,
        after: tiles.len(),
        oriented,
        mirror_reliant,
    };
    Ok((tiles, grid, report))
}

/// タイルと格子から絵を組み直す．**抽出が絵を保っているかを確かめる口である**．
pub fn rebuild(
    tiles: &[IndexedCanvas],
    grid: &TileGrid,
    tile: u32,
    transparent: Option<u8>,
) -> Result<IndexedCanvas> {
    let (w, h) = (grid.width() * tile, grid.height() * tile);
    let mut out = IndexedCanvas::filled(w, h, transparent.unwrap_or(0));
    out.set_transparent(transparent);
    for ty in 0..grid.height() {
        for tx in 0..grid.width() {
            let r = grid.get(tx, ty).ok_or(CoreError::TileSizeZero)?;
            let src = tiles
                .get(r.id as usize)
                .ok_or(CoreError::TileIdOutOfRange { id: r.id })?;
            let moved = apply(
                src.pixels(),
                tile,
                tile,
                id_of(r.flip_x, r.flip_y, r.flip_d),
            );
            for y in 0..tile {
                for x in 0..tile {
                    out.set(
                        (tx * tile + x) as i32,
                        (ty * tile + y) as i32,
                        moved[(y * tile + x) as usize],
                    );
                }
            }
        }
    }
    Ok(out)
}

/// **反転に頼って別々の升を再現している升**を並べる — ルール 7 を掛ける相手．
///
/// > [!warning] **«恒等でない向きで置かれた升» とは別物である．**
/// > 1 度しか現れないタイルでも，正規形が元の向きと違えば旗が立つ — その升は
/// > 描かれたとおりに出るので，陰影の矛盾は起きない．測定で削減率 0% なのに
/// > 旗の立った升が 101 あって気付いた．
/// >
/// > 数えるのは «**同じタイルを 2 通り以上の向きで使っており，そこに反転が
/// > 含まれる**» 群だけである．そういう群でだけ «同じ絵を裏返して別の升に
/// > 使っている» ことになり，陰影があれば光源と矛盾する．
pub fn mirror_reliant_cells(grid: &TileGrid) -> Vec<(u32, u32)> {
    // タイル添字ごとに，使われている向きを集める
    let mut orientations: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    for y in 0..grid.height() {
        for x in 0..grid.width() {
            if let Some(r) = grid.get(x, y) {
                let id = id_of(r.flip_x, r.flip_y, r.flip_d);
                let e = orientations.entry(r.id).or_default();
                if !e.contains(&id) {
                    e.push(id);
                }
            }
        }
    }

    let mut out = Vec::new();
    for y in 0..grid.height() {
        for x in 0..grid.width() {
            let Some(r) = grid.get(x, y) else { continue };
            if !(r.flip_x || r.flip_y) {
                continue;
            }
            // 向きが 1 通りしか無い群は «裏返して保存しただけ» である
            if orientations.get(&r.id).is_some_and(|v| v.len() >= 2) {
                out.push((x, y));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas_of(w: u32, h: u32, pixels: Vec<u8>) -> IndexedCanvas {
        IndexedCanvas::from_pixels(w, h, pixels)
            .expect("画素数が合う")
            .with_transparent(Some(0))
    }

    /// 壊れると: 8 つの «変換» が実は重複していて，正規形が変換の取り方で変わる．
    #[test]
    fn the_eight_transforms_are_all_distinct_on_an_asymmetric_tile() {
        // 対称性の無い 2x2
        let tile = vec![1, 2, 3, 4];
        let mut seen: Vec<Vec<u8>> = (0..8).map(|id| apply(&tile, 2, 2, id)).collect();
        let total = seen.len();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), total, "8 変換のうち重複しているものがある");
    }

    /// 壊れると: 変換が群になっておらず，`rebuild` が元へ戻らない．
    #[test]
    fn every_transform_has_an_inverse_inside_the_set() {
        let tile = vec![1, 2, 3, 4, 5, 6, 7, 8, 9];
        for id in 0..8u8 {
            let moved = apply(&tile, 3, 3, id);
            let back = (0..8u8).find(|&j| apply(&moved, 3, 3, j) == tile);
            assert!(back.is_some(), "変換 {id} の逆が集合の中に無い");
        }
    }

    /// 壊れると: 既定が «完全一致» でなくなり，反転したタイルが黙って束ねられる
    /// (設計書 6.7 «既定は完全一致のみ») ．
    #[test]
    fn the_default_mode_does_not_merge_a_mirrored_tile() {
        // 左右反転すると一致する 2 枚を横に並べる
        let pixels = vec![1, 2, 2, 1, 3, 4, 4, 3];
        let canvas = canvas_of(4, 2, pixels);
        let opts = ExtractOptions {
            tile: 2,
            mode: DedupeMode::Exact,
        };
        let (tiles, _, report) = extract(&canvas, &opts).expect("切れる");
        assert_eq!(tiles.len(), 2);
        assert_eq!(report.after, 2);
        assert_eq!(report.oriented, 0);
        assert_eq!(report.mirror_reliant, 0);

        let opts = ExtractOptions {
            tile: 2,
            mode: DedupeMode::Flip,
        };
        let (tiles, grid, report) = extract(&canvas, &opts).expect("切れる");
        assert_eq!(tiles.len(), 1, "反転で束ねれば 1 枚になる");
        assert_eq!(report.oriented, 1);
        assert_eq!(
            report.mirror_reliant, 1,
            "2 通りの向きで使っているので «反転に頼っている»"
        );
        assert_eq!(mirror_reliant_cells(&grid).len(), 1);
        assert!((report.reduction() - 0.5).abs() < 1e-6);
    }

    /// 壊れると: 束ねたタイルから絵が戻らない — **旗の向きを取り違えている**．
    /// これが 3 モードすべてで成り立たなければ縮約は使えない．
    #[test]
    fn rebuilding_reproduces_the_picture_in_every_mode() {
        let mut pixels = Vec::new();
        // 4x4 のタイルを 3x2 枚 = 12x8 画素．中身をばらばらにする
        for i in 0..(12 * 8) {
            pixels.push((i * 7 % 251) as u8);
        }
        let canvas = canvas_of(12, 8, pixels);
        for mode in [DedupeMode::Exact, DedupeMode::Flip, DedupeMode::FlipRotate] {
            let opts = ExtractOptions { tile: 4, mode };
            let (tiles, grid, _) = extract(&canvas, &opts).expect("切れる");
            let back = rebuild(&tiles, &grid, 4, canvas.transparent()).expect("戻せる");
            assert_eq!(
                back.pixels(),
                canvas.pixels(),
                "{} で絵が戻らない",
                mode.as_str()
            );
        }
    }

    /// 壊れると: 対称なタイルで «どの変換を使ったか» が実行ごとに変わり，
    /// 差分ビルドの鍵が揺れる (設計書 6.15 規則 1) ．
    #[test]
    fn a_symmetric_tile_always_picks_the_smallest_transform_id() {
        // 全部同じ色 — 8 変換すべてが同じバイト列を返す
        let canvas = canvas_of(2, 2, vec![5, 5, 5, 5]);
        let opts = ExtractOptions {
            tile: 2,
            mode: DedupeMode::FlipRotate,
        };
        let (_, grid, report) = extract(&canvas, &opts).expect("切れる");
        let r = grid.get(0, 0).expect("ある");
        assert!(!r.flip_x && !r.flip_y && !r.flip_d);
        assert_eq!(report.oriented, 0, "同点なら恒等変換を採る");
    }

    /// **壊れると: 1 度しか現れないタイルが «反転で束ねた» ものとして数えられ，
    /// 陰影の矛盾が起きていない升にルール 7 が掛かる．**
    ///
    /// 正規形はバイト列が最小のものなので，**1 度しか現れないタイルでも**元の向きと
    /// 違えば旗が立つ．測定で «削減率 0% なのに旗の立った升が 101» と出て気付いた．
    #[test]
    fn a_tile_that_appears_once_is_not_counted_as_mirror_reliant() {
        // 左右非対称な 2 枚．どちらも 1 度しか現れない
        let pixels = vec![9, 1, 8, 2, 0, 0, 0, 0];
        let canvas = canvas_of(4, 2, pixels);
        let opts = ExtractOptions {
            tile: 2,
            mode: DedupeMode::Flip,
        };
        let (_, grid, report) = extract(&canvas, &opts).expect("切れる");
        assert_eq!(report.after, report.before, "束ねられるものは無い");
        assert!(
            report.oriented > 0,
            "正規形の向きが元と違う升があるはずである (この試験の前提)"
        );
        assert_eq!(
            report.mirror_reliant, 0,
            "1 度しか現れないタイルは反転に頼っていない"
        );
        assert!(mirror_reliant_cells(&grid).is_empty());
    }

    /// 壊れると: 端数の帯が黙って切り落とされ，«そこには何も無かった» ことになる．
    #[test]
    fn a_size_that_is_not_a_multiple_of_the_tile_is_an_error() {
        let canvas = canvas_of(5, 4, vec![0; 20]);
        let opts = ExtractOptions {
            tile: 2,
            mode: DedupeMode::Exact,
        };
        assert!(matches!(
            extract(&canvas, &opts),
            Err(CoreError::TileSizeMismatch { .. })
        ));
    }

    /// 壊れると: 回転を許したときだけ束ねられるはずのタイルが，反転だけで束ねられる．
    #[test]
    fn rotation_only_merges_under_flip_rotate() {
        // 90 度回すと一致するが，反転では一致しない 2 枚
        let a = [1, 1, 0, 0];
        let b = [0, 1, 0, 1];
        let mut pixels = Vec::new();
        for y in 0..2 {
            for x in 0..2 {
                pixels.push(a[y * 2 + x]);
            }
            for x in 0..2 {
                pixels.push(b[y * 2 + x]);
            }
        }
        let canvas = canvas_of(4, 2, pixels);
        let flip = extract(
            &canvas,
            &ExtractOptions {
                tile: 2,
                mode: DedupeMode::Flip,
            },
        )
        .expect("切れる")
        .2;
        let rot = extract(
            &canvas,
            &ExtractOptions {
                tile: 2,
                mode: DedupeMode::FlipRotate,
            },
        )
        .expect("切れる")
        .2;
        assert_eq!(flip.after, 2, "反転だけでは束ねられない");
        assert_eq!(rot.after, 1, "回転を許せば束ねられる");
    }
}
