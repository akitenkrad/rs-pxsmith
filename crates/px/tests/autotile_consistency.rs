//! **autotile の «数え上げ» と «自己整合性» を固定する** (設計書 6.8 ・4.3)．
//!
//! 押さえるのは 3 つ．
//!
//! 1. **47 という数**．設計書 6.8 が挙げている数え上げの結果であって調整できない
//! 2. **自動ミラーで組んだタイルにルール 7 が掛かること** (設計書 4.3)
//! 3. **掛からないタイルの理由が «見逃し» ではないこと**
//!
//! > [!warning] **ルール 7 は象限ではなくタイルに掛ける** (D100) ．
//! > 16 画素のタイルの象限は 8x8 で，勾配を測れる画素は上限 $6 \times 6 = 36$ しか
//! > 無く `shading_min_pixels` (既定 64) に**構造的に届かない**．

use std::collections::BTreeMap;

use px_core::autotile::{
    CornerState, QUADRANTS, Quadrant, STATES, blob_masks, build, canonical_mask, corner_state,
    mirror_to_all_quadrants,
};
use px_core::canvas::IndexedCanvas;
use px_core::color::Rgba8;
use px_core::math::Vec2;
use px_core::palette::Palette;
use px_core::ramp::LightSource;

/// 斜めの平行光源．横成分が 0.474 を超えているので反転で矛盾が出る (D96)．
fn diagonal() -> LightSource {
    LightSource::Directional {
        dir: Vec2 { x: 0.6, y: -0.8 },
    }
}

/// 5 段の明度ランプ．
fn palette() -> Palette {
    Palette::new(vec![
        Rgba8::rgb(0x1a, 0x1c, 0x2c),
        Rgba8::rgb(0x3f, 0x4f, 0x6f),
        Rgba8::rgb(0x6f, 0x8f, 0x5f),
        Rgba8::rgb(0x9f, 0xbf, 0x7f),
        Rgba8::rgb(0xcf, 0xe0, 0x8f),
    ])
    .expect("5 色")
}

/// `NW` 象限の絵．**左と上が «外»** で，光は左上から当たる想定にする．
fn quadrant_art(state: CornerState, half: u32) -> IndexedCanvas {
    let mut c = IndexedCanvas::filled(half, half, 0);
    for y in 0..half as i32 {
        for x in 0..half as i32 {
            let level = match state {
                CornerState::Convex => x.min(y),
                CornerState::EdgeH => y,
                CornerState::EdgeV => x,
                CornerState::Inner => 4,
                CornerState::Concave => 4 - x.min(y).min(4),
            };
            c.set(x, y, level.clamp(0, 4) as u8);
        }
    }
    c
}

fn base_art(half: u32) -> BTreeMap<CornerState, IndexedCanvas> {
    STATES
        .into_iter()
        .map(|s| (s, quadrant_art(s, half)))
        .collect()
}

/// **壊れると: 縮約の規則が変わったのに気付かない．**
///
/// 47 は設計書 6.8 が挙げている数え上げの結果であって，調整できる値ではない．
/// 内訳まで固定しておく — 数だけ合っていて中身が違う，を防ぐ．
#[test]
fn the_blob_tileset_is_exactly_forty_seven_and_the_breakdown_matches() {
    let masks = blob_masks();
    assert_eq!(masks.len(), 47);

    // 辺の本数ごとの内訳 (設計書 6.8 の数え上げ)
    let mut by_edges = [0usize; 5];
    for m in &masks {
        by_edges[(m & 0x0f).count_ones() as usize] += 1;
    }
    assert_eq!(by_edges, [1, 4, 10, 16, 16]);
    // 2 本のうち隣り合う 4 通りが 2 倍 ・向かい合う 2 通りが 1 倍で 10
    assert_eq!(by_edges.iter().sum::<usize>(), 47);
}

