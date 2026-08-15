//! autotile — 象限合成による 47 枚生成 (設計書 6.8 ・4.3)．
//!
//! # 47 という数は «数え上げ» である
//!
//! 8 近傍の bitmask 256 通りのうち，**対角のビットは «両隣の辺が両方とも立って
//! いるとき» にしか意味を持たない** (辺が繋がっていなければ，対角の先が同じ地形か
//! どうかは見た目に出ない) ．意味を持たない対角ビットを落とすと，残るのは
//! ちょうど 47 通りである．
//!
//! | 辺の立ち方 | 通り | 意味のある対角 | 小計 |
//! | --- | --- | --- | --- |
//! | 0 本 | 1 | 0 | 1 |
//! | 1 本 | 4 | 0 | 4 |
//! | 2 本 (隣り合う) | 4 | 1 | 8 |
//! | 2 本 (向かい合う) | 2 | 0 | 2 |
//! | 3 本 | 4 | 2 | 16 |
//! | 4 本 | 1 | 4 | 16 |
//! | | | | **47** |
//!
//! **これは校正の対象ではない** (D92 ・D101 と同じ側) ．閾値ではなく数え上げなので，
//! 試験で 47 になることを固定する — 数がずれたら縮約の規則が壊れている．
//!
//! # 象限の状態は厳密に 5 通り
//!
//! 1 つの角から見えるのは «横の辺 ・縦の辺 ・その間の対角» の 3 ビットだが，
//! 辺が揃っていなければ対角は効かないので，**8 通りのうち 3 通りが «内側» へ
//! 縮退して 5 通りになる** (設計書 6.8) ．
//!
//! # 自動ミラーはルール 7 の相手だが，**組んでから掛ける**
//!
//! 設計書 4.3 は «自動ミラーで生成したタイルには lint ルール 7 を blocking で
//! 適用する» と定める．**象限に掛けてはいけない** — 16x16 のタイルの象限は 8x8 で，
//! ルール 7 は勾配を測れる画素が上限 $6 \times 6 = 36$ しか無く
//! `shading_min_pixels` (既定 64) に構造的に届かない (D100) ．
//! **4 象限を組んだタイルに掛けること．**

use std::collections::BTreeMap;

use crate::canvas::IndexedCanvas;
use crate::error::{CoreError, Result};

/// 8 近傍のビット位置．**辺が下位 4 ビット ・対角が上位 4 ビット**である．
pub const N: u8 = 1 << 0;
pub const E: u8 = 1 << 1;
pub const S: u8 = 1 << 2;
pub const W: u8 = 1 << 3;
pub const NE: u8 = 1 << 4;
pub const SE: u8 = 1 << 5;
pub const SW: u8 = 1 << 6;
pub const NW: u8 = 1 << 7;

/// 対角と，その両隣の辺．
const DIAGONALS: [(u8, u8, u8); 4] = [(NE, N, E), (SE, S, E), (SW, S, W), (NW, N, W)];

/// 象限 (設計書 4.3)．
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quadrant {
    NW,
    NE,
    SW,
    SE,
}

pub const QUADRANTS: [Quadrant; 4] = [Quadrant::NW, Quadrant::NE, Quadrant::SW, Quadrant::SE];

impl Quadrant {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NW => "NW",
            Self::NE => "NE",
            Self::SW => "SW",
            Self::SE => "SE",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "NW" => Some(Self::NW),
            "NE" => Some(Self::NE),
            "SW" => Some(Self::SW),
            "SE" => Some(Self::SE),
            _ => None,
        }
    }

    /// この象限から見た (横の辺，縦の辺，対角) のビット．
    fn bits(self) -> (u8, u8, u8) {
        match self {
            Self::NW => (W, N, NW),
            Self::NE => (E, N, NE),
            Self::SW => (W, S, SW),
            Self::SE => (E, S, SE),
        }
    }

    /// タイルの中でこの象限が占める左上の位置 (象限の一辺を `half` として)．
    fn origin(self, half: u32) -> (u32, u32) {
        match self {
            Self::NW => (0, 0),
            Self::NE => (half, 0),
            Self::SW => (0, half),
            Self::SE => (half, half),
        }
    }

    /// `NW` の絵をこの象限へ持ってくるときの反転．
    fn mirror_from_nw(self) -> (bool, bool) {
        match self {
            Self::NW => (false, false),
            Self::NE => (true, false),
            Self::SW => (false, true),
            Self::SE => (true, true),
        }
    }
}

/// 象限の状態 (設計書 6.8 の 5 通り)．
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CornerState {
    /// 辺が両方とも無い (凸角)．
    Convex,
    /// 横の辺だけある．
    EdgeH,
    /// 縦の辺だけある．
    EdgeV,
    /// 両辺あり + 対角あり (内側)．
    Inner,
    /// 両辺あり + 対角なし (凹角)．
    Concave,
}

pub const STATES: [CornerState; 5] = [
    CornerState::Convex,
    CornerState::EdgeH,
    CornerState::EdgeV,
    CornerState::Inner,
    CornerState::Concave,
];

impl CornerState {
    /// L0 のフレーム名 (設計書 6.8 の表)．
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Convex => "corner_convex",
            Self::EdgeH => "edge_h",
            Self::EdgeV => "edge_v",
            Self::Inner => "inner",
            Self::Concave => "corner_concave",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        STATES.into_iter().find(|v| v.as_str() == s)
    }
}

