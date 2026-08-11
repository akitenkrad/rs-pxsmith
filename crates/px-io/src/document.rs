//! 保持層 (設計書 3.1)．
//!
//! [`Document`] は `aseprite-io` の型を薄く包むだけで独自フィールドを足さない
//! (不変条件 1)．射影で失われる情報は保持層に残し，[`Document::merge_back`] で
//! 保存する (不変条件 2)．
//!
//! # バイト一致往復
//!
//! `.aseprite` の読み書きは往復でバイト一致する必要がある (M0 の中核，R3・R15)．
//! そのために [`Document::merge_back`] は**変わっていないレイヤに一切触れない**．
//! 触らなければ元の圧縮バイト列・リンクセル・未知チャンクがそのまま残る．
//!
//! # 保持層へ書き戻せないもの
//!
//! `aseprite-io` 0.2 にはフレーム長・レイヤ属性の設定 API が無い．作業層でこれらを
//! 変えたまま `merge_back` すると，黙って落とさずエラーを返す
//! ([`IoError::UnsupportedWriteback`])．

use std::path::Path;

use aseprite::{AsepriteFile, CelKind, ColorMode, LayerKind, Pixels};
use px_core::canvas::{IndexedCanvas, RgbaCanvas};
use px_core::frame::{Layer, LayerMeta, Surface, TileGrid, TileRef, TilesetId};
use px_core::math::{IVec2, UVec2, ivec2, uvec2};
use px_core::{Frame, FrameId, Palette, Rgba8};

use crate::error::{IoError, Result};

/// 射影時の設定．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectOptions {
    /// パレットの半透明色を 128 を境に 2 値へ丸める．
    ///
    /// 既定は偽で，半透明色があればエラーにする．アルファ 2 値は作業層の不変条件
    /// (D4) なので，黙って丸めると色が静かに変わる．
    pub binarize_alpha: bool,
}

/// 保持層．`.aseprite` の内容をそのまま持つ．
#[derive(Clone, Debug)]
pub struct Document {
    raw: AsepriteFile,
}

impl Document {
    pub fn from_raw(raw: AsepriteFile) -> Self {
        Self { raw }
    }

