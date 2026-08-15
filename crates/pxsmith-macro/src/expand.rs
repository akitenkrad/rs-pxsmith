//! `pixels!` の展開．
//!
//! 2 つの書き方をどちらも [`pxsmith_io::l0::L0Document`] へ寄せてから
//! `to_frames` に通す．**実行時と同じ経路で検証する**ので，マクロだけが通って
//! 実行時に落ちる (あるいはその逆) ということが起きない．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use proc_macro2::TokenStream;
use quote::quote;

use pxsmith_io::l0::{L0Document, L0Frame, L0Meta, L0PaletteSpec, RawColorKey};
use pxsmith_io::pxsmith_core::frame::FrameKind;
use pxsmith_io::pxsmith_core::{Frame, Rgba8};

use crate::input::{Input, MapValue};

/// 呼び出し側クレートの根．
fn manifest_dir() -> PathBuf {
    std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn expand(input: &Input) -> syn::Result<TokenStream> {
    match input {
        Input::File(path_lit) => {
            let path = manifest_dir().join(path_lit.value());
            let doc = L0Document::read(&path)
                .map_err(|e| syn::Error::new(path_lit.span(), e.to_string()))?;
            let frames = doc
                .to_frames(&path)
                .map_err(|e| syn::Error::new(path_lit.span(), e.to_string()))?;
            let palette_path = path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&doc.palette.reference);
            Ok(emit(&frames, &[path.clone(), palette_path]))
        }
        Input::Inline(inline) => {
            // その場に書いた指定を L0 の姿へ移す
            let mut map = BTreeMap::new();
            for entry in &inline.map {
                let key = entry.key.value().to_string();
                let value = match &entry.value {
                    MapValue::Transparent => RawColorKey::Name("transparent".to_string()),
                    MapValue::Index(lit) => RawColorKey::Index(lit.base10_parse::<u8>()?),
                };
                if map.insert(key, value).is_some() {
                    return Err(syn::Error::new(
                        entry.key.span(),
                        "同じ色キーが 2 回出てくる",
                    ));
                }
            }

            if inline.rows.is_empty() {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "`rows` が空",
                ));
            }
            let data = inline
                .rows
                .iter()
                .map(|r| r.value())
                .collect::<Vec<_>>()
                .join("\n");

            let name = inline
                .name
                .as_ref()
                .map(|l| l.value())
                .unwrap_or_else(|| "pixels".to_string());
            let doc = L0Document {
                meta: L0Meta {
                    format: pxsmith_io::l0::FORMAT_VERSION,
                    name: name.clone(),
                    layer: inline.layer.as_ref().map(|l| l.value()),
                    ..L0Meta::default()
                },
                palette: L0PaletteSpec {
                    reference: PathBuf::from(inline.palette.value()),
                    map,
                },
                frames: vec![L0Frame {
                    name,
                    kind: inline
                        .kind
                        .as_ref()
                        .map(|l| l.value())
                        .unwrap_or_else(|| "key".to_string()),
                    duration_ms: inline
                        .duration_ms
                        .as_ref()
                        .map(|l| l.base10_parse::<u32>())
                        .transpose()?
                        .unwrap_or(100),
                    quadrant: None,
                    data,
                }],
            };

            // `ref` は L0 ファイルからの相対で解かれるので，マニフェスト直下の
            // 架空のファイルを基準にすると呼び出し側から見た相対パスと一致する
            let pseudo = manifest_dir().join("<pixels!>");
            let frames = doc.to_frames(&pseudo).map_err(|e| {
                let span = row_span(inline, &e).unwrap_or_else(|| inline.palette.span());
                syn::Error::new(span, e.to_string())
            })?;

            let palette_path = manifest_dir().join(inline.palette.value());
            Ok(emit(&frames, &[palette_path]))
        }
    }
}

/// 失敗した行の字句範囲を引く．行長の食い違いをその行に下線で示すため．
fn row_span(inline: &crate::input::Inline, e: &pxsmith_io::IoError) -> Option<proc_macro2::Span> {
    let pxsmith_io::IoError::Parse { line, .. } = e else {
        return None;
    };
    if *line == 0 {
        return None;
    }
    inline.rows.get(line - 1).map(|r| r.span())
}

/// フレーム列を構築するコードを吐く．
///
/// `deps` に挙げたファイルは `include_str!` で参照して cargo に依存を追跡させる．
/// これで `.hex` を書き換えたときに再コンパイルが走る．
fn emit(frames: &[Frame], deps: &[PathBuf]) -> TokenStream {
    let includes = deps.iter().map(|p| {
        let s = p.to_string_lossy().into_owned();
        quote! { const _: &str = ::core::include_str!(#s); }
    });

    let palette = frames
        .first()
        .map(|f| f.palette.entries().to_vec())
        .unwrap_or_default();
    let entries = palette.iter().map(|c: &Rgba8| {
        let (r, g, b, a) = (c.r, c.g, c.b, c.a);
        quote! { ::pxsmith_core::Rgba8::new(#r, #g, #b, #a) }
    });

    let bodies = frames.iter().map(|f| {
        let layer = f.layers.first().expect("L0 は必ず 1 レイヤを作る");
        let canvas = layer
            .surface
            .as_indexed()
            .expect("L0 はインデックスカラーのみ");
        let (w, h) = (canvas.width(), canvas.height());
        let bytes = proc_macro2::Literal::byte_string(canvas.pixels());
        let transparent = match canvas.transparent() {
            Some(t) => quote! { ::core::option::Option::Some(#t) },
            None => quote! { ::core::option::Option::None },
        };
        let duration = f.duration_ms;
        let kind = match f.kind {
            FrameKind::Key => quote! { ::pxsmith_core::FrameKind::Key },
            FrameKind::Breakdown => quote! { ::pxsmith_core::FrameKind::Breakdown },
            FrameKind::Inbetween => quote! { ::pxsmith_core::FrameKind::Inbetween },
        };
        let layer_name = layer.meta.name.clone();
        let subpixel_exclude = layer.meta.subpixel_exclude;

        quote! {{
            let __canvas = ::pxsmith_core::IndexedCanvas::from_pixels(#w, #h, (#bytes as &[u8]).to_vec())
                .expect("pixels!: 画素数は展開時に検証済み")
                .with_transparent(#transparent);
            let mut __meta = ::pxsmith_core::LayerMeta::named(#layer_name);
            __meta.subpixel_exclude = #subpixel_exclude;
            let mut __frame = ::pxsmith_core::Frame::new(::pxsmith_core::uvec2(#w, #h), __palette.clone());
            __frame.duration_ms = #duration;
            __frame.kind = #kind;
            __frame.layers.push(::pxsmith_core::Layer::new(
                __meta,
                ::pxsmith_core::Surface::Indexed(__canvas),
            ));
            __frames.push(__frame);
        }}
    });

    let count = frames.len();
    quote! {{
        #(#includes)*
        let __palette = ::pxsmith_core::Palette::new(::std::vec![#(#entries),*])
            .expect("pixels!: パレットは展開時に検証済み");
        let mut __frames: ::std::vec::Vec<::pxsmith_core::Frame> =
            ::std::vec::Vec::with_capacity(#count);
        #(#bodies)*
        __frames
    }}
}
