//! フォント焼き込みを実プロセスと独立な画素配置で突き合わせる．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::{
    Font, FontLayoutOptions, IndexedCanvas, Kinsoku, Rgba8, RgbaCanvas, layout_text,
};
use pxsmith_io::pxfont::PxFontDocument;

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fonts")
        .join(path)
}

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-font-render-{name}-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
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

fn import_misaki(root: &Path) -> (PathBuf, Font) {
    let output = root.join("misaki.pxfont.toml");
    let process = run(&[
        "font".as_ref(),
        "import".as_ref(),
        fixture("misaki/misaki_gothic.bdf").as_os_str(),
        output.as_os_str(),
    ]);
    assert_success(&process);
    let font = PxFontDocument::read(&output)
        .unwrap()
        .to_font(&output)
        .unwrap();
    (output, font)
}

/// 製品の組版処理を使わず，基底字形を左から字送りどおりに置く．
fn assemble_lines(font: &Font, lines: &[&str], ink: Rgba8, kern: bool) -> RgbaCanvas {
    let glyphs = font
        .glyphs
        .iter()
        .filter(|glyph| glyph.variant.is_none())
        .map(|glyph| (glyph.character, glyph))
        .collect::<BTreeMap<_, _>>();
    let kerns = font
        .kerns
        .iter()
        .map(|item| (item.pair.as_str(), item.amount))
        .collect::<BTreeMap<_, _>>();
    let line_width = |line: &str| {
        let characters = line.chars().collect::<Vec<_>>();
        let advances: i32 = characters
            .iter()
            .map(|character| glyphs[character].effective_advance(&font.meta) as i32)
            .sum();
        let adjustment = if kern {
            characters
                .windows(2)
                .map(|pair| {
                    let key = format!("{}{}", pair[0], pair[1]);
                    kerns.get(key.as_str()).copied().unwrap_or(0)
                })
                .sum()
        } else {
            0
        };
        (advances + adjustment) as u32
    };
    let width = lines.iter().map(|line| line_width(line)).max().unwrap_or(0);
    let height = font.meta.line_height * lines.len() as u32;
    let mut output = RgbaCanvas::filled(width, height, Rgba8::TRANSPARENT);
    for (line_index, line) in lines.iter().enumerate() {
        let mut pen = 0;
        let mut previous = None;
        for character in line.chars() {
            let glyph = glyphs[&character];
            if let Some(previous) = previous {
                let previous_glyph = glyphs[&previous];
                pen += previous_glyph.effective_advance(&font.meta) as i32;
                if kern {
                    let key = format!("{previous}{character}");
                    pen += kerns.get(key.as_str()).copied().unwrap_or(0);
                }
            }
            for y in 0..glyph.canvas.height() as i32 {
                for x in 0..glyph.canvas.width() as i32 {
                    let index = glyph.canvas.get(x, y).unwrap();
                    if glyph.canvas.transparent() != Some(index) {
                        output.set(
                            pen + x,
                            line_index as i32 * font.meta.line_height as i32 + y,
                            ink,
                        );
                    }
                }
            }
            previous = Some(character);
        }
    }
    output
}

fn indexed_mask(canvas: &IndexedCanvas) -> Vec<bool> {
    canvas
        .pixels()
        .iter()
        .map(|index| Some(*index) != canvas.transparent())
        .collect()
}

fn read_indexed(path: &Path) -> IndexedCanvas {
    let name = path.to_string_lossy();
    if name.ends_with(".px.toml") {
        return pxsmith_io::l0::L0Document::read(path)
            .unwrap()
            .to_frames(path)
            .unwrap()[0]
            .layers[0]
            .surface
            .as_indexed()
            .unwrap()
            .clone();
    }
    pxsmith_io::Document::read(path)
        .unwrap()
        .project(pxsmith_core::FrameId(0))
        .unwrap()
        .layers[0]
        .surface
        .as_indexed()
        .unwrap()
        .clone()
}

