//! パレットの面積レポート (`px palette report`．設計書 5 章 ・G5)．
//!
//! 参考書籍の第四章 «色のデザイン» は，**主な色を 2 〜 3 色に収める**ことを勧める．
//! この道具はそれを**処方せずに数える** — «何色が絵の大半を占めているか» と
//! «主な色どうしがどれだけ離れているか» を出すだけで，直し方は言わない
//! (D101 ・D107 と同じ側．入力で決まる量である) ．
//!
//! # «大半» をどの割合で読むかで答えが変わる
//!
//! 実素材 61 枚で数えると，覆う割合ごとに要る色数の中央値はこうなる．
//!
//! | 覆う割合 | 50% | 80% | 90% |
//! | --- | --- | --- | --- |
//! | 要る色数の中央 | **2** | 4 | 5 |
//!
//! **書籍の «2 〜 3 色» に合うのは «面積の半分» で読んだときである** — 80% で
//! 読むと 2 〜 3 色に収まる絵は 45.9% しかない．だから[`PaletteReport`]は
//! **1 つの数字に決めず 4 通りの割合を並べる**．どこで読むかは絵の狙いで決まる．
//!
//! # «面積上位色» には 2 通りの読み方がある
//!
//! 同じ添字の画素をぜんぶ足した量と，**1 つながりの塊として最大**の量は違う．
//! 前者は «その色をどれだけ使ったか»，後者は «その色がどれだけまとまって
//! 見えるか» である — ディザに使った色は前者では大きく，後者では小さい．
//! **両方出す**．片方だけだと，散らばった色を «主な色» と読み違える．
//!
//! # コントラストは «明度差» で測る
//!
//! 書籍が可読性の章で言っているのは «色が近すぎると形が読めない» ことなので，
//! 見るのは明度である — [`crate::color::delta_e`] は色相の差も混ぜてしまい，
//! **明度が同じで色相だけ違う 2 色**を «離れている» と数えてしまう．
//! 両方返して，どちらで見ているかを言う．

use std::collections::BTreeMap;

use crate::canvas::IndexedCanvas;
use crate::color::{Rgba8, delta_e};
use crate::geom::regions::label_regions;
use crate::palette::Palette;

/// 1 色ぶん．
#[derive(Clone, Debug, PartialEq)]
pub struct ColourArea {
    pub index: u8,
    pub colour: Rgba8,
    /// その添字の画素数の合計．
    pub area: u32,
    /// **1 つながりの塊として最大**の画素数．
    pub largest_region: u32,
    /// その添字が分かれている領域の数．
    pub regions: usize,
    /// 不透明な画素に対する割合．
    pub share: f32,
    /// OKLab の明度．
    pub lightness: f32,
}

/// 主な色どうしの隔たり．
#[derive(Clone, Debug, PartialEq)]
pub struct ContrastPair {
    pub a: u8,
    pub b: u8,
    /// 明度差 — **形が読めるかを見るのはこちら**．
    pub lightness: f32,
    /// OKLab の色距離 (明度 ・色相 ・彩度をまとめたもの)．
    pub delta_e: f32,
}

/// 報告．
#[derive(Clone, Debug)]
pub struct PaletteReport {
    /// 不透明な画素の数．
    pub opaque: usize,
    /// パレットの色数．
    pub palette_len: usize,
    /// **実際に使われた色数** — パレットに載っていても使っていない色がある．
    pub used: usize,
    /// 面積の大きい順．
    pub by_area: Vec<ColourArea>,
    /// 面積の割合が積み上がって各しきい値を超えるのに要った色数．
    ///
    /// 書籍の «主な色は 2 〜 3 色» を突き合わせる先である．
    pub colours_to_cover: BTreeMap<u32, usize>,
    /// 主な色どうしの隔たり (面積上位から総当たり)．
    pub contrast: Vec<ContrastPair>,
}

impl PaletteReport {
    /// 面積の `percent`% を覆うのに要る色数．
    pub fn cover(&self, percent: u32) -> usize {
        self.colours_to_cover.get(&percent).copied().unwrap_or(0)
    }

