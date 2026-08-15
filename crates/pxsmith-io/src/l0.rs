//! L0 テキストビットマップ (設計書 4.1)．
//!
//! 1 文字 = 1 ピクセルのテキスト形式．人間が直接書けることと，git の差分が
//! そのまま絵の差分になることが要点である．
//!
//! | 規則 | 内容 |
//! | --- | --- |
//! | 文字数 | 1 文字 = 1 ピクセル |
//! | キー文字 | `0-9 a-z A-Z` の 62 種 + 透明の既定 `.` |
//! | 色数上限 | 62 色．超える場合はエラー (D8) |
//! | ファイル粒度 | 1 ファイル = 1 レイヤ分のフレーム列 (D9) |
//! | 適用範囲 | スプライト専用 (48x48 程度が上限)．背景画は `.aseprite` (D17) |
//! | 文字列リテラル | `'''` (literal string) を使う |
//! | 改行 | 開き区切り直後と閉じ区切り直前の改行は無視．行数は高さと一致 |
//!
//! 開き区切り直後の改行は TOML の仕様で落ちるが，閉じ区切り直前の改行は落ちない
//! ので，こちらで 1 つだけ取り除く．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::frame::{Depth, FrameKind, Layer, LayerMeta, Surface};
use pxsmith_core::math::uvec2;
use pxsmith_core::{Frame, Palette};
use serde::{Deserialize, Serialize};

use crate::error::{IoError, Result};

/// L0 形式の版．
pub const FORMAT_VERSION: u32 = 1;

/// 透明の既定キー．
pub const TRANSPARENT_KEY: char = '.';

/// 色キーに使える 62 文字．添字を割り当てるときはこの順に消費する．
pub const COLOR_KEYS: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// 助言としての大きさの上限 (D17)．超えても失敗はしない．
pub const ADVISED_MAX_SIDE: u32 = 48;

/// 象限 (設計書 4.3)．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quadrant {
    NW,
    NE,
    SW,
    SE,
}

