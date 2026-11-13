//! `pxsmith quantize` が透明の形を各出力形式で保つことを実プロセスで確かめる．

use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::quantize::{ApplyOptions, QuantizeMethod};
use pxsmith_core::{Frame, FrameId, IndexedCanvas, RgbaCanvas};

fn repo(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-quantize-{name}-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("作業場所を作れる");
    root
}

fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .args(args)
        .output()
        .expect("pxsmith を起こせる")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "pxsmith が失敗した:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn rgba_mask(canvas: &RgbaCanvas) -> Vec<bool> {
    canvas.pixels().iter().map(|color| color.a != 0).collect()
}

fn indexed_mask(canvas: &IndexedCanvas) -> Vec<bool> {
    (0..canvas.height() as i32)
        .flat_map(|y| {
            (0..canvas.width() as i32)
                .map(move |x| !canvas.is_transparent_at(pxsmith_core::ivec2(x, y)))
        })
        .collect()
}

fn frame_mask(frame: &Frame) -> Vec<bool> {
    let canvas = frame.layers[0]
        .surface
        .as_indexed()
        .expect("インデックスカラー");
    indexed_mask(canvas)
}

#[test]
fn real_art_keeps_the_same_opaque_pixels_in_png_aseprite_and_l0() {
    let root = scratch("all-formats");
    let input = repo("testdata/grid-eval/seeds/crawl_cap1.png");
    let source = pxsmith_io::png::read_rgba(&input).expect("入力 PNG を読める");
    let expected = rgba_mask(&source);
    assert_eq!(expected.iter().filter(|opaque| **opaque).count(), 571);

    let png = root.join("out.png");
    let aseprite = root.join("out.aseprite");
    let l0 = root.join("out.px.toml");
    for output in [&png, &aseprite, &l0] {
        let result = run(&[
            "quantize".as_ref(),
            input.as_os_str(),
            output.as_os_str(),
            "--colors".as_ref(),
            "255".as_ref(),
        ]);
        assert_success(&result);
    }

    let png_mask = rgba_mask(&pxsmith_io::png::read_rgba(&png).expect("出力 PNG を読める"));
    assert_eq!(png_mask, expected, "PNG の不透明画素の位置");

    let document = pxsmith_io::Document::read(&aseprite).expect("Aseprite を読める");
    let aseprite_mask = frame_mask(&document.project(FrameId(0)).expect("フレームを射影できる"));
    assert_eq!(aseprite_mask, expected, "Aseprite の不透明画素の位置");

    let l0_document = pxsmith_io::l0::L0Document::read(&l0).expect("L0 を読める");
    let l0_frames = l0_document.to_frames(&l0).expect("L0 を作業層へ変換できる");
    assert_eq!(frame_mask(&l0_frames[0]), expected, "L0 の不透明画素の位置");
    assert!(
        std::fs::read_to_string(&l0)
            .expect("L0 本文を読める")
            .contains("\".\" = \"transparent\""),
        "L0 の透明画素は '.' で表す"
    );
}

#[test]
fn opaque_png_output_is_byte_identical_to_the_previous_pipeline() {
    let root = scratch("opaque-byte-stability");
    let input = repo("testdata/lint-cases/negative/jaggy-00.png");
    let actual = root.join("actual.png");
    let expected = root.join("expected.png");

    let result = run(&[
        "quantize".as_ref(),
        input.as_os_str(),
        actual.as_os_str(),
        "--colors".as_ref(),
        "62".as_ref(),
    ]);
    assert_success(&result);

    let image = pxsmith_io::png::read_rgba(&input).expect("入力 PNG を読める");
    assert!(image.pixels().iter().all(|color| color.a != 0));
    let palette = pxsmith_core::quantize::quantize(
        &image,
        NonZeroU8::new(62).expect("非ゼロ"),
        QuantizeMethod::Wu,
    )
    .expect("従来どおり量子化できる");
    let canvas = pxsmith_core::quantize::apply_palette(&image, &palette, &ApplyOptions::default());
    pxsmith_io::png::write_indexed(&expected, &canvas, &palette).expect("期待 PNG を書ける");

    assert_eq!(
        std::fs::read(&actual).expect("実出力を読める"),
        std::fs::read(&expected).expect("期待出力を読める")
    );
}