    /// バイト列から読む．
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(Self {
            raw: AsepriteFile::from_reader(bytes)?,
        })
    }

    /// ファイルから読む．
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| IoError::File {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_bytes(&bytes)
    }

    /// バイト列へ書き出す．
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.raw.write_to(&mut out)?;
        Ok(out)
    }

    /// ファイルへ原子的に書き出す．
    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        crate::atomic::write(path, &self.to_bytes()?)
    }

    pub fn raw(&self) -> &AsepriteFile {
        &self.raw
    }

    /// 保持層を直接いじる口．独自フィールドを足さない限りにおいて使ってよい．
    pub fn raw_mut(&mut self) -> &mut AsepriteFile {
        &mut self.raw
    }

    pub fn size(&self) -> UVec2 {
        uvec2(self.raw.width() as u32, self.raw.height() as u32)
    }

    pub fn frame_count(&self) -> usize {
        self.raw.frames().len()
    }

    /// 中身を持つレイヤ (グループを除く) の保持層での添字．
    ///
    /// 作業層の [`Frame::layers`] はこの順に並ぶので，作業層の添字 `i` は
    /// `content_layers()[i]` で保持層の添字へ戻る．
    pub fn content_layers(&self) -> Vec<usize> {
        self.raw
            .layers()
            .iter()
            .enumerate()
            .filter(|(_, l)| !matches!(l.kind, LayerKind::Group))
            .map(|(i, _)| i)
            .collect()
    }

    /// 作業層へ射影する．
    pub fn project(&self, frame: FrameId) -> Result<Frame> {
        self.project_with(frame, ProjectOptions::default())
    }

    /// 設定を指定して作業層へ射影する．
    pub fn project_with(&self, frame: FrameId, opts: ProjectOptions) -> Result<Frame> {
        let len = self.frame_count();
        if frame.0 >= len {
            return Err(IoError::FrameOutOfRange {
                index: frame.0,
                len,
            });
        }

        let palette = self.project_palette(opts)?;
        let mut out = Frame::new(self.size(), palette);
        out.duration_ms = self.raw.frames()[frame.0].duration_ms as u32;

        for layer_index in self.content_layers() {
            let meta = self.project_meta(layer_index);
            let surface = self.project_surface(layer_index, frame.0)?;
            out.layers.push(Layer::new(meta, surface));
        }
        Ok(out)
    }

    fn project_palette(&self, opts: ProjectOptions) -> Result<Palette> {
        let mut entries = Vec::with_capacity(self.raw.palette().len());
        for (index, c) in self.raw.palette().iter().enumerate() {
            let a = match c.a {
                0 | 255 => c.a,
                a if opts.binarize_alpha => {
                    if a >= 128 {
                        255
                    } else {
                        0
                    }
                }
                a => return Err(IoError::NonBinaryPaletteAlpha { index, alpha: a }),
            };
            entries.push(Rgba8::new(c.r, c.g, c.b, a));
        }
        Ok(Palette::new(entries)?)
    }

    fn project_meta(&self, layer_index: usize) -> LayerMeta {
        let l = &self.raw.layers()[layer_index];
        let (group_path, group_opacity) = self.group_chain(layer_index);
        LayerMeta {
            name: l.name.clone(),
            opacity: l.opacity,
            blend: l.blend_mode,
            visible: l.visible,
            group_path,
            group_opacity,
            // depth と subpixel_exclude は `.aseprite` に対応する概念が無く，
            // L0 テキスト (設計書 4.1) かレシピが供給する．
            depth: None,
            subpixel_exclude: false,
        }
    }

    /// 親グループの名前と不透明度を根から並べる (不変条件 4)．
    fn group_chain(&self, layer_index: usize) -> (Vec<String>, Vec<u8>) {
        let layers = self.raw.layers();
        let mut names = Vec::new();
        let mut opacities = Vec::new();
        let mut cur = layers[layer_index].parent;
        // 自己参照や循環があっても止まるよう，レイヤ数を上限にする
        for _ in 0..layers.len() {
            let Some(i) = cur else { break };
            let Some(g) = layers.get(i) else { break };
            names.push(g.name.clone());
            opacities.push(g.opacity);
            cur = g.parent;
        }
        names.reverse();
        opacities.reverse();
        (names, opacities)
    }

    fn project_surface(&self, layer_index: usize, frame: usize) -> Result<Surface> {
        let layer = &self.raw.layers()[layer_index];
        let layer_ref = self
            .raw
            .layer_ref(layer_index)
            .expect("content_layers はグループを除いている");
        let cel = self.raw.resolve_cel(layer_ref, frame);

        if let LayerKind::Tilemap { tileset_index } = layer.kind {
            let grid = match cel.map(|c| &c.kind) {
                Some(CelKind::Tilemap {
                    width,
                    height,
                    tiles,
                    tile_id_bitmask,
                    x_flip_bitmask,
                    y_flip_bitmask,
                    d_flip_bitmask,
                    ..
                }) => {
                    let refs = tiles
                        .iter()
                        .map(|&w| TileRef {
                            id: w & tile_id_bitmask,
                            flip_x: w & x_flip_bitmask != 0,
                            flip_y: w & y_flip_bitmask != 0,
                            flip_d: w & d_flip_bitmask != 0,
                        })
                        .collect();
                    TileGrid::from_tiles(*width as u32, *height as u32, refs)?
                }
                _ => TileGrid::from_tiles(0, 0, Vec::new())?,
            };
            return Ok(Surface::Tiles {
                grid,
                set: TilesetId(tileset_index),
            });
        }

        let size = self.size();
        match self.raw.color_mode() {
            ColorMode::Indexed => {
                let t = self.raw.transparent_index();
                let mut canvas = IndexedCanvas::filled(size.x, size.y, t).with_transparent(Some(t));
                if let Some((pixels, at)) = cel_pixels(cel) {
                    let src = IndexedCanvas::from_pixels(
                        pixels.width as u32,
                        pixels.height as u32,
                        pixels.data.clone(),
                    )?;
                    canvas.blit(&src, at, false);
                }
                Ok(Surface::Indexed(canvas))
            }
            ColorMode::Rgba | ColorMode::Grayscale => {
                let mut canvas = RgbaCanvas::filled(size.x, size.y, Rgba8::TRANSPARENT);
                if let Some((pixels, at)) = cel_pixels(cel) {
                    let bpp = self.raw.color_mode().bytes_per_pixel();
                    for y in 0..pixels.height as i32 {
                        for x in 0..pixels.width as i32 {
                            let i = (y as usize * pixels.width as usize + x as usize) * bpp;
                            let c = match bpp {
                                4 => Rgba8::new(
                                    pixels.data[i],
                                    pixels.data[i + 1],
                                    pixels.data[i + 2],
                                    pixels.data[i + 3],
                                ),
                                2 => {
                                    let v = pixels.data[i];
                                    Rgba8::new(v, v, v, pixels.data[i + 1])
                                }
                                _ => Rgba8::TRANSPARENT,
                            };
                            canvas.set(at.x + x, at.y + y, c);
                        }
                    }
                }
                Ok(Surface::Rgba(canvas))
            }
            // `ColorMode` は #[non_exhaustive] なので，将来の追加をここで受け止める
            _ => Err(IoError::UnsupportedColorMode("未知の")),
        }
    }

    /// 作業層のフレーム列から新しい `.aseprite` を組み立てる．
    ///
    /// L0 テキストからの取り込み (`px text import`) に使う．保持層に相当する元が
    /// 無いので**ここでだけは作業層が真になる**．全フレームで大きさ・レイヤ構成・
    /// パレットが揃っている必要がある．
    pub fn from_frames(frames: &[Frame]) -> Result<Self> {
        let Some(first) = frames.first() else {
            return Err(IoError::FrameOutOfRange { index: 0, len: 0 });
        };
        check_cel_size(first.size.x, first.size.y)?;
        if first.palette.is_normalized() {
            return Err(IoError::NormalizedPaletteWriteback);
        }

        for f in frames {
            if f.size != first.size {
                return Err(IoError::SizeMismatch {
                    expected: (first.size.x, first.size.y),
                    actual: (f.size.x, f.size.y),
                });
            }
            if f.layers.len() != first.layers.len() {
                return Err(IoError::LayerCountMismatch {
                    expected: first.layers.len(),
                    actual: f.layers.len(),
                });
            }
        }

        let mut raw =
            AsepriteFile::new(first.size.x as u16, first.size.y as u16, ColorMode::Indexed);
        raw.set_palette(
            &first
                .palette
                .entries()
                .iter()
                .map(|c| aseprite::Color {
                    r: c.r,
                    g: c.g,
                    b: c.b,
                    a: c.a,
                    name: None,
                })
                .collect::<Vec<_>>(),
        )?;

        // 透明添字はキャンバスのメタから取る．無ければ 0 のままにする (D3)
        if let Some(t) = first
            .layers
            .iter()
            .find_map(|l| l.surface.as_indexed().and_then(|c| c.transparent()))
        {
            raw.set_transparent_index(t);
        }

        for layer in &first.layers {
            raw.add_layer(&layer.meta.name);
        }
        for f in frames {
            raw.add_frame(f.duration_ms.min(u16::MAX as u32) as u16);
        }

        let mut doc = Self { raw };
        for (frame_index, f) in frames.iter().enumerate() {
            for (layer_index, layer) in f.layers.iter().enumerate() {
                doc.write_surface(layer_index, frame_index, &layer.surface)?;
            }
        }
        Ok(doc)
    }

    /// 作業層の変更を保持層へ書き戻す (不変条件 2)．
    ///
    /// **変わっていないレイヤには触れない**ので，リンクセル・元の圧縮バイト列・
    /// 未知チャンクはそのまま残る．
    pub fn merge_back(&mut self, frame: FrameId, f: &Frame) -> Result<()> {
        let len = self.frame_count();
        if frame.0 >= len {
            return Err(IoError::FrameOutOfRange {
                index: frame.0,
                len,
            });
        }
        if f.size != self.size() {
            return Err(IoError::SizeMismatch {
                expected: (self.size().x, self.size().y),
                actual: (f.size.x, f.size.y),
            });
        }
        if f.palette.is_normalized() {
            return Err(IoError::NormalizedPaletteWriteback);
        }

        let content = self.content_layers();
        if content.len() != f.layers.len() {
            return Err(IoError::LayerCountMismatch {
                expected: content.len(),
                actual: f.layers.len(),
            });
        }
        if f.duration_ms != self.raw.frames()[frame.0].duration_ms as u32 {
            return Err(IoError::UnsupportedWriteback {
                field: "フレームの duration_ms",
            });
        }

        for (working_index, &layer_index) in content.iter().enumerate() {
            let working = &f.layers[working_index];
            let original_meta = self.project_meta(layer_index);
            if meta_differs(&original_meta, &working.meta) {
                return Err(IoError::UnsupportedWriteback {
                    field: "レイヤの名前・不透明度・ブレンドモード・可視性",
                });
            }

            let current = self.project_surface(layer_index, frame.0)?;
            if std::mem::discriminant(&current) != std::mem::discriminant(&working.surface) {
                return Err(IoError::SurfaceKindMismatch {
                    name: working.meta.name.clone(),
                });
            }
            if current == working.surface {
                // 触らないことがバイト一致往復の条件である
                continue;
            }
            self.write_surface(layer_index, frame.0, &working.surface)?;
        }

        // パレットは全レイヤの後に書く．色数が変わると添字の意味が変わるため．
        let current_palette = self.project_palette(ProjectOptions {
            binarize_alpha: true,
        })?;
        if current_palette.entries() != f.palette.entries() {
            let colors: Vec<aseprite::Color> = f
                .palette
                .entries()
                .iter()
                .map(|c| aseprite::Color {
                    r: c.r,
                    g: c.g,
                    b: c.b,
                    a: c.a,
                    name: None,
                })
                .collect();
            self.raw.set_palette(&colors)?;
        }
        Ok(())
    }

    fn write_surface(&mut self, layer_index: usize, frame: usize, surface: &Surface) -> Result<()> {
        let layer_ref = self
            .raw
            .layer_ref(layer_index)
            .expect("content_layers はグループを除いている");

        match surface {
            Surface::Indexed(canvas) => {
                let t = self.raw.transparent_index();
                // 透明な余白を含めずに済むよう，中身の外接矩形だけを cel にする．
                let rect = canvas
                    .opaque_bbox()
                    .unwrap_or(px_core::IRect::new(0, 0, 1, 1));
                check_cel_size(rect.w, rect.h)?;
                let cropped = canvas.crop(rect, t);
                let pixels = Pixels::new(
                    cropped.pixels().to_vec(),
                    rect.w as u16,
                    rect.h as u16,
                    ColorMode::Indexed,
                )?;
                self.raw
                    .set_cel(layer_ref, frame, pixels, rect.x as i16, rect.y as i16)?;
                Ok(())
            }
            Surface::Rgba(canvas) => {
                if self.raw.color_mode() != ColorMode::Rgba {
                    return Err(IoError::UnsupportedColorMode("グレースケールの"));
                }
                let w = canvas.width();
                let h = canvas.height();
                check_cel_size(w, h)?;
                let mut data = Vec::with_capacity(w as usize * h as usize * 4);
                for c in canvas.pixels() {
                    data.extend_from_slice(&[c.r, c.g, c.b, c.a]);
                }
                let pixels = Pixels::new(data, w as u16, h as u16, ColorMode::Rgba)?;
                self.raw.set_cel(layer_ref, frame, pixels, 0, 0)?;
                Ok(())
            }
            Surface::Tiles { grid, .. } => {
                check_cel_size(grid.width(), grid.height())?;
                // set_tilemap_cel は標準のビットマスクで書き出す
                let tiles: Vec<u32> = grid
                    .tiles()
                    .iter()
                    .map(|t| {
                        (t.id & 0x1fff_ffff)
                            | if t.flip_x { 0x2000_0000 } else { 0 }
                            | if t.flip_y { 0x4000_0000 } else { 0 }
                            | if t.flip_d { 0x8000_0000 } else { 0 }
                    })
                    .collect();
                self.raw.set_tilemap_cel(
                    layer_ref,
                    frame,
                    tiles,
                    grid.width() as u16,
                    grid.height() as u16,
                    0,
                    0,
                )?;
                Ok(())
            }
        }
    }
}

