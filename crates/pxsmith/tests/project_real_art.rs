//! **投影を実素材に掛けて «壊していないか» を測る** (設計書 6.13)．
//!
//! 投影も «標本を選ぶ» 道具なので，不変条件は拡縮 ・回転と同じ **色を作らない**
//! (D94) である．そこに投影だけの不変条件が 3 つ足される．
//!
//! | 主張 | 不変条件 |
//! | --- | --- |
//! | 横から見た絵を写す | **垂直線は立ったまま** ($y$ 軸が動かない) |
//! | 斜投影は縮まない | **面積が変わらない** (行列式 1) |
//! | 等角は 2:1 | **受ける軸が 26.57 度** (画布は切り上げの端数まで) |
//!
//! 3 つ目は**変換が持つ性質**であって画布が持つ性質ではない — 連続量の比は
//! ちょうど 2.000 だが，**切り上げは軸ごとに独立**なので 16x16 は 24x12 では
//! なく 23x12 になる (代数であって測定ではない．D105 ・D122 と同じ側) ．
//!
//! **飛ばした件も数える** (D128)．

use std::path::{Path, PathBuf};

use pxsmith_core::canvas::{IndexedCanvas, RgbaCanvas};
use pxsmith_core::color::Rgba8;
use pxsmith_core::palette::Palette;
use pxsmith_core::project::{Facing, ProjectOptions, Projection, SourcePlane, Step, project};
use pxsmith_core::resample::{ResampleAlgo, ResampleOptions};

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

/// 実素材と，**添字にできなかった枚数**．
fn corpus() -> (Vec<(String, IndexedCanvas, Palette)>, usize) {
    let mut skipped = 0usize;
    let art = png_files(&seeds())
        .into_iter()
        .filter_map(|p| {
            let Ok(img) = pxsmith_io::png::read_rgba(&p) else {
                skipped += 1;
                return None;
            };
            match index_exactly(&img) {
                Some((c, pal)) => Some((
                    p.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    c,
                    pal,
                )),
                None => {
                    skipped += 1;
                    None
                }
            }
        })
        .collect();
    (art, skipped)
}

fn opts(projection: Projection, plane: SourcePlane) -> ProjectOptions {
    ProjectOptions {
        projection,
        plane,
        facing: Facing::Right,
        step: None,
        resample: ResampleOptions {
            algo: ResampleAlgo::Nearest,
            grow: true,
        },
    }
}

/// **壊れると: 投影が入力に無い添字を出す** (D94)．
#[test]
fn projecting_real_art_never_creates_a_colour() {
    let (art, skipped) = corpus();
    assert!(art.len() >= 60, "実素材が足りない: {}", art.len());
    for (name, canvas, palette) in &art {
        let mut seen: Vec<u8> = canvas.pixels().to_vec();
        seen.sort_unstable();
        seen.dedup();
        for projection in Projection::ALL {
            for plane in SourcePlane::ALL {
                for facing in Facing::ALL {
                    let mut o = opts(projection, plane);
                    o.facing = facing;
                    let (out, r) = project(canvas, palette, &o).expect("写せる");
                    for i in out.pixels() {
                        assert!(
                            seen.binary_search(i).is_ok(),
                            "{name}: {}/{plane:?} で入力に無い添字 {i} が出た",
                            projection.as_str()
                        );
                    }
                    assert_eq!(
                        r.resample.clipped,
                        0,
                        "{name}: {}/{plane:?} で広げたのに切れた",
                        projection.as_str()
                    );
                }
            }
        }
    }
    println!(
        "実素材 {} 枚 x 投影 3 x 面 2 x 向き 2 (飛ばした {skipped})",
        art.len()
    );
}

