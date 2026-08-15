//! オニオンスキン (`pxsmith view --onion`．設計書 5 章 ・D52)．
//!
//! **輪郭のみを表示する** (D52) ．前後のコマを塗り潰して重ねると，いま描いて
//! いるコマがその下に隠れてしまうからである — 参考書籍もオニオンスキンを
//! «前のコマを透かして動きの幅を見る» ものとして扱っている．
//!
//! # 隠す量が «塗り潰し» と «輪郭» でどれだけ違うかは測れる
//!
//! D52 は理由を書いていないが，**理由の方は数えられる** — 前後のコマが
//! いまのコマの何画素に重なるかを，塗り潰した場合と輪郭だけの場合で数えれば
//! よい ([`OnionReport`]) ．D126 «残像が «見える» のはどれくらい動いたときか»
//! と同じ形の問いである．
//!
//! # ここでは色を作ってよい
//!
//! D94 «色を作らない» は**絵を作る道具**の不変条件であって，確認用の表示には
//! 掛からない — 出力は RGBA の画像であり，パレットへ書き戻さない．
//! `pxsmith view --luma` が明度を灰色へ写しているのと同じ側である．

use image::{Rgba, RgbaImage};
use pxsmith_core::Rgba8;
use pxsmith_core::frame::{Frame, Surface};
use pxsmith_core::geom::Mask;
use pxsmith_core::geom::contour::trace_contours;
use pxsmith_core::math::ivec2;

use crate::render::{RenderOptions, to_rgba_image};

/// 設定．
#[derive(Copy, Clone, Debug, Default)]
pub struct OnionOptions {
    /// 何コマ前まで重ねるか．
    pub before: usize,
    /// 何コマ後まで重ねるか．
    pub after: usize,
    /// **塗り潰して重ねる** (既定は輪郭のみ．D52)．
    ///
    /// 比べるために残してあるが，**これが D52 が «採らない» と言う側**である．
    pub filled: bool,
}

impl OnionOptions {
    pub fn is_off(&self) -> bool {
        self.before == 0 && self.after == 0
    }
}

/// 結果の素性．
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnionReport {
    /// 重ねたコマの数．
    pub drawn: usize,
    /// **頼まれたのに無かったコマの数** — 端では前後が足りない．
    ///
    /// «重ねなかった» と «重ねるものが無かった» を分けて言う (D77 ・D104)．
    pub missing: usize,
    /// 輪郭が覆った画素．
    pub contour_pixels: usize,
    /// 塗り潰しなら覆っていた画素．
    pub filled_pixels: usize,
    /// **輪郭がいまのコマに重なった画素** (隠した量)．
    pub obscured: usize,
    /// **塗り潰しならいまのコマに重なっていた画素**．
    pub obscured_if_filled: usize,
}

impl OnionReport {
    /// 塗り潰しに対して隠す量が何割に減ったか．
    pub fn obscured_ratio(&self) -> f32 {
        if self.obscured_if_filled == 0 {
            return 0.0;
        }
        self.obscured as f32 / self.obscured_if_filled as f32
    }
}

/// フレームのシルエット (不可視レイヤは飛ばす)．
fn silhouette(frame: &Frame) -> Mask {
    let mut m = Mask::new(frame.size.x, frame.size.y);
    for layer in &frame.layers {
        if !layer.meta.visible {
            continue;
        }
        match &layer.surface {
            Surface::Indexed(c) => {
                for y in 0..c.height() as i32 {
                    for x in 0..c.width() as i32 {
                        let p = ivec2(x, y);
                        if !c.is_transparent_at(p) {
                            m.set(p, true);
                        }
                    }
                }
            }
            Surface::Rgba(c) => {
                for y in 0..c.height() as i32 {
                    for x in 0..c.width() as i32 {
                        if c.get(x, y).is_some_and(|c| c.a != 0) {
                            m.set(ivec2(x, y), true);
                        }
                    }
                }
            }
            // タイルマップはタイルセットが要るので確認表示では飛ばす
            Surface::Tiles { .. } => continue,
        }
    }
    m
}