/// 色キーの割り当て先．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ColorKey {
    Transparent,
    Index(u8),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct L0Meta {
    pub format: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub anchors: BTreeMap<String, [i32; 2]>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub subpixel_exclude: bool,
    /// `autotile_quadrants` のときだけ象限として解釈する (設計書 4.3)．
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile_size: Option<u32>,
    /// ディザを含むタイルの位相バリアント数 (D45)．
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dither_phase: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct L0PaletteSpec {
    /// 正規形 `.hex` への相対パス (設計書 4.5)．
    #[serde(rename = "ref")]
    pub reference: PathBuf,
    /// 色キーから添字への対応．値は整数か `"transparent"`．
    pub map: BTreeMap<String, RawColorKey>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RawColorKey {
    Index(u8),
    Name(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct L0Frame {
    pub name: String,
    /// `key` (既定) / `breakdown` / `inbetween` (D47)．
    #[serde(default = "default_kind")]
    pub kind: String,
    /// コマ打ち x FPS 表から求めた値 (D40)．
    #[serde(default = "default_duration")]
    pub duration_ms: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quadrant: Option<Quadrant>,
    pub data: String,
}

fn default_kind() -> String {
    "key".to_string()
}

fn default_duration() -> u32 {
    100
}

/// L0 ファイルの内容．
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct L0Document {
    pub meta: L0Meta,
    pub palette: L0PaletteSpec,
    #[serde(rename = "frame")]
    pub frames: Vec<L0Frame>,
}

/// L0 の制約を超えたときの理由 (M1 の完了条件「制約超過時は理由を返す」)．
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Violation {
    /// インデックスカラーでないレイヤは L0 で表せない．
    NotIndexed { layer: String },
    /// 62 色を超えている (D8)．
    TooManyColors { used: usize },
    /// 助言としての上限を超えている (失敗ではない)．
    OverAdvisedSize { w: u32, h: u32 },
    /// フレームごとに大きさが違う．
    InconsistentSize { frame: String, w: u32, h: u32 },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotIndexed { layer } => {
                write!(f, "レイヤ '{layer}' はインデックスカラーではない")
            }
            Self::TooManyColors { used } => write!(
                f,
                "{used} 色を使っているが L0 の上限は 62 色 (使える文字が 0-9 a-z A-Z のため)"
            ),
            Self::OverAdvisedSize { w, h } => write!(
                f,
                "{w}x{h} は L0 の想定 ({ADVISED_MAX_SIDE}x{ADVISED_MAX_SIDE} 程度) を超えている．背景画は .aseprite で扱うこと"
            ),
            Self::InconsistentSize { frame, w, h } => {
                write!(f, "フレーム '{frame}' だけ {w}x{h} で他と大きさが違う")
            }
        }
    }
}

impl Violation {
    /// 変換を止めるほどの違反か．
    pub fn is_blocking(&self) -> bool {
        !matches!(self, Self::OverAdvisedSize { .. })
    }
}

/// 書き出しの結果．助言レベルの違反はここに載せて処理は続ける．
#[derive(Clone, Debug)]
pub struct Exported {
    pub document: L0Document,
    pub advisories: Vec<Violation>,
}

impl L0Document {
    /// TOML 本文を解釈する．
    pub fn parse(text: &str, path: &Path) -> Result<Self> {
        let doc: Self = toml::from_str(text).map_err(|e| IoError::Parse {
            path: path.to_path_buf(),
            line: e.span().map_or(0, |s| text[..s.start].lines().count()),
            message: e.message().to_string(),
        })?;
        if doc.meta.format != FORMAT_VERSION {
            return Err(IoError::Parse {
                path: path.to_path_buf(),
                line: 0,
                message: format!(
                    "format = {} は扱えない (対応しているのは {FORMAT_VERSION})",
                    doc.meta.format
                ),
            });
        }
        Ok(doc)
    }

    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| IoError::File {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text, path)
    }

    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        crate::atomic::write(path, self.to_toml()?.as_bytes())
    }

    /// 色キーの対応表を検証して取り出す．
    pub fn color_map(&self, path: &Path) -> Result<BTreeMap<char, ColorKey>> {
        let mut out = BTreeMap::new();
        for (key, value) in &self.palette.map {
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(IoError::Parse {
                    path: path.to_path_buf(),
                    line: 0,
                    message: format!("色キー '{key}' が 1 文字でない"),
                });
            };
            if c != TRANSPARENT_KEY && !COLOR_KEYS.contains(c) {
                return Err(IoError::Core(pxsmith_core::CoreError::InvalidColorKey(c)));
            }
            let resolved = match value {
                RawColorKey::Index(i) => ColorKey::Index(*i),
                RawColorKey::Name(name) if name == "transparent" => ColorKey::Transparent,
                RawColorKey::Name(name) => {
                    return Err(IoError::Parse {
                        path: path.to_path_buf(),
                        line: 0,
                        message: format!(
                            "色キー '{key}' の値 '{name}' を解釈できない (整数か \"transparent\")"
                        ),
                    });
                }
            };
            out.insert(c, resolved);
        }
        // 明示されていなければ '.' を透明として補う
        out.entry(TRANSPARENT_KEY).or_insert(ColorKey::Transparent);
        Ok(out)
    }

    /// パレットを読む．`ref` は L0 ファイルからの相対パスとして解決する．
    pub fn load_palette(&self, path: &Path) -> Result<Palette> {
        let base = path.parent().unwrap_or(Path::new("."));
        crate::hex::read(base.join(&self.palette.reference))
    }

    /// パレットを読み，透明の受け皿を用意する．
    ///
    /// `.hex` はアルファを持たない (設計書 4.5) ので，透明が**実際に使われていたら**
    /// アルファ 0 の色をパレットへ足す．透明はメタ (D3) だが，添字がパレットの
    /// 範囲外だと保持層へ書き出せないため，実体を持たせる．
    ///
    /// 使われていなければ足さない．`.` は既定で透明なので，宣言の有無だけで
    /// 判定すると**透明を 1 画素も使っていない絵でもパレットが 1 色増える**．
    pub fn resolve_palette(&self, path: &Path) -> Result<(Palette, Option<u8>)> {
        let mut palette = self.load_palette(path)?;
        let map = self.color_map(path)?;

        let transparent_keys: Vec<char> = map
            .iter()
            .filter(|(_, v)| **v == ColorKey::Transparent)
            .map(|(k, _)| *k)
            .collect();
        let used = self
            .frames
            .iter()
            .any(|f| f.data.chars().any(|c| transparent_keys.contains(&c)));
        if !used {
            return Ok((palette, None));
        }
        if let Some(i) = palette.entries().iter().position(|c| c.a == 0) {
            return Ok((palette, Some(i as u8)));
        }
        let index = palette.push(pxsmith_core::Rgba8::TRANSPARENT)?;
        Ok((palette, Some(index)))
    }

    /// 作業層のフレーム列へ変換する．`path` はパレット解決とエラー表示に使う．
    pub fn to_frames(&self, path: &Path) -> Result<Vec<Frame>> {
        let (palette, transparent) = self.resolve_palette(path)?;
        let map = self.color_map(path)?;

        let mut out = Vec::with_capacity(self.frames.len());
        let mut expected: Option<(u32, u32)> = None;
        for frame in &self.frames {
            let canvas = decode(&frame.data, &map, transparent, path, &frame.name)?;
            let size = (canvas.width(), canvas.height());
            match expected {
                None => expected = Some(size),
                Some(e) if e != size => {
                    return Err(IoError::L0 {
                        path: path.to_path_buf(),
                        violation: Violation::InconsistentSize {
                            frame: frame.name.clone(),
                            w: size.0,
                            h: size.1,
                        },
                    });
                }
                _ => {}
            }

            let mut meta = LayerMeta::named(
                self.meta
                    .layer
                    .clone()
                    .unwrap_or_else(|| self.meta.name.clone()),
            );
            meta.subpixel_exclude = self.meta.subpixel_exclude;
            meta.depth = self.meta.depth.as_deref().and_then(Depth::parse);

            let mut f = Frame::new(uvec2(size.0, size.1), palette.clone());
            f.duration_ms = frame.duration_ms;
            f.kind = FrameKind::parse(&frame.kind).ok_or_else(|| IoError::Parse {
                path: path.to_path_buf(),
                line: 0,
                message: format!(
                    "kind = '{}' を解釈できない (key / breakdown / inbetween)",
                    frame.kind
                ),
            })?;
            f.layers.push(Layer::new(meta, Surface::Indexed(canvas)));
            out.push(f);
        }
        Ok(out)
    }

    /// 作業層のフレーム列から L0 を作る．
    ///
    /// `layer` はどのレイヤを書き出すかの添字．L0 は 1 ファイル 1 レイヤである (D9)．
    pub fn from_frames(
        name: &str,
        palette_ref: impl Into<PathBuf>,
        frames: &[Frame],
        layer: usize,
    ) -> std::result::Result<Exported, Violation> {
        let mut advisories = Vec::new();

        // 使われている添字を集める．透明はキャンバスのメタから取る
        let mut used = std::collections::BTreeSet::new();
        let mut transparent = None;
        let mut layer_name = None;
        let mut subpixel_exclude = false;
        let mut depth = None;
        let mut size: Option<(u32, u32)> = None;

        for f in frames {
            let l = f.layers.get(layer).ok_or(Violation::NotIndexed {
                layer: format!("#{layer}"),
            })?;
            let canvas = l.surface.as_indexed().ok_or(Violation::NotIndexed {
                layer: l.meta.name.clone(),
            })?;
            layer_name.get_or_insert_with(|| l.meta.name.clone());
            subpixel_exclude |= l.meta.subpixel_exclude;
            depth = depth.or(l.meta.depth);
            transparent = transparent.or(canvas.transparent());

            let s = (canvas.width(), canvas.height());
            match size {
                None => size = Some(s),
                Some(e) if e != s => {
                    return Err(Violation::InconsistentSize {
                        frame: name.to_string(),
                        w: s.0,
                        h: s.1,
                    });
                }
                _ => {}
            }
            used.extend(canvas.pixels().iter().copied());
        }

        if let Some((w, h)) = size
            && (w > ADVISED_MAX_SIDE || h > ADVISED_MAX_SIDE)
        {
            advisories.push(Violation::OverAdvisedSize { w, h });
        }

        // 透明は '.' に固定し，残りを COLOR_KEYS の順に割り当てる
        let opaque: Vec<u8> = used
            .iter()
            .copied()
            .filter(|i| Some(*i) != transparent)
            .collect();
        if opaque.len() > COLOR_KEYS.len() {
            return Err(Violation::TooManyColors { used: opaque.len() });
        }

        let mut map = BTreeMap::new();
        let mut to_char = BTreeMap::new();
        if let Some(t) = transparent {
            map.insert(
                TRANSPARENT_KEY.to_string(),
                RawColorKey::Name("transparent".to_string()),
            );
            to_char.insert(t, TRANSPARENT_KEY);
        }
        for (index, key) in opaque.iter().zip(COLOR_KEYS.chars()) {
            map.insert(key.to_string(), RawColorKey::Index(*index));
            to_char.insert(*index, key);
        }

        let out_frames = frames
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let canvas = f.layers[layer].surface.as_indexed().expect("検証済み");
                L0Frame {
                    name: format!("{name}_{i}"),
                    kind: f.kind.as_str().to_string(),
                    duration_ms: f.duration_ms,
                    quadrant: None,
                    data: encode(canvas, &to_char),
                }
            })
            .collect();

        Ok(Exported {
            document: L0Document {
                meta: L0Meta {
                    format: FORMAT_VERSION,
                    name: name.to_string(),
                    layer: layer_name,
                    depth: depth.map(|d| d.as_str().to_string()),
                    anchors: BTreeMap::new(),
                    subpixel_exclude,
                    kind: None,
                    tile_size: None,
                    dither_phase: None,
                },
                palette: L0PaletteSpec {
                    reference: palette_ref.into(),
                    map,
                },
                frames: out_frames,
            },
            advisories,
        })
    }

    /// TOML 本文を作る．`data` は必ず `'''` の複数行リテラルで書く．
    pub fn to_toml(&self) -> Result<String> {
        // toml の serializer は複数行リテラル文字列を選んでくれないので，
        // `data` だけ差し込み用の目印に置き換えてから戻す．目印には
        // エスケープされない ASCII だけを使う (制御文字だと `\u0000` のような形へ化ける)．
        const MARKER: &str = "@@PXFORGE_DATA";
        let mut shadow = self.clone();
        let originals: Vec<String> = shadow
            .frames
            .iter_mut()
            .enumerate()
            .map(|(i, f)| std::mem::replace(&mut f.data, format!("{MARKER}{i}@@")))
            .collect();

        let mut text = toml::to_string_pretty(&shadow).map_err(|e| IoError::Parse {
            path: PathBuf::from("<memory>"),
            line: 0,
            message: e.to_string(),
        })?;

        for (i, original) in originals.iter().enumerate() {
            let needle = format!("\"{MARKER}{i}@@\"");
            let body = original.trim_end_matches('\n');
            text = text.replace(&needle, &format!("'''\n{body}\n'''"));
        }
        Ok(text)
    }
}