/// **壊れると: 横から見た絵を写したのに垂直線が倒れる．**
#[test]
fn a_side_view_keeps_its_vertical_lines_on_real_art() {
    let (art, _) = corpus();
    for (name, canvas, palette) in &art {
        for projection in Projection::ALL {
            let (_, r) =
                project(canvas, palette, &opts(projection, SourcePlane::Side)).expect("写せる");
            assert!(
                r.keeps_vertical,
                "{name}: {} で垂直線が倒れた",
                projection.as_str()
            );
        }
    }
    assert!(art.len() >= 60);
}

/// **壊れると: 斜投影が縮む (斜投影は純粋な歪みである)．**
#[test]
fn an_oblique_projection_preserves_area_on_real_art() {
    let (art, _) = corpus();
    for (name, canvas, palette) in &art {
        for plane in SourcePlane::ALL {
            let (_, r) =
                project(canvas, palette, &opts(Projection::Oblique, plane)).expect("写せる");
            assert!(
                (r.area_ratio - 1.0).abs() < 1e-4,
                "{name}: {plane:?} で面積が {} 倍になった",
                r.area_ratio
            );
        }
    }
    assert!(art.len() >= 60);
}

/// **壊れると: 等角の段が 2:1 でなくなる．**
///
/// > [!warning] **2:1 が在るのは変換の側であって，整数の画布の側ではない．**
/// > 最初この試験を «正方形の画布は幅が高さのちょうど 2 倍になる» と書いたら
/// > 落ちた — 16x16 は $16\sqrt 2 = 22.627$ と $11.314$ へ写るので，
/// > **切り上げを軸ごとに独立に取ると 23x12 になる** (24x12 ではない) ．
/// > 連続量の比はちょうど 2.000 で，1 画素の差は切り上げの端数である．
/// >
/// > 画布を 2:1 へ丸めれば «きれい» にはなるが，それは**描かれていない画素を
/// > 足す**ことである (D93 «画布は結果であって入力ではない») ．
/// > **足さずに，比が在る場所を正しく言う．**
#[test]
fn an_iso_projection_recedes_at_two_to_one() {
    let (art, _) = corpus();
    let mut square = 0usize;
    for (name, canvas, palette) in &art {
        if canvas.width() != canvas.height() {
            continue;
        }
        square += 1;
        let (_, r) =
            project(canvas, palette, &opts(Projection::Iso, SourcePlane::Top)).expect("写せる");

        // 段は変換が持っている — ここがちょうど 2:1 である
        assert_eq!(r.step, Step::TWO_TO_ONE);
        assert!(
            (r.degrees - 26.565).abs() < 0.01,
            "{name}: 受ける軸が {} 度になった",
            r.degrees
        );

        // 画布は連続量を軸ごとに切り上げたものなので，差は 1 画素まで
        let (w, h) = (r.resample.size.1.0 as i64, r.resample.size.1.1 as i64);
        assert!(
            (w - h * 2).abs() <= 1,
            "{name}: {w}x{h} は 2:1 から切り上げの端数を超えて離れている"
        );
    }
    assert!(square >= 40, "正方形の種が足りない: {square}");
}

/// **壊れると: 透明の宣言が無い絵を広げたとき，実色で埋めたことを黙る．**
///
/// 全面不透明な素材では «不透明な画素が増えた» としか見えず，増えた分が絵なのか
/// 埋め草なのか読めなくなる (端から端まで CLI で通して出た)．
#[test]
fn art_with_no_transparent_index_reports_the_filler_it_painted() {
    let (art, _) = corpus();
    let (mut opaque_art, mut reported) = (0usize, 0usize);
    for (_, canvas, palette) in &art {
        if canvas.transparent().is_some() {
            continue;
        }
        opaque_art += 1;
        let (_, r) =
            project(canvas, palette, &opts(Projection::Iso, SourcePlane::Top)).expect("写せる");
        if r.resample.filled_opaque > 0 {
            reported += 1;
        }
    }
    assert!(opaque_art > 0, "透明を宣言していない素材が 1 枚も無い");
    assert_eq!(
        reported, opaque_art,
        "透明添字の無い絵を広げたのに埋め草を数えていない"
    );
}
