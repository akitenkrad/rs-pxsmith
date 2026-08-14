//! **ルール 19 ・20 ・21 の閾値を測る** (`px-calib lint-shape`)．
//!
//! 付録 C 要調査事項 #2 «形の乱雑さ (周囲長/面積比) の閾値» を閉じるための口である．
//!
//! # 良い絵に掛けて何が鳴るかを先に見る (D70)
//!
//! 設計書の «周囲長/面積比» をそのまま $P^2/A$ で取ると，**良い絵の 93.8% が
//! 鳴る** — 閾値を校正しても意味が無い状態である．原因は «乱雑さ» ではなく
//! **細さ**を測っていること: ドット絵の陰影の帯は 1 画素幅が普通なので，
//! 乱れていなくても $P^2/A$ が大きくなる．
//!
//! そこで**外接矩形の周囲長で割った量**を並べて測る (`boundary_excess`) ．

use anyhow::Result;
use std::path::Path;

use px_core::canvas::IndexedCanvas;
use px_core::geom::regions::label_regions;
use px_core::palette::Palette;

use crate::animcal::{indexed, name_of, png_files};
use crate::rng::Rng;

/// 領域 1 つぶんの測定値．
pub struct ShapeRow {
    pub file: String,
    pub group: &'static str,
    pub index: u8,
    pub area: u32,
    pub compactness: f32,
    pub excess: f32,
}

pub const SHAPE_HEADER: &str = "file,group,index,area,compactness,excess";

pub fn shape_csv(r: &ShapeRow) -> String {
    format!(
        "{},{},{},{},{:.3},{:.3}",
        r.file, r.group, r.index, r.area, r.compactness, r.excess
    )
}

fn rows_of(
    file: &str,
    group: &'static str,
    canvas: &IndexedCanvas,
    min_area: u32,
) -> Vec<ShapeRow> {
    label_regions(canvas)
        .regions()
        .iter()
        .filter(|r| r.area >= min_area && canvas.transparent() != Some(r.index))
        .map(|r| ShapeRow {
            file: file.to_string(),
            group,
            index: r.index,
            area: r.area,
            compactness: r.compactness(),
            excess: r.boundary_excess(),
        })
        .collect()
}

/// **縁を荒らす** — 面積をおおむね変えずに輪郭だけをでこぼこにする．
///
/// > [!warning] **片側だけ削ると «滑らかになる»．**
/// > 最初は境界画素を隣の色へ置き換えるだけにしていたが，それは領域を
/// > **侵食して単純にする**操作である — 実測で荒らした側の方が良い絵より
/// > $P^2/A$ が小さくなった (中央 40.50 対 44.00) ．負例になっていない．
/// > **出す側と入れる側の両方を触る**こと．
fn roughen(canvas: &IndexedCanvas, palette: &Palette, seed: u64) -> Option<IndexedCanvas> {
    let map = label_regions(canvas);
    let target = map
        .regions()
        .iter()
        .filter(|r| canvas.transparent() != Some(r.index))
        .max_by_key(|r| r.area)?;
    let other = map
        .regions()
        .iter()
        .find(|r| r.index != target.index && canvas.transparent() != Some(r.index))?
        .index;
    let _ = palette;

    let mut out = canvas.clone();
    let mut rng = Rng::new(seed);
    let mut touched = 0usize;
    let inside = |p: px_core::math::IVec2| map.at(p).map(|r| r.id) == Some(target.id);
    let neighbours = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    for p in target.bbox.iter() {
        let here = canvas.get(p.x, p.y);
        let touches = neighbours
            .iter()
            .any(|(dx, dy)| inside(px_core::math::ivec2(p.x + dx, p.y + dy)));
        if inside(p) {
            // **出す** — 縁の画素を隣の色にする
            if touches && rng.below(2) == 0 {
                out.set(p.x, p.y, other);
                touched += 1;
            }
        } else if here == Some(other) && touches && rng.below(2) == 0 {
            // **入れる** — 隣の色の画素を領域の色にする
            out.set(p.x, p.y, target.index);
            touched += 1;
        }
    }
    (touched > 0).then_some(out)
}

