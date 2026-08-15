//! 作業層のフレームとレイヤ (設計書 3.1)．
//!
//! 保持層 (`pxsmith-io` の `Document`) が `.aseprite` の情報をそのまま持ち，こちらは
//! アルゴリズムが触る正規形を持つ．射影で失われる情報は保持層に残す (不変条件 2)．

use crate::canvas::{IndexedCanvas, RgbaCanvas};
use crate::error::{CoreError, Result};
use crate::math::UVec2;
use crate::palette::Palette;

/// ブレンドモードは自前で列挙せず `aseprite-io` の型を再輸出する (D53)．
///
/// 自前定義は第 2 の真実になり，保持層の不変条件 1 の趣旨に反する．
pub use aseprite::BlendMode;

/// フレームの役割 (D47)．lint のスコープ判定に使う．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum FrameKind {
    /// キーフレーム (既定)．
    #[default]
    Key,
    /// ブレイクダウン (明示された中間キーフレーム)．
    Breakdown,
    /// 中割り．一瞬しか映らないのでジャギー・AA 系の lint を適用しない．
    Inbetween,
}

impl FrameKind {
    /// 中間フレームか．**ルール 26 を外す判定に使う** (7.1 ・D47)．
    pub fn is_inbetween(self) -> bool {
        matches!(self, Self::Inbetween)
    }

    /// 静止画 lint (`keyframe` スコープ) の対象かどうか (設計書 7.1)．
    pub fn is_keyframe(self) -> bool {
        matches!(self, Self::Key | Self::Breakdown)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Breakdown => "breakdown",
            Self::Inbetween => "inbetween",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "key" => Some(Self::Key),
            "breakdown" => Some(Self::Breakdown),
            "inbetween" => Some(Self::Inbetween),
            _ => None,
        }
    }
}

/// 奥行き．`pxsmith atmos` の入力になる．
///
/// 並びは**手前から奥へ**である — `pxsmith atmos` の «奥へ行くほど霞む» はこの順に
/// 単調であることを要求する．
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Depth {
    Foreground,
    Midground,
    Background,
}

impl Depth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::Midground => "midground",
            Self::Background => "background",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "foreground" => Some(Self::Foreground),
            "midground" => Some(Self::Midground),
            "background" => Some(Self::Background),
            _ => None,
        }
    }
}

/// タイルセットの識別子．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TilesetId(pub u32);

/// タイルへの参照．反転フラグを含む (タイル ID は `u32`，D2)．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TileRef {
    pub id: u32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub flip_d: bool,
}

/// タイルマップの格子．**フラット化しない** (不変条件 3)．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileGrid {
    width: u32,
    height: u32,
    tiles: Vec<TileRef>,
}

