//! **拡縮と回転を実素材に掛けて «壊していないか» を測る** (設計書 5 章 ・D18)．
//!
//! 拡縮も回転も «標本を選ぶ» 道具である．したがって不変条件は 2 つ —
//! **色を作らないこと** (D94) と，**厳密に書ける場面では厳密であること**．
//!
//! | 場面 | 真値 |
//! | --- | --- |
//! | 90 度の倍数の回転 | **厳密**．4 回まわせば元に戻る |
//! | 整数倍の拡大 (`nearest`) | **厳密**．1 画素が $k \times k$ になる |
//! | 任意の角度 | 色を作らない ・画布を広げれば切れない |
//!
//! **飛ばした件も数える** (D128)．

use std::path::{Path, PathBuf};

use px_core::canvas::{IndexedCanvas, RgbaCanvas};
use px_core::color::Rgba8;
use px_core::palette::Palette;
use px_core::resample::{ResampleAlgo, ResampleOptions, rotate, scale};

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

fn corpus() -> Vec<(String, IndexedCanvas, Palette)> {
    png_files(&seeds())
        .into_iter()
        .filter_map(|p| {
            let img = px_io::png::read_rgba(&p).ok()?;
            let (c, pal) = index_exactly(&img)?;
            Some((p.file_stem()?.to_string_lossy().to_string(), c, pal))
        })
        .collect()
}

/// **壊れると: 拡縮 ・回転が入力に無い添字を出す** (D94)．
#[test]
fn resampling_real_art_never_creates_a_colour() {
    let art = corpus();
    assert!(art.len() >= 60, "実素材が足りない: {}", art.len());
    for (name, canvas, palette) in &art {
        let mut seen: Vec<u8> = canvas.pixels().to_vec();
        seen.sort_unstable();
        seen.dedup();
        for algo in ResampleAlgo::ALL {
            let opts = ResampleOptions {
                algo,
                ..Default::default()
            };
            for degrees in [13.0f32, 30.0, 47.0] {
                let (out, r) = rotate(canvas, palette, degrees, &opts).expect("回せる");
                for i in out.pixels() {
                    assert!(
                        seen.binary_search(i).is_ok(),
                        "{name}: {} で入力に無い添字 {i} が出た",
                        algo.as_str()
                    );
                }
                assert_eq!(r.clipped, 0, "{name}: 広げたのに切れた");
            }
            let (out, _) = scale(canvas, palette, 2.5, &opts).expect("拡縮できる");
            for i in out.pixels() {
                assert!(
                    seen.binary_search(i).is_ok(),
                    "{name}: 拡縮で入力に無い添字 {i} が出た"
                );
            }
        }
    }
    println!("実素材 {} 枚 x 流儀 2 x 角度 3 + 拡縮", art.len());
}

/// **壊れると: 90 度の倍数が厳密でなくなる．**
#[test]
fn quarter_turns_are_exact_on_real_art() {
    let art = corpus();
    for (name, canvas, palette) in &art {
        for algo in ResampleAlgo::ALL {
            let opts = ResampleOptions {
                algo,
                ..Default::default()
            };
            let mut cur = canvas.clone();
            for _ in 0..4 {
                cur = rotate(&cur, palette, 90.0, &opts).expect("回せる").0;
            }
            assert_eq!(
                cur.pixels(),
                canvas.pixels(),
                "{name}: 4 回まわして戻らない"
            );
            assert_eq!(cur.size(), canvas.size());
        }
    }
    assert!(art.len() >= 60);
}

/// **壊れると: `nearest` の整数倍拡大が厳密でなくなる．**
///
/// **`cleanedge` は厳密ではない** — 縁を作り直すのだから当然で，
/// 実測で 61 枚すべてが «1 画素が $k \times k$» にならない．
/// 拡大だけが目的なら `nearest` を使う，という判断の根拠である．
#[test]
fn integer_upscale_is_exact_for_nearest_only() {
    let art = corpus();
    let (mut nearest_ok, mut cleanedge_ok) = (0usize, 0usize);
    for (_, canvas, palette) in &art {
        for algo in ResampleAlgo::ALL {
            let opts = ResampleOptions {
                algo,
                ..Default::default()
            };
            let (big, _) = scale(canvas, palette, 3.0, &opts).expect("拡縮できる");
            let exact = (0..canvas.height() as i32).all(|y| {
                (0..canvas.width() as i32).all(|x| {
                    (0..3).all(|dy| {
                        (0..3).all(|dx| big.get(x * 3 + dx, y * 3 + dy) == canvas.get(x, y))
                    })
                })
            });
            if exact {
                match algo {
                    ResampleAlgo::Nearest => nearest_ok += 1,
                    ResampleAlgo::CleanEdge => cleanedge_ok += 1,
                }
            }
        }
    }
    assert_eq!(nearest_ok, art.len(), "nearest は全件で厳密のはず");
    assert_eq!(
        cleanedge_ok, 0,
        "cleanedge が厳密になったなら移植が効いていない"
    );
}
