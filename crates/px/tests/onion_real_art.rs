//! **オニオンスキンが «輪郭のみ» である理由を実素材で数える** (D52)．
//!
//! D52 は «輪郭のみを表示する» と決めているが**理由は書いていない**．理由の方は
//! 数えられる — 前後のコマがいまのコマの何画素に重なるかを，塗り潰した場合と
//! 輪郭だけの場合で数えればよい (D126 «残像が «見える» のはどれくらい動いた
//! ときか» と同じ形の問い) ．
//!
//! **場面は «動きの幅を見たいとき»** である．オニオンスキンは前のコマとの
//! ずれを見る道具なので，**平行移動した列**を作って測る (D114 ・D139 が
//! «正しい動きとは平行移動» と決めているのと同じ根拠) ．
//!
//! **飛ばした件も数える** (D128)．

use std::path::{Path, PathBuf};

use px_core::canvas::{IndexedCanvas, RgbaCanvas};
use px_core::color::Rgba8;
use px_core::frame::{Frame, Layer, LayerMeta, Surface};
use px_core::math::uvec2;
use px_core::palette::Palette;
use px_view::onion::{OnionOptions, onion_image};
use px_view::render::RenderOptions;

fn seeds() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/grid-eval/seeds")
        .canonicalize()
        .expect("種の置き場所がある")
}

fn png_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("読める")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    v.sort();
    v
}

fn index_exactly(img: &RgbaCanvas) -> Option<(IndexedCanvas, Palette)> {
    let mut colors: Vec<Rgba8> = img.pixels().to_vec();
    colors.sort_unstable_by_key(|c| c.sort_key());
    colors.dedup();
    if colors.len() > 256 {
        return None;
    }
    let palette = Palette::new(colors).ok()?;
    let transparent = palette
        .entries()
        .iter()
        .position(|c| c.a == 0)
        .map(|i| i as u8);
    let pixels: Vec<u8> = img
        .pixels()
        .iter()
        .map(|c| match (c.a, transparent) {
            (0, Some(i)) => i,
            _ => palette.nearest(*c, 1.0).unwrap_or(0),
        })
        .collect();
    let canvas = IndexedCanvas::from_pixels(img.width(), img.height(), pixels)
        .ok()?
        .with_transparent(transparent);
    Some((canvas, palette))
}

/// 絵を `dx` 画素だけ平行移動したコマ．
fn shifted_frame(canvas: &IndexedCanvas, palette: &Palette, dx: i32) -> Frame {
    let fill = canvas.transparent().unwrap_or(0);
    let mut out = IndexedCanvas::filled(canvas.width(), canvas.height(), fill);
    out.set_transparent(canvas.transparent());
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            if let Some(i) = canvas.get(x, y) {
                out.set(x + dx, y, i);
            }
        }
    }
    let mut f = Frame::new(uvec2(out.width(), out.height()), palette.clone());
    f.layers
        .push(Layer::new(LayerMeta::named("art"), Surface::Indexed(out)));
    f
}

/// 平行移動した 5 コマの列を作る．
fn walk(canvas: &IndexedCanvas, palette: &Palette) -> Vec<Frame> {
    (0..5)
        .map(|k| shifted_frame(canvas, palette, k - 2))
        .collect()
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
/// 実素材を 1 画素ずつ平行移動した 5 コマの列で，前後 2 コマを重ねて数える．
/// **輪郭のみは «透けている» のではなく «覆う画素が少ない»** ので，
/// 隠す量が塗り潰しに対してどれだけ減るかがそのまま D52 の根拠になる．
#[test]
fn a_contour_onion_hides_far_less_of_the_current_frame_than_a_filled_one() {
    let opts_contour = OnionOptions {
        before: 2,
        after: 2,
        filled: false,
    };

    let (mut n, mut skipped) = (0usize, 0usize);
    let (mut obscured, mut obscured_filled) = (0usize, 0usize);
    let mut worst = 0.0f32;

    for path in png_files(&seeds()) {
        let Ok(img) = px_io::png::read_rgba(&path) else {
            skipped += 1;
            continue;
        };
        let Some((canvas, palette)) = index_exactly(&img) else {
            skipped += 1;
            continue;
        };
        // 透明が無い絵は平行移動しても «動いた» と読めないので測らない
        if canvas.transparent().is_none() {
            skipped += 1;
            continue;
        }
        let frames = walk(&canvas, &palette);
        let (_, r) = onion_image(&frames, 2, &opts_contour, &render());
        if r.obscured_if_filled == 0 {
            skipped += 1;
            continue;
        }
        n += 1;
        obscured += r.obscured;
        obscured_filled += r.obscured_if_filled;
        worst = worst.max(r.obscured_ratio());
        assert_eq!(r.drawn, 4, "{path:?}: 前後 2 コマずつ重ねられていない");
        assert_eq!(r.missing, 0);
        assert!(
            r.obscured < r.obscured_if_filled,
            "{path:?}: 輪郭が塗り潰しと同じだけ隠している"
        );
    }

    assert!(n >= 30, "測れた素材が足りない: {n} (飛ばした {skipped})");
    let ratio = obscured as f32 / obscured_filled as f32;
    println!(
        "実素材 {n} 枚 (飛ばした {skipped}) — 輪郭が隠した画素は塗り潰しの {:.1}% \
         (最悪の 1 枚でも {:.1}%)",
        ratio * 100.0,
        worst * 100.0
    );
    // **半分より少なくならないなら «輪郭のみ» にする理由が無い**
    assert!(
        ratio < 0.5,
        "輪郭でも塗り潰しの {:.1}% を隠している — D52 の根拠が立たない",
        ratio * 100.0
    );
}

/// **壊れると: 列の端で «重ねなかった» と «重ねるものが無かった» が混ざる** (D77 ・D104)．
#[test]
fn the_ends_of_a_sequence_report_the_neighbours_that_were_not_there() {
    let path = png_files(&seeds())
        .into_iter()
        .find(|p| {
            px_io::png::read_rgba(p)
                .ok()
                .and_then(|img| index_exactly(&img))
                .is_some_and(|(c, _)| c.transparent().is_some())
        })
        .expect("透明を持つ素材がある");
    let img = px_io::png::read_rgba(&path).expect("読める");
    let (canvas, palette) = index_exactly(&img).expect("添字にできる");
    let frames = walk(&canvas, &palette);

    let opts = OnionOptions {
        before: 2,
        after: 2,
        filled: false,
    };
    for (index, drawn, missing) in [(0usize, 2usize, 2usize), (2, 4, 0), (4, 2, 2)] {
        let (_, r) = onion_image(&frames, index, &opts, &render());
        assert_eq!(r.drawn, drawn, "コマ {index} で重ねた数が違う");
        assert_eq!(r.missing, missing, "コマ {index} で足りない数が違う");
    }
}