/// 対角のビットのうち**両隣の辺が揃っていないもの**を落とす．
///
/// 256 通りをこれに通すと，異なる値はちょうど 47 個になる (試験で固定してある) ．
pub fn canonical_mask(mask: u8) -> u8 {
    let mut out = mask;
    for (d, a, b) in DIAGONALS {
        if mask & a == 0 || mask & b == 0 {
            out &= !d;
        }
    }
    out
}

/// 正規形の bitmask をすべて並べる．**必ず 47 個**で，**昇順**である
/// (設計書 6.15 規則 1) ．
pub fn blob_masks() -> Vec<u8> {
    let mut out: Vec<u8> = (0u16..=255).map(|m| canonical_mask(m as u8)).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// bitmask からこの象限の状態を決める．
pub fn corner_state(mask: u8, q: Quadrant) -> CornerState {
    let (h, v, d) = q.bits();
    match (mask & h != 0, mask & v != 0, mask & d != 0) {
        (false, false, _) => CornerState::Convex,
        (true, false, _) => CornerState::EdgeH,
        (false, true, _) => CornerState::EdgeV,
        (true, true, true) => CornerState::Inner,
        (true, true, false) => CornerState::Concave,
    }
}

/// 象限の絵．鍵は (象限，状態)．
pub type QuadrantArt = BTreeMap<(Quadrant, CornerState), IndexedCanvas>;

/// 5 枚だけ (象限を指定しない) の入力から 4 象限ぶんへ広げる — **自動ミラー**．
///
/// 設計書 4.3 の «`quadrant` を省略したフレームは全象限に適用され，必要に応じて
/// 自動ミラーされる» にあたる．`NW` を基準に左右 ・上下へ反転する．
pub fn mirror_to_all_quadrants(base: &BTreeMap<CornerState, IndexedCanvas>) -> QuadrantArt {
    let mut out = QuadrantArt::new();
    for q in QUADRANTS {
        let (fx, fy) = q.mirror_from_nw();
        for (state, art) in base {
            out.insert((q, *state), flip(art, fx, fy));
        }
    }
    out
}

fn flip(canvas: &IndexedCanvas, fx: bool, fy: bool) -> IndexedCanvas {
    let (w, h) = (canvas.width(), canvas.height());
    let mut out = canvas.clone();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let sx = if fx { w as i32 - 1 - x } else { x };
            let sy = if fy { h as i32 - 1 - y } else { y };
            out.set(x, y, canvas.get(sx, sy).expect("範囲内"));
        }
    }
    out
}

/// **象限の継ぎ目で «1 画素の縞» が «2 画素» になっている箇所の数．**
///
/// # 設計書 4.3 の主張は測ると成り立たなかった (D105)
///
/// 設計書 4.3 / D45 は «**タイルの幅は必ず偶数なので，同一タイルを並べるとディザの
/// ドットが連結する**» と言うが，**逆である**．市松 (周期 2) を偶数幅のタイルに
/// 入れて 2 枚並べると，継ぎ目で同色が隣り合う行は **0 / 16** — 幅が偶数だからこそ
/// 位相が続く．**連結するのは奇数幅のときである．**
///
/// | 場面 | 継ぎ目で同色が隣り合う |
/// | --- | --- |
/// | 偶数幅 ・同一タイルの反復 | **0 / 16** |
/// | 奇数幅 ・同一タイルの反復 | 16 / 16 |
/// | **偶数幅 ・鏡像を隣に置く** | **16 / 16** |
///
/// # 連結するのは «自動ミラー» のときで，しかもタイルの内側である
///
/// 左右反転は $x \mapsto w - 1 - x$ なので，**継ぎ目をまたぐ 2 列は必ず同じ列に
/// なる** — これは測定ではなく代数である．象限を鏡像で組む autotile では
/// **タイルの内側の継ぎ目 ($x = half$，$y = half$) でこれが起きる**．
/// 市松の象限で組むと 16x16 のタイル 1 枚に同色 4 隣接が 32 件できる．
///
/// # だから «位相バリアントを交互配置» は採らない (D106)
///
/// D45 の処方をそのまま当てると，**継ぎ目が合っている偶数幅の反復を壊す**
/// (同色の隣接が 0 → 16 に増える) ．**問題はタイル間ではなくタイル内にある**ので，
/// タイルを 2 種類作って交互に置いても直らない．
///
/// 直し方は陰影とまったく同じである — **ディザを持つ素材では自動ミラーを使わず，
/// 4 象限を描くこと**．ここはそれを**検出して報告する**側である．
///
/// # 数え方 — 閾値は無い
///
/// 継ぎ目で同色が隣り合っていても，**元から同じ色が続いていたなら見た目は
/// 変わらない**．被害が出るのは «その外側で色が変わっている» ところ，つまり
/// **1 画素の縞が 2 画素になった**ところだけである．それを数える．
pub fn seam_doubled(tile: &IndexedCanvas) -> usize {
    let (w, h) = (tile.width() as i32, tile.height() as i32);
    if w < 4 || h < 4 {
        return 0;
    }
    let (mx, my) = (w / 2, h / 2);
    let mut n = 0usize;

    // 縦の継ぎ目 — 左右の列が同じで，かつ左の外側で色が変わっている
    for y in 0..h {
        if tile.get(mx - 1, y) == tile.get(mx, y)
            && tile.get(mx - 2, y) != tile.get(mx - 1, y)
            && tile.get(mx + 1, y) != tile.get(mx, y)
        {
            n += 1;
        }
    }
    // 横の継ぎ目
    for x in 0..w {
        if tile.get(x, my - 1) == tile.get(x, my)
            && tile.get(x, my - 2) != tile.get(x, my - 1)
            && tile.get(x, my + 1) != tile.get(x, my)
        {
            n += 1;
        }
    }
    n
}

