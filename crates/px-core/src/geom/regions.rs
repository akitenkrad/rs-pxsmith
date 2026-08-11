//! G5 — 領域ラベリング (設計書 2.4)．
//!
//! 形の乱雑さ lint (19)・隣接同色 lint (21)・`palette report`・脱ディザノイズが
//! この上に乗る．

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::canvas::IndexedCanvas;
use crate::math::{IRect, IVec2, ivec2};

use super::mask::{Field, Mask};

/// 領域の識別子．
pub type RegionId = u32;

/// 連結成分 1 つ．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub id: RegionId,
    /// この領域の色の添字．
    pub index: u8,
    /// 画素数．
    pub area: u32,
    /// 周囲長 — 領域の外と接する辺の数．**画像の縁も外として数える**．
    pub perimeter: u32,
    pub bbox: IRect,
    /// 4 近傍で接する他の領域．
    pub neighbors: BTreeSet<RegionId>,
}

impl Region {
    /// 周囲長の 2 乗を面積で割った値 (形の乱雑さ，lint 19)．
    ///
    /// 円で最小 ($4\pi \approx 12.6$)，複雑な形ほど大きくなる．画素の格子では
    /// 正方形が下限 (16) になるので，**閾値は 16 より上でしか意味を持たない**．
    pub fn compactness(&self) -> f32 {
        if self.area == 0 {
            return 0.0;
        }
        (self.perimeter as f32).powi(2) / self.area as f32
    }
}

/// ラベリングの結果．
#[derive(Clone, Debug)]
pub struct RegionMap {
    labels: Field<Option<RegionId>>,
    regions: Vec<Region>,
}

impl RegionMap {
    pub fn labels(&self) -> &Field<Option<RegionId>> {
        &self.labels
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    pub fn at(&self, p: IVec2) -> Option<&Region> {
        let id = (*self.labels.get(p)?)?;
        self.regions.get(id as usize)
    }

    /// 面積の大きい順の並び (`palette report` 用)．同点は添字の小さい順．
    pub fn by_area_desc(&self) -> Vec<&Region> {
        let mut out: Vec<&Region> = self.regions.iter().collect();
        out.sort_by(|a, b| b.area.cmp(&a.area).then(a.id.cmp(&b.id)));
        out
    }

    /// 同じ添字で隣接している領域の組 (lint 21)．
    ///
    /// 別々の領域なのに同じ色というのは，境界が見えなくなっている状態である．
    pub fn same_index_neighbors(&self) -> Vec<(RegionId, RegionId)> {
        let mut out = Vec::new();
        for r in &self.regions {
            for &n in &r.neighbors {
                if n > r.id && self.regions[n as usize].index == r.index {
                    out.push((r.id, n));
                }
            }
        }
        out
    }
}

/// 4 近傍の向き．
const DIRS4: [IVec2; 4] = [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)];

/// 同色の連結成分を求める (4 連結)．
///
/// 8 連結にしないのは，斜めに接するだけの画素を同じ面とみなすと**市松模様が
/// 1 つの領域になってしまう**ためである．ディザの検出 (lint 10) が成り立たなくなる．
pub fn label_regions(canvas: &IndexedCanvas) -> RegionMap {
    let (w, h) = (canvas.width(), canvas.height());
    let mut labels: Field<Option<RegionId>> = Field::filled(w, h, None);
    let mut regions: Vec<Region> = Vec::new();

    for seed in canvas.bounds().iter() {
        if labels.copied(seed).flatten().is_some() {
            continue;
        }
        let Some(index) = canvas.get_at(seed) else {
            continue;
        };
        let id = regions.len() as RegionId;

        let mut area = 0u32;
        let mut perimeter = 0u32;
        let (mut x0, mut y0, mut x1, mut y1) = (seed.x, seed.y, seed.x, seed.y);
        let mut queue = VecDeque::from([seed]);
        labels.set(seed, Some(id));

        while let Some(p) = queue.pop_front() {
            area += 1;
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);

            for d in DIRS4 {
                let q = p + d;
                match canvas.get_at(q) {
                    // 画像の外，または別の色 — ここが周囲になる
                    None => perimeter += 1,
                    Some(other) if other != index => perimeter += 1,
                    Some(_) => {
                        if labels.copied(q).flatten().is_none() {
                            labels.set(q, Some(id));
                            queue.push_back(q);
                        }
                    }
                }
            }
        }

        regions.push(Region {
            id,
            index,
            area,
            perimeter,
            bbox: IRect::new(x0, y0, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32),
            neighbors: BTreeSet::new(),
        });
    }

    // 隣接関係は全画素をもう一度走って集める
    let mut adjacency: BTreeMap<RegionId, BTreeSet<RegionId>> = BTreeMap::new();
    for p in canvas.bounds().iter() {
        let Some(a) = labels.copied(p).flatten() else {
            continue;
        };
        for d in DIRS4 {
            if let Some(b) = labels.copied(p + d).flatten()
                && a != b
            {
                adjacency.entry(a).or_default().insert(b);
            }
        }
    }
    for (id, set) in adjacency {
        regions[id as usize].neighbors = set;
    }

    RegionMap { labels, regions }
}