/// マスクの輪郭だけを残したマスク．
fn contour_mask(mask: &Mask) -> Mask {
    let mut out = Mask::new(mask.width(), mask.height());
    for contour in trace_contours(mask) {
        for p in contour.points() {
            out.set(*p, true);
        }
    }
    out
}

/// 前のコマは寒色 ・後のコマは暖色で，遠いほど薄くする．
fn onion_colour(offset: i32, reach: usize) -> Rgba8 {
    let far = reach.max(1) as f32;
    let fade = 1.0 - (offset.unsigned_abs() as f32 - 1.0) / far;
    let a = (0x90 as f32 * fade.clamp(0.25, 1.0)).round() as u8;
    if offset < 0 {
        Rgba8::new(0x41, 0xa6, 0xf6, a)
    } else {
        Rgba8::new(0xef, 0x7d, 0x57, a)
    }
}

fn over(dst: Rgba<u8>, src: Rgba8) -> Rgba<u8> {
    let a = src.a as u32;
    if a == 0 {
        return dst;
    }
    let blend = |d: u8, s: u8| (((s as u32 * a) + (d as u32 * (255 - a))) / 255) as u8;
    Rgba([
        blend(dst.0[0], src.r),
        blend(dst.0[1], src.g),
        blend(dst.0[2], src.b),
        dst.0[3].max(src.a),
    ])
}