impl TileGrid {
    pub fn from_tiles(width: u32, height: u32, tiles: Vec<TileRef>) -> Result<Self> {
        let expected = UVec2 {
            x: width,
            y: height,
        }
        .area();
        if tiles.len() != expected {
            return Err(CoreError::PixelCountMismatch {
                width,
                height,
                expected,
                actual: tiles.len(),
            });
        }
        Ok(Self {
            width,
            height,
            tiles,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn tiles(&self) -> &[TileRef] {
        &self.tiles
    }

    pub fn tiles_mut(&mut self) -> &mut [TileRef] {
        &mut self.tiles
    }

    pub fn get(&self, x: u32, y: u32) -> Option<TileRef> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.tiles
            .get(y as usize * self.width as usize + x as usize)
            .copied()
    }
}

/// レイヤの中身．
#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    Indexed(IndexedCanvas),
    Rgba(RgbaCanvas),
    Tiles { grid: TileGrid, set: TilesetId },
}

impl Surface {
    pub fn as_indexed(&self) -> Option<&IndexedCanvas> {
        match self {
            Self::Indexed(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_indexed_mut(&mut self) -> Option<&mut IndexedCanvas> {
        match self {
            Self::Indexed(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_tiles(&self) -> Option<(&TileGrid, TilesetId)> {
        match self {
            Self::Tiles { grid, set } => Some((grid, *set)),
            _ => None,
        }
    }
}

/// レイヤのメタ情報．
#[derive(Clone, Debug, PartialEq)]
pub struct LayerMeta {
    pub name: String,
    pub opacity: u8,
    pub blend: BlendMode,
    pub visible: bool,
    /// 親グループの名前を根から並べたもの．
    pub group_path: Vec<String>,
    /// 親グループの不透明度．**権威は保持層側にある** (不変条件 4)．
    /// 射影時に複製される読み取り専用の値であり，`merge_back` は書き戻さない．
    pub group_opacity: Vec<u8>,
    pub depth: Option<Depth>,
    /// 顔・目などサブピクセルの対象外にする指定 (D38)．
    pub subpixel_exclude: bool,
}

impl Default for LayerMeta {
    fn default() -> Self {
        Self {
            name: String::new(),
            opacity: 255,
            blend: BlendMode::Normal,
            visible: true,
            group_path: Vec::new(),
            group_opacity: Vec::new(),
            depth: None,
            subpixel_exclude: false,
        }
    }
}

impl LayerMeta {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// グループ不透明度を畳み込んだ実効不透明度．
    pub fn effective_opacity(&self) -> f32 {
        let mut v = self.opacity as f32 / 255.0;
        for &g in &self.group_opacity {
            v *= g as f32 / 255.0;
        }
        v
    }
}

/// レイヤ．
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub meta: LayerMeta,
    pub surface: Surface,
}

impl Layer {
    pub fn new(meta: LayerMeta, surface: Surface) -> Self {
        Self { meta, surface }
    }
}

/// 作業層のフレーム．
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub size: UVec2,
    pub layers: Vec<Layer>,
    pub palette: Palette,
    /// アニメーションのタイミング (D40)．
    pub duration_ms: u32,
    /// lint スコープの判定に使う (D47)．
    pub kind: FrameKind,
}

impl Frame {
    pub fn new(size: UVec2, palette: Palette) -> Self {
        Self {
            size,
            layers: Vec::new(),
            palette,
            duration_ms: 100,
            kind: FrameKind::Key,
        }
    }

    pub fn layer(&self, index: usize) -> Result<&Layer> {
        self.layers.get(index).ok_or(CoreError::LayerOutOfRange {
            index,
            len: self.layers.len(),
        })
    }

    pub fn layer_mut(&mut self, index: usize) -> Result<&mut Layer> {
        let len = self.layers.len();
        self.layers
            .get_mut(index)
            .ok_or(CoreError::LayerOutOfRange { index, len })
    }

    /// 名前でレイヤを探す．同名は最初の 1 つ．
    pub fn layer_by_name(&self, name: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.meta.name == name)
    }
}

/// フレーム列のパレットを 1 つに束ね，添字を付け替える．
///
/// `.aseprite` はスプライトに 1 つのパレットしか持てないので，別々の由来で
/// 作ったフレームを 1 本の列にするときに要る (`pxsmith anim tween` が中割りだけを
/// 陰影付けする場面など) ．
///
/// **束ねるのは «画素が実際に指している色» だけである** (D93 と同じ作法) —
/// 素材のパレットには使っていない色が普通に入っており，全項目を写そうとすると
/// 256 色を使い切る．
pub fn unify_palettes(frames: &mut [Frame]) -> Result<Palette> {
    let mut sources: Vec<(&IndexedCanvas, &Palette)> = Vec::new();
    for frame in frames.iter() {
        for layer in &frame.layers {
            if let Some(c) = layer.surface.as_indexed() {
                sources.push((c, &frame.palette));
            }
        }
    }
    let mut palette = Palette::extract_from(sources.iter().copied())?;
    let transparent = match palette.entries().iter().position(|c| c.a == 0) {
        Some(i) => i as u8,
        None => palette.push(crate::color::Rgba8::TRANSPARENT)?,
    };

    for frame in frames.iter_mut() {
        let mut used = [false; 256];
        for layer in &frame.layers {
            if let Some(c) = layer.surface.as_indexed() {
                for v in c.pixels() {
                    used[*v as usize] = true;
                }
            }
        }
        let mut map = vec![transparent; 256];
        for (i, u) in used.iter().enumerate() {
            if !u {
                continue;
            }
            // **黙って透明にしない** — 元の絵が壊れているなら，そう言う
            let color = frame
                .palette
                .get(i as u8)
                .ok_or(CoreError::PaletteIndexMissing {
                    index: i as u8,
                    len: frame.palette.len(),
                })?;
            if color.a == 0 {
                continue;
            }
            map[i] = palette
                .entries()
                .iter()
                .position(|d| *d == color)
                .ok_or(CoreError::ComposeColorLost { color })? as u8;
        }
        for layer in &mut frame.layers {
            if let Surface::Indexed(c) = &mut layer.surface {
                c.remap(&map)?;
                c.set_transparent(Some(transparent));
            }
        }
        frame.palette = palette.clone();
    }
    Ok(palette)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;

    fn frame() -> Frame {
        let palette = Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(255, 0, 0)]).unwrap();
        let mut f = Frame::new(UVec2 { x: 4, y: 4 }, palette);
        f.layers.push(Layer::new(
            LayerMeta::named("body"),
            Surface::Indexed(IndexedCanvas::filled(4, 4, 0).with_transparent(Some(0))),
        ));
        f
    }

    #[test]
    fn keyframe_scope_excludes_inbetween() {
        assert!(FrameKind::Key.is_keyframe());
        assert!(FrameKind::Breakdown.is_keyframe());
        assert!(!FrameKind::Inbetween.is_keyframe());
    }

    #[test]
    fn frame_kind_string_round_trip() {
        for k in [FrameKind::Key, FrameKind::Breakdown, FrameKind::Inbetween] {
            assert_eq!(FrameKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(FrameKind::parse("keyframe"), None);
    }

    #[test]
    fn layer_out_of_range_is_an_error_not_a_panic() {
        let f = frame();
        assert!(matches!(
            f.layer(3).unwrap_err(),
            CoreError::LayerOutOfRange { index: 3, len: 1 }
        ));
    }

    #[test]
    fn effective_opacity_folds_group_chain() {
        let mut m = LayerMeta::named("x");
        m.opacity = 255;
        m.group_opacity = vec![128, 255];
        assert!((m.effective_opacity() - 128.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn tile_grid_rejects_size_mismatch() {
        assert!(TileGrid::from_tiles(2, 2, vec![TileRef::default(); 3]).is_err());
        assert!(TileGrid::from_tiles(2, 2, vec![TileRef::default(); 4]).is_ok());
    }

    #[test]
    fn layer_lookup_by_name() {
        let f = frame();
        assert_eq!(f.layer_by_name("body"), Some(0));
        assert_eq!(f.layer_by_name("face"), None);
    }
}
