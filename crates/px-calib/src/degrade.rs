//! 劣化パイプライン (実装計画書 M2 の評価データセット)．
//!
//! きれいなドット絵を「拡大 → 非整数倍リサイズ → 圧縮 → 位相をずらしてクロップ」の
//! 順に通し，格子推定が復元すべき入力を作る．種になったのは
//! `px-io/examples/degrade.rs` で，**JPEG 圧縮の水準はここで足した**．
//!
//! | 要因 | 水準 |
//! | --- | --- |
//! | 拡大率 $s$ | 2, 3, 4, 6, 8, 12 |
//! | 補間法 | nearest, bilinear, bicubic, lanczos |
//! | 非整数倍リサイズ | なし / 1.3 倍 / 0.85 倍 |
//! | 圧縮 | PNG / JPEG 品質 95, 80, 60 |
//! | 位相 | $d_x, d_y$ をランダムにずらしてクロップ |
//!
//! ## 正解の定義
//!
//! 非整数倍リサイズを挟むと格子の周期が $s \cdot r$ (非整数) になり，**整数の格子は
//! 存在しない**．そのため正解は 2 通りに分かれる．
//!
//! | 条件 | 正解 |
//! | --- | --- |
//! | リサイズなし | $(s, d_x, d_y)$ をすべて復元できること |
//! | リサイズあり | 整数の格子は無い．$\hat{s}$ が $\mathrm{round}(s \cdot r)$ に一致するか，あるいは棄却されるかを別枠で見る |
//!
//! 完全一致率の目標 95% (実装計画書) はリサイズなしの部分集合に対する指標として扱い，
//! リサイズありは実データと同じく**別枠で報告する**．混ぜると「格子が存在しない入力で
//! 何を答えるべきか」という別の問いが完全一致率に紛れ込む．

use anyhow::{Context, Result};
use image::imageops::FilterType;
use px_core::{Rgba8, RgbaCanvas};
use serde::{Deserialize, Serialize};

/// 拡大率の水準．
pub const SCALES: [u32; 6] = [2, 3, 4, 6, 8, 12];

/// 補間法の水準．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filter {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos,
}

pub const FILTERS: [Filter; 4] = [
    Filter::Nearest,
    Filter::Bilinear,
    Filter::Bicubic,
    Filter::Lanczos,
];

impl Filter {
    fn as_image(self) -> FilterType {
        match self {
            Self::Nearest => FilterType::Nearest,
            Self::Bilinear => FilterType::Triangle,
            Self::Bicubic => FilterType::CatmullRom,
            Self::Lanczos => FilterType::Lanczos3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Bilinear => "bilinear",
            Self::Bicubic => "bicubic",
            Self::Lanczos => "lanczos",
        }
    }
}

/// 非整数倍リサイズの水準．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resize {
    /// かけない (整数の格子が残る唯一の水準)．
    Keep,
    Up13,
    Down085,
}

pub const RESIZES: [Resize; 3] = [Resize::Keep, Resize::Up13, Resize::Down085];

impl Resize {
    pub fn factor(self) -> f32 {
        match self {
            Self::Keep => 1.0,
            Self::Up13 => 1.3,
            Self::Down085 => 0.85,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Up13 => "1.30",
            Self::Down085 => "0.85",
        }
    }
}

/// 圧縮の水準．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    /// 可逆．
    Png,
    Jpeg95,
    Jpeg80,
    Jpeg60,
}

pub const COMPRESSIONS: [Compression; 4] = [
    Compression::Png,
    Compression::Jpeg95,
    Compression::Jpeg80,
    Compression::Jpeg60,
];

impl Compression {
    /// JPEG のときだけ品質を返す．
    pub fn quality(self) -> Option<u8> {
        match self {
            Self::Png => None,
            Self::Jpeg95 => Some(95),
            Self::Jpeg80 => Some(80),
            Self::Jpeg60 => Some(60),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg95 => "jpeg95",
            Self::Jpeg80 => "jpeg80",
            Self::Jpeg60 => "jpeg60",
        }
    }
}

