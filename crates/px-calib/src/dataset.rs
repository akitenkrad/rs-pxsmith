//! 評価データセット (合成 500 件) の生成と目録．
//!
//! | 項目 | 値 |
//! | --- | --- |
//! | 件数 | 500 (実装計画書 M2 で確定) |
//! | 分割 | 検証 60% (300) / テスト 40% (200) |
//! | 正解 | $(s, d_x, d_y)$．リサイズありは整数の格子が無いので位相は持たない |
//!
//! **検証セットで閾値を決め，テストセットでの再調整はしない．** 分割は id から
//! 決まるので，何度生成しても同じ件が同じ側に入る．
//!
//! 劣化条件は 6 (拡大率) x 4 (補間) x 3 (リサイズ) x 4 (圧縮) = 288 通りある．
//! 500 件を id の剰余で割り当てるので，各条件は 1 回か 2 回ずつ現れる (288 と 500 の
//! 最大公約数は 4 なので偏りは 1 件以内に収まる)．元絵と位相は件ごとに引き直す．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::degrade::{COMPRESSIONS, Degradation, FILTERS, RESIZES, Resize, SCALES};
use crate::rng::Rng;
use crate::sprite;

/// 既定の件数．
pub const TOTAL: u32 = 500;

/// 劣化条件の総数 (完全実施要因計画の格子)．
pub const CELLS: u32 =
    SCALES.len() as u32 * FILTERS.len() as u32 * RESIZES.len() as u32 * COMPRESSIONS.len() as u32;

/// 目録のファイル名．
pub const MANIFEST: &str = "manifest.json";

/// 分割．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Split {
    Validation,
    Test,
}

impl Split {
    /// id から分割を決める．**5 件に 3 件を検証へ**回すと 500 件で 300 / 200 になる．
    /// 劣化条件は 288 周期なので，5 周期と互いに素であり条件が片側へ寄らない．
    pub fn of(id: u32) -> Self {
        if id % 5 < 3 {
            Self::Validation
        } else {
            Self::Test
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Test => "test",
        }
    }
}

/// 評価データ 1 件．
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: u32,
    /// 目録から見た画像の相対パス．
    pub file: String,
    /// 元絵を作った種．
    pub source_seed: u64,
    /// 実物のドット絵を種にした場合のファイル名．合成なら `None`．
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    /// 元絵の大きさ．
    pub source_size: (u32, u32),
    pub degradation: Degradation,
    /// 正解のスケール．
    pub truth_scale: u32,
    /// 正解の位相．リサイズありは `None`．
    pub truth_phase: Option<(u32, u32)>,
    /// リサイズ後の実効的な周期 $s \cdot r$．
    pub effective_scale: f32,
    pub split: Split,
}

impl Item {
    /// 整数の格子が存在する件か (リサイズなし)．完全一致率はこの部分集合で測る．
    pub fn has_integer_grid(&self) -> bool {
        self.degradation.resize == Resize::Keep
    }
}

/// 評価データセットの目録．
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub seed: u64,
    pub count: u32,
    pub items: Vec<Item>,
}

/// 目録を組み立てる (画像はまだ書かない)．
///
/// `seeds` が空なら元絵を合成する．**実物のドット絵を渡せるようにしてある** —
/// 合成の元絵は平坦な 13 色で，手打ちアンチエイリアス ・ディザ ・1 画素の輪郭線が
/// 無い．セルの中が最初から平坦なので，補間で崩れたときの振る舞いが実物と違い，
/// 閾値の最良点が実データと逆を向く原因になっていた (調査記録) ．
pub fn build(seed: u64, count: u32, seeds: &[sprite::Seed]) -> Manifest {
    let mut items = Vec::with_capacity(count as usize);
    for id in 0..count {
        // 件ごとに種を分ける．splitmix64 は種の分配にそのまま使える
        let mut rng = Rng::new(seed ^ (u64::from(id).wrapping_mul(0x9e37_79b9_7f4a_7c15)));
        let source_seed = rng.next_u64();
        let (source_file, source) = source_of(seeds, source_seed);

        let cell = id % CELLS;
        let scale = SCALES[(cell % SCALES.len() as u32) as usize];
        let filter = FILTERS[((cell / SCALES.len() as u32) % FILTERS.len() as u32) as usize];
        let resize = RESIZES
            [((cell / (SCALES.len() * FILTERS.len()) as u32) % RESIZES.len() as u32) as usize];
        let compression = COMPRESSIONS[((cell
            / (SCALES.len() * FILTERS.len() * RESIZES.len()) as u32)
            % COMPRESSIONS.len() as u32) as usize];

        let degradation = Degradation {
            scale,
            filter,
            resize,
            compression,
            crop: (rng.below(scale), rng.below(scale)),
        };

        items.push(Item {
            id,
            file: format!("images/{id:04}.png"),
            source_seed,
            source_file,
            source_size: (source.width(), source.height()),
            truth_scale: scale,
            truth_phase: degradation.truth_phase(),
            effective_scale: degradation.effective_scale(),
            degradation,
            split: Split::of(id),
        });
    }
    Manifest { seed, count, items }
}