#[test]
fn misaki_ascii_and_japanese_match_independent_pixel_assembly() {
    let root = scratch("misaki-lines");
    let (font_path, font) = import_misaki(&root);
    let palette = fixture("handmade.hex");
    let ink = Rgba8::rgb(0xff, 0xff, 0x00);

    for (name, text) in [
        ("score", "SCORE 0120"),
        ("neko", "吾輩は猫である。名前はまだ無い。"),
    ] {
        let output = root.join(format!("{name}.png"));
        let process = run(&[
            "font".as_ref(),
            "render".as_ref(),
            font_path.as_os_str(),
            text.as_ref(),
            output.as_os_str(),
            "--palette".as_ref(),
            palette.as_os_str(),
            "--ink".as_ref(),
            "1".as_ref(),
        ]);
        assert_success(&process);
        let actual = pxsmith_io::png::read_rgba(&output).unwrap();
        assert_eq!(
            actual,
            assemble_lines(&font, &[text], ink, false),
            "{name} の画素"
        );
    }
}

#[test]
fn misaki_wrap_matches_independent_two_line_assembly() {
    let root = scratch("misaki-wrap");
    let (font_path, font) = import_misaki(&root);
    let output = root.join("wrapped.png");
    let process = run(&[
        "font".as_ref(),
        "render".as_ref(),
        font_path.as_os_str(),
        "SCORE 0120".as_ref(),
        output.as_os_str(),
        "--palette".as_ref(),
        fixture("handmade.hex").as_os_str(),
        "--ink".as_ref(),
        "1".as_ref(),
        "--wrap".as_ref(),
        "20".as_ref(),
    ]);
    assert_success(&process);
    let actual = pxsmith_io::png::read_rgba(&output).unwrap();
    assert_eq!(
        actual,
        assemble_lines(
            &font,
            &["SCORE", "0120"],
            Rgba8::rgb(0xff, 0xff, 0x00),
            false,
        )
    );
}

#[test]
fn misaki_exercises_uax14_kinsoku_and_hanging() {
    let root = scratch("misaki-kinsoku");
    let (_, font) = import_misaki(&root);
    let layout = |text, kinsoku, hang| {
        layout_text(
            &font,
            text,
            &FontLayoutOptions {
                wrap: Some(16),
                kinsoku,
                hang,
                ..FontLayoutOptions::default()
            },
        )
        .unwrap()
    };
    assert_eq!(
        layout("あい、う", Kinsoku::Normal, false).report.lines,
        ["あ", "い、", "う"]
    );
    assert_eq!(
        layout("あ「いう", Kinsoku::Normal, false).report.lines,
        ["あ", "「い", "う"]
    );
    assert_eq!(
        layout("あいっう", Kinsoku::Normal, false).report.lines,
        ["あい", "っう"]
    );
    assert_eq!(
        layout("あいっう", Kinsoku::Strict, false).report.lines,
        ["あ", "いっ", "う"]
    );
    let hung = layout("あい。", Kinsoku::Normal, true);
    assert_eq!(hung.report.lines, ["あい。"]);
    assert_eq!(hung.canvas.width(), 24);
}

#[test]
fn imported_gpos_kerning_moves_v_left_and_no_kern_disables_it() {
    let root = scratch("kerning");
    let font_path = root.join("synthetic.pxfont.toml");
    let imported = run(&[
        "font".as_ref(),
        "import".as_ref(),
        fixture("synthetic/pxsynth-gpos.ttf").as_os_str(),
        font_path.as_os_str(),
        "--size".as_ref(),
        "8".as_ref(),
    ]);
    assert_success(&imported);
    let font = PxFontDocument::read(&font_path)
        .unwrap()
        .to_font(&font_path)
        .unwrap();
    let palette = fixture("handmade.hex");

    for (name, no_kern) in [("kerned", false), ("plain", true)] {
        let output = root.join(format!("{name}.png"));
        let mut args = vec![
            "font".as_ref(),
            "render".as_ref(),
            font_path.as_os_str(),
            "AV".as_ref(),
            output.as_os_str(),
            "--palette".as_ref(),
            palette.as_os_str(),
            "--ink".as_ref(),
            "0".as_ref(),
        ];
        if no_kern {
            args.push("--no-kern".as_ref());
        }
        let process = run(&args);
        assert_success(&process);
        let actual = pxsmith_io::png::read_rgba(&output).unwrap();
        assert_eq!(
            actual,
            assemble_lines(&font, &["AV"], Rgba8::rgb(0xff, 0x00, 0x00), !no_kern)
        );
    }
}

