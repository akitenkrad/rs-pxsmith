use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_io::pxfont::PxFontDocument;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fonts")
        .join(name)
}

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "pxsmith-font-{name}-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .args(args)
        .output()
        .expect("pxsmith を起こせる")
}

#[test]
fn help_describes_all_import_formats_and_shared_clipping() {
    let font_help = run(&["font".as_ref(), "--help".as_ref()]);
    assert!(font_help.status.success());
    let font_stdout = String::from_utf8_lossy(&font_help.stdout);
    assert!(
        font_stdout.contains("BDF ・TTF ・OTF ・TTC を `.pxfont.toml` へ取り込む"),
        "{font_stdout}"
    );

    let import_help = run(&["font".as_ref(), "import".as_ref(), "--help".as_ref()]);
    assert!(import_help.status.success());
    let import_stdout = String::from_utf8_lossy(&import_help.stdout);
    assert!(
        import_stdout.contains("升からはみ出す字形を切る"),
        "{import_stdout}"
    );
}

#[test]
fn check_returns_one_and_lists_missing_characters() {
    let root = scratch("missing");
    let text = root.join("text.txt");
    std::fs::write(&text, "A C\nZ\r\n").unwrap();
    let font = fixture("handmade-mono.pxfont.toml");
    let output = run(&[
        "font".as_ref(),
        "check".as_ref(),
        font.as_os_str(),
        "--text-file".as_ref(),
        text.as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains('C'), "不足字 C が無い:\n{stderr}");
    assert!(stderr.contains('Z'), "不足字 Z が無い:\n{stderr}");
    assert!(
        !stderr.contains("U+000A"),
        "改行を不足字にしている:\n{stderr}"
    );
    assert!(
        !stderr.contains("U+000D"),
        "復帰を不足字にしている:\n{stderr}"
    );
}

#[test]
fn check_returns_zero_and_prints_shape_advisories() {
    let root = scratch("advisories");
    let text = root.join("text.txt");
    std::fs::write(&text, "A ").unwrap();
    let font = fixture("handmade-mono.pxfont.toml");
    let output = run(&[
        "font".as_ref(),
        "check".as_ref(),
        font.as_os_str(),
        "--text-file".as_ref(),
        text.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "検査が失敗した:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("右端の列に墨がある字 1 件（罫線・ブロック要素を除く．隣の字とくっつく）"),
        "右端の助言が規則どおりでない:\n{stdout}"
    );
    assert!(
        stdout.contains("1 画素も無い"),
        "空の字の助言が無い:\n{stdout}"
    );
    assert!(
        stdout.contains("字送りが data の幅より小さい"),
        "字送りの助言が無い:\n{stdout}"
    );
}

#[test]
fn vertical_check_reports_tu_or_tr_without_a_variant() {
    let root = scratch("vertical");
    let text = root.join("text.txt");
    std::fs::write(&text, "、").unwrap();
    let font = fixture("handmade-mono.pxfont.toml");
    let output = run(&[
        "font".as_ref(),
        "check".as_ref(),
        font.as_os_str(),
        "--text-file".as_ref(),
        text.as_os_str(),
        "--vertical".as_ref(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("縦書きの変種不足"), "{stderr}");
    assert!(stderr.contains('、'), "{stderr}");
}

#[test]
fn ttf_import_writes_a_pxfont_document() {
    let root = scratch("ttf");
    let written = root.join("out.pxfont.toml");
    let output = run(&[
        "font".as_ref(),
        "import".as_ref(),
        fixture("synthetic/pxsynth-gpos.ttf").as_os_str(),
        written.as_os_str(),
        "--size".as_ref(),
        "8".as_ref(),
    ]);
    assert!(
        output.status.success(),
        "TTF 取り込みに失敗した:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let font = PxFontDocument::read(&written)
        .unwrap()
        .to_font(&written)
        .unwrap();
    assert_eq!(font.meta.line_height, 8);
    assert_eq!(font.meta.baseline, 6);
    assert!(font.glyphs.iter().any(|glyph| glyph.character == 'A'));
}

#[test]
fn bdf_import_with_chars_file_keeps_only_requested_characters() {
    let root = scratch("chars-file");
    let chars = root.join("used.txt");
    let output = root.join("selected.pxfont.toml");
    std::fs::write(&chars, "AA\nあ\r\n").unwrap();
    let input = fixture("misaki/misaki_gothic.bdf");
    let process = run(&[
        "font".as_ref(),
        "import".as_ref(),
        input.as_os_str(),
        output.as_os_str(),
        "--chars-file".as_ref(),
        chars.as_os_str(),
    ]);
    assert!(
        process.status.success(),
        "取り込みに失敗した:\n{}",
        String::from_utf8_lossy(&process.stderr)
    );
    let font = PxFontDocument::read(&output)
        .unwrap()
        .to_font(&output)
        .unwrap();
    assert_eq!(
        font.glyphs
            .iter()
            .map(|glyph| glyph.character)
            .collect::<Vec<_>>(),
        vec!['A', 'あ']
    );
    assert!(
        String::from_utf8_lossy(&process.stdout).contains("対象外 7169 字"),
        "{}",
        String::from_utf8_lossy(&process.stdout)
    );
}