/// 1 件分の劣化条件．
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Degradation {
    pub scale: u32,
    pub filter: Filter,
    pub resize: Resize,
    pub compression: Compression,
    /// クロップで捨てる左上の画素数．
    pub crop: (u32, u32),
}

impl Degradation {
    /// 復元されるべき位相．**リサイズを挟んだ場合は整数の格子が無いので `None`**．
    ///
    /// クロップで左端の `dx` 列を捨てると，セルの境界は新しい座標で
    /// $-d_x \bmod s$ に来る．`px conform` の位相はセルの開始位置なので
    /// $(s - d_x \bmod s) \bmod s$ が正解になる．
    pub fn truth_phase(&self) -> Option<(u32, u32)> {
        if self.resize != Resize::Keep {
            return None;
        }
        let s = self.scale;
        Some(((s - self.crop.0 % s) % s, (s - self.crop.1 % s) % s))
    }

    /// リサイズ後の実効的な周期 $s \cdot r$．
    pub fn effective_scale(&self) -> f32 {
        self.scale as f32 * self.resize.factor()
    }

    /// 劣化させる．
    pub fn apply(&self, src: &RgbaCanvas) -> Result<RgbaCanvas> {
        let filter = self.filter.as_image();
        let buf = to_image(src);

        // 1. 整数倍に拡大
        let big = image::imageops::resize(
            &buf,
            src.width() * self.scale,
            src.height() * self.scale,
            filter,
        );

        // 2. 非整数倍のリサイズ (格子を壊す要因)
        let r = self.resize.factor();
        let resized = if self.resize == Resize::Keep {
            big
        } else {
            let w = ((big.width() as f32) * r).round().max(1.0) as u32;
            let h = ((big.height() as f32) * r).round().max(1.0) as u32;
            image::imageops::resize(&big, w, h, filter)
        };

        // 3. 圧縮 (JPEG のときだけ実際に符号化して読み直す)
        let compressed = match self.compression.quality() {
            None => resized,
            Some(q) => jpeg_round_trip(&resized, q)?,
        };

        // 4. 位相をずらしてクロップ
        let dx = self.crop.0.min(compressed.width().saturating_sub(1));
        let dy = self.crop.1.min(compressed.height().saturating_sub(1));
        let cropped = image::imageops::crop_imm(
            &compressed,
            dx,
            dy,
            compressed.width() - dx,
            compressed.height() - dy,
        )
        .to_image();

        Ok(from_image(&cropped))
    }
}

/// JPEG に通して読み直す．**アルファは持てないので不透明前提**である
/// (元絵は [`crate::sprite`] が不透明で作る)．
fn jpeg_round_trip(img: &image::RgbaImage, quality: u8) -> Result<image::RgbaImage> {
    let rgb = image::RgbImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y).0;
        image::Rgb([p[0], p[1], p[2]])
    });

    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
        std::io::Cursor::new(&mut bytes),
        quality,
    );
    encoder
        .encode_image(&rgb)
        .context("JPEG に符号化できない")?;

    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
        .context("JPEG を読み直せない")?;
    Ok(decoded.to_rgba8())
}

fn to_image(src: &RgbaCanvas) -> image::RgbaImage {
    let mut buf = image::RgbaImage::new(src.width(), src.height());
    for (i, p) in buf.pixels_mut().enumerate() {
        let c = src.pixels()[i];
        *p = image::Rgba([c.r, c.g, c.b, c.a]);
    }
    buf
}

fn from_image(img: &image::RgbaImage) -> RgbaCanvas {
    let pixels: Vec<Rgba8> = img
        .pixels()
        .map(|p| Rgba8::new(p.0[0], p.0[1], p.0[2], p.0[3]))
        .collect();
    RgbaCanvas::from_pixels(img.width(), img.height(), pixels)
        .expect("画素数は width*height で作っている")
}

#[cfg(test)]
mod tests {
    use px_core::grid::{GridParams, downscale_modal, estimate_grid};
    use px_core::ivec2;

    use super::*;
    use crate::sprite;