#[test]
fn color_font_keeps_colors_and_rejects_a_palette_missing_them() {
    let root = scratch("color");
    let font = fixture("handmade-color.pxfont.toml");
    let output = root.join("star.png");
    let process = run(&[
        "font".as_ref(),
        "render".as_ref(),
        font.as_os_str(),
        "★".as_ref(),
        output.as_os_str(),
    ]);
    assert_success(&process);
    let actual = pxsmith_io::png::read_rgba(&output).unwrap();
    assert_eq!(
        actual.pixels(),
        &[
            Rgba8::rgb(0xff, 0x00, 0x00),
            Rgba8::rgb(0xff, 0xff, 0x00),
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0xff, 0x00, 0x00),
        ]
    );

    let incomplete = root.join("red-only.hex");
    std::fs::write(&incomplete, "ff0000\n").unwrap();
    let rejected = run(&[
        "font".as_ref(),
        "render".as_ref(),
        font.as_os_str(),
        "★".as_ref(),
        root.join("bad.png").as_os_str(),
        "--palette".as_ref(),
        incomplete.as_os_str(),
    ]);
    assert_eq!(rejected.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(stderr.contains("ffff00"), "{stderr}");

    let reordered = root.join("reordered.hex");
    std::fs::write(&reordered, "ffff00\nff0000\n").unwrap();
    let reordered_output = root.join("reordered.png");
    let process = run(&[
        "font".as_ref(),
        "render".as_ref(),
        font.as_os_str(),
        "★".as_ref(),
        reordered_output.as_os_str(),
        "--palette".as_ref(),
        reordered.as_os_str(),
    ]);
    assert_success(&process);
    assert_eq!(
        pxsmith_io::png::read_rgba(&reordered_output)
            .unwrap()
            .pixels(),
        actual.pixels()
    );
}