#[derive(Clone, Debug)]
pub struct AutotileReport {
    /// 生成したタイルの数．
    pub tiles: usize,
    /// 入力に渡された象限の絵の枚数．
    pub given: usize,
    /// **自動ミラーで作った象限の絵の枚数** — ルール 7 を掛ける相手の元である．
    pub mirrored: usize,
    /// 自動ミラーを使ったか (使ったならタイルにルール 7 を掛けること)．
    pub used_mirror: bool,
}

/// 47 枚のタイルを組む．
///
/// 返すのは `(bitmask, タイル)` を **bitmask の昇順**に並べたもの．
pub fn build(art: &QuadrantArt, tile: u32) -> Result<(Vec<(u8, IndexedCanvas)>, usize)> {
    if tile == 0 || !tile.is_multiple_of(2) {
        return Err(CoreError::AutotileOddTile { tile });
    }
    let half = tile / 2;

    // 足りない (象限，状態) を先に全部数える — 1 枚ずつ落とすと «どれが足りないか»
    // が分からないまま止まる
    let mut missing: Vec<(Quadrant, CornerState)> = Vec::new();
    for q in QUADRANTS {
        for state in STATES {
            if !art.contains_key(&(q, state)) {
                missing.push((q, state));
            }
        }
    }
    if !missing.is_empty() {
        return Err(CoreError::AutotileMissingQuadrants {
            missing: missing
                .iter()
                .map(|(q, s)| format!("{}/{}", q.as_str(), s.as_str()))
                .collect(),
        });
    }
    for ((q, s), c) in art {
        if c.width() != half || c.height() != half {
            return Err(CoreError::AutotileQuadrantSize {
                quadrant: format!("{}/{}", q.as_str(), s.as_str()),
                width: c.width(),
                height: c.height(),
                expected: half,
            });
        }
    }

    let transparent = art.values().next().and_then(|c| c.transparent());
    let masks = blob_masks();
    let mut out = Vec::with_capacity(masks.len());
    for mask in &masks {
        let mut canvas = IndexedCanvas::filled(tile, tile, transparent.unwrap_or(0));
        canvas.set_transparent(transparent);
        for q in QUADRANTS {
            let state = corner_state(*mask, q);
            let src = art.get(&(q, state)).expect("欠けは先に数えた");
            let (ox, oy) = q.origin(half);
            for y in 0..half {
                for x in 0..half {
                    canvas.set(
                        (ox + x) as i32,
                        (oy + y) as i32,
                        src.get(x as i32, y as i32).expect("範囲内"),
                    );
                }
            }
        }
        out.push((*mask, canvas));
    }
    Ok((out, masks.len()))
}

/// 入力の 1 枚 (L0 の 1 フレームにあたる)．
#[derive(Clone, Debug)]
pub struct Piece {
    /// 状態の名前 (`corner_convex` など．設計書 6.8 の表)．
    pub name: String,
    /// 明示された象限．`None` なら全象限に使う．
    pub quadrant: Option<Quadrant>,
    pub art: IndexedCanvas,
}

/// 設計書 4.3 の解決規則を当てて，(象限，状態) → 絵の表を作る．
///
/// **同じ `name` を持つフレーム群ごとに，上から順に判定する．**
///
/// | 段 | 条件 | 挙動 |
/// | --- | --- | --- |
/// | 1 | 全フレームが `quadrant` を持つ | 指定どおりに置く．**自動ミラーしない** |
/// | 2 | 全フレームが `quadrant` を持たない | 全象限に使い，必要な象限で自動ミラーする |
/// | 3 | **一部だけが持つ** | **エラー** (`E_QUADRANT_PARTIAL`) |
/// | 4 | `quadrant` 付きだが 4 象限を網羅していない | エラー．欠けを列挙する |
///
/// 段 3 を許すと «指定された象限だけ手描き ・残りは自動ミラー» という混在が生まれ，
/// **陰影の整合が象限ごとに変わる**．
///
/// 返り値の 2 つ目は**自動ミラーを使ったか** — 真なら呼ぶ側が組んだタイルへ
/// ルール 7 を blocking で掛けること (設計書 4.3) ．
pub fn resolve(pieces: &[Piece]) -> Result<(QuadrantArt, bool)> {
    if pieces.is_empty() {
        return Err(CoreError::AutotileNoPieces);
    }

    // 名前ごとにまとめる．**`BTreeMap` で並びを固定する** (設計書 6.15 規則 1)
    let mut groups: BTreeMap<String, Vec<&Piece>> = BTreeMap::new();
    for p in pieces {
        groups.entry(p.name.clone()).or_default().push(p);
    }

    let mut out = QuadrantArt::new();
    let mut used_mirror = false;

    for (name, group) in &groups {
        let state = CornerState::parse(name).ok_or_else(|| CoreError::AutotileUnknownState {
            name: name.clone(),
            known: STATES.iter().map(|s| s.as_str().to_string()).collect(),
        })?;
        let with = group.iter().filter(|p| p.quadrant.is_some()).count();

        if with == 0 {
            // 段 2 — 全象限へ自動ミラーする
            let art = &group[0].art;
            for q in QUADRANTS {
                let (fx, fy) = q.mirror_from_nw();
                if fx || fy {
                    used_mirror = true;
                }
                out.insert((q, state), flip(art, fx, fy));
            }
            continue;
        }
        if with != group.len() {
            // 段 3 — 混在は許さない
            return Err(CoreError::QuadrantPartial {
                name: name.clone(),
                with,
                total: group.len(),
            });
        }

        // 段 1 — 指定どおりに置く．段 4 で網羅を確かめる
        let mut missing: Vec<Quadrant> = Vec::new();
        for q in QUADRANTS {
            match group.iter().find(|p| p.quadrant == Some(q)) {
                Some(p) => {
                    out.insert((q, state), p.art.clone());
                }
                None => missing.push(q),
            }
        }
        if !missing.is_empty() {
            return Err(CoreError::AutotileQuadrantsNotCovered {
                name: name.clone(),
                missing: missing.iter().map(|q| q.as_str().to_string()).collect(),
            });
        }
    }

    Ok((out, used_mirror))
}

