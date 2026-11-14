use std::path::{Path, PathBuf};

use pxsmith_io::pxfont::PxFontDocument;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fonts")
        .join(name)
}

#[test]
fn handwritten_mono_and_color_fonts_are_canonical() {
    for name in ["handmade-mono.pxfont.toml", "handmade-color.pxfont.toml"] {
        let path = fixture(name);
        let original = std::fs::read_to_string(&path).expect("手書き素材を読める");
        let document = PxFontDocument::read(&path).expect("pxfont を読める");
        let font = document.to_font(&path).expect("フォントモデルへ変換できる");
        let rewritten = PxFontDocument::from_font(&font)
            .expect("文書へ変換できる")
            .to_toml()
            .expect("正規 TOML を作れる");
        assert_eq!(rewritten, original, "{name} が正規形でない");

        let reparsed = PxFontDocument::parse(&rewritten, &path)
            .expect("書いた正規 TOML を読み直せる")
            .to_font(&path)
            .expect("読み直した文書をモデルへ変換できる");
        assert_eq!(font, reparsed, "{name} の読み書きでモデルが変わった");
    }
}

#[test]
fn canonical_output_sorts_glyphs_and_kerns_by_codepoint() {
    let path = fixture("handmade-mono.pxfont.toml");
    let mut font = PxFontDocument::read(&path).unwrap().to_font(&path).unwrap();
    font.glyphs.reverse();
    font.kerns.reverse();

    let text = PxFontDocument::from_font(&font).unwrap().to_toml().unwrap();
    let space = text.find("char = \"U+0020\"").unwrap();
    let latin = text.find("char = \"A\"").unwrap();
    let long_mark = text.find("char = \"ー\"").unwrap();
    assert!(
        space < latin && latin < long_mark,
        "符号位置順でない:\n{text}"
    );
}