/// **壊れると: 47 枚が «同じ絵» になり，autotile として使えない．**
///
/// bitmask が違えば，どこかの象限の状態が違うのだから絵も違うはずである．
#[test]
fn every_one_of_the_forty_seven_tiles_is_distinct() {
    let art = mirror_to_all_quadrants(&base_art(8));
    let (tiles, n) = build(&art, 16).expect("組める");
    assert_eq!(n, 47);
    let mut seen: Vec<&[u8]> = tiles.iter().map(|(_, t)| t.pixels()).collect();
    seen.sort();
    let total = seen.len();
    seen.dedup();
    assert_eq!(seen.len(), total, "同じ絵になったタイルがある");
}

/// **壊れると: bitmask とタイルの対応が崩れ，並べたときに継ぎ目が合わなくなる．**
///
/// 正規形でない mask を渡しても，正規形と同じ象限の状態になること．
#[test]
fn a_non_canonical_mask_picks_the_same_quadrant_states_as_its_canonical_form() {
    for m in 0u16..=255 {
        let m = m as u8;
        let c = canonical_mask(m);
        for q in QUADRANTS {
            assert_eq!(
                corner_state(m, q),
                corner_state(c, q),
                "mask {m:#04x} と正規形 {c:#04x} で象限 {} の状態が違う",
                q.as_str()
            );
        }
    }
}

/// **壊れると: 設計書 4.3 の «自動ミラーにルール 7 を blocking» が空振りする．**
///
/// 光の向きを持つ象限を鏡像で組むと，**左右非対称なタイルは必ず矛盾する**．
#[test]
fn auto_mirrored_tiles_contradict_a_declared_diagonal_light() {
    let art = mirror_to_all_quadrants(&base_art(8));
    let (tiles, _) = build(&art, 16).expect("組める");
    let palette = palette();
    let threshold = px_lint::LintConfig::default().min_shading_agreement;

    let (mut measured, mut fired) = (0usize, 0usize);
    for (_, t) in &tiles {
        let Some(a) = px_lint::rules::shading_agreement(t, &palette, diagonal()) else {
            continue;
        };
        measured += 1;
        if a < threshold {
            fired += 1;
        }
    }
    assert!(measured > 0, "1 枚も測れないなら検査が働いていない");
    assert!(
        fired > 0,
        "自動ミラーしたのに 1 枚も鳴らない — 検査が空振りしている"
    );
}

/// **壊れると: «測れなかった» を «画素が足りない» と報告し，理由を取り違える．**
///
/// 象限を鏡像で組むと**対称なタイル**ができる．対称な絵は勾配が打ち消し合うので
/// «向きが無い» になり，**光源と矛盾のしようがない** — 見逃しではない．
///
/// > [!warning] **対称は «左右» だけではない．**
/// > 最初この試験は «測れない ⇒ 左右対称» と書いて落ちた — `0x5f`
/// > (NW と SE が凹角 ・NE と SW が内側) は**180 度回転で重なる**のであって
/// > 左右対称ではない．勾配を打ち消すのは «向きを反転させる対称» すべてである．
///
/// 16x16 の全面不透明なタイルは勾配を測れる画素が $14 \times 14 = 196$ あるので，
/// **標本不足では «決して» ない**．
#[test]
fn unmeasurable_tiles_are_symmetric_not_too_small() {
    let art = mirror_to_all_quadrants(&base_art(8));
    let (tiles, _) = build(&art, 16).expect("組める");
    let palette = palette();
    let min = px_lint::LintConfig::default().shading_min_pixels as usize;

    let mut unmeasurable = 0usize;
    for (mask, t) in &tiles {
        let samples = px_lint::rules::shading_sample_count(t, &palette);
        assert_eq!(
            samples, 196,
            "16x16 の全面不透明なら標本は 196 のはずである"
        );
        assert!(samples >= min);
        if px_lint::rules::shading_agreement(t, &palette, diagonal()).is_none() {
            unmeasurable += 1;
            // 測れないなら **«向きを反転させる対称» のどれかを持つ**はずである
            let mirror_x = (0..16i32).all(|y| (0..16i32).all(|x| t.get(x, y) == t.get(15 - x, y)));
            let mirror_y = (0..16i32).all(|y| (0..16i32).all(|x| t.get(x, y) == t.get(x, 15 - y)));
            let rot180 =
                (0..16i32).all(|y| (0..16i32).all(|x| t.get(x, y) == t.get(15 - x, 15 - y)));
            assert!(
                mirror_x || mirror_y || rot180,
                "mask {mask:#04x} は測れないのに対称でもない — 打ち消しの理由が説明できない"
            );
        }
    }
    assert!(
        unmeasurable > 0,
        "対称なタイルが 1 枚も無いなら，この試験は何も見ていない"
    );
}

