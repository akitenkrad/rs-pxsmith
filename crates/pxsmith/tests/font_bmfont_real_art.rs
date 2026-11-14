//! BMFont を実プロセスで書き，独立な読み手で全画素を升へ戻して検査する．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::{Font, Rgba8, RgbaCanvas};
use pxsmith_io::pxfont::PxFontDocument;

#[derive(Debug)]
struct CharacterRecord {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    xoffset: u32,
    yoffset: u32,
    xadvance: u32,
    page: usize,
}

#[derive(Debug)]
struct Descriptor {
    line_height: u32,
    pages: Vec<String>,
    characters: BTreeMap<char, CharacterRecord>,
}

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fonts")
        .join(path)
}

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-font-bmfont-{name}-{}-{}",
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

fn import_font(input: &Path, output: &Path, extra: &[&std::ffi::OsStr]) -> Font {
    let mut args = vec![
        "font".as_ref(),
        "import".as_ref(),
        input.as_os_str(),
        output.as_os_str(),
    ];
    args.extend_from_slice(extra);
    assert_success(&run(&args));
    PxFontDocument::read(output)
        .unwrap()
        .to_font(output)
        .unwrap()
}

fn number(line: &str, key: &str) -> u32 {
    let prefix = format!("{key}=");
    line.split_whitespace()
        .find_map(|field| field.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("{key} が無い: {line}"))
        .parse()
        .unwrap()
}

/// AngelCode のテキスト形式のうち，画素の復元に必要な属性だけを独立に読む．
fn read_descriptor(path: &Path) -> Descriptor {
    let text = std::fs::read_to_string(path).unwrap();
    let common = text
        .lines()
        .find(|line| line.starts_with("common "))
        .expect("common 行");
    let line_height = number(common, "lineHeight");
    let declared_pages = number(common, "pages") as usize;
    let mut pages = Vec::new();
    let mut characters = BTreeMap::new();
    for line in text.lines() {
        if line.starts_with("page ") {
            let file = line
                .split_once(" file=\"")
                .and_then(|(_, tail)| tail.strip_suffix('"'))
                .expect("page の file");
            pages.push(file.to_string());
        } else if line.starts_with("char ") {
            let id = number(line, "id");
            let character = char::from_u32(id).expect("Unicode の符号位置");
            let old = characters.insert(
                character,
                CharacterRecord {
                    x: number(line, "x"),
                    y: number(line, "y"),
                    width: number(line, "width"),
                    height: number(line, "height"),
                    xoffset: number(line, "xoffset"),
                    yoffset: number(line, "yoffset"),
                    xadvance: number(line, "xadvance"),
                    page: number(line, "page") as usize,
                },
            );
            assert!(old.is_none(), "char id={id} が重複している");
        }
    }
    assert_eq!(pages.len(), declared_pages);
    Descriptor {
        line_height,
        pages,
        characters,
    }
}

/// `.fnt` とページ PNG だけから，各字を字送り幅×行高の升へ戻す．
fn restore_glyphs(path: &Path) -> (Descriptor, BTreeMap<char, RgbaCanvas>) {
    let descriptor = read_descriptor(path);
    let parent = path.parent().unwrap();
    let pages = descriptor
        .pages
        .iter()
        .map(|page| pxsmith_io::png::read_rgba(parent.join(page)).unwrap())
        .collect::<Vec<_>>();
    let glyphs = descriptor
        .characters
        .iter()
        .map(|(&character, record)| {
            let mut canvas =
                RgbaCanvas::filled(record.xadvance, descriptor.line_height, Rgba8::TRANSPARENT);
            for dy in 0..record.height {
                for dx in 0..record.width {
                    let color = pages[record.page]
                        .get((record.x + dx) as i32, (record.y + dy) as i32)
                        .expect("ページ内の矩形");
                    canvas.set(
                        (record.xoffset + dx) as i32,
                        (record.yoffset + dy) as i32,
                        color,
                    );
                }
            }
            (character, canvas)
        })
        .collect();
    (descriptor, glyphs)
}

fn expected_mono(font: &Font, character: char) -> RgbaCanvas {
    let glyph = font
        .glyphs
        .iter()
        .find(|glyph| glyph.character == character && glyph.variant.is_none())
        .expect("基底字形");
    let mut output = RgbaCanvas::filled(
        glyph.effective_advance(&font.meta),
        font.meta.line_height,
        Rgba8::TRANSPARENT,
    );
    for y in 0..glyph.canvas.height() {
        for x in 0..glyph.canvas.width() {
            let index = glyph.canvas.get(x as i32, y as i32).unwrap();
            if glyph.canvas.transparent() != Some(index) {
                output.set(x as i32, y as i32, Rgba8::rgb(255, 255, 255));
            }
        }
    }
    output
}