    /// **主な色どうしで最も近い組の明度差**．
    ///
    /// 小さいほど «形が読めない» 側である．主な色が 1 色しかなければ `None`．
    pub fn closest_main_lightness(&self) -> Option<f32> {
        self.contrast
            .iter()
            .map(|c| c.lightness)
            .fold(None, |acc: Option<f32>, l| {
                Some(acc.map_or(l, |m| m.min(l)))
            })
    }
}

/// 積み上げがしきい値を超えるのに要った色数を数える．
fn cover_counts(by_area: &[ColourArea], opaque: usize) -> BTreeMap<u32, usize> {
    let mut out = BTreeMap::new();
    for percent in [50u32, 80, 90, 95] {
        let target = opaque as f32 * percent as f32 / 100.0;
        let mut acc = 0f32;
        let mut n = 0usize;
        for c in by_area {
            if acc >= target {
                break;
            }
            acc += c.area as f32;
            n += 1;
        }
        out.insert(percent, n);
    }
    out
}

/// 面積レポートを作る．
///
/// `main_colours` は «主な色» として突き合わせる上位の数 (書籍は 2 〜 3 と言う)．
pub fn report(canvas: &IndexedCanvas, palette: &Palette, main_colours: usize) -> PaletteReport {
    let transparent = canvas.transparent();
    let regions = label_regions(canvas);

    // 添字ごとに «合計面積» と «最大の塊» を集める
    let mut total: BTreeMap<u8, u32> = BTreeMap::new();
    let mut largest: BTreeMap<u8, u32> = BTreeMap::new();
    let mut count: BTreeMap<u8, usize> = BTreeMap::new();
    for region in regions.regions() {
        if transparent == Some(region.index) {
            continue;
        }
        *total.entry(region.index).or_default() += region.area;
        let slot = largest.entry(region.index).or_default();
        *slot = (*slot).max(region.area);
        *count.entry(region.index).or_default() += 1;
    }

    let opaque: usize = total.values().map(|a| *a as usize).sum();
    let mut by_area: Vec<ColourArea> = total
        .iter()
        .map(|(index, area)| {
            let colour = palette.get(*index).unwrap_or(Rgba8::TRANSPARENT);
            ColourArea {
                index: *index,
                colour,
                area: *area,
                largest_region: largest.get(index).copied().unwrap_or(0),
                regions: count.get(index).copied().unwrap_or(0),
                share: if opaque == 0 {
                    0.0
                } else {
                    *area as f32 / opaque as f32
                },
                lightness: colour.to_oklab().l,
            }
        })
        .collect();
    // 面積の大きい順．同点は添字の小さい順 (並びが実行ごとに変わらないように)
    by_area.sort_by(|a, b| b.area.cmp(&a.area).then(a.index.cmp(&b.index)));

    // 主な色どうしの総当たり
    let main = by_area.iter().take(main_colours).collect::<Vec<_>>();
    let mut contrast = Vec::new();
    for (i, a) in main.iter().enumerate() {
        for b in main.iter().skip(i + 1) {
            contrast.push(ContrastPair {
                a: a.index,
                b: b.index,
                lightness: (a.lightness - b.lightness).abs(),
                delta_e: delta_e(a.colour.to_oklab(), b.colour.to_oklab()),
            });
        }
    }

    PaletteReport {
        opaque,
        palette_len: palette.len(),
        used: by_area.len(),
        colours_to_cover: cover_counts(&by_area, opaque),
        by_area,
        contrast,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0x1a, 0x1c, 0x2c),
            Rgba8::rgb(0xf4, 0xf4, 0xf4),
            Rgba8::rgb(0xb1, 0x3e, 0x53),
        ])
        .unwrap()
    }

    /// 左半分が添字 1，右半分の上が 2 ・下が 3 の 8x8．
    fn art() -> IndexedCanvas {
        let mut c = IndexedCanvas::filled(8, 8, 0);
        c.set_transparent(Some(0));
        for y in 0..8i32 {
            for x in 0..8i32 {
                let index = if x < 4 {
                    1
                } else if y < 6 {
                    2
                } else {
                    3
                };
                c.set(x, y, index);
            }
        }
        c
    }

    /// **壊れると: 面積の順が狂う (主な色が主でなくなる)．**
    #[test]
    fn colours_come_back_in_order_of_area() {
        let r = report(&art(), &palette(), 3);
        let order: Vec<u8> = r.by_area.iter().map(|c| c.index).collect();
        assert_eq!(order, vec![1, 2, 3], "面積の大きい順になっていない");
        assert_eq!(r.by_area[0].area, 32);
        assert_eq!(r.by_area[1].area, 24);
        assert_eq!(r.by_area[2].area, 8);
        assert_eq!(r.opaque, 64);
    }

    /// **壊れると: 透明を «色» として数える．**
    #[test]
    fn the_transparent_index_is_not_a_colour() {
        let mut c = IndexedCanvas::filled(8, 8, 0);
        c.set_transparent(Some(0));
        for x in 0..4i32 {
            c.set(x, 0, 1);
        }
        let r = report(&c, &palette(), 3);
        assert_eq!(r.used, 1, "透明を数えている");
        assert_eq!(r.opaque, 4);
        assert!(!r.by_area.iter().any(|a| a.index == 0));
    }

    /// **壊れると: 散らばった色を «主な色» と読み違える．**
    ///
    /// 同じ添字をぜんぶ足した量と，1 つながりの塊として最大の量は違う —
    /// 市松に撒いた色は前者では大きく，後者では 1 である．
    #[test]
    fn a_scattered_colour_has_a_large_area_but_a_tiny_largest_region() {
        let mut c = IndexedCanvas::filled(8, 8, 1);
        c.set_transparent(Some(0));
        for y in 0..8i32 {
            for x in 0..8i32 {
                if (x + y) % 2 == 0 {
                    c.set(x, y, 2);
                }
            }
        }
        let r = report(&c, &palette(), 2);
        let dither = r.by_area.iter().find(|a| a.index == 2).expect("在る");
        assert_eq!(dither.area, 32, "撒いた量が合わない");
        assert_eq!(dither.largest_region, 1, "市松の塊は 1 画素のはず");
        assert_eq!(dither.regions, 32, "領域の数が合わない");
    }

    /// **壊れると: «何色で大半を占めるか» の数え方が狂う．**
    #[test]
    fn covering_counts_say_how_many_colours_carry_the_picture() {
        let r = report(&art(), &palette(), 3);
        // 32 / 64 = 50% ちょうどなので 1 色で 50% に届く
        assert_eq!(r.cover(50), 1);
        // 32 + 24 = 56 / 64 = 87.5% なので 80% は 2 色
        assert_eq!(r.cover(80), 2);
        // 90% には 3 色要る
        assert_eq!(r.cover(90), 3);
    }

    /// **壊れると: 明度が同じで色相だけ違う 2 色を «離れている» と数える．**
    ///
    /// 書籍が可読性の章で問うているのは «形が読めるか» なので，見るのは明度である．
    #[test]
    fn contrast_reports_lightness_apart_from_overall_colour_distance() {
        // 明度がほぼ同じで色相が違う 2 色
        let pal = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0xc0, 0x40, 0x40),
            Rgba8::rgb(0x00, 0x82, 0x6e),
        ])
        .unwrap();
        let mut c = IndexedCanvas::filled(8, 8, 1);
        c.set_transparent(Some(0));
        for y in 0..4i32 {
            for x in 0..8i32 {
                c.set(x, y, 2);
            }
        }
        let r = report(&c, &pal, 2);
        let pair = &r.contrast[0];
        assert!(
            pair.lightness < 0.10,
            "明度差が {} と出た (近いはず)",
            pair.lightness
        );
        assert!(
            pair.delta_e > pair.lightness,
            "色距離が明度差より小さい ({} 対 {})",
            pair.delta_e,
            pair.lightness
        );
    }

    /// **壊れると: 使っていない色を «使った色» として数える．**
    #[test]
    fn unused_palette_entries_are_not_counted_as_used() {
        let r = report(&art(), &palette(), 3);
        assert_eq!(r.palette_len, 4);
        assert_eq!(r.used, 3, "使っていない色まで数えている");
    }

    /// **壊れると: 主な色が 1 色しかないときに «最も近い組» を作ってしまう．**
    #[test]
    fn a_single_main_colour_has_no_pair_to_compare() {
        let mut c = IndexedCanvas::filled(8, 8, 1);
        c.set_transparent(Some(0));
        let r = report(&c, &palette(), 3);
        assert_eq!(r.used, 1);
        assert!(r.contrast.is_empty());
        assert_eq!(r.closest_main_lightness(), None);
    }
}
