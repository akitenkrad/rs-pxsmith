//! `.hex` パレットファイル (設計書 4.5)．
//!
//! 正規形はこの 1 形式のみ (D54)．L0 の `[palette] ref`・レシピの `[project] palette`・
//! `pixels!` の 3 経路がすべてこれを参照する．Lospec の事実上の標準に合わせてあり，
//! 外部のパレット資産をそのまま読めることを設計の都合より優先している．
//!
//! | 規則 | 内容 |
//! | --- | --- |
//! | 1 行 1 色 | `RRGGBB` の 6 桁 16 進．大文字小文字は問わない |
//! | 行頭の `#` | コメント行．**色コード側には付けない** |
//! | アルファ | 持たない (透明は L0 の `map` で表す) |
//! | 添字 | ファイル内の出現順が添字 0, 1, 2, … |
//! | 空行 | 無視する |
//! | 上限 | 256 行．超過はエラー |

use std::path::{Path, PathBuf};

use pxsmith_core::{Palette, Rgba8};

use crate::error::{IoError, Result};

/// `.hex` の本文を解釈する．`path` はエラー表示にのみ使う．
pub fn parse(text: &str, path: &Path) -> Result<Palette> {
    let mut entries = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let color = Rgba8::from_hex_str(line).map_err(|_| IoError::Parse {
            path: path.to_path_buf(),
            line: i + 1,
            message: format!("'{line}' を RRGGBB の 6 桁 16 進として解釈できない"),
        })?;
        if entries.len() >= Palette::MAX_COLORS {
            return Err(IoError::PaletteTooLarge(entries.len() + 1));
        }
        entries.push(color);
    }
    Ok(Palette::new(entries)?)
}

/// `.hex` ファイルを読む．
pub fn read(path: impl AsRef<Path>) -> Result<Palette> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|source| IoError::File {
        path: path.to_path_buf(),
        source,
    })?;
    parse(&text, path)
}

/// `.hex` の本文を作る．
///
/// **アルファ 0 の色も 1 行として書く．** この形式はアルファを持たない (設計書 4.5)
/// ので透明という情報自体は落ちるが，飛ばしてしまうと**それ以降の添字が 1 つずつ
/// ずれる**．「ファイル内の出現順が添字 0, 1, 2, … に対応する」という規則が
/// 崩れる方が害が大きい．透明は L0 の `map` で `"transparent"` を割り当てて表す．
pub fn to_string(palette: &Palette) -> String {
    let mut out = String::new();
    for c in palette.entries() {
        out.push_str(&c.to_hex_string());
        out.push('\n');
    }
    out
}

/// `.hex` ファイルへ書き出す (原子的置換)．
pub fn write(path: impl AsRef<Path>, palette: &Palette) -> Result<()> {
    crate::atomic::write(path, to_string(palette).as_bytes())
}

/// 変換入力としてのみ受け付ける形式 (設計書 4.5)．正規形には昇格させない．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ImportFormat {
    /// GIMP パレット．
    Gpl,
    /// JASC-PAL．
    Pal,
    /// Adobe Color Table (768 バイト固定)．
    Act,
}

impl ImportFormat {
    /// 拡張子から判定する．
    pub fn from_path(path: &Path) -> Option<Self> {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("gpl") => Some(Self::Gpl),
            Some("pal") => Some(Self::Pal),
            Some("act") => Some(Self::Act),
            _ => None,
        }
    }
}

/// 変換入力を読む．正規形 (`.hex`) へ変換して初めてパレットとして扱える．
pub fn import(path: impl AsRef<Path>) -> Result<Palette> {
    let path: PathBuf = path.as_ref().to_path_buf();
    let format = ImportFormat::from_path(&path).ok_or_else(|| IoError::Parse {
        path: path.clone(),
        line: 0,
        message: "拡張子から形式を判定できない (.gpl / .pal / .act のいずれか)".to_string(),
    })?;
    let bytes = std::fs::read(&path).map_err(|source| IoError::File {
        path: path.clone(),
        source,
    })?;

    let entries = match format {
        ImportFormat::Act => parse_act(&bytes),
        ImportFormat::Gpl => parse_gpl(&String::from_utf8_lossy(&bytes), &path)?,
        ImportFormat::Pal => parse_pal(&String::from_utf8_lossy(&bytes), &path)?,
    };
    if entries.len() > Palette::MAX_COLORS {
        return Err(IoError::PaletteTooLarge(entries.len()));
    }
    Ok(Palette::new(entries)?)
}

fn parse_act(bytes: &[u8]) -> Vec<Rgba8> {
    // 256 色 x 3 バイト．末尾 4 バイトの色数・透明添字は任意なので長さで判断する．
    let count = (bytes.len() / 3).min(Palette::MAX_COLORS);
    (0..count)
        .map(|i| Rgba8::rgb(bytes[i * 3], bytes[i * 3 + 1], bytes[i * 3 + 2]))
        .collect()
}

fn parse_gpl(text: &str, path: &Path) -> Result<Vec<Rgba8>> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("GIMP Palette") {
            continue;
        }
        // "Name: ..." や "Columns: ..." などのヘッダは飛ばす
        let mut it = line.split_whitespace();
        let (Some(r), Some(g), Some(b)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        match (r.parse::<u8>(), g.parse::<u8>(), b.parse::<u8>()) {
            (Ok(r), Ok(g), Ok(b)) => out.push(Rgba8::rgb(r, g, b)),
            _ if line.contains(':') => continue,
            _ => {
                return Err(IoError::Parse {
                    path: path.to_path_buf(),
                    line: i + 1,
                    message: format!("'{line}' を R G B の 3 値として解釈できない"),
                });
            }
        }
    }
    Ok(out)
}

