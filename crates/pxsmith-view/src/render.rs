//! キャンバスから画像への変換．
//!
//! 端末は 1 画素をそのまま出すと小さすぎるので整数倍に拡大する．**拡大は
//! ニアレストネイバー固定**である — 確認用の表示で補間を挟むと，見えているものが
//! データと違ってしまう．

use image::{Rgba, RgbaImage};
use pxsmith_core::canvas::{IndexedCanvas, RgbaCanvas};
use pxsmith_core::frame::{Frame, Surface};
use pxsmith_core::{Palette, Rgba8};

/// 表示の設定．
#[derive(Copy, Clone, Debug)]
pub struct RenderOptions {
    /// 整数倍の拡大率．0 は 1 として扱う．
    pub zoom: u32,
    /// 透明部分に市松模様を敷く．
    pub checkerboard: bool,
    /// 市松の 1 マスの大きさ (拡大前の画素単位)．
    pub checker_size: u32,
    /// 明度のみで表示する (`pxsmith view --luma`)．
    pub luma: bool,
    /// 単色で塗り潰して形だけ見る (`pxsmith view --silhouette`)．
    pub silhouette: Option<Rgba8>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            zoom: 8,
            checkerboard: true,
            checker_size: 4,
            luma: false,
            silhouette: None,
        }
    }
}

impl RenderOptions {
    fn zoom(&self) -> u32 {
        self.zoom.max(1)
    }
}

fn checker(x: u32, y: u32, size: u32) -> Rgba<u8> {
    let s = size.max(1);
    if (x / s + y / s).is_multiple_of(2) {
        Rgba([0x30, 0x30, 0x38, 0xff])
    } else {
        Rgba([0x24, 0x24, 0x2c, 0xff])
    }
}

fn apply_effects(color: Rgba8, opts: &RenderOptions) -> Rgba8 {
    if color.a == 0 {
        return color;
    }
    if let Some(fill) = opts.silhouette {
        return fill;
    }
    if opts.luma {
        // OKLab の明度を sRGB の灰色へ写す．知覚的な明るさの並びが保たれる
        let l = color.to_oklab().l.clamp(0.0, 1.0);
        let v = (l.powf(1.0 / 2.2) * 255.0).round() as u8;
        return Rgba8::new(v, v, v, color.a);
    }
    color
}

/// インデックスカラーのキャンバスを画像へ．
pub fn indexed_to_rgba_image(
    canvas: &IndexedCanvas,
    palette: &Palette,
    opts: &RenderOptions,
) -> RgbaImage {
    let zoom = opts.zoom();
    let mut img = RgbaImage::new(canvas.width() * zoom, canvas.height() * zoom);

    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            let index = canvas.get(x as i32, y as i32).unwrap_or_default();
            let transparent = canvas.transparent() == Some(index);
            let color = if transparent {
                Rgba8::TRANSPARENT
            } else {
                apply_effects(palette.get(index).unwrap_or(Rgba8::TRANSPARENT), opts)
            };

            for dy in 0..zoom {
                for dx in 0..zoom {
                    let (px, py) = (x * zoom + dx, y * zoom + dy);
                    let out = if color.a == 0 {
                        if opts.checkerboard {
                            checker(x, y, opts.checker_size)
                        } else {
                            Rgba([0, 0, 0, 0])
                        }
                    } else {
                        Rgba([color.r, color.g, color.b, color.a])
                    };
                    img.put_pixel(px, py, out);
                }
            }
        }
    }
    img
}

/// RGBA のキャンバスを画像へ．
pub fn rgba_to_rgba_image(canvas: &RgbaCanvas, opts: &RenderOptions) -> RgbaImage {
    let zoom = opts.zoom();
    let mut img = RgbaImage::new(canvas.width() * zoom, canvas.height() * zoom);
    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            let color = apply_effects(
                canvas.get(x as i32, y as i32).unwrap_or(Rgba8::TRANSPARENT),
                opts,
            );
            for dy in 0..zoom {
                for dx in 0..zoom {
                    let out = if color.a == 0 && opts.checkerboard {
                        checker(x, y, opts.checker_size)
                    } else {
                        Rgba([color.r, color.g, color.b, color.a])
                    };
                    img.put_pixel(x * zoom + dx, y * zoom + dy, out);
                }
            }
        }
    }
    img
}