    fn plain(scale: u32, crop: (u32, u32)) -> Degradation {
        Degradation {
            scale,
            filter: Filter::Nearest,
            resize: Resize::Keep,
            compression: Compression::Png,
            crop,
        }
    }

    #[test]
    fn nearest_without_resize_is_an_exact_upscale() {
        let src = sprite::synthesize(5);
        let out = plain(4, (0, 0)).apply(&src).unwrap();
        assert_eq!(out.width(), src.width() * 4);
        assert_eq!(out.height(), src.height() * 4);
        // 縮小し直すと元絵に戻る
        let back = downscale_modal(&out, 4, ivec2(0, 0));
        assert_eq!(back.pixels(), src.pixels());
    }

    #[test]
    fn cropping_shifts_the_phase_as_the_truth_says() {
        let src = sprite::synthesize(6);
        let d = plain(6, (2, 3));
        assert_eq!(d.truth_phase(), Some((4, 3)));
        let out = d.apply(&src).unwrap();
        let e = estimate_grid(&out, &GridParams::default()).unwrap();
        assert_eq!(e.scale, 6);
        assert_eq!((e.phase.x as u32, e.phase.y as u32), (4, 3));
    }

    #[test]
    fn a_phase_of_zero_stays_zero_for_every_scale() {
        for s in SCALES {
            assert_eq!(plain(s, (0, 0)).truth_phase(), Some((0, 0)));
            // s ずらすとちょうど 1 セル分なので位相は 0 に戻る
            assert_eq!(plain(s, (s, s)).truth_phase(), Some((0, 0)));
        }
    }

    #[test]
    fn resizing_leaves_no_integer_grid() {
        let d = Degradation {
            resize: Resize::Up13,
            ..plain(6, (1, 1))
        };
        assert_eq!(d.truth_phase(), None, "非整数の格子に位相の正解は無い");
        assert!((d.effective_scale() - 7.8).abs() < 1e-5);
    }

    #[test]
    fn jpeg_changes_the_pixels_but_not_the_size() {
        let src = sprite::synthesize(7);
        let png = plain(4, (0, 0)).apply(&src).unwrap();
        let jpeg = Degradation {
            compression: Compression::Jpeg60,
            ..plain(4, (0, 0))
        }
        .apply(&src)
        .unwrap();

        assert_eq!((png.width(), png.height()), (jpeg.width(), jpeg.height()));
        assert_ne!(png.pixels(), jpeg.pixels(), "JPEG が効いていない");
        assert!(
            jpeg.pixels().iter().all(|p| p.a == 255),
            "JPEG 経由で透明が混ざった"
        );
    }

    #[test]
    fn higher_jpeg_quality_stays_closer_to_the_original() {
        let src = sprite::synthesize(8);
        let base = plain(4, (0, 0)).apply(&src).unwrap();
        let err = |c: Compression| -> u64 {
            let out = Degradation {
                compression: c,
                ..plain(4, (0, 0))
            }
            .apply(&src)
            .unwrap();
            base.pixels()
                .iter()
                .zip(out.pixels())
                .map(|(a, b)| {
                    let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).unsigned_abs() as u64;
                    d(a.r, b.r) + d(a.g, b.g) + d(a.b, b.b)
                })
                .sum()
        };
        assert!(err(Compression::Jpeg95) < err(Compression::Jpeg60));
    }

    #[test]
    fn resize_changes_the_size_by_the_factor() {
        let src = sprite::synthesize(9);
        let out = Degradation {
            resize: Resize::Up13,
            ..plain(4, (0, 0))
        }
        .apply(&src)
        .unwrap();
        let expected = ((src.width() * 4) as f32 * 1.3).round() as u32;
        assert_eq!(out.width(), expected);
    }

    #[test]
    fn the_same_condition_gives_the_same_image() {
        let src = sprite::synthesize(10);
        let d = Degradation {
            filter: Filter::Lanczos,
            resize: Resize::Down085,
            compression: Compression::Jpeg80,
            ..plain(6, (2, 1))
        };
        assert_eq!(
            d.apply(&src).unwrap().pixels(),
            d.apply(&src).unwrap().pixels()
        );
    }
}