/// オニオンスキンを重ねた画像を作る．
///
/// `index` がいま見ているコマ．前後のコマは**輪郭だけ**を重ねる (D52)．
pub fn onion_image(
    frames: &[Frame],
    index: usize,
    opts: &OnionOptions,
    render: &RenderOptions,
) -> (RgbaImage, OnionReport) {
    let mut report = OnionReport::default();
    let Some(current) = frames.get(index) else {
        return (RgbaImage::new(1, 1), report);
    };
    let mut img = to_rgba_image(current, render);
    if opts.is_off() {
        return (img, report);
    }

    let zoom = render.zoom.max(1);
    let here = silhouette(current);
    let reach = opts.before.max(opts.after);

    // 遠いコマから順に重ねる — 近いコマが上に来る
    let mut offsets: Vec<i32> = Vec::new();
    for k in 1..=opts.before {
        offsets.push(-(k as i32));
    }
    for k in 1..=opts.after {
        offsets.push(k as i32);
    }
    offsets.sort_by_key(|o| -o.abs());

    for offset in offsets {
        let Some(target) = index.checked_add_signed(offset as isize) else {
            report.missing += 1;
            continue;
        };
        let Some(frame) = frames.get(target) else {
            report.missing += 1;
            continue;
        };
        report.drawn += 1;

        let solid = silhouette(frame);
        let outline = contour_mask(&solid);
        report.filled_pixels += solid.count();
        report.contour_pixels += outline.count();
        // **いまのコマに重なった画素**を数える — 隠した量である
        report.obscured += outline.iter_set().filter(|p| here.get(*p)).count();
        report.obscured_if_filled += solid.iter_set().filter(|p| here.get(*p)).count();

        let paint = if opts.filled { &solid } else { &outline };
        let colour = onion_colour(offset, reach);
        for p in paint.iter_set() {
            for dy in 0..zoom {
                for dx in 0..zoom {
                    let (px, py) = (p.x as u32 * zoom + dx, p.y as u32 * zoom + dy);
                    if px < img.width() && py < img.height() {
                        let dst = *img.get_pixel(px, py);
                        img.put_pixel(px, py, over(dst, colour));
                    }
                }
            }
        }
    }

    (img, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxsmith_core::canvas::IndexedCanvas;
    use pxsmith_core::frame::{Layer, LayerMeta};
    use pxsmith_core::math::uvec2;
    use pxsmith_core::palette::Palette;

    fn palette() -> Palette {
        Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(255, 0, 0)]).unwrap()
    }

    /// 左上を原点に `w x h` の塗り潰した四角を `at` へ置いたコマ．
    fn block(at: (i32, i32), size: (u32, u32)) -> Frame {
        let mut c = IndexedCanvas::filled(24, 24, 0);
        c.set_transparent(Some(0));
        for y in 0..size.1 as i32 {
            for x in 0..size.0 as i32 {
                c.set(at.0 + x, at.1 + y, 1);
            }
        }
        let mut f = Frame::new(uvec2(24, 24), palette());
        f.layers
            .push(Layer::new(LayerMeta::named("art"), Surface::Indexed(c)));
        f
    }

    fn render() -> RenderOptions {
        RenderOptions {
            zoom: 1,
            checkerboard: false,
            ..RenderOptions::default()
        }
    }

    /// **壊れると: オニオンスキンが塗り潰しになり，いまのコマが隠れる** (D52)．
    ///
    /// **理由の方を数えている** — 同じ場所に重なった 2 コマでは，塗り潰しは
    /// 内側まで覆うが輪郭は縁しか覆わない．
    #[test]
    fn a_contour_onion_hides_far_less_than_a_filled_one() {
        let frames = [block((4, 4), (10, 10)), block((4, 4), (10, 10))];
        let opts = OnionOptions {
            before: 1,
            after: 0,
            filled: false,
        };
        let (_, r) = onion_image(&frames, 1, &opts, &render());
        assert_eq!(r.drawn, 1);
        assert!(r.obscured > 0, "輪郭が 1 画素も重なっていない");
        assert!(
            r.obscured < r.obscured_if_filled,
            "輪郭が塗り潰しと同じだけ隠している ({} 対 {})",
            r.obscured,
            r.obscured_if_filled
        );
        // 10x10 の塗り潰しは 100 画素，その輪郭は縁の 36 画素
        assert_eq!(r.obscured_if_filled, 100);
        assert!(r.obscured <= 36, "輪郭が縁より多い: {}", r.obscured);
    }

    /// **壊れると: 端で «重ねなかった» と «重ねるものが無かった» が混ざる** (D77 ・D104)．
    #[test]
    fn missing_neighbours_are_counted_separately_from_drawn_ones() {
        let frames = [block((0, 0), (4, 4)), block((2, 0), (4, 4))];
        let opts = OnionOptions {
            before: 2,
            after: 2,
            filled: false,
        };
        // 先頭のコマ — 前は 2 つとも無く，後は 1 つだけある
        let (_, r) = onion_image(&frames, 0, &opts, &render());
        assert_eq!(r.drawn, 1, "重ねられたのは後ろの 1 コマだけのはず");
        assert_eq!(r.missing, 3, "足りなかった 3 コマを数えていない");
    }

    /// **壊れると: オニオンスキンを頼んでいないのに絵が変わる．**
    #[test]
    fn asking_for_no_onion_leaves_the_frame_alone() {
        let frames = [block((0, 0), (4, 4)), block((8, 8), (4, 4))];
        let plain = to_rgba_image(&frames[1], &render());
        let (img, r) = onion_image(&frames, 1, &OnionOptions::default(), &render());
        assert_eq!(img.as_raw(), plain.as_raw(), "重ねていないのに絵が変わった");
        assert_eq!(r, OnionReport::default());
    }

    /// **壊れると: 前のコマと後のコマが同じ色で出る (どちらへ動くか読めない)．**
    #[test]
    fn frames_before_and_after_get_different_colours() {
        let a = onion_colour(-1, 2);
        let b = onion_colour(1, 2);
        assert_ne!((a.r, a.g, a.b), (b.r, b.g, b.b), "前後が同じ色になっている");
    }

    /// **壊れると: 遠いコマが近いコマと同じ濃さで出る．**
    #[test]
    fn farther_frames_are_fainter() {
        assert!(
            onion_colour(-3, 3).a < onion_colour(-1, 3).a,
            "遠いコマの方が濃い"
        );
    }
}
