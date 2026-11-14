//! 美咲フォントの全字を，BDF 仕様だけから書いた読み手と突き合わせる．

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use pxsmith_io::pxfont::PxFontDocument;

#[derive(Debug)]
struct ExpectedGlyph {
    width: u32,
    height: u32,
    ink: BTreeSet<(u32, u32)>,
}

#[derive(Debug, Default)]
struct ExpectedFont {
    startchar: usize,
    unencoded: usize,
    glyphs: BTreeMap<char, ExpectedGlyph>,
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/fonts/misaki/misaki_gothic.bdf")
}

fn scratch() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-font-real-{}-{}",
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

/// BDF 2.1 の必要な欄だけを，製品側を呼ばずに読む．
fn parse_independently(text: &str) -> ExpectedFont {
    let lines = text.lines().collect::<Vec<_>>();
    let ascent: i32 = lines
        .iter()
        .find_map(|line| line.strip_prefix("FONT_ASCENT "))
        .unwrap()
        .parse()
        .unwrap();
    let descent: i32 = lines
        .iter()
        .find_map(|line| line.strip_prefix("FONT_DESCENT "))
        .unwrap()
        .parse()
        .unwrap();
    let line_height = ascent + descent;
    let mut output = ExpectedFont::default();
    let mut at = 0;
    while at < lines.len() {
        if !lines[at].starts_with("STARTCHAR ") {
            at += 1;
            continue;
        }
        output.startchar += 1;
        let mut encoding = None;
        let mut advance = None;
        let mut bbx = None;
        let mut bitmap = Vec::new();
        let mut in_bitmap = false;
        at += 1;
        while lines[at] != "ENDCHAR" {
            let line = lines[at];
            if in_bitmap {
                bitmap.push(line);
            } else if line == "BITMAP" {
                in_bitmap = true;
            } else if let Some(value) = line.strip_prefix("ENCODING ") {
                encoding = Some(
                    value
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .parse::<i32>()
                        .unwrap(),
                );
            } else if let Some(value) = line.strip_prefix("DWIDTH ") {
                advance = Some(
                    value
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .parse::<i32>()
                        .unwrap(),
                );
            } else if let Some(value) = line.strip_prefix("BBX ") {
                let values = value
                    .split_whitespace()
                    .map(|value| value.parse::<i32>().unwrap())
                    .collect::<Vec<_>>();
                bbx = Some((values[0], values[1], values[2], values[3]));
            }
            at += 1;
        }
        at += 1;

        let encoding = encoding.unwrap();
        if encoding == -1 {
            output.unencoded += 1;
            continue;
        }
        let character = char::from_u32(encoding as u32).expect("ISO10646 の符号位置");
        let advance = advance.unwrap();
        let (width, height, x_offset, y_offset) = bbx.unwrap();
        assert_eq!(bitmap.len(), height as usize);
        let top = ascent - (y_offset + height);
        let mut ink = BTreeSet::new();
        for (row, hex) in bitmap.iter().enumerate() {
            let bytes = (0..hex.len() / 2)
                .map(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap())
                .collect::<Vec<_>>();
            for x in 0..width {
                if bytes[x as usize / 8] & (0x80 >> (x as usize % 8)) == 0 {
                    continue;
                }
                let placed_x = x_offset + x;
                let placed_y = top + row as i32;
                assert!(
                    placed_x >= 0 && placed_x < advance && placed_y >= 0 && placed_y < line_height,
                    "{character} が独立読み手でも升外へ出た: ({placed_x}, {placed_y})"
                );
                ink.insert((placed_x as u32, placed_y as u32));
            }
        }
        assert!(
            output
                .glyphs
                .insert(
                    character,
                    ExpectedGlyph {
                        width: advance as u32,
                        height: line_height as u32,
                        ink,
                    },
                )
                .is_none(),
            "符号位置 {character} が重複している"
        );
    }
    output
}

#[test]
fn cli_import_matches_every_misaki_bdf_pixel_and_is_canonical() {
    let input = fixture();
    let root = scratch();
    let output = root.join("misaki.pxfont.toml");
    let process = Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .args(["font", "import"])
        .arg(&input)
        .arg(&output)
        .output()
        .expect("pxsmith を起こせる");
    assert!(
        process.status.success(),
        "取り込みに失敗した:\n{}",
        String::from_utf8_lossy(&process.stderr)
    );

    let source = std::fs::read_to_string(&input).unwrap();
    let expected = parse_independently(&source);
    assert_eq!(expected.startchar, 7_171);
    let document = PxFontDocument::read(&output).unwrap();
    let font = document.to_font(&output).unwrap();
    assert_eq!(font.glyphs.len(), expected.glyphs.len());
    assert_eq!(font.glyphs.len() + expected.unencoded, expected.startchar);

    for glyph in &font.glyphs {
        let wanted = expected
            .glyphs
            .get(&glyph.character)
            .unwrap_or_else(|| panic!("{} が独立読み手に無い", glyph.character));
        assert_eq!(
            glyph.canvas.width(),
            wanted.width,
            "{} の幅",
            glyph.character
        );
        assert_eq!(
            glyph.canvas.height(),
            wanted.height,
            "{} の高さ",
            glyph.character
        );
        assert_eq!(
            glyph.effective_advance(&font.meta),
            wanted.width,
            "{} の DWIDTH",
            glyph.character
        );
        assert_eq!(glyph.canvas.transparent(), Some(0));
        let actual = (0..glyph.canvas.height())
            .flat_map(|y| (0..glyph.canvas.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| glyph.canvas.get(x as i32, y as i32) == Some(1))
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, wanted.ink, "{} の墨の画素", glyph.character);
    }

    let bytes = std::fs::read(&output).unwrap();
    let rewritten = PxFontDocument::from_font(&font).unwrap().to_toml().unwrap();
    assert_eq!(bytes, rewritten.as_bytes(), "読み→書きでバイトが変わった");
    eprintln!(
        "美咲 BDF: STARTCHAR {} 件，取り込み {} 字，ENCODING -1 は {} 字",
        expected.startchar,
        font.glyphs.len(),
        expected.unencoded
    );
}
