//! `.pxfont.toml` の I/O に依存しないフォントモデル．

use std::collections::{BTreeMap, BTreeSet};

use crate::IndexedCanvas;

/// `.pxfont.toml` の版．
pub const FORMAT_VERSION: u32 = 1;

/// グリフの画素が単色か，パレット添字を持つか．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum FontMode {
    #[default]
    Mono,
    Color,
}

/// 多色フォントの記号が指すもの．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FontColorKey {
    Transparent,
    Index(u8),
}

/// 多色フォントの外部パレットと，1 文字 1 画素の記号表．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontPalette {
    pub reference: String,
    pub map: BTreeMap<char, FontColorKey>,
}

/// フォント全体に共通する寸法と由来．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontMeta {
    pub format: u32,
    pub name: String,
    pub mode: FontMode,
    pub line_height: u32,
    pub baseline: u32,
    pub default_advance: u32,
    pub source: Option<String>,
    pub copyright: Option<String>,
    pub palette: Option<FontPalette>,
}

/// 1 字分の升．`canvas` の幅は字形の幅，高さは `line_height` である．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub character: char,
    pub variant: Option<String>,
    pub advance: Option<u32>,
    pub canvas: IndexedCanvas,
}

impl Glyph {
    /// 個別指定が無いときは画布の幅を使い，幅 0 の字だけフォントの既定字送りを使う．
    pub fn effective_advance(&self, meta: &FontMeta) -> u32 {
        self.advance.unwrap_or_else(|| self.implicit_advance(meta))
    }

    fn implicit_advance(&self, meta: &FontMeta) -> u32 {
        let width = self.canvas.width();
        if width == 0 {
            meta.default_advance
        } else {
            width
        }
    }
}

/// 横書きの字の組に対する字送りの調整量．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kern {
    pub pair: String,
    pub amount: i32,
}

/// 1 ファイル分のフォント．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Font {
    pub meta: FontMeta,
    pub glyphs: Vec<Glyph>,
    pub kerns: Vec<Kern>,
}

/// フォントモデルの不変条件に反した理由．
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FontError {
    #[error("format = {found} は扱えない（対応しているのは {FORMAT_VERSION}）")]
    Format { found: u32 },
    #[error("baseline {baseline} が line_height {line_height} を超えている")]
    Baseline { baseline: u32, line_height: u32 },
    #[error("字 '{character}' の高さ {actual} が line_height {expected} と一致しない")]
    GlyphHeight {
        character: char,
        expected: u32,
        actual: u32,
    },
    #[error("単色の字 '{character}' は透明添字を Some(0) にしなければならない")]
    MonoTransparentIndex { character: char },
    #[error("単色の字 '{character}' が添字 {index} を使っている（使えるのは 0 と 1）")]
    MonoIndex { character: char, index: u8 },
    #[error("多色フォントに [palette] が無い")]
    ColorPaletteMissing,
    #[error("単色フォントに [palette] がある")]
    MonoPalettePresent,
    #[error("字 '{character}' の variant が空である")]
    EmptyVariant { character: char },
    #[error("字 '{character}' の variant {variant:?} が重複している")]
    DuplicateGlyph {
        character: char,
        variant: Option<String>,
    },
    #[error("kern の pair '{pair}' は 2 文字でなければならない（{actual} 文字）")]
    KernPairLength { pair: String, actual: usize },
    #[error("kern の pair '{pair}' が重複している")]
    DuplicateKern { pair: String },
}

impl Font {
    /// 高さ，透明添字，重複，カーニング対を検査する．
    pub fn validate(&self) -> Result<(), FontError> {
        if self.meta.format != FORMAT_VERSION {
            return Err(FontError::Format {
                found: self.meta.format,
            });
        }
        if self.meta.baseline > self.meta.line_height {
            return Err(FontError::Baseline {
                baseline: self.meta.baseline,
                line_height: self.meta.line_height,
            });
        }
        match (&self.meta.mode, &self.meta.palette) {
            (FontMode::Mono, Some(_)) => return Err(FontError::MonoPalettePresent),
            (FontMode::Color, None) => return Err(FontError::ColorPaletteMissing),
            _ => {}
        }

        let mut glyph_keys = BTreeSet::new();
        for glyph in &self.glyphs {
            if glyph.canvas.height() != self.meta.line_height {
                return Err(FontError::GlyphHeight {
                    character: glyph.character,
                    expected: self.meta.line_height,
                    actual: glyph.canvas.height(),
                });
            }
            if glyph.variant.as_deref() == Some("") {
                return Err(FontError::EmptyVariant {
                    character: glyph.character,
                });
            }
            if self.meta.mode == FontMode::Mono {
                if glyph.canvas.transparent() != Some(0) {
                    return Err(FontError::MonoTransparentIndex {
                        character: glyph.character,
                    });
                }
                if let Some(&index) = glyph.canvas.pixels().iter().find(|&&index| index > 1) {
                    return Err(FontError::MonoIndex {
                        character: glyph.character,
                        index,
                    });
                }
            }
            let key = (glyph.character, glyph.variant.clone());
            if !glyph_keys.insert(key.clone()) {
                return Err(FontError::DuplicateGlyph {
                    character: key.0,
                    variant: key.1,
                });
            }
        }

        let mut kern_pairs = BTreeSet::new();
        for kern in &self.kerns {
            let actual = kern.pair.chars().count();
            if actual != 2 {
                return Err(FontError::KernPairLength {
                    pair: kern.pair.clone(),
                    actual,
                });
            }
            if !kern_pairs.insert(kern.pair.clone()) {
                return Err(FontError::DuplicateKern {
                    pair: kern.pair.clone(),
                });
            }
        }
        Ok(())
    }