fn check_cel_size(w: u32, h: u32) -> Result<()> {
    if w > u16::MAX as u32 || h > u16::MAX as u32 {
        Err(IoError::CelTooLarge { w, h })
    } else {
        Ok(())
    }
}

fn cel_pixels(cel: Option<&aseprite::Cel>) -> Option<(&Pixels, IVec2)> {
    match cel.map(|c| &c.kind) {
        Some(CelKind::Raw { pixels, x, y }) => Some((pixels, ivec2(*x as i32, *y as i32))),
        Some(CelKind::Compressed { pixels, x, y, .. }) => {
            Some((pixels, ivec2(*x as i32, *y as i32)))
        }
        _ => None,
    }
}

/// 保持層へ書き戻せない属性が変わっているか．
fn meta_differs(a: &LayerMeta, b: &LayerMeta) -> bool {
    a.name != b.name || a.opacity != b.opacity || a.blend != b.blend || a.visible != b.visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use aseprite::ColorMode;
    use px_core::{IndexedCanvas, Rgba8};

    /// 添字 0 = 透明，1 = 赤，2 = 緑の 8x8 インデックスカラー 2 フレーム．
    fn indexed_doc() -> Document {
        let mut raw = AsepriteFile::new(8, 8, ColorMode::Indexed);
        raw.set_palette(&[
            aseprite::Color {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
                name: None,
            },
            aseprite::Color {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
                name: None,
            },
            aseprite::Color {
                r: 0,
                g: 255,
                b: 0,
                a: 255,
                name: None,
            },
        ])
        .unwrap();
        let body = raw.add_layer("body");
        let f0 = raw.add_frame(83);
        let f1 = raw.add_frame(83);

        let mut data = vec![0u8; 4 * 4];
        data[5] = 1;
        data[6] = 2;
        raw.set_cel(
            body,
            f0,
            Pixels::new(data, 4, 4, ColorMode::Indexed).unwrap(),
            2,
            1,
        )
        .unwrap();
        raw.set_linked_cel(body, f1, f0).unwrap();
        Document::from_raw(raw)
    }

    #[test]
    fn byte_round_trip_of_generated_file() {
        let doc = indexed_doc();
        let bytes = doc.to_bytes().unwrap();
        let reparsed = Document::from_bytes(&bytes).unwrap();
        assert_eq!(reparsed.to_bytes().unwrap(), bytes);
    }

    #[test]
    fn projection_places_cel_at_its_offset() {
        let doc = indexed_doc();
        let f = doc.project(FrameId(0)).unwrap();
        assert_eq!(f.size, uvec2(8, 8));
        assert_eq!(f.layers.len(), 1);
        assert_eq!(f.duration_ms, 83);
        let c = f.layers[0].surface.as_indexed().unwrap();
        // cel は (2, 1) 起点，cel 内 (1, 1) が添字 1
        assert_eq!(c.get(3, 2), Some(1));
        assert_eq!(c.get(4, 2), Some(2));
        assert_eq!(c.get(0, 0), Some(0));
        assert_eq!(c.transparent(), Some(0));
    }

    #[test]
    fn linked_cel_projects_the_same_pixels() {
        let doc = indexed_doc();
        let a = doc.project(FrameId(0)).unwrap();
        let b = doc.project(FrameId(1)).unwrap();
        assert_eq!(
            a.layers[0].surface.as_indexed().unwrap().pixels(),
            b.layers[0].surface.as_indexed().unwrap().pixels()
        );
    }

    #[test]
    fn merge_back_without_changes_keeps_bytes_identical() {
        let mut doc = indexed_doc();
        let before = doc.to_bytes().unwrap();
        let f = doc.project(FrameId(0)).unwrap();
        doc.merge_back(FrameId(0), &f).unwrap();
        assert_eq!(
            doc.to_bytes().unwrap(),
            before,
            "無変更の往復でバイトが変わった"
        );
    }

    #[test]
    fn merge_back_without_changes_preserves_linked_cel() {
        let mut doc = indexed_doc();
        let f = doc.project(FrameId(1)).unwrap();
        doc.merge_back(FrameId(1), &f).unwrap();
        let layer_ref = doc.raw().layer_ref(0).unwrap();
        assert!(
            matches!(
                doc.raw().cel(layer_ref, 1).map(|c| &c.kind),
                Some(CelKind::Linked { .. })
            ),
            "無変更なのにリンクセルが実体化した"
        );
    }

    #[test]
    fn merge_back_then_project_returns_the_same_frame() {
        let mut doc = indexed_doc();
        let mut f = doc.project(FrameId(0)).unwrap();
        f.layers[0].surface.as_indexed_mut().unwrap().set(7, 7, 2);
        doc.merge_back(FrameId(0), &f).unwrap();
        let again = doc.project(FrameId(0)).unwrap();
        assert_eq!(again.layers[0].surface, f.layers[0].surface);
    }

    #[test]
    fn merge_back_survives_a_write_read_cycle() {
        let mut doc = indexed_doc();
        let mut f = doc.project(FrameId(0)).unwrap();
        f.layers[0].surface.as_indexed_mut().unwrap().set(0, 7, 1);
        doc.merge_back(FrameId(0), &f).unwrap();

        let bytes = doc.to_bytes().unwrap();
        let reloaded = Document::from_bytes(&bytes).unwrap();
        assert_eq!(reloaded.project(FrameId(0)).unwrap(), f);
        assert_eq!(reloaded.to_bytes().unwrap(), bytes);
    }

    #[test]
    fn merge_back_writes_palette_changes() {
        let mut doc = indexed_doc();
        let mut f = doc.project(FrameId(0)).unwrap();
        f.palette.set_entry(1, Rgba8::rgb(1, 2, 3)).unwrap();
        doc.merge_back(FrameId(0), &f).unwrap();
        assert_eq!(
            doc.project(FrameId(0)).unwrap().palette.get(1).unwrap(),
            Rgba8::rgb(1, 2, 3)
        );
    }

    #[test]
    fn merge_back_rejects_normalized_palette() {
        let mut doc = indexed_doc();
        let mut f = doc.project(FrameId(0)).unwrap();
        f.palette.normalize_by_lightness().unwrap();
        assert!(matches!(
            doc.merge_back(FrameId(0), &f).unwrap_err(),
            IoError::NormalizedPaletteWriteback
        ));
    }

    #[test]
    fn merge_back_rejects_unwritable_metadata_change() {
        let mut doc = indexed_doc();
        let mut f = doc.project(FrameId(0)).unwrap();
        f.layers[0].meta.opacity = 128;
        assert!(matches!(
            doc.merge_back(FrameId(0), &f).unwrap_err(),
            IoError::UnsupportedWriteback { .. }
        ));
    }

    #[test]
    fn merge_back_rejects_duration_change() {
        let mut doc = indexed_doc();
        let mut f = doc.project(FrameId(0)).unwrap();
        f.duration_ms = 42;
        assert!(matches!(
            doc.merge_back(FrameId(0), &f).unwrap_err(),
            IoError::UnsupportedWriteback { .. }
        ));
    }

    #[test]
    fn merge_back_rejects_layer_count_mismatch() {
        let mut doc = indexed_doc();
        let mut f = doc.project(FrameId(0)).unwrap();
        f.layers.clear();
        assert!(matches!(
            doc.merge_back(FrameId(0), &f).unwrap_err(),
            IoError::LayerCountMismatch { .. }
        ));
    }

    #[test]
    fn out_of_range_frame_is_an_error() {
        let doc = indexed_doc();
        assert!(matches!(
            doc.project(FrameId(9)).unwrap_err(),
            IoError::FrameOutOfRange { index: 9, len: 2 }
        ));
    }

    #[test]
    fn group_chain_is_recorded_root_first() {
        let mut raw = AsepriteFile::new(4, 4, ColorMode::Rgba);
        raw.add_frame(100);
        let outer = raw.add_group("outer");
        let inner = raw.add_group_in("inner", outer);
        raw.add_layer_in("leaf", inner);
        let doc = Document::from_raw(raw);
        let f = doc.project(FrameId(0)).unwrap();
        assert_eq!(f.layers.len(), 1, "グループはレイヤとして数えない");
        assert_eq!(f.layers[0].meta.group_path, vec!["outer", "inner"]);
        assert_eq!(f.layers[0].meta.group_opacity.len(), 2);
    }

    #[test]
    fn tilemap_layer_is_not_flattened() {
        let mut raw = AsepriteFile::new(32, 32, ColorMode::Indexed);
        raw.set_palette(&[aseprite::Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
            name: None,
        }])
        .unwrap();
        raw.add_frame(100);
        let tm = raw.add_tilemap_layer("ground", 0);
        raw.set_tilemap_cel(tm, 0, vec![1, 2 | 0x2000_0000, 3, 4], 2, 2, 0, 0)
            .unwrap();
        let doc = Document::from_raw(raw);
        let f = doc.project(FrameId(0)).unwrap();
        let (grid, set) = f.layers[0].surface.as_tiles().unwrap();
        assert_eq!(set, TilesetId(0));
        assert_eq!(grid.get(0, 0).unwrap().id, 1);
        let flipped = grid.get(1, 0).unwrap();
        assert_eq!(flipped.id, 2);
        assert!(flipped.flip_x);
    }

    #[test]
    fn tilemap_round_trips_through_merge_back() {
        let mut raw = AsepriteFile::new(32, 32, ColorMode::Indexed);
        raw.set_palette(&[aseprite::Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
            name: None,
        }])
        .unwrap();
        raw.add_frame(100);
        let tm = raw.add_tilemap_layer("ground", 0);
        raw.set_tilemap_cel(tm, 0, vec![1, 2, 3, 4], 2, 2, 0, 0)
            .unwrap();
        let mut doc = Document::from_raw(raw);

        let mut f = doc.project(FrameId(0)).unwrap();
        if let Surface::Tiles { grid, .. } = &mut f.layers[0].surface {
            grid.tiles_mut()[3] = TileRef {
                id: 9,
                flip_y: true,
                ..TileRef::default()
            };
        }
        doc.merge_back(FrameId(0), &f).unwrap();
        let again = doc.project(FrameId(0)).unwrap();
        assert_eq!(again.layers[0].surface, f.layers[0].surface);
    }

    #[test]
    fn partial_alpha_palette_is_rejected_by_default() {
        let mut raw = AsepriteFile::new(4, 4, ColorMode::Indexed);
        raw.add_frame(100);
        raw.set_palette(&[aseprite::Color {
            r: 1,
            g: 2,
            b: 3,
            a: 128,
            name: None,
        }])
        .unwrap();
        let doc = Document::from_raw(raw);
        assert!(matches!(
            doc.project(FrameId(0)).unwrap_err(),
            IoError::NonBinaryPaletteAlpha {
                index: 0,
                alpha: 128
            }
        ));
        let opts = ProjectOptions {
            binarize_alpha: true,
        };
        let f = doc.project_with(FrameId(0), opts).unwrap();
        assert_eq!(f.palette.get(0).unwrap().a, 255);
    }

    #[test]
    fn rgba_surface_round_trips_through_merge_back() {
        let mut raw = AsepriteFile::new(4, 4, ColorMode::Rgba);
        raw.add_frame(100);
        let l = raw.add_layer("art");
        raw.set_cel(
            l,
            0,
            Pixels::new(vec![0u8; 4 * 4 * 4], 4, 4, ColorMode::Rgba).unwrap(),
            0,
            0,
        )
        .unwrap();
        let mut doc = Document::from_raw(raw);

        let mut f = doc.project(FrameId(0)).unwrap();
        if let Surface::Rgba(c) = &mut f.layers[0].surface {
            c.set(1, 1, Rgba8::rgb(10, 20, 30));
        }
        doc.merge_back(FrameId(0), &f).unwrap();
        assert_eq!(doc.project(FrameId(0)).unwrap(), f);
    }

    #[test]
    fn from_frames_builds_a_readable_file() {
        let doc = indexed_doc();
        let a = doc.project(FrameId(0)).unwrap();
        let b = doc.project(FrameId(1)).unwrap();

        let built = Document::from_frames(&[a.clone(), b.clone()]).unwrap();
        let bytes = built.to_bytes().unwrap();
        let reloaded = Document::from_bytes(&bytes).unwrap();

        assert_eq!(reloaded.frame_count(), 2);
        assert_eq!(reloaded.project(FrameId(0)).unwrap(), a);
        assert_eq!(reloaded.project(FrameId(1)).unwrap(), b);
    }

    #[test]
    fn from_frames_rejects_inconsistent_input() {
        let doc = indexed_doc();
        let a = doc.project(FrameId(0)).unwrap();
        let mut b = a.clone();
        b.layers.clear();
        assert!(matches!(
            Document::from_frames(&[a, b]).unwrap_err(),
            IoError::LayerCountMismatch { .. }
        ));
        assert!(matches!(
            Document::from_frames(&[]).unwrap_err(),
            IoError::FrameOutOfRange { .. }
        ));
    }

    #[test]
    fn indexed_canvas_helper_matches_projection_shape() {
        let doc = indexed_doc();
        let f = doc.project(FrameId(0)).unwrap();
        let c: &IndexedCanvas = f.layers[0].surface.as_indexed().unwrap();
        assert_eq!((c.width(), c.height()), (8, 8));
    }
}
