//! **格子を当てても元絵が戻るのか**を測る．
//!
//! `pxsmith conform` は格子に沿ってセルの最頻色へ縮小する ([`downscale_modal`]) ．ところが
//! 強い補間で滲んだセルの最頻色は元の色とは限らない — セルの中がもう平坦ではないからで
//! ある．**当てても戻らないなら，当てにいく意味が薄い**．
//!
//! そこで推定器を通さず，**正解の $(s, d)$ を与えて**縮小し，元絵と突き合わせる．
//! 測っているのは「格子が当たった場合の上限」であり，格子推定の成績ではない．
//!
//! この結果で完了条件の区分 (D66 の C) が決まる — 戻らないなら，強い補間の入力に
//! 対する正解は「棄却」であって「当てること」ではなくなる．

use std::path::Path;

use anyhow::{Context, Result};
use pxsmith_core::color::{delta_e, oklab_of};
use pxsmith_core::grid::downscale_modal;
use pxsmith_core::{RgbaCanvas, ivec2};
use rayon::prelude::*;

use crate::dataset::{Manifest, Split, source_of};
use crate::sprite::Seed;

/// 1 件分の測定．
#[derive(Clone, Debug)]
pub struct Record {
    pub item_id: u32,
    pub filter: String,
    pub compression: String,
    pub scale: u32,
    /// 突き合わせた画素数．
    pub pixels: usize,
    /// **元絵と完全に一致した画素の割合．**
    pub exact: f32,
    /// 色差の中央値 (OKLab の $\Delta E$)．
    pub median_delta_e: f32,
    /// 元絵の色数 (突き合わせた範囲) ．
    pub colors_source: usize,
    /// 復元した絵の色数．**滲むと増える** — 元のパレットへ戻せなくなる目安である．
    pub colors_recovered: usize,
    /// **元絵のパレットへ寄せてから**突き合わせた一致率．
    ///
    /// `conform` の後ろには量子化とパレット適用が続く (設計書 6.2) ．色がずれていても
    /// 元の色へ寄せ直せるなら，格子を当てる価値は残る．各画素を元絵のパレットの最近色
    /// (OKLab) へ写してから比べる — **どんな量子化を使っても超えられない上限**である．
    pub exact_snapped: f32,
}

pub const HEADER: &str = "item_id,filter,compression,scale,pixels,exact,exact_snapped,\
median_delta_e,colors_source,colors_recovered";

impl Record {
    pub fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{},{:.5},{:.5},{:.5},{},{}",
            self.item_id,
            self.filter,
            self.compression,
            self.scale,
            self.pixels,
            self.exact,
            self.exact_snapped,
            self.median_delta_e,
            self.colors_source,
            self.colors_recovered,
        )
    }
}

/// 復元した絵の左上が，元絵のどの画素に当たるか．
///
/// クロップで左端の `crop` 列を捨てると，最初の**完全な**セルは元絵の
/// $\lceil \mathrm{crop} / s \rceil$ 番目の画素から始まる．端の欠けたセルは
/// [`downscale_modal`] が含めないので，ここがずれると全体が 1 画素ずれて
/// **一致率が意味を失う**．
fn first_source_pixel(crop: u32, scale: u32) -> u32 {
    crop.div_ceil(scale)
}

/// 正解の格子で縮小し，元絵と突き合わせる．
pub fn run(
    dir: &Path,
    manifest: &Manifest,
    only: Option<Split>,
    seeds: &[Seed],
) -> Result<Vec<Record>> {
    let items: Vec<_> = manifest
        .items
        .iter()
        .filter(|i| only.is_none_or(|s| i.split == s))
        // 整数の格子が無い件には「正解の格子」が無いので測れない
        .filter(|i| i.has_integer_grid())
        .collect();

    items
        .par_iter()
        .map(|item| -> Result<Record> {
            let (_, source) = source_of(seeds, item.source_seed);
            let img = pxsmith_io::png::read_rgba(dir.join(&item.file))
                .with_context(|| format!("{} を読めない", item.file))?;
            let (s, phase) = (
                item.truth_scale,
                item.truth_phase.context("整数の格子がある件のはずである")?,
            );
            let recovered = downscale_modal(&img, s, ivec2(phase.0 as i32, phase.1 as i32));

            let (ox, oy) = (
                first_source_pixel(item.degradation.crop.0, s),
                first_source_pixel(item.degradation.crop.1, s),
            );
            Ok(compare(item.id, item, &source, &recovered, ox, oy))
        })
        .collect()
}

