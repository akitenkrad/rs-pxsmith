//! PNG の入出力．
//!
//! 生成 AI の出力・3D レンダ・スクリーンショットは PNG で来るので，`px conform` の
//! 入口になる．
//!
//! # 書き出しはインデックス PNG にしない
//!
//! `image` 0.25 の PNG エンコーダはパレット付き PNG を書けない．インデックスカラーを
//! 保ちたい場合は `.aseprite` で書き出す — そちらは添字をそのまま持てる．PNG は
//! **確認用と外部ツールへの受け渡し**に使い，RGBA で書く．

use std::path::Path;

use px_core::canvas::{IndexedCanvas, RgbaCanvas};
use px_core::{Palette, Rgba8};

use crate::error::{IoError, Result};

/// PNG を RGBA として読む．
pub fn read_rgba(path: impl AsRef<Path>) -> Result<RgbaCanvas> {
    let path = path.as_ref();
    let img = image::open(path)
        .map_err(|e| IoError::Parse {
            path: path.to_path_buf(),
            line: 0,
            message: e.to_string(),
        })?
        .to_rgba8();

    let pixels: Vec<Rgba8> = img
        .pixels()
        .map(|p| Rgba8::new(p.0[0], p.0[1], p.0[2], p.0[3]))
        .collect();
    Ok(RgbaCanvas::from_pixels(img.width(), img.height(), pixels)?)
}

/// RGBA として書き出す (原子的置換)．
pub fn write_rgba(path: impl AsRef<Path>, canvas: &RgbaCanvas) -> Result<()> {
    let mut buf = image::RgbaImage::new(canvas.width(), canvas.height());
    for (i, p) in buf.pixels_mut().enumerate() {
        let c = canvas.pixels()[i];
        *p = image::Rgba([c.r, c.g, c.b, c.a]);
    }
    let mut bytes = Vec::new();
    buf.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .map_err(|e| IoError::Parse {
        path: path.as_ref().to_path_buf(),
        line: 0,
        message: e.to_string(),
    })?;
    crate::atomic::write(path, &bytes)
}

/// インデックスカラーをパレットで解決して書き出す．
pub fn write_indexed(
    path: impl AsRef<Path>,
    canvas: &IndexedCanvas,
    palette: &Palette,
) -> Result<()> {
    write_rgba(path, &resolve(canvas, palette))
}

/// 添字をパレットで解決して RGBA にする．
pub fn resolve(canvas: &IndexedCanvas, palette: &Palette) -> RgbaCanvas {
    let mut out = RgbaCanvas::filled(canvas.width(), canvas.height(), Rgba8::TRANSPARENT);
    for p in canvas.bounds().iter() {
        let Some(index) = canvas.get_at(p) else {
            continue;
        };
        let color = if canvas.transparent() == Some(index) {
            Rgba8::TRANSPARENT
        } else {
            palette.get(index).unwrap_or(Rgba8::TRANSPARENT)
        };
        out.set(p.x, p.y, color);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("pxforge-png-test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn rgba_round_trips_through_a_file() {
        let mut c = RgbaCanvas::filled(4, 3, Rgba8::TRANSPARENT);
        c.set(0, 0, Rgba8::rgb(0x1a, 0x1c, 0x2c));
        c.set(3, 2, Rgba8::new(0xff, 0x00, 0x00, 0xff));
        let path = scratch("round.png");
        write_rgba(&path, &c).unwrap();
        assert_eq!(read_rgba(&path).unwrap(), c);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn indexed_is_resolved_through_the_palette() {
        let palette = Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(1, 2, 3)]).unwrap();
        let canvas = IndexedCanvas::from_pixels(2, 1, vec![0, 1])
            .unwrap()
            .with_transparent(Some(0));
        let rgba = resolve(&canvas, &palette);
        assert_eq!(rgba.get(0, 0), Some(Rgba8::TRANSPARENT));
        assert_eq!(rgba.get(1, 0), Some(Rgba8::rgb(1, 2, 3)));
    }

    #[test]
    fn reading_a_missing_file_is_an_error() {
        assert!(read_rgba(scratch("nope.png")).is_err());
    }
}