/// **壊れると: 象限 (タイルの半分) にルール 7 を掛けてしまい，
/// «測れない» が «鳴らない» として通る** (D100) ．
#[test]
fn a_quadrant_is_too_small_for_rule_7_even_when_the_tile_is_not() {
    let art = mirror_to_all_quadrants(&base_art(8));
    let palette = palette();
    let min = px_lint::LintConfig::default().shading_min_pixels as usize;

    for q in QUADRANTS {
        for state in STATES {
            let c = &art[&(q, state)];
            let samples = px_lint::rules::shading_sample_count(c, &palette);
            assert!(
                samples < min,
                "8x8 の象限で標本が {samples} — 下限 {min} に届いてしまっている"
            );
            assert!(
                px_lint::rules::shading_agreement(c, &palette, diagonal()).is_none(),
                "象限で勾配が測れてしまった (タイルに掛けるべきである)"
            );
        }
    }

    // 同じ絵でも 4 枚組んでタイルにすれば測れる
    let (tiles, _) = build(&art, 16).expect("組める");
    let measurable = tiles
        .iter()
        .filter(|(_, t)| px_lint::rules::shading_agreement(t, &palette, diagonal()).is_some())
        .count();
    assert!(measurable > 0, "組んでも測れないなら D100 の見立てが違う");
}

/// **壊れると: 象限を全部明示した入力で自動ミラーが働き，手描きが上書きされる．**
#[test]
fn explicit_quadrants_are_used_as_drawn() {
    let mut art = px_core::autotile::QuadrantArt::new();
    for (i, q) in QUADRANTS.into_iter().enumerate() {
        for state in STATES {
            art.insert((q, state), IndexedCanvas::filled(8, 8, i as u8));
        }
    }
    let (tiles, _) = build(&art, 16).expect("組める");
    for (_, t) in &tiles {
        assert_eq!(t.get(0, 0), Some(0), "NW");
        assert_eq!(t.get(15, 0), Some(1), "NE");
        assert_eq!(t.get(0, 15), Some(2), "SW");
        assert_eq!(t.get(15, 15), Some(3), "SE");
    }
}

/// **壊れると: 同じ入力で並びが変わり，差分ビルドの鍵が揺れる**
/// (設計書 6.15 規則 1) ．
#[test]
fn building_twice_gives_the_same_tiles_in_the_same_order() {
    let art = mirror_to_all_quadrants(&base_art(8));
    let (a, _) = build(&art, 16).expect("組める");
    let (b, _) = build(&art, 16).expect("組める");
    assert_eq!(a.len(), b.len());
    for ((ma, ta), (mb, tb)) in a.iter().zip(&b) {
        assert_eq!(ma, mb);
        assert_eq!(ta.pixels(), tb.pixels());
    }
    // 昇順であること
    assert!(a.windows(2).all(|w| w[0].0 < w[1].0));
}

/// **壊れると: 象限の状態が «内側» へ縮退せず，入力が 20 枚を超える．**
#[test]
fn the_quadrant_states_needed_across_all_forty_seven_masks_are_exactly_twenty() {
    let mut needed: Vec<(Quadrant, CornerState)> = Vec::new();
    for mask in blob_masks() {
        for q in QUADRANTS {
            let s = corner_state(mask, q);
            if !needed.contains(&(q, s)) {
                needed.push((q, s));
            }
        }
    }
    assert_eq!(needed.len(), 20, "象限 4 x 状態 5 = 20 で足りるはずである");
}