fn parse_pal(text: &str, path: &Path) -> Result<Vec<Rgba8>> {
    let mut lines = text.lines().map(str::trim);
    if lines.next() != Some("JASC-PAL") {
        return Err(IoError::Parse {
            path: path.to_path_buf(),
            line: 1,
            message: "先頭行が 'JASC-PAL' でない".to_string(),
        });
    }
    let _version = lines.next();
    let _count = lines.next();
    let mut out = Vec::new();
    for (i, line) in lines.enumerate() {
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let (Some(r), Some(g), Some(b)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        match (r.parse::<u8>(), g.parse::<u8>(), b.parse::<u8>()) {
            (Ok(r), Ok(g), Ok(b)) => out.push(Rgba8::rgb(r, g, b)),
            _ => {
                return Err(IoError::Parse {
                    path: path.to_path_buf(),
                    line: i + 4,
                    message: format!("'{line}' を R G B の 3 値として解釈できない"),
                });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AURORA: &str = "# palettes/aurora.hex — 行頭 # はコメント\n\
                          1a1c2c\n\
                          5d275d\n\
                          \n\
                          B13E53\n\
                          ef7d57\n";

    fn p() -> &'static Path {
        Path::new("test.hex")
    }

    #[test]
    fn parses_comments_blank_lines_and_mixed_case() {
        let pal = parse(AURORA, p()).unwrap();
        assert_eq!(pal.len(), 4);
        assert_eq!(pal.get(0).unwrap(), Rgba8::rgb(0x1a, 0x1c, 0x2c));
        assert_eq!(pal.get(2).unwrap(), Rgba8::rgb(0xb1, 0x3e, 0x53));
    }

    #[test]
    fn index_follows_file_order() {
        let pal = parse("ff0000\n00ff00\n0000ff\n", p()).unwrap();
        assert_eq!(pal.get(1).unwrap(), Rgba8::rgb(0, 255, 0));
    }

    #[test]
    fn round_trip_normalizes_to_lowercase() {
        let pal = parse(AURORA, p()).unwrap();
        let text = to_string(&pal);
        assert_eq!(text, "1a1c2c\n5d275d\nb13e53\nef7d57\n");
        assert_eq!(parse(&text, p()).unwrap().entries(), pal.entries());
    }

    /// 透明色を飛ばすと以降の添字がずれる．設計書 4.5 の「出現順が添字」を守る．
    #[test]
    fn transparent_entries_keep_their_slot() {
        let pal = Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(1, 2, 3)]).unwrap();
        assert_eq!(to_string(&pal), "000000\n010203\n");
        let back = parse(&to_string(&pal), p()).unwrap();
        assert_eq!(back.len(), 2, "添字の位置が保たれていない");
        assert_eq!(back.get(1).unwrap(), Rgba8::rgb(1, 2, 3));
    }

    #[test]
    fn malformed_line_reports_line_number() {
        let e = parse("ff0000\nzzz\n", p()).unwrap_err();
        match e {
            IoError::Parse { line, .. } => assert_eq!(line, 2),
            other => panic!("想定外のエラー: {other}"),
        }
    }

    #[test]
    fn hash_prefixed_color_is_treated_as_comment() {
        // 設計書 4.5: 色コード側に # は付けない．行頭の # はコメント行
        assert_eq!(parse("#ff0000\n00ff00\n", p()).unwrap().len(), 1);
    }

    #[test]
    fn rejects_more_than_256_colors() {
        let text: String = (0..=256).map(|i| format!("{:06x}\n", i)).collect();
        assert!(matches!(
            parse(&text, p()).unwrap_err(),
            IoError::PaletteTooLarge(257)
        ));
    }

    #[test]
    fn accepts_exactly_256_colors() {
        let text: String = (0..256).map(|i| format!("{:06x}\n", i)).collect();
        assert_eq!(parse(&text, p()).unwrap().len(), 256);
    }

    #[test]
    fn import_format_detected_from_extension() {
        assert_eq!(
            ImportFormat::from_path(Path::new("a/b.GPL")),
            Some(ImportFormat::Gpl)
        );
        assert_eq!(ImportFormat::from_path(Path::new("a/b.hex")), None);
    }

    #[test]
    fn gpl_header_lines_are_skipped() {
        let text = "GIMP Palette\nName: test\nColumns: 4\n#\n 26  28  44 dark\n 93  39  93\n";
        let colors = parse_gpl(text, p()).unwrap();
        assert_eq!(colors, vec![Rgba8::rgb(26, 28, 44), Rgba8::rgb(93, 39, 93)]);
    }

    #[test]
    fn jasc_pal_requires_magic_line() {
        assert!(parse_pal("NOT-PAL\n0100\n2\n1 2 3\n", p()).is_err());
        let colors = parse_pal("JASC-PAL\n0100\n2\n1 2 3\n4 5 6\n", p()).unwrap();
        assert_eq!(colors.len(), 2);
    }

    #[test]
    fn act_reads_triplets() {
        let mut bytes = vec![0u8; 768];
        bytes[3] = 10;
        bytes[4] = 20;
        bytes[5] = 30;
        let colors = parse_act(&bytes);
        assert_eq!(colors.len(), 256);
        assert_eq!(colors[1], Rgba8::rgb(10, 20, 30));
    }
}
