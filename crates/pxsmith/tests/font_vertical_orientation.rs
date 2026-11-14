//! core の縦向き表を Unicode の配布データと全符号位置で突き合わせる．

use std::path::Path;

use pxsmith_core::{VerticalOrientation, vertical_orientation};

#[test]
fn generated_table_matches_unicode_18_for_every_code_point() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/unicode/VerticalOrientation.txt");
    let text = std::fs::read_to_string(path).unwrap();
    let mut expected = vec![VerticalOrientation::R; 0x11_0000];

    for line in text.lines() {
        if !line.starts_with(|character: char| character.is_ascii_hexdigit()) {
            continue;
        }
        let Some((range, rest)) = line.split_once(';') else {
            continue;
        };
        let property = match rest.split('#').next().unwrap_or("").trim() {
            "U" => VerticalOrientation::U,
            "R" => VerticalOrientation::R,
            "Tu" => VerticalOrientation::Tu,
            "Tr" => VerticalOrientation::Tr,
            _ => continue,
        };
        let range = range.trim();
        let (start, end) = range.split_once("..").unwrap_or((range, range));
        let start = u32::from_str_radix(start, 16).unwrap();
        let end = u32::from_str_radix(end, 16).unwrap();
        expected[start as usize..=end as usize].fill(property);
    }

    for code_point in 0..=0x10_FFFF {
        assert_eq!(
            vertical_orientation(code_point),
            expected[code_point as usize],
            "U+{code_point:04X}"
        );
    }
}
