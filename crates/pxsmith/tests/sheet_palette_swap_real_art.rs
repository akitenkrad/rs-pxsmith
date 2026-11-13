//! 実素材全件で，パレットスワップ梱包と従来梱包の色を突き合わせる．

use std::path::{Path, PathBuf};

use pxsmith_core::canvas::{IndexedCanvas, RgbaCanvas};
use pxsmith_core::color::Rgba8;
use pxsmith_core::error::CoreError;
use pxsmith_core::palette::Palette;
use pxsmith_core::sheet::{
    PackOptions, PaletteSwapItem, SheetItem, pack, pack_palette_swap, resolve,
};

fn seeds() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/grid-eval/seeds")
        .canonicalize()
        .expect("種の置き場所がある")
}

fn index_exactly(img: &RgbaCanvas) -> Option<(IndexedCanvas, Palette)> {
    let mut colors: Vec<Rgba8> = img.pixels().to_vec();
    colors.sort_unstable_by_key(|color| color.sort_key());
    colors.dedup();
    if colors.len() > 256 {
        return None;
    }
    let palette = Palette::new(colors).ok()?;
    let transparent = palette
        .entries()
        .iter()
        .position(|color| color.a == 0)
        .map(|index| index as u8);
    let pixels: Vec<u8> = img
        .pixels()
        .iter()
        .map(|color| match (color.a, transparent) {
            (0, Some(index)) => index,
            _ => palette.nearest(*color, 1.0).unwrap_or(0),
        })
        .collect();
    let canvas = IndexedCanvas::from_pixels(img.width(), img.height(), pixels)
        .ok()?
        .with_transparent(transparent);
    Some((canvas, palette))
}

fn items() -> Vec<SheetItem> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(seeds())
        .expect("種を読める")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
        })
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|path| {
            let image = pxsmith_io::png::read_rgba(path).ok()?;
            let (canvas, palette) = index_exactly(&image)?;
            Some(SheetItem {
                name: path.file_stem()?.to_string_lossy().to_string(),
                canvas,
                palette,
                duration_ms: 100,
            })
        })
        .collect()
}

fn swap_items(items: &[SheetItem]) -> Vec<PaletteSwapItem> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| PaletteSwapItem {
            item: item.clone(),
            palette_name: Some(format!("palette{index}")),
            atlas_palette: None,
        })
        .collect()
}

fn same_colour(a: Option<Rgba8>, b: Option<Rgba8>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) if x.a == 0 && y.a == 0 => true,
        (x, y) => x == y,
    }
}

#[test]
fn palette_swap_packs_every_real_art_seed_without_merging_colours() {
    let items = items();
    assert!(items.len() > 50, "実素材が {} 枚しかない", items.len());
    assert!(matches!(
        pack(&items, &PackOptions::default()),
        Err(CoreError::SheetTooManyColors { .. })
    ));

    let (_, atlas, doc, report) =
        pack_palette_swap(&swap_items(&items), &[], &PackOptions::default())
            .expect("パレットを併合しなければ全件を梱包できる");
    assert_eq!(doc.cells.len(), items.len());
    assert_eq!(report.items, items.len());
    assert_eq!(
        atlas.height(),
        doc.palettes.as_ref().expect("パレットメタ").rows.len() as u32
    );
}

#[test]
fn every_resolved_real_art_cell_matches_its_individual_legacy_sheet() {
    let items = items();
    let (indices, atlas, doc, _) =
        pack_palette_swap(&swap_items(&items), &[], &PackOptions::default())
            .expect("パレットスワップ梱包");
    let resolved = resolve(&indices, &atlas, &doc, None).expect("基の色で復元");

    let mut checked = 0usize;
    for (item, cell) in items.iter().zip(&doc.cells) {
        let (legacy, palette, _, _) =
            pack(std::slice::from_ref(item), &PackOptions::default()).expect("単体の従来梱包");
        for y in 0..item.canvas.height() as i32 {
            for x in 0..item.canvas.width() as i32 {
                let want = legacy.get(x, y).and_then(|index| palette.get(index));
                let got = resolved.get(cell.x as i32 + x, cell.y as i32 + y);
                assert!(
                    same_colour(want, got),
                    "{} の ({x},{y}) が単体梱包の {want:?} から {got:?} へ変わった",
                    item.name
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 30_000, "{checked} 画素しか見ていない");
}