/// フレームを 1 枚の画像へ畳む．
///
/// 不可視レイヤは飛ばし，下から順に重ねる．**ブレンドモードは適用しない** —
/// 確認用の表示であり，正しい合成は書き出し側の仕事である．
pub fn to_rgba_image(frame: &Frame, opts: &RenderOptions) -> RgbaImage {
    let zoom = opts.zoom();
    let mut out = RgbaImage::new(frame.size.x * zoom, frame.size.y * zoom);
    if opts.checkerboard {
        for y in 0..out.height() {
            for x in 0..out.width() {
                out.put_pixel(x, y, checker(x / zoom, y / zoom, opts.checker_size));
            }
        }
    }

    let flat = RenderOptions {
        checkerboard: false,
        ..*opts
    };
    for layer in &frame.layers {
        if !layer.meta.visible {
            continue;
        }
        let img = match &layer.surface {
            Surface::Indexed(c) => indexed_to_rgba_image(c, &frame.palette, &flat),
            Surface::Rgba(c) => rgba_to_rgba_image(c, &flat),
            // タイルマップはタイルセットが要るので確認表示では飛ばす
            Surface::Tiles { .. } => continue,
        };
        for (x, y, p) in img.enumerate_pixels() {
            if p.0[3] != 0 && x < out.width() && y < out.height() {
                out.put_pixel(x, y, *p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxsmith_core::frame::{Layer, LayerMeta};
    use pxsmith_core::math::uvec2;

    fn palette() -> Palette {
        Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(255, 0, 0),
            Rgba8::rgb(255, 255, 255),
        ])
        .unwrap()
    }

    fn canvas() -> IndexedCanvas {
        IndexedCanvas::from_pixels(2, 1, vec![0, 1])
            .unwrap()
            .with_transparent(Some(0))
    }

    #[test]
    fn zoom_scales_by_an_integer_factor() {
        let opts = RenderOptions {
            zoom: 4,
            ..Default::default()
        };
        let img = indexed_to_rgba_image(&canvas(), &palette(), &opts);
        assert_eq!((img.width(), img.height()), (8, 4));
    }

    #[test]
    fn zoom_is_nearest_neighbour() {
        let opts = RenderOptions {
            zoom: 3,
            checkerboard: false,
            ..Default::default()
        };
        let img = indexed_to_rgba_image(&canvas(), &palette(), &opts);
        // 右半分は全て同じ赤．補間されていたら端がにじむ
        for y in 0..3 {
            for x in 3..6 {
                assert_eq!(img.get_pixel(x, y).0, [255, 0, 0, 255], "({x},{y})");
            }
        }
    }

    #[test]
    fn transparent_pixels_get_the_checkerboard() {
        let opts = RenderOptions {
            zoom: 1,
            checkerboard: true,
            checker_size: 1,
            ..Default::default()
        };
        let img = indexed_to_rgba_image(&canvas(), &palette(), &opts);
        assert_eq!(img.get_pixel(0, 0).0[3], 255, "市松で埋まっているはず");
        assert_ne!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn silhouette_replaces_every_opaque_colour() {
        let opts = RenderOptions {
            zoom: 1,
            checkerboard: false,
            silhouette: Some(Rgba8::rgb(0, 0, 0)),
            ..Default::default()
        };
        let img = indexed_to_rgba_image(&canvas(), &palette(), &opts);
        assert_eq!(img.get_pixel(1, 0).0, [0, 0, 0, 255]);
        assert_eq!(img.get_pixel(0, 0).0[3], 0, "透明はそのまま");
    }

    #[test]
    fn luma_makes_grey_but_keeps_the_lightness_order() {
        let opts = RenderOptions {
            zoom: 1,
            checkerboard: false,
            luma: true,
            ..Default::default()
        };
        let c = IndexedCanvas::from_pixels(2, 1, vec![1, 2]).unwrap();
        let img = indexed_to_rgba_image(&c, &palette(), &opts);
        let red = img.get_pixel(0, 0).0;
        let white = img.get_pixel(1, 0).0;
        assert_eq!(red[0], red[1], "灰色になっていない");
        assert_eq!(red[1], red[2]);
        assert!(white[0] > red[0], "白の方が明るいはず");
    }

    #[test]
    fn hidden_layers_are_skipped() {
        let mut f = Frame::new(uvec2(2, 1), palette());
        let mut meta = LayerMeta::named("hidden");
        meta.visible = false;
        f.layers.push(Layer::new(
            meta,
            Surface::Indexed(IndexedCanvas::filled(2, 1, 1)),
        ));
        let opts = RenderOptions {
            zoom: 1,
            checkerboard: false,
            ..Default::default()
        };
        let img = to_rgba_image(&f, &opts);
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 0]);
    }

    #[test]
    fn upper_layers_paint_over_lower_ones() {
        let mut f = Frame::new(uvec2(2, 1), palette());
        f.layers.push(Layer::new(
            LayerMeta::named("under"),
            Surface::Indexed(IndexedCanvas::filled(2, 1, 1)),
        ));
        f.layers.push(Layer::new(
            LayerMeta::named("over"),
            Surface::Indexed(
                IndexedCanvas::from_pixels(2, 1, vec![0, 2])
                    .unwrap()
                    .with_transparent(Some(0)),
            ),
        ));
        let opts = RenderOptions {
            zoom: 1,
            checkerboard: false,
            ..Default::default()
        };
        let img = to_rgba_image(&f, &opts);
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255], "下が透けるはず");
        assert_eq!(img.get_pixel(1, 0).0, [255, 255, 255, 255]);
    }
}
