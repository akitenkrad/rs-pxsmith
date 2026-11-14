//! `sheet pack --palette-swap` から `sheet resolve` までを実プロセスで通す．

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::{Frame, IndexedCanvas, Layer, LayerMeta, Palette, Rgba8, Surface, uvec2};

fn repo(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn scratch() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-sheet-palette-swap-{}-{}",
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("test")
            .replace("::", "-")
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

fn same_colour(a: Rgba8, b: Rgba8) -> bool {
    (a.a == 0 && b.a == 0) || a == b
}

#[test]
fn pack_then_resolve_matches_plain_sheet_and_variant_changes_only_colour() {
    let root = scratch();
    let input = repo("examples/sprites/hero_body.px.toml");
    let base = repo("palettes/sample.hex");
    let red = root.join("red.hex");
    std::fs::write(&red, "101018\nff1020\nff7080\nfff0d0\n").expect("色違いを書く");

    let plain = root.join("plain.png");
    let plain_meta = root.join("plain.sheet.json");
    let output = run(&[
        "sheet".as_ref(),
        "pack".as_ref(),
        plain.as_os_str(),
        "--input".as_ref(),
        input.as_os_str(),
        "--meta".as_ref(),
        plain_meta.as_os_str(),
    ]);
    assert_success(&output);

    let indexed = root.join("sheet.png");
    let indexed_meta = root.join("sheet.sheet.json");
    let variant_arg = format!("{}={}", base.display(), red.display());
    let output = run(&[
        "sheet".as_ref(),
        "pack".as_ref(),
        indexed.as_os_str(),
        "--input".as_ref(),
        input.as_os_str(),
        "--meta".as_ref(),
        indexed_meta.as_os_str(),
        "--palette-swap".as_ref(),
        "--variant".as_ref(),
        variant_arg.as_ref(),
    ]);
    assert_success(&output);
    assert!(root.join("sheet.palettes.png").is_file());

    let resolved = root.join("resolved.png");
    let output = run(&[
        "sheet".as_ref(),
        "resolve".as_ref(),
        indexed.as_os_str(),
        "--meta".as_ref(),
        indexed_meta.as_os_str(),
        "-o".as_ref(),
        resolved.as_os_str(),
    ]);
    assert_success(&output);

    let changed = root.join("changed.png");
    let output = run(&[
        "sheet".as_ref(),
        "resolve".as_ref(),
        indexed.as_os_str(),
        "--meta".as_ref(),
        indexed_meta.as_os_str(),
        "--variant".as_ref(),
        "red".as_ref(),
        "-o".as_ref(),
        changed.as_os_str(),
    ]);
    assert_success(&output);

    let plain = pxsmith_io::png::read_rgba(&plain).expect("通常シートを読める");
    let resolved = pxsmith_io::png::read_rgba(&resolved).expect("復元シートを読める");
    assert_eq!(
        (plain.width(), plain.height()),
        (resolved.width(), resolved.height())
    );
    assert!(
        plain
            .pixels()
            .iter()
            .zip(resolved.pixels())
            .all(|(a, b)| same_colour(*a, *b)),
        "基の復元は通常シートと全画素一致する"
    );

    let changed = pxsmith_io::png::read_rgba(&changed).expect("色違いシートを読める");
    assert!(
        resolved
            .pixels()
            .iter()
            .zip(changed.pixels())
            .any(|(a, b)| a.a != 0 && a != b),
        "色違いで不透明色が変わる"
    );
    assert!(
        resolved
            .pixels()
            .iter()
            .zip(changed.pixels())
            .all(|(a, b)| (a.a == 0) == (b.a == 0)),
        "色違いでも透明の形は同じ"
    );
}

#[test]
fn l0_without_a_transparent_key_keeps_opaque_index_zero_when_resolved() {
    let root = scratch();
    let input = repo("testdata/lint-cases/negative/jaggy-00.png");
    let l0 = root.join("tile.px.toml");
    let output = run(&[
        "quantize".as_ref(),
        input.as_os_str(),
        l0.as_os_str(),
        "--colors".as_ref(),
        "62".as_ref(),
    ]);
    assert_success(&output);
    assert!(
        !std::fs::read_to_string(&l0)
            .expect("L0 を読める")
            .contains("transparent"),
        "再現用 L0 は透明キーを持たない"
    );

    let plain = root.join("plain.png");
    let output = run(&[
        "sheet".as_ref(),
        "pack".as_ref(),
        plain.as_os_str(),
        "--input".as_ref(),
        l0.as_os_str(),
    ]);
    assert_success(&output);

    let indexed = root.join("indexed.png");
    let meta = root.join("indexed.sheet.json");
    let output = run(&[
        "sheet".as_ref(),
        "pack".as_ref(),
        indexed.as_os_str(),
        "--input".as_ref(),
        l0.as_os_str(),
        "--meta".as_ref(),
        meta.as_os_str(),
        "--palette-swap".as_ref(),
    ]);
    assert_success(&output);

    let resolved = root.join("resolved.png");
    let output = run(&[
        "sheet".as_ref(),
        "resolve".as_ref(),
        indexed.as_os_str(),
        "--meta".as_ref(),
        meta.as_os_str(),
        "-o".as_ref(),
        resolved.as_os_str(),
    ]);
    assert_success(&output);

    let plain = pxsmith_io::png::read_rgba(&plain).expect("通常シートを読める");
    let resolved = pxsmith_io::png::read_rgba(&resolved).expect("復元シートを読める");
    assert_eq!(plain.width(), resolved.width());
    assert_eq!(plain.height(), resolved.height());
    assert!(
        plain
            .pixels()
            .iter()
            .zip(resolved.pixels())
            .all(|(a, b)| same_colour(*a, *b)),
        "透明キーの無い L0 を復元した色が通常梱包と違う"
    );
}

#[test]
fn pack_treats_an_aseprite_transparent_index_as_transparent_even_if_its_colour_is_opaque() {
    let root = scratch();
    let input = root.join("opaque-transparent-index.aseprite");
    let palette =
        Palette::new(vec![Rgba8::rgb(200, 30, 30), Rgba8::rgb(30, 200, 30)]).expect("パレット");
    let canvas = IndexedCanvas::from_pixels(2, 1, vec![1, 0])
        .expect("画素数")
        .with_transparent(Some(1));
    let mut frame = Frame::new(uvec2(2, 1), palette);
    frame.layers.push(Layer::new(
        LayerMeta::named("art"),
        Surface::Indexed(canvas),
    ));
    pxsmith_io::Document::from_frames(&[frame])
        .expect("文書を作れる")
        .write(&input)
        .expect("Aseprite を書ける");

    let sheet = root.join("sheet.png");
    let output = run(&[
        "sheet".as_ref(),
        "pack".as_ref(),
        sheet.as_os_str(),
        "--input".as_ref(),
        input.as_os_str(),
    ]);
    assert_success(&output);

    let sheet = pxsmith_io::png::read_rgba(&sheet).expect("シートを読める");
    assert_eq!(sheet.get(0, 0), Some(Rgba8::TRANSPARENT));
    assert_eq!(sheet.get(1, 0), Some(Rgba8::rgb(200, 30, 30)));
}

#[test]
fn palette_swap_rejects_aseprite_output() {
    let root = scratch();
    let output = run(&[
        "sheet".as_ref(),
        "pack".as_ref(),
        root.join("sheet.aseprite").as_os_str(),
        "--input".as_ref(),
        repo("examples/sprites/hero_body.px.toml").as_os_str(),
        "--palette-swap".as_ref(),
    ]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("PNG"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn export_tiled_warns_that_format_two_is_an_index_texture() {
    let root = scratch();
    let sheet = root.join("sheet.png");
    let meta = root.join("sheet.sheet.json");
    let output = run(&[
        "sheet".as_ref(),
        "pack".as_ref(),
        sheet.as_os_str(),
        "--input".as_ref(),
        repo("examples/sprites/hero_body.px.toml").as_os_str(),
        "--meta".as_ref(),
        meta.as_os_str(),
        "--palette-swap".as_ref(),
    ]);
    assert_success(&output);

    let tiles = root.join("tiles.json");
    std::fs::write(&tiles, r#"{"format":1,"tile":16,"tiles":2}"#).expect("タイルメタを書く");
    let tsx = root.join("tiles.tsx");
    let output = run(&[
        "export".as_ref(),
        "tiled".as_ref(),
        tiles.as_os_str(),
        "--tsx".as_ref(),
        tsx.as_os_str(),
        "--sheet".as_ref(),
        meta.as_os_str(),
    ]);
    assert_success(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("添字テクスチャ") && stderr.contains("Tiled"),
        "警告が標準エラーに無い: {stderr}"
    );
    assert!(tsx.is_file(), "警告しても升の幾何は出力する");
}