/// 象限インポータが受け取る並び (設計書 6.8 «レイアウトの明示を必須とする»)．
///
/// 設計書は «3 レイアウト» としか決めていないので**こちらで選んだ**．
/// 選び方の基準は «**推測が要らないこと**» である — 設計書 6.8 は «1 枚のタイルからの
/// 自動推測では «辺の装飾» と «内側» の区別が一意に決まらず，外れると 47 枚すべてが
/// 静かに壊れる» と言う．どの並びも**枚数と順番だけで決まり，絵の中身を推測しない**．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ImportLayout {
    /// **象限 5 枚** — `NW` 基準の 5 状態．残りの 3 象限は自動ミラーで作る．
    ///
    /// 順番は [`STATES`] と同じ (`corner_convex` ・`edge_h` ・`edge_v` ・`inner` ・
    /// `corner_concave`) ．**陰影やディザを持つ素材には使えない** — 自動ミラーが
    /// 光源を裏返し (設計書 4.3) ，ディザの位相を壊す (D105) ．
    Quadrants5,
    /// **象限 20 枚** — 象限 4 x 状態 5 を全部明示する．自動ミラーを使わない．
    ///
    /// 順番は象限が外側 ([`QUADRANTS`]) ・状態が内側 ([`STATES`]) である．
    Quadrants20,
    /// **組み上がった 47 枚**から象限を取り出す．
    ///
    /// 順番は bitmask の昇順 ([`blob_masks`]) — `pxsmith tileset autotile` の出力そのもの．
    ///
    /// > [!note] **ここだけは «検証» ができる．**
    /// > 同じ (象限，状態) は複数のタイルに現れるので，**食い違ったらエラーにする**．
    /// > 設計書 6.8 が恐れている «推測が外れて 47 枚が静かに壊れる» は，
    /// > **推測しないうえに突き合わせる**ことで起きなくなる．
    Blob47,
}