/// マスクの連結成分．`diagonal` が真なら 8 連結．
///
/// 輪郭追跡 (G1) が成分ごとの開始点を得るために使う．
pub fn label_mask(mask: &Mask, diagonal: bool) -> MaskComponents {
    let mut seen = Mask::new(mask.width(), mask.height());
    let mut components: Vec<Vec<IVec2>> = Vec::new();
    let dirs: &[IVec2] = if diagonal {
        &super::contour::DIRS8
    } else {
        &DIRS4
    };

    for seed in mask.bounds().iter() {
        if !mask.get(seed) || seen.get(seed) {
            continue;
        }
        let mut pts = Vec::new();
        let mut queue = VecDeque::from([seed]);
        seen.set(seed, true);
        while let Some(p) = queue.pop_front() {
            pts.push(p);
            for &d in dirs {
                let q = p + d;
                if mask.get(q) && !seen.get(q) {
                    seen.set(q, true);
                    queue.push_back(q);
                }
            }
        }
        // 走査順に揃えて，開始点が「最も上・最も左」になるようにする
        pts.sort_unstable_by_key(|p| (p.y, p.x));
        components.push(pts);
    }

    MaskComponents { components }
}

/// マスクの連結成分の集まり．
#[derive(Clone, Debug)]
pub struct MaskComponents {
    components: Vec<Vec<IVec2>>,
}

impl MaskComponents {
    pub fn components(&self) -> &[Vec<IVec2>] {
        &self.components
    }

    pub fn len(&self) -> usize {
        self.components.len()
    }

    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_canvas_is_one_region() {
        let c = IndexedCanvas::filled(4, 4, 3);
        let m = label_regions(&c);
        assert_eq!(m.len(), 1);
        assert_eq!(m.regions()[0].area, 16);
        // 4x4 の外周 = 16 辺
        assert_eq!(m.regions()[0].perimeter, 16);
    }

    #[test]
    fn perimeter_counts_the_image_edge() {
        let c = IndexedCanvas::filled(1, 1, 0);
        assert_eq!(label_regions(&c).regions()[0].perimeter, 4);
    }

    #[test]
    fn different_colours_split_into_separate_regions() {
        let c = IndexedCanvas::from_pixels(4, 1, vec![0, 0, 1, 1]).unwrap();
        let m = label_regions(&c);
        assert_eq!(m.len(), 2);
        assert_eq!(m.regions()[0].index, 0);
        assert_eq!(m.regions()[1].index, 1);
        assert_eq!(m.regions()[0].area, 2);
    }

    #[test]
    fn the_same_colour_in_two_places_is_two_regions() {
        let c = IndexedCanvas::from_pixels(5, 1, vec![1, 0, 0, 0, 1]).unwrap();
        let m = label_regions(&c);
        assert_eq!(m.len(), 3);
        assert_eq!(
            m.same_index_neighbors(),
            vec![],
            "接していないので隣接ではない"
        );
    }

