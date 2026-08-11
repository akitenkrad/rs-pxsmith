//! `pixels!` の展開結果を確かめる．
//!
//! 「行長不一致がコンパイルエラー」の側はここでは試せない (コンパイルが通らない
//! コードはテストに書けない)．`tests/compile_fail/` に例を置いてあるので，
//! 挙動を確かめたいときは `docs/investigations/pixels-macro.md` の手順を使う．

use px_core::{FrameKind, IndexedCanvas, Rgba8};
use px_macro::pixels;

fn canvas(frames: &[px_core::Frame], i: usize) -> &IndexedCanvas {
    frames[i].layers[0].surface.as_indexed().unwrap()
}

#[test]
fn file_form_reads_every_frame() {
    let frames = pixels!("tests/fixtures/hero.px.toml");
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].duration_ms, 83);
    assert_eq!(frames[0].kind, FrameKind::Key);
    assert_eq!(frames[1].duration_ms, 42);
    assert_eq!(frames[1].kind, FrameKind::Inbetween);
    assert_eq!(frames[0].layers[0].meta.name, "body");
}

#[test]
fn file_form_pixels_match_the_text() {
    let frames = pixels!("tests/fixtures/hero.px.toml");
    let c = canvas(&frames, 0);
    assert_eq!((c.width(), c.height()), (6, 3));
    let t = c.transparent().unwrap();
    // "..kk.." — 両端が透明，中の 2 つが添字 0
    assert_eq!(c.get(0, 0), Some(t));
    assert_eq!(c.get(2, 0), Some(0));
    assert_eq!(c.get(3, 0), Some(0));
    // ".khhk." — 中が添字 12
    assert_eq!(c.get(1, 1), Some(0));
    assert_eq!(c.get(2, 1), Some(12));
}

#[test]
fn file_form_carries_the_palette() {
    let frames = pixels!("tests/fixtures/hero.px.toml");
    // sweetie-16 の先頭は 1a1c2c
    assert_eq!(frames[0].palette.get(0), Some(Rgba8::rgb(0x1a, 0x1c, 0x2c)));
    // 透明の受け皿がパレット末尾へ足されている
    let t = canvas(&frames, 0).transparent().unwrap();
    assert_eq!(frames[0].palette.get(t), Some(Rgba8::TRANSPARENT));
    assert_eq!(frames[0].palette.len(), 17);
}

#[test]
fn inline_form_builds_one_frame() {
    let frames = pixels! {
        palette = "tests/fixtures/pal.hex",
        layer = "body",
        duration_ms = 83,
        map = { '.' = transparent, 'k' = 0, 'h' = 12 },
        rows = [
            "..kk..",
            ".khhk.",
            "..kk..",
        ],
    };
    assert_eq!(frames.len(), 1);
    let c = canvas(&frames, 0);
    assert_eq!((c.width(), c.height()), (6, 3));
    assert_eq!(c.get(2, 1), Some(12));
    assert_eq!(frames[0].duration_ms, 83);
    assert_eq!(frames[0].layers[0].meta.name, "body");
}

#[test]
fn inline_and_file_forms_agree() {
    let from_file = pixels!("tests/fixtures/hero.px.toml");
    let inline = pixels! {
        palette = "tests/fixtures/pal.hex",
        layer = "body",
        duration_ms = 83,
        map = { '.' = transparent, 'k' = 0, 'h' = 12 },
        rows = [
            "..kk..",
            ".khhk.",
            "..kk..",
        ],
    };
    assert_eq!(canvas(&from_file, 0).pixels(), canvas(&inline, 0).pixels());
    assert_eq!(from_file[0].palette, inline[0].palette);
}

#[test]
fn inline_defaults_are_applied() {
    let frames = pixels! {
        palette = "tests/fixtures/pal.hex",
        map = { '.' = transparent, 'k' = 3 },
        rows = ["kk", ".."],
    };
    assert_eq!(frames[0].duration_ms, 100, "既定は 100ms");
    assert_eq!(frames[0].kind, FrameKind::Key, "既定は key");
    assert_eq!(frames[0].layers[0].meta.name, "pixels");
}

#[test]
fn inline_kind_is_honoured() {
    let frames = pixels! {
        palette = "tests/fixtures/pal.hex",
        kind = "breakdown",
        map = { 'k' = 3 },
        rows = ["kk"],
    };
    assert_eq!(frames[0].kind, FrameKind::Breakdown);
}

#[test]
fn a_sprite_without_transparency_has_no_transparent_index() {
    let frames = pixels! {
        palette = "tests/fixtures/pal.hex",
        map = { 'k' = 3, 'h' = 4 },
        rows = ["kh", "hk"],
    };
    let c = canvas(&frames, 0);
    assert_eq!(c.transparent(), None);
    assert_eq!(frames[0].palette.len(), 16, "受け皿は足されない");
}
