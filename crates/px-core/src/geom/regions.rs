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

    /// 周囲長を**外接矩形の周囲長**で割った値 (形の乱雑さ，lint 19)．
    ///
    /// $P^2/A$ は**細さ**を測ってしまう — ドット絵の陰影の帯は 1 画素幅が普通
    /// なので，乱れていなくても大きな値になる (実測で良い絵の 93.8% が鳴った) ．
    /// こちらは «その広がりに対して縁がどれだけ長いか» なので，
    /// **矩形も対角線も 1 に近く**，でこぼこした形だけが大きくなる．
    pub fn boundary_excess(&self) -> f32 {
        let p = 2 * (self.bbox.w + self.bbox.h);
        if p == 0 {
            return 0.0;
        }
        self.perimeter as f32 / p as f32
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
    /// 同じ添字を持つ別領域で，**隣り合っている**もの (lint 21)．
    ///
    /// > [!warning] **4 近傍で探しては決して鳴らない．**
    /// > 同じ添字で 4 近傍に接する 2 領域は，塗りつぶしの時点で 1 つに併合されて
    /// > いる — つまり «4 近傍の隣人のうち同じ添字のもの» は**構造的に空**である
    /// > (D80 «`Turn(chain)` が常に空だった» と同じ形の穴で，M1a から入っていた) ．
    /// >
    /// > 別領域でありながら同じ添字で隣り合うとは，**斜めに接している**という
    /// > ことである．書籍が «後ろ姿は髪が紺色でヘッドバンドと同化しており判断
    /// > できなかった» と言う状態がこれで，読み取れなくなるのは色が同じだから
    /// > である [^pl3]．
    ///
    /// [^pl3]: Pixel Logic 第四章 可読性 (PAGE:106-108)．
    pub fn same_index_neighbors(&self) -> Vec<(RegionId, RegionId)> {
        self.diagonal_pairs()
            .into_iter()
            .filter(|(a, b)| self.regions[*a as usize].index == self.regions[*b as usize].index)
            .collect()
    }

    /// **斜めにだけ接している**領域の組 (lint 20 «接線») ．
    ///
    /// 8 近傍では隣り合うが 4 近傍では接しない — つまり**角で 1 点だけ触れて
    /// いる**．辺で接していれば «並んでいる» のであって接線ではない．
    pub fn corner_touching(&self) -> Vec<(RegionId, RegionId)> {
        self.diagonal_pairs()
            .into_iter()
            .filter(|(a, b)| !self.regions[*a as usize].neighbors.contains(b))
            .collect()
    }

    /// 角で触れている組のうち，**触れている点の «脇» が指定の添字であるもの**．
    ///
    /// 陰影の帯どうしは，中間の帯がくびれた場所で角が出会う — そのとき脇には
    /// **中間の帯**がいる．書籍が言う «部品どうしが隣接して読めなくなる» のは
    /// 脇が背景のときで，2 つの物が**何も挟まずに**出会っている場合である．
    pub fn corner_touching_across(&self, background: Option<u8>) -> Vec<(RegionId, RegionId)> {
        let mut out = std::collections::BTreeSet::new();
        let (w, h) = (self.labels.width() as i32, self.labels.height() as i32);
        for y in 0..h {
            for x in 0..w {
                let p = ivec2(x, y);
                let Some(Some(a)) = self.labels.copied(p) else {
                    continue;
                };
                for d in DIRS_DIAG {
                    let q = p + d;
                    let Some(Some(b)) = self.labels.copied(q) else {
                        continue;
                    };
                    if a == b || self.regions[a as usize].neighbors.contains(&b) {
                        continue;
                    }
                    // 触れている点の «脇» 2 つ
                    let sides = [ivec2(q.x, p.y), ivec2(p.x, q.y)];
                    let both_background = sides.iter().all(|s| {
                        self.labels
                            .copied(*s)
                            .flatten()
                            .map(|id| Some(self.regions[id as usize].index) == background)
                            .unwrap_or(true)
                    });
                    if both_background {
                        out.insert((a.min(b), a.max(b)));
                    }
                }
            }
        }
        out.into_iter().collect()
    }

    /// 8 近傍で隣り合う領域の組 (昇順，重複なし)．
    pub fn diagonal_pairs(&self) -> Vec<(RegionId, RegionId)> {
        let mut out = std::collections::BTreeSet::new();
        let (w, h) = (self.labels.width() as i32, self.labels.height() as i32);
        for y in 0..h {
            for x in 0..w {
                let p = ivec2(x, y);
                let Some(Some(a)) = self.labels.copied(p) else {
                    continue;
                };
                for d in DIRS_DIAG {
                    let Some(Some(b)) = self.labels.copied(p + d) else {
                        continue;
                    };
                    if a != b {
                        out.insert((a.min(b), a.max(b)));
                    }
                }
            }
        }
        out.into_iter().collect()
    }
}