fn compare(
    id: u32,
    item: &crate::dataset::Item,
    source: &RgbaCanvas,
    recovered: &RgbaCanvas,
    ox: u32,
    oy: u32,
) -> Record {
    let mut matched = 0usize;
    let mut snapped = 0usize;
    let mut total = 0usize;
    let mut deltas: Vec<f32> = Vec::new();
    let mut cs = std::collections::BTreeSet::new();
    let mut cr = std::collections::BTreeSet::new();

    // 元絵のパレット (突き合わせる範囲のもの) ．最近色へ写すために先に集める
    let mut palette: Vec<pxsmith_core::Rgba8> = Vec::new();
    for y in 0..source.height() {
        for x in 0..source.width() {
            if let Some(c) = source.get(x as i32, y as i32)
                && !palette.iter().any(|p| (p.r, p.g, p.b) == (c.r, c.g, c.b))
            {
                palette.push(c);
            }
        }
    }
    let nearest = |c: pxsmith_core::Rgba8| -> pxsmith_core::Rgba8 {
        let lab = oklab_of(c);
        *palette
            .iter()
            .min_by(|a, b| delta_e(oklab_of(**a), lab).total_cmp(&delta_e(oklab_of(**b), lab)))
            .unwrap_or(&c)
    };

    for y in 0..recovered.height() {
        for x in 0..recovered.width() {
            let (Some(got), Some(want)) = (
                recovered.get(x as i32, y as i32),
                source.get((ox + x) as i32, (oy + y) as i32),
            ) else {
                continue;
            };
            total += 1;
            // アルファは劣化前に潰してあるので RGB だけを見る
            if (got.r, got.g, got.b) == (want.r, want.g, want.b) {
                matched += 1;
            }
            let snap = nearest(got);
            if (snap.r, snap.g, snap.b) == (want.r, want.g, want.b) {
                snapped += 1;
            }
            deltas.push(delta_e(oklab_of(got), oklab_of(want)));
            cs.insert((want.r, want.g, want.b));
            cr.insert((got.r, got.g, got.b));
        }
    }

    deltas.sort_by(f32::total_cmp);
    Record {
        item_id: id,
        filter: item.degradation.filter.as_str().to_string(),
        compression: item.degradation.compression.as_str().to_string(),
        scale: item.truth_scale,
        pixels: total,
        exact: if total == 0 {
            0.0
        } else {
            matched as f32 / total as f32
        },
        exact_snapped: if total == 0 {
            0.0
        } else {
            snapped as f32 / total as f32
        },
        median_delta_e: deltas.get(deltas.len() / 2).copied().unwrap_or(0.0),
        colors_source: cs.len(),
        colors_recovered: cr.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_full_cell_follows_the_crop() {
        // 切り落としが 0 なら元絵の 0 番目から
        assert_eq!(first_source_pixel(0, 4), 0);
        // 1〜4 画素捨てたら，最初の完全なセルは元絵の 1 番目
        for crop in 1..=4 {
            assert_eq!(first_source_pixel(crop, 4), 1, "crop {crop}");
        }
        assert_eq!(first_source_pixel(5, 4), 2);
    }

    #[test]
    fn the_header_lists_as_many_columns_as_a_row_writes() {
        let r = Record {
            item_id: 0,
            filter: "nearest".to_string(),
            compression: "png".to_string(),
            scale: 4,
            pixels: 256,
            exact: 1.0,
            exact_snapped: 1.0,
            median_delta_e: 0.0,
            colors_source: 13,
            colors_recovered: 13,
        };
        assert_eq!(HEADER.split(',').count(), r.to_csv().split(',').count());
    }
}