    /// グリフとカーニングを正規形の符号位置順へ並べる．
    pub fn canonicalize(&mut self) {
        for glyph in &mut self.glyphs {
            if glyph.advance == Some(glyph.implicit_advance(&self.meta)) {
                glyph.advance = None;
            }
        }
        self.glyphs
            .sort_by(|a, b| (a.character, &a.variant).cmp(&(b.character, &b.variant)));
        self.kerns.sort_by(|a, b| {
            let a = a.pair.chars().collect::<Vec<_>>();
            let b = b.pair.chars().collect::<Vec<_>>();
            a.cmp(&b)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(character: char, variant: Option<&str>) -> Glyph {
        Glyph {
            character,
            variant: variant.map(str::to_owned),
            advance: None,
            canvas: crate::IndexedCanvas::filled(2, 3, 0).with_transparent(Some(0)),
        }
    }

    fn font(glyphs: Vec<Glyph>, kerns: Vec<Kern>) -> Font {
        Font {
            meta: FontMeta {
                format: FORMAT_VERSION,
                name: "test".into(),
                mode: FontMode::Mono,
                line_height: 3,
                baseline: 2,
                default_advance: 2,
                source: None,
                copyright: None,
                palette: None,
            },
            glyphs,
            kerns,
        }
    }

    #[test]
    fn omitted_advance_uses_nonzero_canvas_width() {
        let mut g = glyph('A', None);
        g.canvas = crate::IndexedCanvas::filled(3, 3, 0).with_transparent(Some(0));
        let value = font(vec![g], vec![]);
        assert_eq!(value.glyphs[0].effective_advance(&value.meta), 3);
    }

    #[test]
    fn omitted_advance_uses_default_for_zero_width_canvas() {
        let mut g = glyph('A', None);
        g.canvas = crate::IndexedCanvas::filled(0, 3, 0).with_transparent(Some(0));
        let value = font(vec![g], vec![]);
        assert_eq!(value.glyphs[0].effective_advance(&value.meta), 2);
    }

    #[test]
    fn mono_glyphs_require_explicit_transparent_index_zero() {
        let mut g = glyph('A', None);
        g.canvas.set_transparent(None);
        let error = font(vec![g], vec![]).validate().unwrap_err();
        assert!(matches!(
            error,
            FontError::MonoTransparentIndex { character: 'A' }
        ));
    }

    #[test]
    fn duplicate_character_and_variant_is_rejected() {
        let error = font(vec![glyph('A', None), glyph('A', None)], vec![])
            .validate()
            .unwrap_err();
        assert!(matches!(
            error,
            FontError::DuplicateGlyph { character: 'A', .. }
        ));
    }

    #[test]
    fn malformed_and_duplicate_kerning_pairs_are_rejected() {
        let malformed = font(
            vec![glyph('A', None)],
            vec![Kern {
                pair: "A".into(),
                amount: -1,
            }],
        )
        .validate()
        .unwrap_err();
        assert!(matches!(
            malformed,
            FontError::KernPairLength { actual: 1, .. }
        ));

        let duplicate = font(
            vec![glyph('A', None)],
            vec![
                Kern {
                    pair: "AV".into(),
                    amount: -1,
                },
                Kern {
                    pair: "AV".into(),
                    amount: -2,
                },
            ],
        )
        .validate()
        .unwrap_err();
        assert!(matches!(duplicate, FontError::DuplicateKern { .. }));
    }

    #[test]
    fn canonicalize_orders_base_before_variants_and_kerns_by_codepoint() {
        let mut value = font(
            vec![
                glyph('ー', Some("vert")),
                glyph('A', None),
                glyph('ー', None),
            ],
            vec![
                Kern {
                    pair: "VA".into(),
                    amount: -1,
                },
                Kern {
                    pair: "AV".into(),
                    amount: -1,
                },
            ],
        );
        value.canonicalize();
        assert_eq!(
            value
                .glyphs
                .iter()
                .map(|g| (g.character, g.variant.as_deref()))
                .collect::<Vec<_>>(),
            vec![('A', None), ('ー', None), ('ー', Some("vert"))]
        );
        assert_eq!(
            value
                .kerns
                .iter()
                .map(|k| k.pair.as_str())
                .collect::<Vec<_>>(),
            vec!["AV", "VA"]
        );
    }
}