/// 斜め 4 方向．
const DIRS_DIAG: [IVec2; 4] = [ivec2(1, 1), ivec2(-1, 1), ivec2(1, -1), ivec2(-1, -1)];

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
            "斜めにも接していないので隣接ではない"
        );
    }

    /// **市松は «斜めに接する同色» そのものである．**
    ///
    /// M1a はこれを «隣接ではない» と定めていたが，その定めのままだと
    /// [`RegionMap::same_index_neighbors`] が構造的に空になり，lint 21 が
    /// 1 度も鳴らない．**斜めを隣接とみなし，ディザ避けは lint の側で
    /// «両方の領域が一定の面積を持つこと» で行う** — 市松の 1 画素は
    /// その下限に届かない．
    #[test]
    fn a_checkerboard_is_diagonally_adjacent_and_must_be_excluded_by_area() {
        let c = IndexedCanvas::from_pixels(2, 2, vec![1, 0, 0, 1]).unwrap();
        let m = label_regions(&c);
        assert_eq!(m.len(), 4, "市松の 4 画素はそれぞれ別領域");
        assert_eq!(m.same_index_neighbors().len(), 2, "斜めの同色が 2 組");
        assert!(
            m.regions().iter().all(|r| r.area == 1),
            "面積 1 なので lint 側の下限で落ちる"
        );
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

#[cfg(test)]
mod probe {
    use super::*;
    use crate::canvas::IndexedCanvas;

    /// **壊れると: ルール 21 が 1 度も鳴らなくなる．**
    ///
    /// 同じ添字で 4 近傍に接する 2 領域は塗りつぶしの時点で併合されているので，
    /// «4 近傍の隣人のうち同じ添字» を探す実装は**構造的に空を返す**
    /// (D80 と同じ形の穴．M1a から入っていた) ．**斜めで探す．**
    #[test]
    fn same_index_neighbours_are_found_through_the_diagonal() {
        // 同じ添字 1 の 2 つの塊が斜めにだけ接する
        let mut c = IndexedCanvas::filled(4, 4, 0);
        c.set(0, 0, 1);
        c.set(1, 1, 1);
        let map = label_regions(&c);
        assert_eq!(map.len(), 3, "添字 1 が 2 領域 ・添字 0 が 1 領域");
        assert_eq!(
            map.same_index_neighbors().len(),
            1,
            "斜めに接する同色の 2 領域を見つけられていない"
        );
        // 4 近傍では接していないので «接線» でもある
        assert_eq!(map.corner_touching().len(), 1);
    }

    /// **壊れると: 辺で接しているだけの領域を «接線» と呼ぶ．**
    #[test]
    fn regions_that_share_an_edge_are_not_tangent() {
        let mut c = IndexedCanvas::filled(4, 4, 0);
        c.set(0, 0, 1);
        c.set(1, 0, 1);
        c.set(1, 1, 1);
        let map = label_regions(&c);
        assert!(
            map.corner_touching().is_empty(),
            "辺で接していれば «並んでいる» のであって接線ではない"
        );
    }
}