/// 目録どおりに画像を書き出し，目録そのものも保存する．
/// 元絵を 1 枚用意する．**種の選び方は `source_seed` だけで決まる** — 目録から
/// 同じ絵を作り直せる．
fn source_of(seeds: &[sprite::Seed], source_seed: u64) -> (Option<String>, px_core::RgbaCanvas) {
    if seeds.is_empty() {
        (None, sprite::synthesize(source_seed))
    } else {
        let (name, art) = sprite::from_seed(seeds, source_seed);
        (Some(name), art)
    }
}

pub fn generate(dir: &Path, manifest: &Manifest, seeds: &[sprite::Seed]) -> Result<()> {
    std::fs::create_dir_all(dir.join("images"))
        .with_context(|| format!("{} を作れない", dir.display()))?;

    for item in &manifest.items {
        let (name, source) = source_of(seeds, item.source_seed);
        // 目録と食い違ったら黙って別のデータセットを書くことになる
        anyhow::ensure!(
            name == item.source_file,
            "{} の元絵が目録と違う (目録 {:?} / いま {:?})．種の並びが変わっている",
            item.file,
            item.source_file,
            name
        );
        let degraded = item.degradation.apply(&source)?;
        px_io::png::write_rgba(dir.join(&item.file), &degraded)
            .with_context(|| format!("{} を書けない", item.file))?;
    }

    let json = serde_json::to_string_pretty(manifest)?;
    px_io::atomic::write(dir.join(MANIFEST), json.as_bytes())
        .with_context(|| format!("{} を書けない", MANIFEST))?;
    Ok(())
}

/// 目録を読む．
pub fn read(dir: &Path) -> Result<Manifest> {
    let path: PathBuf = dir.join(MANIFEST);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("{} が無い．先に gen を実行すること", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn the_split_is_three_hundred_and_two_hundred() {
        let m = build(0, TOTAL, &[]);
        let v = m
            .items
            .iter()
            .filter(|i| i.split == Split::Validation)
            .count();
        let t = m.items.iter().filter(|i| i.split == Split::Test).count();
        assert_eq!((v, t), (300, 200), "検証 60% / テスト 40% になっていない");
    }

    #[test]
    fn every_degradation_cell_is_covered() {
        let m = build(0, TOTAL, &[]);
        let cells: BTreeSet<(u32, &str, &str, &str)> = m
            .items
            .iter()
            .map(|i| {
                (
                    i.degradation.scale,
                    i.degradation.filter.as_str(),
                    i.degradation.resize.as_str(),
                    i.degradation.compression.as_str(),
                )
            })
            .collect();
        assert_eq!(
            cells.len(),
            CELLS as usize,
            "劣化条件の格子に穴がある ({} / {CELLS})",
            cells.len()
        );
    }

    #[test]
    fn each_cell_appears_at_most_twice() {
        let m = build(0, TOTAL, &[]);
        for cell in 0..CELLS {
            let n = m.items.iter().filter(|i| i.id % CELLS == cell).count();
            assert!((1..=2).contains(&n), "条件 {cell} が {n} 件ある");
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_manifest() {
        assert_eq!(build(31, 40, &[]), build(31, 40, &[]));
    }

    #[test]
    fn a_different_seed_changes_the_sources_but_not_the_conditions() {
        let a = build(1, 40, &[]);
        let b = build(2, 40, &[]);
        assert_ne!(a.items[0].source_seed, b.items[0].source_seed);
        // 劣化条件は id から決まるので種に依らない
        assert_eq!(a.items[0].degradation.scale, b.items[0].degradation.scale);
    }

    #[test]
    fn only_the_unresized_items_carry_a_phase_truth() {
        for item in build(3, TOTAL, &[]).items {
            assert_eq!(
                item.truth_phase.is_some(),
                item.has_integer_grid(),
                "id {} の正解の持ち方が条件と食い違う",
                item.id
            );
        }
    }

    #[test]
    fn the_crop_stays_inside_one_cell() {
        for item in build(4, TOTAL, &[]).items {
            let s = item.degradation.scale;
            assert!(item.degradation.crop.0 < s && item.degradation.crop.1 < s);
        }
    }
}
