//! 縦書き焼き込みを実プロセスと独立な画素配置で突き合わせる．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::{Font, Glyph, Rgba8, RgbaCanvas};
use pxsmith_io::pxfont::PxFontDocument;

const INK: Rgba8 = Rgba8::rgb(0xff, 0xff, 0x00);

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fonts")
        .join(path)
}

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-font-render-vertical-{name}-{}-{}",
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("test")
            .replace("::", "-")
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

fn import_font(root: &Path, source: &Path, size: Option<&str>) -> (PathBuf, Font) {
    let output = root.join("font.pxfont.toml");
    let mut args = vec![
        "font".as_ref(),
        "import".as_ref(),
        source.as_os_str(),
        output.as_os_str(),
    ];
    if let Some(size) = size {
        args.extend([std::ffi::OsStr::new("--size"), std::ffi::OsStr::new(size)]);
    }
    let process = run(&args);
    assert_success(&process);
    let font = PxFontDocument::read(&output)
        .unwrap()
        .to_font(&output)
        .unwrap();
    (output, font)
}

fn render(root: &Path, font: &Path, name: &str, text: &str, extra: &[&str]) -> Output {
    let output = root.join(name);
    let palette = fixture("handmade.hex");
    let mut args = vec![
        "font".as_ref(),
        "render".as_ref(),
        font.as_os_str(),
        text.as_ref(),
        output.as_os_str(),
        "--palette".as_ref(),
        palette.as_os_str(),
        "--ink".as_ref(),
        "1".as_ref(),
        "--vertical".as_ref(),
    ];
    args.extend(extra.iter().map(std::ffi::OsStr::new));
    run(&args)
}

fn glyphs(font: &Font) -> BTreeMap<(char, Option<&str>), &Glyph> {
    font.glyphs
        .iter()
        .map(|glyph| ((glyph.character, glyph.variant.as_deref()), glyph))
        .collect()
}

fn draw_upright(output: &mut RgbaCanvas, glyph: &Glyph, x: u32, y: u32) {
    let advance = glyph.advance.unwrap_or(glyph.canvas.width());
    for glyph_y in 0..glyph.canvas.height() {
        for glyph_x in 0..advance {
            let Some(index) = glyph.canvas.get(glyph_x as i32, glyph_y as i32) else {
                continue;
            };
            if glyph.canvas.transparent() != Some(index) {
                output.set((x + glyph_x) as i32, (y + glyph_y) as i32, INK);
            }
        }
    }
}

/// 元の升の `(x, y)` を `(h - 1 - y, x)` へ移す．
fn draw_clockwise(output: &mut RgbaCanvas, glyph: &Glyph, advance: u32, x: u32, y: u32) {
    let height = glyph.canvas.height();
    for source_y in 0..height {
        for source_x in 0..advance {
            let Some(index) = glyph.canvas.get(source_x as i32, source_y as i32) else {
                continue;
            };
            if glyph.canvas.transparent() != Some(index) {
                output.set(
                    (x + height - 1 - source_y) as i32,
                    (y + source_x) as i32,
                    INK,
                );
            }
        }
    }
}

fn image(path: &Path) -> RgbaCanvas {
    pxsmith_io::png::read_rgba(path).unwrap()
}

#[test]
fn synthetic_r_rotates_and_tu_tr_use_vertical_variants() {
    let root = scratch("synthetic");
    let (font_path, font) = import_font(&root, &fixture("synthetic/pxsynth-gpos.ttf"), Some("8"));
    let process = render(&root, &font_path, "synthetic.png", "Aー（", &[]);
    assert_success(&process);

    let glyphs = glyphs(&font);
    let a = glyphs[&('A', None)];
    let bar = glyphs[&('ー', Some("vert"))];
    let paren = glyphs[&('（', Some("vert"))];
    let a_advance = a.effective_advance(&font.meta);
    let bar_advance = bar.effective_advance(&font.meta);
    let paren_advance = paren.effective_advance(&font.meta);
    let column_width = font.meta.line_height.max(bar_advance).max(paren_advance);
    let height = a_advance + font.meta.line_height * 2;
    let mut expected = RgbaCanvas::filled(column_width, height, Rgba8::TRANSPARENT);
    draw_clockwise(&mut expected, a, a_advance, 0, 0);
    draw_upright(
        &mut expected,
        bar,
        (column_width - bar_advance) / 2,
        a_advance,
    );
    draw_upright(
        &mut expected,
        paren,
        (column_width - paren_advance) / 2,
        a_advance + font.meta.line_height,
    );
    assert_eq!(image(&root.join("synthetic.png")), expected);

    let stdout = String::from_utf8_lossy(&process.stdout);
    assert!(stdout.contains("縦書き変種を使った: 2 件"), "{stdout}");
    assert!(stdout.contains("R を回した: 1 件"), "{stdout}");
}