impl ImportLayout {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quadrants5 => "quadrants-5",
            Self::Quadrants20 => "quadrants-20",
            Self::Blob47 => "blob-47",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "quadrants-5" => Some(Self::Quadrants5),
            "quadrants-20" => Some(Self::Quadrants20),
            "blob-47" => Some(Self::Blob47),
            _ => None,
        }
    }

    /// この並びが要求する枚数．
    pub fn expected(self) -> usize {
        match self {
            Self::Quadrants5 => 5,
            Self::Quadrants20 => 20,
            Self::Blob47 => 47,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ImportReport {
    pub layout: ImportLayout,
    /// 取り出した (象限，状態) の数．**必ず 20 である**．
    pub pieces: usize,
    /// 自動ミラーで補ったか．
    pub mirrored: bool,
    /// `Blob47` で突き合わせた組の数 (同じ象限 ・状態が 2 度以上現れた回数)．
    pub cross_checked: usize,
}

/// シートの並びから象限を取り出す．
///
/// **絵の中身を一切推測しない** — 枚数と順番だけで決める (設計書 6.8) ．
pub fn import_quadrants(
    pieces: &[IndexedCanvas],
    layout: ImportLayout,
    tile: u32,
) -> Result<(QuadrantArt, ImportReport)> {
    if pieces.len() != layout.expected() {
        return Err(CoreError::ImportWrongCount {
            layout: layout.as_str().to_string(),
            expected: layout.expected(),
            actual: pieces.len(),
        });
    }
    if tile == 0 || !tile.is_multiple_of(2) {
        return Err(CoreError::AutotileOddTile { tile });
    }
    let half = tile / 2;

    let mut art = QuadrantArt::new();
    let mut cross_checked = 0usize;
    let mut mirrored = false;

    match layout {
        ImportLayout::Quadrants5 => {
            let base: BTreeMap<CornerState, IndexedCanvas> =
                STATES.into_iter().zip(pieces.iter().cloned()).collect();
            for (state, c) in &base {
                check_size(c, half, state.as_str())?;
            }
            art = mirror_to_all_quadrants(&base);
            mirrored = true;
        }
        ImportLayout::Quadrants20 => {
            let mut it = pieces.iter();
            for q in QUADRANTS {
                for state in STATES {
                    let c = it.next().expect("枚数は確かめてある").clone();
                    check_size(&c, half, &format!("{}/{}", q.as_str(), state.as_str()))?;
                    art.insert((q, state), c);
                }
            }
        }
        ImportLayout::Blob47 => {
            for (mask, tile_art) in blob_masks().into_iter().zip(pieces) {
                check_size(tile_art, tile, &format!("mask {mask:#04x}"))?;
                for q in QUADRANTS {
                    let state = corner_state(mask, q);
                    let (ox, oy) = q.origin(half);
                    let cut = crop(tile_art, ox, oy, half);
                    match art.get(&(q, state)) {
                        None => {
                            art.insert((q, state), cut);
                        }
                        Some(existing) => {
                            // **突き合わせる** — 食い違うなら並びが違うか素材が壊れている
                            if existing.pixels() != cut.pixels() {
                                return Err(CoreError::ImportInconsistent {
                                    quadrant: q.as_str().to_string(),
                                    state: state.as_str().to_string(),
                                    mask,
                                });
                            }
                            cross_checked += 1;
                        }
                    }
                }
            }
        }
    }

    Ok((
        art,
        ImportReport {
            layout,
            pieces: 20,
            mirrored,
            cross_checked,
        },
    ))
}

fn check_size(c: &IndexedCanvas, expected: u32, what: &str) -> Result<()> {
    if c.width() != expected || c.height() != expected {
        return Err(CoreError::AutotileQuadrantSize {
            quadrant: what.to_string(),
            width: c.width(),
            height: c.height(),
            expected,
        });
    }
    Ok(())
}

fn crop(src: &IndexedCanvas, ox: u32, oy: u32, size: u32) -> IndexedCanvas {
    let mut out = IndexedCanvas::filled(size, size, 0);
    out.set_transparent(src.transparent());
    for y in 0..size {
        for x in 0..size {
            out.set(
                x as i32,
                y as i32,
                src.get((ox + x) as i32, (oy + y) as i32).unwrap_or(0),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art_of(size: u32, fill: u8) -> IndexedCanvas {
        IndexedCanvas::filled(size, size, fill).with_transparent(Some(0))
    }

    /// **壊れると: 縮約の規則が変わったのに気付かない．**
    ///
    /// 47 は設計書 6.8 が挙げている数え上げの結果であって，調整できる値ではない．
    #[test]
    fn the_blob_reduction_gives_exactly_forty_seven_masks() {
        let masks = blob_masks();
        assert_eq!(masks.len(), 47);
        // 昇順で重複が無い (設計書 6.15 規則 1)
        assert!(masks.windows(2).all(|w| w[0] < w[1]));
        // 正規形は冪等である
        for m in 0u16..=255 {
            let c = canonical_mask(m as u8);
            assert_eq!(canonical_mask(c), c, "mask {m} の正規形が安定していない");
            assert!(masks.contains(&c));
        }
    }

    /// **壊れると: 意味を持つ対角ビットまで落とし，内側と凹角が区別できなくなる．**
    #[test]
    fn a_diagonal_survives_only_when_both_neighbouring_edges_are_set() {
        // 北と東が立っていれば北東は残る
        assert_eq!(canonical_mask(N | E | NE), N | E | NE);
        // 北だけなら北東は落ちる
        assert_eq!(canonical_mask(N | NE), N);
        // 東だけでも落ちる
        assert_eq!(canonical_mask(E | NE), E);
        // 関係の無い対角は落ちる
        assert_eq!(canonical_mask(N | E | SW), N | E);
    }

    /// **壊れると: 象限の状態が 5 通りにならず，入力の枚数が合わなくなる．**
    ///
    /// 8 通りのうち 3 通りが «内側» へ縮退する — 辺が揃っていなければ対角は効かない．
    #[test]
    fn a_corner_has_exactly_five_states() {
        let mut seen: Vec<CornerState> = Vec::new();
        for m in 0u16..=255 {
            let s = corner_state(m as u8, Quadrant::NW);
            if !seen.contains(&s) {
                seen.push(s);
            }
        }
        assert_eq!(seen.len(), 5);

        // 辺が揃っていなければ対角の有無で状態が変わらない
        for q in QUADRANTS {
            let (h, v, d) = q.bits();
            assert_eq!(corner_state(h, q), corner_state(h | d, q));
            assert_eq!(corner_state(v, q), corner_state(v | d, q));
            assert_eq!(corner_state(0, q), corner_state(d, q));
            // 揃っていれば変わる
            assert_eq!(corner_state(h | v, q), CornerState::Concave);
            assert_eq!(corner_state(h | v | d, q), CornerState::Inner);
        }
    }

    /// **壊れると: 象限の絵が足りないまま «47 枚できた» と言う．**
    /// どれが足りないかを 1 枚ずつ並べないと，直すのに 20 回試すことになる．
    #[test]
    fn missing_quadrant_art_is_reported_all_at_once() {
        let mut art = QuadrantArt::new();
        art.insert((Quadrant::NW, CornerState::Convex), art_of(8, 1));
        let err = build(&art, 16).expect_err("足りない");
        let CoreError::AutotileMissingQuadrants { missing } = err else {
            panic!("欠けの報告になっていない");
        };
        assert_eq!(missing.len(), 19, "20 通りのうち 1 つだけ渡した");
        assert!(missing.iter().any(|s| s == "NW/edge_h"));
    }

    /// **壊れると: 5 枚入力の自動ミラーが働かず，20 枚描かないと使えない**
    /// (設計書 4.3 «陰影を持たない素材では 5 枚入力で運用できる») ．
    #[test]
    fn five_frames_expand_to_all_four_quadrants_by_mirroring() {
        let mut base = BTreeMap::new();
        for (i, state) in STATES.into_iter().enumerate() {
            base.insert(state, art_of(8, i as u8 + 1));
        }
        let art = mirror_to_all_quadrants(&base);
        assert_eq!(art.len(), 20);
        let (tiles, n) = build(&art, 16).expect("組める");
        assert_eq!(n, 47);
        assert_eq!(tiles.len(), 47);
        assert!(tiles.windows(2).all(|w| w[0].0 < w[1].0), "bitmask の昇順");
        for (_, t) in &tiles {
            assert_eq!((t.width(), t.height()), (16, 16));
        }
    }

    /// **壊れると: 自動ミラーが左右を入れ替えていない (何もしていない)．**
    #[test]
    fn mirroring_actually_flips_the_quadrant_art() {
        // 左半分だけ塗った 4x4
        let mut src = IndexedCanvas::filled(4, 4, 0).with_transparent(Some(0));
        for y in 0..4 {
            src.set(0, y, 1);
            src.set(1, y, 1);
        }
        let mut base = BTreeMap::new();
        for state in STATES {
            base.insert(state, src.clone());
        }
        let art = mirror_to_all_quadrants(&base);
        let nw = &art[&(Quadrant::NW, CornerState::Convex)];
        let ne = &art[&(Quadrant::NE, CornerState::Convex)];
        assert_eq!(nw.get(0, 0), Some(1));
        assert_eq!(ne.get(0, 0), Some(0), "NE は左右が入れ替わっているはず");
        assert_eq!(ne.get(3, 0), Some(1));
    }

    /// **壊れると: 奇数のタイルを受け取って象限が半分にならず，1 列ずれる．**
    #[test]
    fn an_odd_tile_size_is_an_error() {
        let mut base = BTreeMap::new();
        for state in STATES {
            base.insert(state, art_of(8, 1));
        }
        let art = mirror_to_all_quadrants(&base);
        assert!(matches!(
            build(&art, 15),
            Err(CoreError::AutotileOddTile { .. })
        ));
    }

    /// **壊れると: 象限の絵の大きさがタイルの半分でないまま組み，はみ出すか隙間ができる．**
    #[test]
    fn quadrant_art_must_be_half_the_tile() {
        let mut base = BTreeMap::new();
        for state in STATES {
            base.insert(state, art_of(8, 1));
        }
        let art = mirror_to_all_quadrants(&base);
        assert!(matches!(
            build(&art, 32),
            Err(CoreError::AutotileQuadrantSize { .. })
        ));
        assert!(build(&art, 16).is_ok());
    }

    /// **壊れると: 設計書 4.3 / D45 の «偶数幅だと反復で連結する» を信じたまま
    /// 位相バリアントを実装し，合っている継ぎ目を壊す．**
    ///
    /// 測ると主張は逆だった (D105) — 幅が偶数だからこそ位相が続く．
    #[test]
    fn repeating_an_even_width_dithered_tile_does_not_connect_the_dots() {
        // 16x16 の市松
        let mut t = IndexedCanvas::filled(16, 16, 0);
        for y in 0..16i32 {
            for x in 0..16i32 {
                t.set(x, y, ((x + y) % 2) as u8);
            }
        }
        // 2 枚並べて継ぎ目を見る
        let mut pair = IndexedCanvas::filled(32, 16, 0);
        for y in 0..16i32 {
            for x in 0..16i32 {
                let v = t.get(x, y).expect("範囲内");
                pair.set(x, y, v);
                pair.set(x + 16, y, v);
            }
        }
        let touching = (0..16)
            .filter(|&y| pair.get(15, y) == pair.get(16, y))
            .count();
        assert_eq!(touching, 0, "偶数幅の反復では継ぎ目が合う");

        // 鏡像を並べると全行で同色が隣り合う
        let mut mirrored = IndexedCanvas::filled(32, 16, 0);
        for y in 0..16i32 {
            for x in 0..16i32 {
                mirrored.set(x, y, t.get(x, y).expect("範囲内"));
                mirrored.set(x + 16, y, t.get(15 - x, y).expect("範囲内"));
            }
        }
        let touching = (0..16)
            .filter(|&y| mirrored.get(15, y) == mirrored.get(16, y))
            .count();
        assert_eq!(touching, 16, "鏡像を隣に置くと継ぎ目で必ず同色が並ぶ");
    }

    /// **壊れると: 自動ミラーがディザの位相を壊していることに気付かない．**
    ///
    /// 市松の象限を鏡像で組むと，タイルの内側の継ぎ目で 1 画素の縞が 2 画素になる．
    #[test]
    fn auto_mirroring_a_dithered_quadrant_doubles_the_stripe_at_the_seam() {
        let mut q = IndexedCanvas::filled(8, 8, 0);
        for y in 0..8i32 {
            for x in 0..8i32 {
                q.set(x, y, ((x + y) % 2) as u8);
            }
        }
        let base: BTreeMap<CornerState, IndexedCanvas> =
            STATES.into_iter().map(|s| (s, q.clone())).collect();
        let art = mirror_to_all_quadrants(&base);
        let (tiles, _) = build(&art, 16).expect("組める");
        let doubled: usize = tiles.iter().map(|(_, t)| seam_doubled(t)).sum();
        assert!(
            doubled > 0,
            "鏡像で組んだディザの継ぎ目が 1 件も検出されない"
        );
        // 1 枚あたり縦 16 + 横 16 = 32 件
        assert_eq!(seam_doubled(&tiles[0].1), 32);
    }

    /// **壊れると: ディザを持たない素材にまで «継ぎ目が壊れた» と言う．**
    /// 元から同じ色が続いていれば，列が複製されても見た目は変わらない．
    #[test]
    fn solid_quadrants_report_no_seam_damage() {
        let base: BTreeMap<CornerState, IndexedCanvas> = STATES
            .into_iter()
            .map(|s| (s, IndexedCanvas::filled(8, 8, 3)))
            .collect();
        let art = mirror_to_all_quadrants(&base);
        let (tiles, _) = build(&art, 16).expect("組める");
        for (mask, t) in &tiles {
            assert_eq!(seam_doubled(t), 0, "mask {mask:#04x} で誤検出");
        }
    }

    /// **壊れると: 象限を全部描いた (自動ミラーを使わない) 素材にも鳴る．**
    #[test]
    fn explicitly_drawn_quadrants_can_keep_the_dither_phase() {
        // 4 象限とも «タイル全体で連続する市松» になるように描く
        let mut art = QuadrantArt::new();
        for q in QUADRANTS {
            let (ox, oy) = match q {
                Quadrant::NW => (0i32, 0i32),
                Quadrant::NE => (8, 0),
                Quadrant::SW => (0, 8),
                Quadrant::SE => (8, 8),
            };
            let mut c = IndexedCanvas::filled(8, 8, 0);
            for y in 0..8i32 {
                for x in 0..8i32 {
                    c.set(x, y, ((x + ox + y + oy) % 2) as u8);
                }
            }
            for state in STATES {
                art.insert((q, state), c.clone());
            }
        }
        let (tiles, _) = build(&art, 16).expect("組める");
        for (mask, t) in &tiles {
            assert_eq!(
                seam_doubled(t),
                0,
                "mask {mask:#04x} — 位相を揃えて描けば継ぎ目は壊れない"
            );
        }
    }

    /// **壊れると: 組んだ 47 枚から象限を取り出せない．**
    ///
    /// `build` の逆が `import_quadrants` である — **往復が合わないなら
    /// どちらかが壊れている**．設計書 6.8 が恐れる «47 枚が静かに壊れる» は，
    /// この往復で捕まえられる．
    #[test]
    fn importing_our_own_forty_seven_tiles_reproduces_the_quadrants() {
        // 象限ごとに違う絵にして «取り違え» が起きたら分かるようにする
        let mut art = QuadrantArt::new();
        for (qi, q) in QUADRANTS.into_iter().enumerate() {
            for (si, state) in STATES.into_iter().enumerate() {
                let mut c = IndexedCanvas::filled(8, 8, 0);
                for y in 0..8i32 {
                    for x in 0..8i32 {
                        c.set(x, y, (qi * 5 + si) as u8 + (x == y) as u8 * 100);
                    }
                }
                art.insert((q, state), c);
            }
        }
        let (tiles, _) = build(&art, 16).expect("組める");
        let sheet: Vec<IndexedCanvas> = tiles.iter().map(|(_, t)| t.clone()).collect();

        let (back, report) =
            import_quadrants(&sheet, ImportLayout::Blob47, 16).expect("取り出せる");
        assert_eq!(report.pieces, 20);
        assert!(!report.mirrored);
        assert!(
            report.cross_checked > 0,
            "同じ象限 ・状態が 1 度も重ならないなら突き合わせていない"
        );
        assert_eq!(back.len(), 20);
        for (key, c) in &art {
            assert_eq!(
                back[key].pixels(),
                c.pixels(),
                "象限 {}/{} が戻らない",
                key.0.as_str(),
                key.1.as_str()
            );
        }
    }

    /// **壊れると: 象限に分解できない素材を «取り出せた» ことにして，
    /// 47 枚が静かに壊れる** (設計書 6.8 が名指ししている失敗) ．
    #[test]
    fn a_sheet_that_does_not_decompose_into_quadrants_is_an_error() {
        let base: BTreeMap<CornerState, IndexedCanvas> = STATES
            .into_iter()
            .enumerate()
            .map(|(i, s)| (s, IndexedCanvas::filled(8, 8, i as u8)))
            .collect();
        let art = mirror_to_all_quadrants(&base);
        let (tiles, _) = build(&art, 16).expect("組める");
        let mut sheet: Vec<IndexedCanvas> = tiles.iter().map(|(_, t)| t.clone()).collect();
        // 1 枚だけ書き換える — 同じ (象限，状態) が食い違うようになる
        sheet[10].set(0, 0, 200);
        let err = import_quadrants(&sheet, ImportLayout::Blob47, 16).expect_err("食い違う");
        assert!(matches!(err, CoreError::ImportInconsistent { .. }));
    }

    /// **壊れると: 枚数の違うシートを黙って受け取り，並びがずれたまま通る．**
    #[test]
    fn the_layout_decides_how_many_pieces_are_required() {
        let one = vec![IndexedCanvas::filled(8, 8, 1)];
        for layout in [
            ImportLayout::Quadrants5,
            ImportLayout::Quadrants20,
            ImportLayout::Blob47,
        ] {
            assert!(matches!(
                import_quadrants(&one, layout, 16),
                Err(CoreError::ImportWrongCount { .. })
            ));
        }
        assert_eq!(ImportLayout::Quadrants5.expected(), 5);
        assert_eq!(ImportLayout::Quadrants20.expected(), 20);
        assert_eq!(ImportLayout::Blob47.expected(), 47);
    }

    /// **壊れると: 5 枚の並びが自動ミラーを使ったことを報告しない．**
    /// 報告しないと，呼ぶ側がルール 7 とディザの検査を掛ける相手を見失う．
    #[test]
    fn the_five_piece_layout_reports_that_it_mirrored() {
        let pieces: Vec<IndexedCanvas> = (0..5)
            .map(|i| IndexedCanvas::filled(8, 8, i as u8))
            .collect();
        let (art, report) =
            import_quadrants(&pieces, ImportLayout::Quadrants5, 16).expect("取り出せる");
        assert!(report.mirrored);
        assert_eq!(art.len(), 20);

        // 20 枚の並びはミラーを使わない
        let pieces: Vec<IndexedCanvas> = (0..20)
            .map(|i| IndexedCanvas::filled(8, 8, i as u8))
            .collect();
        let (art, report) =
            import_quadrants(&pieces, ImportLayout::Quadrants20, 16).expect("取り出せる");
        assert!(!report.mirrored);
        // 象限が外側 ・状態が内側の順で読む
        assert_eq!(art[&(Quadrant::NW, CornerState::Convex)].get(0, 0), Some(0));
        assert_eq!(art[&(Quadrant::NE, CornerState::Convex)].get(0, 0), Some(5));
        assert_eq!(
            art[&(Quadrant::SE, CornerState::Concave)].get(0, 0),
            Some(19)
        );
    }

    fn piece(name: &str, quadrant: Option<Quadrant>, fill: u8) -> Piece {
        Piece {
            name: name.to_string(),
            quadrant,
            art: art_of(8, fill),
        }
    }

    /// **壊れると: 設計書 4.3 段 3 が通り，手描きと自動ミラーが混在する．**
    /// 混在すると陰影の整合が象限ごとに変わる．
    #[test]
    fn a_partly_specified_quadrant_group_is_an_error() {
        let pieces = vec![
            piece("inner", Some(Quadrant::NW), 1),
            piece("inner", None, 2),
        ];
        let err = resolve(&pieces).expect_err("混在は許さない");
        assert!(matches!(err, CoreError::QuadrantPartial { .. }));
    }

    /// **壊れると: 象限を明示したのに網羅していない入力が通り，残りが黙って欠ける．**
    #[test]
    fn specified_quadrants_must_cover_all_four() {
        let pieces = vec![
            piece("inner", Some(Quadrant::NW), 1),
            piece("inner", Some(Quadrant::NE), 2),
        ];
        let err = resolve(&pieces).expect_err("網羅していない");
        let CoreError::AutotileQuadrantsNotCovered { missing, .. } = err else {
            panic!("欠けの報告になっていない");
        };
        assert_eq!(missing, vec!["SW", "SE"]);
    }

    /// **壊れると: 象限を全部明示したのに自動ミラーが働き，手描きが上書きされる**
    /// (設計書 4.3 段 1 «自動ミラーは行わない») ．
    #[test]
    fn fully_specified_quadrants_do_not_trigger_the_auto_mirror() {
        let mut pieces = Vec::new();
        for state in STATES {
            for (i, q) in QUADRANTS.into_iter().enumerate() {
                pieces.push(piece(state.as_str(), Some(q), i as u8 + 1));
            }
        }
        let (art, used_mirror) = resolve(&pieces).expect("解決できる");
        assert!(!used_mirror, "全象限を明示したのでミラーは要らない");
        assert_eq!(art.len(), 20);
        // NE は «NW を反転したもの» ではなく，渡した絵そのものである
        assert_eq!(art[&(Quadrant::NE, CornerState::Inner)].get(0, 0), Some(2));
    }

    /// **壊れると: 5 枚入力で自動ミラーが働かない，または «働いた» と報告しない．**
    /// 報告しないと，呼ぶ側がルール 7 を掛ける相手を見失う (設計書 4.3) ．
    #[test]
    fn five_unspecified_frames_use_the_auto_mirror_and_say_so() {
        let pieces: Vec<Piece> = STATES
            .into_iter()
            .enumerate()
            .map(|(i, s)| piece(s.as_str(), None, i as u8 + 1))
            .collect();
        let (art, used_mirror) = resolve(&pieces).expect("解決できる");
        assert!(used_mirror, "自動ミラーを使ったと報告すること");
        assert_eq!(art.len(), 20);
        assert!(build(&art, 16).is_ok());
    }

    /// **壊れると: 綴りを間違えた状態名が黙って捨てられ，欠けとして後で出る．**
    #[test]
    fn an_unknown_state_name_is_an_error_naming_the_known_ones() {
        let pieces = vec![piece("corner_convex_", None, 1)];
        let err = resolve(&pieces).expect_err("知らない名前");
        let CoreError::AutotileUnknownState { known, .. } = err else {
            panic!("名前の誤りとして報告されていない");
        };
        assert_eq!(known.len(), 5);
    }

    /// **壊れると: 象限が正しい位置に置かれない．**
    /// 4 象限に違う色を置いて，組んだタイルの四隅を見る．
    #[test]
    fn each_quadrant_lands_in_its_own_corner() {
        let mut art = QuadrantArt::new();
        for (i, q) in QUADRANTS.into_iter().enumerate() {
            for state in STATES {
                art.insert((q, state), art_of(8, i as u8 + 1));
            }
        }
        let (tiles, _) = build(&art, 16).expect("組める");
        let (_, t) = &tiles[0];
        assert_eq!(t.get(0, 0), Some(1), "NW");
        assert_eq!(t.get(15, 0), Some(2), "NE");
        assert_eq!(t.get(0, 15), Some(3), "SW");
        assert_eq!(t.get(15, 15), Some(4), "SE");
    }
}
