//! 合成 TTF の画素・GSUB・GPOS/kern・格子外助言を CLI から検査する．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::font::{Font, Glyph};
use pxsmith_io::pxfont::PxFontDocument;

const EXPECTED: &[(char, Option<&str>, &[&str])] = &[
    (
        ' ',
        None,
        &[
            "....", "....", "....", "....", "....", "....", "....", "....",
        ],
    ),
    (
        'A',
        None,
        &[
            "..#...", ".#.#..", "#...#.", "#####.", "#...#.", "#...#.", "......", "......",
        ],
    ),
    (
        'O',
        None,
        &[
            ".###..", "#...#.", "#...#.", "#...#.", "#...#.", ".###..", "......", "......",
        ],
    ),
    (
        'V',
        None,
        &[
            "#...#.", "#...#.", "#...#.", ".#.#..", ".#.#..", "..#...", "......", "......",
        ],
    ),
    (
        'ー',
        None,
        &[
            "........", "........", "........", "#######.", "........", "........", "........",
            "........",
        ],
    ),
    (
        'ー',
        Some("vert"),
        &[
            "...#....", "...#....", "...#....", "...#....", "...#....", "...#....", "...#....",
            "........",
        ],
    ),
    (
        '（',
        None,
        &[
            ".....#..", "....#...", "....#...", "....#...", "....#...", "....#...", ".....#..",
            "........",
        ],
    ),
    (
        '（',
        Some("vert"),
        &[
            "........", "........", "........", "........", "........", ".######.", "#......#",
            "........",
        ],
    ),
];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fonts/synthetic")
        .join(name)
}

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-font-ttf-synth-{name}-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn import(name: &str) -> (Font, Output) {
    let root = scratch(name);
    let output = root.join("out.pxfont.toml");
    let process = Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .args(["font", "import"])
        .arg(fixture(name))
        .arg(&output)
        .args(["--size", "8"])
        .output()
        .expect("pxsmith を起こせる");
    assert!(
        process.status.success(),
        "{name} の取り込みに失敗した:\n{}",
        String::from_utf8_lossy(&process.stderr)
    );
    let font = PxFontDocument::read(&output)
        .unwrap()
        .to_font(&output)
        .unwrap();
    (font, process)
}

fn rows(glyph: &Glyph) -> Vec<String> {
    (0..glyph.canvas.height())
        .map(|y| {
            (0..glyph.canvas.width())
                .map(|x| {
                    if glyph.canvas.get(x as i32, y as i32) == Some(1) {
                        '#'
                    } else {
                        '.'
                    }
                })
                .collect()
        })
        .collect()
}

fn assert_synthetic(name: &str) {
    let (font, process) = import(name);
    assert_eq!(font.meta.line_height, 8);
    assert_eq!(font.meta.baseline, 6);
    assert_eq!(
        font.meta.copyright.as_deref(),
        Some("pxsmith synthetic test font (CC0)\nCC0 1.0 Universal")
    );
    let glyphs = font
        .glyphs
        .iter()
        .map(|glyph| ((glyph.character, glyph.variant.as_deref()), glyph))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(glyphs.len(), EXPECTED.len());
    for &(character, variant, expected) in EXPECTED {
        let glyph = glyphs
            .get(&(character, variant))
            .unwrap_or_else(|| panic!("{character:?}/{variant:?} が無い"));
        assert_eq!(glyph.canvas.transparent(), Some(0));
        assert_eq!(rows(glyph), expected, "{character:?}/{variant:?} の墨");
    }
    assert_eq!(font.kerns.len(), 1);
    assert_eq!(font.kerns[0].pair, "AV");
    assert_eq!(font.kerns[0].amount, -1);
    let stdout = String::from_utf8_lossy(&process.stdout);
    assert!(
        stdout.contains("格子") && stdout.contains("助言"),
        "格子外の助言が無い:\n{stdout}"
    );
    assert!(
        stdout.contains("縦書き 2"),
        "縦書き字形の件数が無い:\n{stdout}"
    );
    assert!(
        stdout.contains("カーニング 1"),
        "カーニングの件数が無い:\n{stdout}"
    );
}

#[test]
fn gpos_font_matches_source_bitmaps_and_layout() {
    assert_synthetic("pxsynth-gpos.ttf");
}

#[test]
fn legacy_kern_font_matches_source_bitmaps_and_layout() {
    assert_synthetic("pxsynth-kern.ttf");
}