#[test]
fn misaki_reports_tr_tu_and_r_fallbacks() {
    let root = scratch("fallbacks");
    let (font_path, font) = import_font(&root, &fixture("misaki/misaki_gothic.bdf"), None);
    let process = render(&root, &font_path, "fallbacks.png", "あー、1", &[]);
    assert_success(&process);

    let glyphs = glyphs(&font);
    let mut expected = RgbaCanvas::filled(8, 28, Rgba8::TRANSPARENT);
    draw_upright(&mut expected, glyphs[&('あ', None)], 0, 0);
    draw_clockwise(&mut expected, glyphs[&('ー', None)], 8, 0, 8);
    draw_upright(&mut expected, glyphs[&('、', None)], 0, 16);
    draw_clockwise(&mut expected, glyphs[&('1', None)], 4, 0, 24);
    assert_eq!(image(&root.join("fallbacks.png")), expected);

    let stdout = String::from_utf8_lossy(&process.stdout);
    assert!(
        stdout.contains("Tr の変種が無いため回した: 1 件"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Tu の変種が無いため正立にした: 1 件"),
        "{stdout}"
    );
    assert!(stdout.contains("R を回した: 1 件"), "{stdout}");
}

#[test]
fn tcy_combines_at_most_n_digits_and_reports_too_wide_groups() {
    let root = scratch("tcy");
    let (font_path, font) = import_font(&root, &fixture("misaki/misaki_gothic.bdf"), None);
    let glyphs = glyphs(&font);

    let combined = render(&root, &font_path, "combined.png", "第12回", &["--tcy", "2"]);
    assert_success(&combined);
    let mut expected = RgbaCanvas::filled(8, 24, Rgba8::TRANSPARENT);
    draw_upright(&mut expected, glyphs[&('第', None)], 0, 0);
    draw_upright(&mut expected, glyphs[&('1', None)], 0, 8);
    draw_upright(&mut expected, glyphs[&('2', None)], 4, 8);
    draw_upright(&mut expected, glyphs[&('回', None)], 0, 16);
    assert_eq!(image(&root.join("combined.png")), expected);
    let stdout = String::from_utf8_lossy(&combined.stdout);
    assert!(stdout.contains("縦中横: 1 組"), "{stdout}");

    let over_limit = render(&root, &font_path, "over-limit.png", "123", &["--tcy", "2"]);
    assert_success(&over_limit);
    let mut expected = RgbaCanvas::filled(8, 12, Rgba8::TRANSPARENT);
    for (index, character) in "123".chars().enumerate() {
        draw_clockwise(
            &mut expected,
            glyphs[&(character, None)],
            4,
            0,
            index as u32 * 4,
        );
    }
    assert_eq!(image(&root.join("over-limit.png")), expected);
    assert!(
        !String::from_utf8_lossy(&over_limit.stdout).contains("縦中横:"),
        "{}",
        String::from_utf8_lossy(&over_limit.stdout)
    );

    let too_wide = render(
        &root,
        &font_path,
        "too-wide.png",
        "第123回",
        &["--tcy", "3"],
    );
    assert_success(&too_wide);
    let mut expected = RgbaCanvas::filled(8, 40, Rgba8::TRANSPARENT);
    draw_upright(&mut expected, glyphs[&('第', None)], 0, 0);
    for (index, character) in "123".chars().enumerate() {
        draw_upright(
            &mut expected,
            glyphs[&(character, None)],
            2,
            8 + index as u32 * 8,
        );
    }
    draw_upright(&mut expected, glyphs[&('回', None)], 0, 32);
    assert_eq!(image(&root.join("too-wide.png")), expected);
    let stdout = String::from_utf8_lossy(&too_wide.stdout);
    assert!(stdout.contains("縦中横を諦めた: 1 組"), "{stdout}");
}

#[test]
fn wrapped_columns_run_from_right_to_left_and_obey_kinsoku() {
    let root = scratch("columns");
    let (font_path, font) = import_font(&root, &fixture("misaki/misaki_gothic.bdf"), None);
    let glyphs = glyphs(&font);

    let wrapped = render(
        &root,
        &font_path,
        "wrapped.png",
        "あいうえ",
        &["--wrap", "16"],
    );
    assert_success(&wrapped);
    let mut expected = RgbaCanvas::filled(16, 16, Rgba8::TRANSPARENT);
    for (index, character) in "あい".chars().enumerate() {
        draw_upright(
            &mut expected,
            glyphs[&(character, None)],
            8,
            index as u32 * 8,
        );
    }
    for (index, character) in "うえ".chars().enumerate() {
        draw_upright(
            &mut expected,
            glyphs[&(character, None)],
            0,
            index as u32 * 8,
        );
    }
    assert_eq!(image(&root.join("wrapped.png")), expected);

    let kinsoku = render(
        &root,
        &font_path,
        "kinsoku.png",
        "あい、う",
        &["--wrap", "16"],
    );
    assert_success(&kinsoku);
    let mut expected = RgbaCanvas::filled(24, 16, Rgba8::TRANSPARENT);
    draw_upright(&mut expected, glyphs[&('あ', None)], 16, 0);
    draw_upright(&mut expected, glyphs[&('い', None)], 8, 0);
    draw_upright(&mut expected, glyphs[&('、', None)], 8, 8);
    draw_upright(&mut expected, glyphs[&('う', None)], 0, 0);
    assert_eq!(image(&root.join("kinsoku.png")), expected);
}

#[test]
fn hanging_punctuation_extends_the_bottom_of_the_canvas() {
    let root = scratch("hang");
    let (font_path, font) = import_font(&root, &fixture("misaki/misaki_gothic.bdf"), None);
    let glyphs = glyphs(&font);
    let process = render(
        &root,
        &font_path,
        "hang.png",
        "あい。",
        &["--wrap", "16", "--hang"],
    );
    assert_success(&process);

    let mut expected = RgbaCanvas::filled(8, 24, Rgba8::TRANSPARENT);
    for (index, character) in "あい。".chars().enumerate() {
        draw_upright(
            &mut expected,
            glyphs[&(character, None)],
            0,
            index as u32 * 8,
        );
    }
    assert_eq!(image(&root.join("hang.png")), expected);
    let stdout = String::from_utf8_lossy(&process.stdout);
    assert!(
        stdout.contains("句読点を行幅の外へぶら下げた: 1 件"),
        "{stdout}"
    );
}