/// シルエット (不透明な画素) を 1 つの形として測る．
///
/// 領域ごとに測ると，荒らしたときに**領域が分裂して個々は単純になる**ので，
/// 欠陥が «形の乱雑さ» として現れない．シルエットなら分裂しても 1 つの形の
/// ままである．
fn silhouette_row(
    file: &str,
    group: &'static str,
    canvas: &IndexedCanvas,
    palette: &Palette,
) -> Option<ShapeRow> {
    use px_core::geom::{Mask, regions::label_mask};
    let mut m = Mask::new(canvas.width(), canvas.height());
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            let Some(i) = canvas.get(x, y) else { continue };
            if canvas.transparent() == Some(i) || palette.get(i).is_some_and(|c| c.a == 0) {
                continue;
            }
            m.set(px_core::math::ivec2(x, y), true);
        }
    }
    if m.is_empty() {
        return None;
    }
    // 一番大きい連結成分を 1 つの形として測る
    let comps = label_mask(&m, false);
    let biggest = comps.components().iter().max_by_key(|c| c.len())?;
    let mut only = Mask::new(m.width(), m.height());
    for p in biggest {
        only.set(*p, true);
    }
    let bbox = only.bbox()?;
    let area = only.count() as u32;
    let mut perimeter = 0u32;
    for p in only.iter_set() {
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            if !only.get(px_core::math::ivec2(p.x + dx, p.y + dy)) {
                perimeter += 1;
            }
        }
    }
    let box_p = 2 * (bbox.w + bbox.h);
    Some(ShapeRow {
        file: file.to_string(),
        group,
        index: 0,
        area,
        compactness: (perimeter as f32).powi(2) / area.max(1) as f32,
        excess: perimeter as f32 / box_p.max(1) as f32,
    })
}

/// **シルエットの縁を荒らす** — 不透明と透明を境界で入れ替える．
///
/// 色どうしを入れ替える [`roughen`] ではシルエットが 1 画素も変わらないので，
/// «形の乱雑さ» を測る負例にならない (実測で影の分布が完全に一致した) ．
fn roughen_silhouette(
    canvas: &IndexedCanvas,
    palette: &Palette,
    seed: u64,
) -> Option<IndexedCanvas> {
    let transparent = canvas.transparent()?;
    let opaque = (0..=255u8).find(|i| {
        *i != transparent
            && canvas.pixels().contains(i)
            && palette.get(*i).is_some_and(|c| c.a != 0)
    })?;
    let is_opaque = |x: i32, y: i32| canvas.get(x, y).is_some_and(|i| i != transparent);

    let mut out = canvas.clone();
    let mut rng = Rng::new(seed);
    let mut touched = 0usize;
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            let touches = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|(dx, dy)| is_opaque(x + dx, y + dy));
            if is_opaque(x, y) {
                if touches && !is_opaque(x, y) {
                    continue;
                }
                let edge = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| !is_opaque(x + dx, y + dy));
                if edge && rng.below(2) == 0 {
                    out.set(x, y, transparent);
                    touched += 1;
                }
            } else if touches && rng.below(2) == 0 {
                out.set(x, y, opaque);
                touched += 1;
            }
        }
    }
    (touched > 0).then_some(out)
}

/// 実素材と «縁を荒らした» 負例を測る．
pub fn build(dir: &Path, min_area: u32) -> Result<(Vec<ShapeRow>, usize)> {
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for path in png_files(dir)? {
        let file = name_of(&path);
        let Some((canvas, palette)) = indexed(&path) else {
            skipped += 1;
            continue;
        };
        out.extend(rows_of(&file, "good", &canvas, min_area));
        if let Some(r) = silhouette_row(&file, "sil-good", &canvas, &palette) {
            out.push(r);
        }
        match roughen(&canvas, &palette, 0) {
            Some(rough) => {
                out.extend(rows_of(&file, "rough", &rough, min_area));
            }
            None => skipped += 1,
        }
        if let Some(rough) = roughen_silhouette(&canvas, &palette, 0)
            && let Some(r) = silhouette_row(&file, "sil-rough", &rough, &palette)
        {
            out.push(r);
        }
    }
    Ok((out, skipped))
}

/// 閾値を掃く．返り値は (値, 良い絵で鳴った領域, 良い絵の領域数, 負例で鳴った領域, 負例の領域数)．
pub fn sweep(
    rows: &[ShapeRow],
    values: &[f32],
    use_excess: bool,
    good: &str,
    bad: &str,
) -> Vec<(f32, usize, usize, usize, usize)> {
    values
        .iter()
        .map(|&v| {
            let pick = |r: &ShapeRow| if use_excess { r.excess } else { r.compactness };
            let count = |g: &str| -> (usize, usize) {
                let set: Vec<&ShapeRow> = rows.iter().filter(|r| r.group == g).collect();
                (set.iter().filter(|r| pick(r) > v).count(), set.len())
            };
            let (gf, gn) = count(good);
            let (rf, rn) = count(bad);
            (v, gf, gn, rf, rn)
        })
        .collect()
}

