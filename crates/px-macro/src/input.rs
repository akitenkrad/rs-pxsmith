//! `pixels!` の入力構文．

use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitChar, LitInt, LitStr, Token, braced, bracketed};

/// 色キーの割り当て先．
pub enum MapValue {
    Transparent,
    Index(LitInt),
}

pub struct MapEntry {
    pub key: LitChar,
    pub value: MapValue,
}

/// その場に書く形．
pub struct Inline {
    pub palette: LitStr,
    pub name: Option<LitStr>,
    pub layer: Option<LitStr>,
    pub duration_ms: Option<LitInt>,
    pub kind: Option<LitStr>,
    pub map: Vec<MapEntry>,
    pub rows: Vec<LitStr>,
}

/// `pixels!` の入力．
pub enum Input {
    /// L0 ファイルを読む形．
    File(LitStr),
    Inline(Box<Inline>),
}

impl Parse for MapEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: LitChar = input.parse()?;
        input.parse::<Token![=]>()?;
        let value = if input.peek(LitInt) {
            MapValue::Index(input.parse()?)
        } else {
            let ident: Ident = input.parse()?;
            if ident != "transparent" {
                return Err(syn::Error::new(
                    ident.span(),
                    "色キーの値は整数か `transparent` のいずれか",
                ));
            }
            MapValue::Transparent
        };
        Ok(Self { key, value })
    }
}

impl Parse for Input {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(LitStr) {
            let path: LitStr = input.parse()?;
            if !input.is_empty() {
                return Err(input.error(
                    "L0 ファイルを読む形では引数はパス 1 つだけ．\
                     その場に書く形は `palette = ..., map = { .. }, rows = [ .. ]`",
                ));
            }
            return Ok(Self::File(path));
        }

        let mut palette = None;
        let mut name = None;
        let mut layer = None;
        let mut duration_ms = None;
        let mut kind = None;
        let mut map = None;
        let mut rows = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "palette" => palette = Some(input.parse::<LitStr>()?),
                "name" => name = Some(input.parse::<LitStr>()?),
                "layer" => layer = Some(input.parse::<LitStr>()?),
                "kind" => kind = Some(input.parse::<LitStr>()?),
                "duration_ms" => duration_ms = Some(input.parse::<LitInt>()?),
                "map" => {
                    let content;
                    braced!(content in input);
                    let entries: syn::punctuated::Punctuated<MapEntry, Token![,]> =
                        content.parse_terminated(MapEntry::parse, Token![,])?;
                    map = Some(entries.into_iter().collect::<Vec<_>>());
                }
                "rows" => {
                    let content;
                    bracketed!(content in input);
                    let entries: syn::punctuated::Punctuated<LitStr, Token![,]> = content
                        .parse_terminated(|s: ParseStream| s.parse::<LitStr>(), Token![,])?;
                    rows = Some(entries.into_iter().collect::<Vec<_>>());
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "`{other}` は使えない (palette / name / layer / kind / duration_ms / map / rows)"
                        ),
                    ));
                }
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error("項目の区切りにカンマが要る"));
            }
        }

        let span = proc_macro2::Span::call_site();
        Ok(Self::Inline(Box::new(Inline {
            palette: palette.ok_or_else(|| syn::Error::new(span, "`palette = \"...\"` が要る"))?,
            name,
            layer,
            duration_ms,
            kind,
            map: map.ok_or_else(|| syn::Error::new(span, "`map = { .. }` が要る"))?,
            rows: rows.ok_or_else(|| syn::Error::new(span, "`rows = [ .. ]` が要る"))?,
        })))
    }
}