/// テキストをキャンバスへ．
fn decode(
    data: &str,
    map: &BTreeMap<char, ColorKey>,
    transparent: Option<u8>,
    path: &Path,
    frame: &str,
) -> Result<IndexedCanvas> {
    // 閉じ区切り直前の改行を 1 つだけ落とし，行へ分ける (開き直後の改行は TOML が落としている)．
    //
    // **`str::lines()` を使う** — `split('\n')` だと行末の `\r` が画素として残るので，
    // **CRLF で届いた L0 が «文字 '␍' が [palette] map に無い» で落ちる**．
    // L0 は人が編集する形式であり，Windows の git は既定でチェックアウト時に
    // CRLF へ変えるので，これは実利用で踏む (`.hex` 側は元から `lines()` である)．
    // 書く側は常に `\n` を書く — 寛容なのは読む側だけでよい．
    let rows: Vec<&str> = data.lines().collect();
    if rows.is_empty() || rows.iter().all(|r| r.is_empty()) {
        return Err(IoError::Parse {
            path: path.to_path_buf(),
            line: 0,
            message: format!("フレーム '{frame}' の data が空"),
        });
    }

    let width = rows[0].chars().count();
    let mut pixels = Vec::with_capacity(width * rows.len());
    for (y, row) in rows.iter().enumerate() {
        let len = row.chars().count();
        if len != width {
            return Err(IoError::Parse {
                path: path.to_path_buf(),
                line: y + 1,
                message: format!(
                    "フレーム '{frame}' の {} 行目が {len} 文字 (1 行目は {width} 文字)",
                    y + 1
                ),
            });
        }
        for c in row.chars() {
            let key = map.get(&c).ok_or_else(|| IoError::Parse {
                path: path.to_path_buf(),
                line: y + 1,
                message: format!("フレーム '{frame}' の文字 '{c}' が [palette] map に無い"),
            })?;
            pixels.push(match key {
                ColorKey::Index(i) => *i,
                ColorKey::Transparent => transparent.unwrap_or(0),
            });
        }
    }

    let canvas = IndexedCanvas::from_pixels(width as u32, rows.len() as u32, pixels)?;
    Ok(canvas.with_transparent(transparent))
}