#[test]
fn color_palette_validation_only_checks_glyphs_drawn_after_replacement() {
    let root = scratch("color-selection");
    let font = root.join("selective-color.pxfont.toml");
    let palette = root.join("red-only.hex");
    std::fs::write(root.join("source.hex"), "ff0000\nffff00\n").unwrap();
    std::fs::write(&palette, "ff0000\n").unwrap();
    std::fs::write(
        &font,
        r#"[meta]
format = 1
name = "selective-color"
mode = "color"
line_height = 1
baseline = 1
default_advance = 1

[palette]
ref = "source.hex"

[palette.map]
"." = "transparent"
r = 0
y = 1

[[glyph]]
char = "A"
data = '''
r
'''

[[glyph]]
char = "B"
data = '''
y
'''
"#,
    )
    .unwrap();

    for (text, missing, name) in [("A", None, "a.png"), ("Z", Some("A"), "replacement.png")] {
        let output = root.join(name);
        let mut args = vec![
            "font".as_ref(),
            "render".as_ref(),
            font.as_os_str(),
            text.as_ref(),
            output.as_os_str(),
            "--palette".as_ref(),
            palette.as_os_str(),
        ];
        if let Some(character) = missing {
            args.push(std::ffi::OsStr::new("--missing"));
            args.push(std::ffi::OsStr::new(character));
        }
        assert_success(&run(&args));
    }

    let rejected_output = root.join("b.png");
    let rejected = run(&[
        "font".as_ref(),
        "render".as_ref(),
        font.as_os_str(),
        "B".as_ref(),
        rejected_output.as_os_str(),
        "--palette".as_ref(),
        palette.as_os_str(),
    ]);
    assert_eq!(rejected.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(stderr.contains("ffff00"), "{stderr}");
}

#[test]
fn missing_glyphs_are_listed_and_can_be_replaced() {
    let root = scratch("missing");
    let font = fixture("handmade-mono.pxfont.toml");
    let palette = fixture("handmade.hex");
    let rejected = run(&[
        "font".as_ref(),
        "render".as_ref(),
        font.as_os_str(),
        "AZ".as_ref(),
        root.join("missing.png").as_os_str(),
        "--palette".as_ref(),
        palette.as_os_str(),
        "--ink".as_ref(),
        "0".as_ref(),
    ]);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("Z(U+005A)"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );

    let replaced = run(&[
        "font".as_ref(),
        "render".as_ref(),
        font.as_os_str(),
        "AZ".as_ref(),
        root.join("replaced.png").as_os_str(),
        "--palette".as_ref(),
        palette.as_os_str(),
        "--ink".as_ref(),
        "0".as_ref(),
        "--missing".as_ref(),
        "U+0041".as_ref(),
    ]);
    assert_success(&replaced);
    assert!(String::from_utf8_lossy(&replaced.stdout).contains("1 件"));
}

#[test]
fn png_aseprite_and_l0_keep_the_background_transparent() {
    let root = scratch("transparency");
    let font_path = fixture("handmade-mono.pxfont.toml");
    let font = PxFontDocument::read(&font_path)
        .unwrap()
        .to_font(&font_path)
        .unwrap();
    let expected = font
        .glyphs
        .iter()
        .find(|glyph| glyph.character == 'A' && glyph.variant.is_none())
        .unwrap();
    let expected_mask = indexed_mask(&expected.canvas);
    let palette = fixture("handmade.hex");

    for extension in ["png", "aseprite", "px.toml"] {
        let output = root.join(format!("out.{extension}"));
        let process = run(&[
            "font".as_ref(),
            "render".as_ref(),
            font_path.as_os_str(),
            "A".as_ref(),
            output.as_os_str(),
            "--palette".as_ref(),
            palette.as_os_str(),
            "--ink".as_ref(),
            "0".as_ref(),
        ]);
        assert_success(&process);
        if extension == "png" {
            let actual = pxsmith_io::png::read_rgba(&output).unwrap();
            assert_eq!(
                actual
                    .pixels()
                    .iter()
                    .map(|color| color.a != 0)
                    .collect::<Vec<_>>(),
                expected_mask
            );
        } else {
            let actual = read_indexed(&output);
            assert!(actual.transparent().is_some());
            assert_eq!(indexed_mask(&actual), expected_mask);
        }
    }
}

#[test]
fn text_file_normalizes_crlf_to_one_explicit_newline() {
    let root = scratch("text-file");
    let text = root.join("text.txt");
    std::fs::write(&text, "A\r\nA").unwrap();
    let output = root.join("text.png");
    let process = run(&[
        "font".as_ref(),
        "render".as_ref(),
        fixture("handmade-mono.pxfont.toml").as_os_str(),
        "--text-file".as_ref(),
        text.as_os_str(),
        output.as_os_str(),
        "--palette".as_ref(),
        fixture("handmade.hex").as_os_str(),
        "--ink".as_ref(),
        "0".as_ref(),
    ]);
    assert_success(&process);
    let actual = pxsmith_io::png::read_rgba(&output).unwrap();
    assert_eq!(actual.width(), 3);
    assert_eq!(actual.height(), 6);
}

#[test]
fn full_palette_fails_instead_of_reusing_an_opaque_index_for_transparency() {
    let root = scratch("full-palette");
    let palette = root.join("full.hex");
    let text = (0..=u8::MAX)
        .map(|value| format!("{value:02x}{value:02x}{value:02x}\n"))
        .collect::<String>();
    std::fs::write(&palette, text).unwrap();
    let output = run(&[
        "font".as_ref(),
        "render".as_ref(),
        fixture("handmade-mono.pxfont.toml").as_os_str(),
        "A".as_ref(),
        root.join("full.png").as_os_str(),
        "--palette".as_ref(),
        palette.as_os_str(),
        "--ink".as_ref(),
        "0".as_ref(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("透明背景"), "{stderr}");
}