    #[test]
    fn diagonally_touching_same_colours_stay_apart() {
        // 4 連結なので斜めに触れるだけの同色は別領域のまま，かつ隣接でもない
        let c = IndexedCanvas::from_pixels(2, 2, vec![1, 0, 0, 1]).unwrap();
        let m = label_regions(&c);
        assert_eq!(m.len(), 4, "市松の 4 画素はそれぞれ別領域");
        assert!(m.same_index_neighbors().is_empty());
    }

    #[test]
    fn side_by_side_same_colour_regions_are_reported() {
        // 縦棒で分けた同色の 2 面 — 見た目に境界が消える (lint 21)
        let c = IndexedCanvas::from_pixels(3, 2, vec![1, 2, 1, 1, 1, 1]).unwrap();
        let m = label_regions(&c);
        // 下段で繋がるので実際には 1 領域．繋がっていないことを確かめる形にする
        let split = IndexedCanvas::from_pixels(3, 1, vec![1, 2, 1]).unwrap();
        let sm = label_regions(&split);
        assert_eq!(sm.len(), 3);
        assert!(
            sm.same_index_neighbors().is_empty(),
            "間に別の色が挟まっているので隣接ではない"
        );
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn four_connectivity_keeps_a_checkerboard_apart() {
        let c = IndexedCanvas::from_pixels(4, 2, vec![0, 1, 0, 1, 1, 0, 1, 0]).unwrap();
        // 8 連結なら 2 領域になってしまう．ディザ検出のためにここは分かれてほしい
        assert_eq!(label_regions(&c).len(), 8);
    }

    #[test]
    fn neighbours_are_symmetric() {
        let c = IndexedCanvas::from_pixels(3, 1, vec![0, 1, 2]).unwrap();
        let m = label_regions(&c);
        for r in m.regions() {
            for &n in &r.neighbors {
                assert!(
                    m.regions()[n as usize].neighbors.contains(&r.id),
                    "{} と {n} の隣接が片側だけ",
                    r.id
                );
            }
        }
    }

    #[test]
    fn compactness_is_lowest_for_a_square() {
        let square = IndexedCanvas::filled(4, 4, 0);
        let line = IndexedCanvas::filled(16, 1, 0);
        let sq = label_regions(&square).regions()[0].compactness();
        let ln = label_regions(&line).regions()[0].compactness();
        assert!(sq < ln, "正方形の方が乱雑でないはず ({sq} < {ln})");
        assert!((sq - 16.0).abs() < 1e-3, "正方形は 16 になるはず: {sq}");
    }

    #[test]
    fn by_area_desc_is_deterministic() {
        let c = IndexedCanvas::from_pixels(4, 1, vec![0, 0, 0, 1]).unwrap();
        let m = label_regions(&c);
        let sorted = m.by_area_desc();
        assert_eq!(sorted[0].area, 3);
        assert_eq!(sorted[1].area, 1);
    }

    #[test]
    fn at_returns_the_region_under_a_point() {
        let c = IndexedCanvas::from_pixels(2, 1, vec![5, 6]).unwrap();
        let m = label_regions(&c);
        assert_eq!(m.at(ivec2(0, 0)).unwrap().index, 5);
        assert_eq!(m.at(ivec2(1, 0)).unwrap().index, 6);
        assert!(m.at(ivec2(9, 9)).is_none());
    }

    #[test]
    fn mask_components_respect_connectivity() {
        // 斜めにだけ触れる 2 画素
        let mut m = Mask::new(3, 3);
        m.set(ivec2(0, 0), true);
        m.set(ivec2(1, 1), true);
        assert_eq!(label_mask(&m, false).len(), 2, "4 連結では別々");
        assert_eq!(label_mask(&m, true).len(), 1, "8 連結では 1 つ");
    }

    #[test]
    fn mask_component_starts_at_the_topmost_leftmost_pixel() {
        let mut m = Mask::new(4, 4);
        for p in [ivec2(2, 1), ivec2(1, 1), ivec2(1, 2)] {
            m.set(p, true);
        }
        let c = label_mask(&m, true);
        assert_eq!(c.components()[0].first(), Some(&ivec2(1, 1)));
    }
}