// ------------------------------------------------- ルール 20 ・21 の負例

/// **同化させる** — 斜めに接する 2 領域を同じ添字にする (ルール 21 の負例)．
///
/// 書籍の «後ろ姿は髪が紺色でヘッドバンドと同化しており判断できなかった» を
/// そのまま作る [^pl6]．
///
/// [^pl6]: Pixel Logic 第四章 可読性 (PAGE:108)．
pub fn merge_colours(canvas: &IndexedCanvas, min_area: u32) -> Option<IndexedCanvas> {
    let map = label_regions(canvas);
    // **角でだけ触れている組を選ぶ．** 辺で接する組を同じ色にすると，塗り
    // つぶしで 1 領域に併合されて «同色の隣接» が消えてしまう (最初これで
    // 負例が良い絵と区別できなかった)
    let (a, b) = map.corner_touching().into_iter().find(|(a, b)| {
        let (ra, rb) = (&map.regions()[*a as usize], &map.regions()[*b as usize]);
        ra.index != rb.index
            && ra.area >= min_area
            && rb.area >= min_area
            && canvas.transparent() != Some(ra.index)
            && canvas.transparent() != Some(rb.index)
    })?;
    let (ra, rb) = (&map.regions()[a as usize], &map.regions()[b as usize]);
    let mut out = canvas.clone();
    for p in rb.bbox.iter() {
        if map.at(p).map(|r| r.id) == Some(rb.id) {
            out.set(p.x, p.y, ra.index);
        }
    }
    Some(out)
}

/// **角で触れさせる** — 別の色の四角を，大きい領域の角に 1 点だけ接して置く
/// (ルール 20 の負例)．
///
/// **画布の中に収まる置き場所を探す**こと — 外接矩形の外へ置くと画布からはみ出て
/// 1 画素も置けない (最初これで 61 枚中 3 枚しか負例が作れなかった) ．
pub fn add_tangent(canvas: &IndexedCanvas, palette: &Palette, size: i32) -> Option<IndexedCanvas> {
    let transparent = canvas.transparent()?;
    let map = label_regions(canvas);
    let target = map
        .regions()
        .iter()
        .filter(|r| canvas.transparent() != Some(r.index))
        .max_by_key(|r| r.area)?;
    let ink = (0..=255u8).find(|i| {
        *i != transparent
            && *i != target.index
            && palette.get(*i).is_some_and(|c| c.a != 0)
            && canvas.pixels().contains(i)
    })?;
    let belongs =
        |x: i32, y: i32| map.at(px_core::math::ivec2(x, y)).map(|r| r.id) == Some(target.id);
    let free = |x: i32, y: i32| canvas.get(x, y) == Some(transparent);

    for p in target.bbox.iter() {
        if !belongs(p.x, p.y) {
            continue;
        }
        // 右下へ 1 画素ずらした位置に四角を置く．**辺では触れないこと**
        let (ox, oy) = (p.x + 1, p.y + 1);
        if belongs(ox, p.y) || belongs(p.x, oy) {
            continue;
        }
        if !(0..size).all(|dy| (0..size).all(|dx| free(ox + dx, oy + dy))) {
            continue;
        }
        let mut out = canvas.clone();
        for dy in 0..size {
            for dx in 0..size {
                out.set(ox + dx, oy + dy, ink);
            }
        }
        return Some(out);
    }
    None
}

/// 分布の要約 (中央値と上側の分位点)．
pub fn quantiles(rows: &[ShapeRow], group: &str, use_excess: bool) -> (f32, f32, f32, usize) {
    let mut v: Vec<f32> = rows
        .iter()
        .filter(|r| r.group == group)
        .map(|r| if use_excess { r.excess } else { r.compactness })
        .collect();
    v.sort_by(f32::total_cmp);
    let at = |q: f32| -> f32 {
        if v.is_empty() {
            return 0.0;
        }
        v[((v.len() as f32 - 1.0) * q).round() as usize]
    };
    (at(0.5), at(0.9), at(0.99), v.len())
}