/// キャンバスをテキストへ．
fn encode(canvas: &IndexedCanvas, to_char: &BTreeMap<u8, char>) -> String {
    let mut out = String::with_capacity((canvas.width() as usize + 1) * canvas.height() as usize);
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            let index = canvas.get(x, y).unwrap_or_default();
            out.push(to_char.get(&index).copied().unwrap_or('?'));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxsmith_core::Rgba8;

    const HERO: &str = r#"
[meta]
format = 1
name = "hero_body"
layer = "body"
subpixel_exclude = false

[palette]
ref = "test.hex"
map = { "." = "transparent", "k" = 1, "h" = 2 }

[[frame]]
name = "idle"
kind = "key"
duration_ms = 83
data = '''
..kk..
.khhk.
..kk..
'''
"#;

    /// テストごとに別のディレクトリを作る．`tag` はテスト名にする．
    ///
    /// 共有ディレクトリへ毎回 `test.hex` を書き直す形だと，`fs::write` が中身を
    /// 空にしてから書き足す間に別のテストが読み，パレットが 0 色に見えることが
    /// ある (透明添字が末尾ではなく 0 になる)．並列実行で稀に落ちるので分ける．
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pxforge-l0-test-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.hex"), "000000\nff0000\n00ff00\n").unwrap();
        dir.join("hero.px.toml")
    }

    #[test]
    fn parses_the_documented_example() {
        let doc = L0Document::parse(HERO, Path::new("hero.px.toml")).unwrap();
        assert_eq!(doc.meta.name, "hero_body");
        assert_eq!(doc.meta.layer.as_deref(), Some("body"));
        assert_eq!(doc.frames.len(), 1);
        assert_eq!(doc.frames[0].duration_ms, 83);
        assert_eq!(doc.frames[0].kind, "key");
    }

    #[test]
    fn rejects_unknown_format_version() {
        let text = HERO.replace("format = 1", "format = 99");
        assert!(L0Document::parse(&text, Path::new("x")).is_err());
    }

    #[test]
    fn decodes_to_the_expected_pixels() {
        let path = scratch("decodes_to_the_expected_pixels");
        let doc = L0Document::parse(HERO, &path).unwrap();
        let frames = doc.to_frames(&path).unwrap();
        assert_eq!(frames.len(), 1);
        let c = frames[0].layers[0].surface.as_indexed().unwrap();
        assert_eq!((c.width(), c.height()), (6, 3));
        // .hex はアルファを持たないので，透明の受け皿がパレット末尾へ足される
        let t = c.transparent().expect("透明添字が要る");
        assert_eq!(t, 3, "3 色の .hex に対して添字 3 が足されるはず");
        assert_eq!(frames[0].palette.get(t).unwrap(), Rgba8::TRANSPARENT);
        assert_eq!(c.get(0, 0), Some(t));
        assert_eq!(c.get(2, 0), Some(1));
        assert_eq!(c.get(2, 1), Some(2));
        assert_eq!(frames[0].duration_ms, 83);
        assert_eq!(frames[0].kind, FrameKind::Key);
        assert_eq!(frames[0].layers[0].meta.name, "body");
    }

    #[test]
    fn trailing_newline_before_closing_delimiter_is_ignored() {
        let path = scratch("trailing_newline_before_closing_delimiter_is_ignored");
        let doc = L0Document::parse(HERO, &path).unwrap();
        let frames = doc.to_frames(&path).unwrap();
        assert_eq!(
            frames[0].layers[0].surface.as_indexed().unwrap().height(),
            3,
            "閉じ区切り直前の改行が余分な行として数えられている"
        );
    }

    #[test]
    fn row_length_mismatch_reports_the_line() {
        let path = scratch("row_length_mismatch_reports_the_line");
        let text = HERO.replace(".khhk.\n", ".khhk\n");
        let doc = L0Document::parse(&text, &path).unwrap();
        match doc.to_frames(&path).unwrap_err() {
            IoError::Parse { line, message, .. } => {
                assert_eq!(line, 2);
                assert!(message.contains("5 文字"), "{message}");
            }
            other => panic!("想定外のエラー: {other}"),
        }
    }

    #[test]
    fn unknown_character_reports_the_line() {
        let path = scratch("unknown_character_reports_the_line");
        let text = HERO.replace("..kk..\n.khhk.", "..kk..\n.kZZk.");
        let doc = L0Document::parse(&text, &path).unwrap();
        match doc.to_frames(&path).unwrap_err() {
            IoError::Parse { line, message, .. } => {
                assert_eq!(line, 2);
                assert!(message.contains('Z'), "{message}");
            }
            other => panic!("想定外のエラー: {other}"),
        }
    }

    #[test]
    fn invalid_color_key_is_rejected() {
        let path = scratch("invalid_color_key_is_rejected");
        let text = HERO.replace(r#""k" = 1"#, r#""!" = 1"#);
        let doc = L0Document::parse(&text, &path).unwrap();
        assert!(matches!(
            doc.color_map(&path).unwrap_err(),
            IoError::Core(pxsmith_core::CoreError::InvalidColorKey('!'))
        ));
    }

    /// M1 の完了条件 — L0 制約内で双方向変換が画素一致すること．
    #[test]
    fn text_to_frames_to_text_is_pixel_identical() {
        let path = scratch("text_to_frames_to_text_is_pixel_identical");
        let doc = L0Document::parse(HERO, &path).unwrap();
        let frames = doc.to_frames(&path).unwrap();

        let exported = L0Document::from_frames("hero_body", "test.hex", &frames, 0).unwrap();
        let text = exported.document.to_toml().unwrap();

        // 書き出した TOML を読み直して同じ画素になるか
        let path2 = path.parent().unwrap().join("hero2.px.toml");
        std::fs::write(&path2, &text).unwrap();
        let reparsed = L0Document::read(&path2).unwrap();
        let frames2 = reparsed.to_frames(&path2).unwrap();

        assert_eq!(frames2.len(), frames.len());
        for (a, b) in frames.iter().zip(&frames2) {
            assert_eq!(
                a.layers[0].surface.as_indexed().unwrap().pixels(),
                b.layers[0].surface.as_indexed().unwrap().pixels()
            );
            assert_eq!(a.duration_ms, b.duration_ms);
            assert_eq!(a.kind, b.kind);
        }
    }

    #[test]
    fn exported_toml_uses_multiline_literal_strings() {
        let path = scratch("exported_toml_uses_multiline_literal_strings");
        let doc = L0Document::parse(HERO, &path).unwrap();
        let frames = doc.to_frames(&path).unwrap();
        let text = L0Document::from_frames("hero", "test.hex", &frames, 0)
            .unwrap()
            .document
            .to_toml()
            .unwrap();
        assert!(
            text.contains("'''"),
            "複数行リテラルで書かれていない:\n{text}"
        );
        assert!(!text.contains("\\n"), "改行がエスケープされている:\n{text}");
    }

    #[test]
    fn too_many_colors_is_rejected() {
        // 63 色 + 透明
        let palette = Palette::new(
            (0..64)
                .map(|i| {
                    if i == 0 {
                        Rgba8::TRANSPARENT
                    } else {
                        Rgba8::rgb(i as u8, 0, 0)
                    }
                })
                .collect(),
        )
        .unwrap();
        let mut f = Frame::new(uvec2(63, 1), palette);
        let canvas = IndexedCanvas::from_pixels(63, 1, (1..64).collect())
            .unwrap()
            .with_transparent(Some(0));
        f.layers.push(Layer::new(
            LayerMeta::named("big"),
            Surface::Indexed(canvas),
        ));

        assert_eq!(
            L0Document::from_frames("big", "p.hex", &[f], 0).unwrap_err(),
            Violation::TooManyColors { used: 63 }
        );
    }

    #[test]
    fn oversized_sprite_is_only_an_advisory() {
        let palette = Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(1, 1, 1)]).unwrap();
        let mut f = Frame::new(uvec2(64, 64), palette);
        f.layers.push(Layer::new(
            LayerMeta::named("bg"),
            Surface::Indexed(IndexedCanvas::filled(64, 64, 1).with_transparent(Some(0))),
        ));
        let exported = L0Document::from_frames("bg", "p.hex", &[f], 0).unwrap();
        assert_eq!(
            exported.advisories,
            vec![Violation::OverAdvisedSize { w: 64, h: 64 }]
        );
        assert!(!exported.advisories[0].is_blocking());
    }

    #[test]
    fn non_indexed_layer_is_blocking() {
        let palette = Palette::new(vec![Rgba8::rgb(1, 1, 1)]).unwrap();
        let mut f = Frame::new(uvec2(2, 2), palette);
        f.layers.push(Layer::new(
            LayerMeta::named("art"),
            Surface::Rgba(pxsmith_core::RgbaCanvas::filled(2, 2, Rgba8::TRANSPARENT)),
        ));
        let e = L0Document::from_frames("art", "p.hex", &[f], 0).unwrap_err();
        assert_eq!(
            e,
            Violation::NotIndexed {
                layer: "art".to_string()
            }
        );
        assert!(e.is_blocking());
    }

    #[test]
    fn frame_kind_and_duration_survive_the_round_trip() {
        let path = scratch("frame_kind_and_duration_survive_the_round_trip");
        let text = HERO
            .replace(r#"kind = "key""#, r#"kind = "inbetween""#)
            .replace("duration_ms = 83", "duration_ms = 42");
        let doc = L0Document::parse(&text, &path).unwrap();
        let frames = doc.to_frames(&path).unwrap();
        assert_eq!(frames[0].kind, FrameKind::Inbetween);
        assert_eq!(frames[0].duration_ms, 42);
    }

    /// **L0 は人が編集する形式なので，CRLF で届く．**
    /// Windows の git は既定でチェックアウト時に `\n` を `\r\n` へ変える．
    /// 行末の `\r` を画素として読むと «文字 '␍' が [palette] map に無い» で落ちる —
    /// これは Windows の CI でのみ出た (`px-macro` の fixture が CRLF になるため)．
    /// **`.hex` 側は `str::lines()` を使っていて元から通っていた**ので，
    /// 差は «行の分け方» 1 か所にしかなかった．
    #[test]
    fn crlf_line_endings_parse_the_same_as_lf() {
        let lf = scratch("crlf_line_endings_parse_the_same_as_lf_lf");
        let crlf = scratch("crlf_line_endings_parse_the_same_as_lf_crlf");

        let want = L0Document::parse(HERO, &lf)
            .unwrap()
            .to_frames(&lf)
            .unwrap();
        let got = L0Document::parse(&HERO.replace('\n', "\r\n"), &crlf)
            .unwrap()
            .to_frames(&crlf)
            .unwrap();

        assert_eq!(got.len(), want.len());
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g.size, w.size, "画布が CRLF で変わった");
            let (gi, wi) = (
                g.layers[0].surface.as_indexed().unwrap(),
                w.layers[0].surface.as_indexed().unwrap(),
            );
            assert_eq!(gi.width(), wi.width(), "幅が CRLF で変わった");
            assert_eq!(gi.pixels(), wi.pixels(), "画素が CRLF で変わった");
        }
    }

    #[test]
    fn dot_defaults_to_transparent_without_an_explicit_entry() {
        let path = scratch("dot_defaults_to_transparent_without_an_explicit_entry");
        let text = HERO.replace(r#"{ "." = "transparent", "#, "{ ");
        let doc = L0Document::parse(&text, &path).unwrap();
        assert_eq!(
            doc.color_map(&path).unwrap().get(&'.'),
            Some(&ColorKey::Transparent)
        );
    }
}