#[test]
fn misaki_all_glyphs_round_trip_through_multiple_pages() {
    let root = scratch("misaki-all");
    let imported = root.join("misaki.pxfont.toml");
    let font = import_font(&fixture("misaki/misaki_gothic.bdf"), &imported, &[]);
    let fnt = root.join("misaki.fnt");
    let process = run(&[
        "font".as_ref(),
        "bmfont".as_ref(),
        imported.as_os_str(),
        fnt.as_os_str(),
        "--page-size".as_ref(),
        "256".as_ref(),
    ]);
    assert_success(&process);

    let (descriptor, actual) = restore_glyphs(&fnt);
    let base_count = font
        .glyphs
        .iter()
        .filter(|glyph| glyph.variant.is_none())
        .count();
    assert_eq!(actual.len(), base_count);
    assert!(
        descriptor.pages.len() > 1,
        "全字が 1 ページに入ってしまった"
    );
    for &character in actual.keys() {
        assert_eq!(
            actual[&character],
            expected_mono(&font, character),
            "{character}"
        );
    }
    eprintln!(
        "美咲 BMFont: {} 字，{} ページ",
        actual.len(),
        descriptor.pages.len()
    );
}

#[test]
fn synthetic_gpos_kerning_is_written_once() {
    let root = scratch("gpos");
    let imported = root.join("gpos.pxfont.toml");
    import_font(
        &fixture("synthetic/pxsynth-gpos.ttf"),
        &imported,
        &["--size".as_ref(), "8".as_ref()],
    );
    let fnt = root.join("gpos.fnt");
    assert_success(&run(&[
        "font".as_ref(),
        "bmfont".as_ref(),
        imported.as_os_str(),
        fnt.as_os_str(),
    ]));
    let text = std::fs::read_to_string(fnt).unwrap();
    assert_eq!(text.matches("kernings count=1\n").count(), 1);
    assert_eq!(
        text.matches("kerning first=65 second=86 amount=-1\n")
            .count(),
        1
    );
}

#[test]
fn color_font_page_keeps_source_colors() {
    let root = scratch("color");
    let fnt = root.join("color.fnt");
    assert_success(&run(&[
        "font".as_ref(),
        "bmfont".as_ref(),
        fixture("handmade-color.pxfont.toml").as_os_str(),
        fnt.as_os_str(),
    ]));
    let (_, glyphs) = restore_glyphs(&fnt);
    let star = &glyphs[&'★'];
    assert_eq!(star.get(0, 0), Some(Rgba8::rgb(255, 0, 0)));
    assert_eq!(star.get(1, 0), Some(Rgba8::rgb(255, 255, 0)));
    assert_eq!(star.get(0, 1), Some(Rgba8::TRANSPARENT));
    assert_eq!(star.get(1, 1), Some(Rgba8::rgb(255, 0, 0)));
}

#[test]
fn chars_file_selects_a_subset_including_space_and_reports_missing() {
    let root = scratch("chars");
    let selected = root.join("selected.txt");
    std::fs::write(&selected, "A\n").unwrap();
    let fnt = root.join("selected.fnt");
    assert_success(&run(&[
        "font".as_ref(),
        "bmfont".as_ref(),
        fixture("handmade-mono.pxfont.toml").as_os_str(),
        fnt.as_os_str(),
        "--chars-file".as_ref(),
        selected.as_os_str(),
    ]));
    let descriptor = read_descriptor(&fnt);
    assert_eq!(
        descriptor.characters.keys().copied().collect::<Vec<_>>(),
        vec![' ', 'A']
    );

    std::fs::write(&selected, "AZ\n").unwrap();
    let missing = root.join("missing.fnt");
    let process = run(&[
        "font".as_ref(),
        "bmfont".as_ref(),
        fixture("handmade-mono.pxfont.toml").as_os_str(),
        missing.as_os_str(),
        "--chars-file".as_ref(),
        selected.as_os_str(),
    ]);
    assert!(!process.status.success());
    assert!(String::from_utf8_lossy(&process.stderr).contains("Z(U+005A)"));
    assert!(!missing.exists());
}
