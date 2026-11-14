//! 美咲フォントの TTF 版を，同じ配布物の BDF 版と全字・全画素で照合する．

use std::path::{Path, PathBuf};
use std::process::Command;

use pxsmith_core::font::Font;
use pxsmith_io::pxfont::PxFontDocument;

fn fixture(extension: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fonts/misaki")
        .join(format!("misaki_gothic.{extension}"))
}

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-font-ttf-real-{name}-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn import(input: &Path, output: &Path, extra: &[&str]) -> (Font, String) {
    let process = Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .args(["font", "import"])
        .arg(input)
        .arg(output)
        .args(extra)
        .output()
        .expect("pxsmith を起こせる");
    assert!(
        process.status.success(),
        "{} の取り込みに失敗した:\n{}",
        input.display(),
        String::from_utf8_lossy(&process.stderr)
    );
    let font = PxFontDocument::read(output)
        .unwrap()
        .to_font(output)
        .unwrap();
    (font, String::from_utf8(process.stdout).unwrap())
}

fn assert_matches_bdf(actual: &Font, expected: &Font, route: &str) {
    assert_eq!(actual.meta.line_height, 8, "{route} の line_height");
    assert_eq!(actual.meta.baseline, 6, "{route} の baseline");
    assert_eq!(actual.glyphs.len(), expected.glyphs.len(), "{route} の字数");
    for (actual_glyph, expected_glyph) in actual.glyphs.iter().zip(&expected.glyphs) {
        assert_eq!(
            actual_glyph.character, expected_glyph.character,
            "{route} の字の集合"
        );
        assert_eq!(
            actual_glyph.canvas.width(),
            expected_glyph.canvas.width(),
            "{route}: {} の升幅",
            actual_glyph.character
        );
        assert_eq!(
            actual_glyph.canvas.height(),
            expected_glyph.canvas.height(),
            "{route}: {} の高さ",
            actual_glyph.character
        );
        assert_eq!(
            actual_glyph.effective_advance(&actual.meta),
            expected_glyph.effective_advance(&expected.meta),
            "{route}: {} の advance",
            actual_glyph.character
        );
        assert_eq!(
            actual_glyph.canvas.transparent(),
            Some(0),
            "{route}: {} の透明添字",
            actual_glyph.character
        );
        assert_eq!(
            actual_glyph.canvas.pixels(),
            expected_glyph.canvas.pixels(),
            "{route}: {} の墨",
            actual_glyph.character
        );
    }
}

#[test]
fn embedded_bitmap_matches_every_misaki_bdf_glyph() {
    let root = scratch("bitmap");
    let (bdf, _) = import(&fixture("bdf"), &root.join("bdf.pxfont.toml"), &[]);
    let (ttf, stdout) = import(
        &fixture("ttf"),
        &root.join("ttf.pxfont.toml"),
        &["--size", "8"],
    );
    assert_matches_bdf(&ttf, &bdf, "埋め込みビットマップ");
    assert!(stdout.contains("埋め込み"), "経路の報告が無い:\n{stdout}");
}

#[test]
fn outline_matches_every_misaki_bdf_glyph() {
    let root = scratch("outline");
    let (bdf, _) = import(&fixture("bdf"), &root.join("bdf.pxfont.toml"), &[]);
    let (ttf, stdout) = import(
        &fixture("ttf"),
        &root.join("ttf.pxfont.toml"),
        &["--size", "8", "--outline"],
    );
    assert_matches_bdf(&ttf, &bdf, "輪郭");
    assert!(stdout.contains("輪郭"), "経路の報告が無い:\n{stdout}");
}
