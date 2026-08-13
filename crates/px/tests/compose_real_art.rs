//! **`px compose` を実素材に掛けて «壊していないか» を測る．**
//!
//! `px-calib compose` で測ると，実素材を重ねると **lint の blocking が増える組が
//! ある** (ルール 3 «孤立ピクセル» が 10 組 ・ルール 4 «アウトライン角の重なり» が
//! 3 組) ．D58 の作法どおり «どちらが正しいか» を先に決める必要がある．
//!
//! **決め手は «合成が色を作ったり変えたりしていないか» である．**
//! 1 画素も色が変わっていないなら，増えた違反は**重ねた絵が本当にそうなっている**
//! のであって，道具の誤りではない — 帽子を胴体に載せれば胴体の縁が 1 画素だけ
//! 覗くことがあり，それは lint が言うとおり «孤立ピクセル» である．
//! そこで**色の保存を不変条件として固定し**，lint の結果は報告に回す．
//!
//! 合成した形では捕まらないので実素材を全件通す (M3 の教訓) ．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_core::canvas::IndexedCanvas;
use px_core::color::Rgba8;
use px_core::compose::{Alignment, ComposeOptions, Part, compose};
use px_core::frame::{Frame, FrameKind, Layer, LayerMeta, Surface};
use px_core::math::{IVec2, ivec2, uvec2};
use px_core::palette::Palette;

const BASES: &[&str] = &[
    "crawl_goblin",
    "crawl_donald",
    "crawl_saint_roka",
    "crawl_urand_fencer",
    "crawl_naga_warrior",
    "crawl_deformed_orc",
    "crawl_centaur_darkgrey_f",
    "crawl_salamander",
    "crawl_elephant",
    "crawl_holy_dragon",
    "crawl_tentacled_monstrosity",
    "crawl_siren_water",
];

const EQUIPS: &[&str] = &["crawl_cap1", "crawl_helmet1_visored", "crawl_cloth"];

fn shifts() -> Vec<IVec2> {
    vec![
        ivec2(0, 0),
        ivec2(1, 0),
        ivec2(0, -2),
        ivec2(-3, 2),
        ivec2(4, -5),
    ]
}

fn seeds() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/grid-eval/seeds")
        .canonicalize()
        .expect("種の置き場所がある")
}

/// PNG を色を落とさずに添字の面へ写す (`px-calib` の `index_exactly` と同じ)．
fn index_exactly(img: &px_core::canvas::RgbaCanvas) -> Option<(IndexedCanvas, Palette)> {
    let mut colors: Vec<Rgba8> = img.pixels().to_vec();
    colors.sort_unstable_by_key(|c| c.sort_key());
    colors.dedup();
    if colors.len() > 256 {
        return None;
    }
    let palette = Palette::new(colors).ok()?;
    let transparent = palette
        .entries()
        .iter()
        .position(|c| c.a == 0)
        .map(|i| i as u8);
    let pixels: Vec<u8> = img
        .pixels()
        .iter()
        .map(|c| match (c.a, transparent) {
            (0, Some(i)) => i,
            _ => palette.nearest(*c, 1.0).unwrap_or(0),
        })
        .collect();
    let canvas = IndexedCanvas::from_pixels(img.width(), img.height(), pixels)
        .ok()?
        .with_transparent(transparent);
    Some((canvas, palette))
}

fn part_of(name: &str) -> Option<Part> {
    let img = px_io::png::read_rgba(seeds().join(format!("{name}.png"))).ok()?;
    let (canvas, palette) = index_exactly(&img)?;
    let frame = Frame {
        size: uvec2(img.width(), img.height()),
        layers: vec![Layer::new(
            LayerMeta {
                name: "art".to_string(),
                ..LayerMeta::default()
            },
            Surface::Indexed(canvas),
        )],
        palette,
        duration_ms: 100,
        kind: FrameKind::Key,
    };
    Some(Part::new(name, vec![frame]))
}

/// 1 組ぶんのパーツ (基準と，`shift` だけずらして重ねるもの)．
fn pair(base: &str, equip: &str, shift: IVec2) -> Option<Vec<Part>> {
    let mut a = part_of(base)?;
    a.anchors.insert("joint".into(), shift);
    let mut b = part_of(equip)?;
    b.anchors.insert("joint".into(), ivec2(0, 0));
    b.align = Some(Alignment {
        part: "joint".into(),
        base: "joint".into(),
    });
    Some(vec![a, b])
}

/// **壊れると: 合成が色を作る ・変える．**
///
/// 色が変わらないことが `px compose` の不変条件である．ここが崩れると，
/// 合成の後に lint が鳴ったとき «重ねた絵がそうなのか道具が壊したのか» を
/// 二度と分けられなくなる (D58 の判断ができなくなる) ．
#[test]
fn compose_never_invents_or_changes_a_colour() {
    let mut checked = 0usize;
    for base in BASES {
        for equip in EQUIPS {
            for shift in shifts() {
                let Some(parts) = pair(base, equip, shift) else {
                    continue;
                };
                let sources: Vec<(IndexedCanvas, Palette, IVec2)> = parts
                    .iter()
                    .map(|p| {
                        let c = p.frames[0].layers[0]
                            .surface
                            .as_indexed()
                            .expect("添字の面")
                            .clone();
                        (c, p.frames[0].palette.clone(), ivec2(0, 0))
                    })
                    .collect();

                let (frames, report) =
                    compose(&parts, &ComposeOptions::default()).expect("合成できる");
                assert_eq!(frames.len(), 1);
                let out = &frames[0];
                assert_eq!(out.layers.len(), 2, "パーツごとに 1 レイヤ");

                for (k, (src, src_palette, _)) in sources.iter().enumerate() {
                    let at = report.placements[k].offset;
                    let dst = out.layers[k].surface.as_indexed().expect("添字の面");
                    for p in src.bounds().iter() {
                        let want = src
                            .get_at(p)
                            .and_then(|i| src_palette.get(i))
                            .expect("元の色");
                        let got = dst
                            .get_at(p + at)
                            .and_then(|i| out.palette.get(i))
                            .expect("移した先の色");
                        // 透明どうしは色の値が違っていてもよい (透明は 1 つに束ねる)
                        if want.a == 0 {
                            assert_eq!(got.a, 0, "{base} + {equip} の透明が不透明になった");
                        } else {
                            assert_eq!(
                                got, want,
                                "{base} + {equip} ずらし ({},{}) の画素 ({},{}) で色が変わった",
                                shift.x, shift.y, p.x, p.y
                            );
                        }
                    }
                }
                checked += 1;
            }
        }
    }
    assert!(checked >= 150, "実素材を全件通す (通ったのは {checked} 組)");
}

/// **壊れると: 動かしたパーツが画布の外で黙って切れる．**
///
/// 実素材のシルエットは 38 枚中 33 枚が画布の縁に接している (`px-calib compose`) ．
/// 既定で切る実装にすると，1 画素動かしただけで必ず絵が欠ける．
#[test]
fn growing_the_canvas_never_throws_a_pixel_away() {
    for base in BASES {
        for equip in EQUIPS {
            for shift in shifts() {
                let Some(parts) = pair(base, equip, shift) else {
                    continue;
                };
                let (_, report) = compose(&parts, &ComposeOptions::default()).expect("合成できる");
                assert_eq!(
                    report.clipped(),
                    0,
                    "{base} + {equip} ずらし ({},{}) で {} 画素が切れた",
                    shift.x,
                    shift.y,
                    report.clipped()
                );
                // ずらしていないなら画布は 1 画素も動かない
                if shift == ivec2(0, 0) {
                    assert!(!report.grew, "{base} + {equip} で画布が動いた");
                }
            }
        }
    }
}

/// **壊れると: `--clip` が «切った» と言わずに絵を欠けさせる．**
#[test]
fn clipping_says_how_much_it_threw_away() {
    let mut clipped_cases = 0usize;
    for base in BASES {
        for equip in EQUIPS {
            for shift in shifts() {
                let Some(parts) = pair(base, equip, shift) else {
                    continue;
                };
                let opts = ComposeOptions {
                    clip: true,
                    ..ComposeOptions::default()
                };
                let (frames, report) = compose(&parts, &opts).expect("合成できる");
                assert_eq!(report.canvas, frames[0].size);
                assert_eq!(report.canvas, uvec2(32, 32), "切る側は基準の画布のまま");
                if shift != ivec2(0, 0) {
                    // ずらせば必ず何かが外へ出る — 実素材は縁まで描いてあるため
                    clipped_cases += usize::from(report.clipped() > 0);
                }
            }
        }
    }
    assert!(
        clipped_cases > 0,
        "実素材ではずらすと切れるはずである (切れた組が 0 なら測り方が壊れている)"
    );
}

/// **壊れると: 同じ入力で出力が変わり，差分ビルドの鍵が毎回ずれる**
/// (設計書 6.15 規則 1) ．
#[test]
fn composing_twice_gives_the_same_bytes() {
    for base in BASES.iter().take(4) {
        for equip in EQUIPS {
            let Some(parts) = pair(base, equip, ivec2(-3, 2)) else {
                continue;
            };
            let (a, ra) = compose(&parts, &ComposeOptions::default()).expect("合成できる");
            let (b, rb) = compose(&parts, &ComposeOptions::default()).expect("合成できる");
            assert_eq!(ra.canvas, rb.canvas);
            assert_eq!(ra.colors, rb.colors);
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(&b) {
                assert_eq!(x.palette.entries(), y.palette.entries());
                for (lx, ly) in x.layers.iter().zip(&y.layers) {
                    assert_eq!(
                        lx.surface.as_indexed().map(|c| c.pixels()),
                        ly.surface.as_indexed().map(|c| c.pixels()),
                        "{base} + {equip} が 2 度目で変わった"
                    );
                }
            }
        }
    }
}

/// **壊れると: 重ね順を間違えたパーツが «見えていない» まま出荷される．**
///
/// 実素材で数える — 帽子を下に敷けば胴体に隠れる画素が必ず出る．
#[test]
fn a_part_hidden_under_another_is_counted() {
    let mut hidden_total = 0usize;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for base in BASES {
        for equip in EQUIPS {
            // 装備を下 ・胴体を上にする (わざと逆の重ね順)
            let Some(mut parts) = pair(base, equip, ivec2(0, 0)) else {
                continue;
            };
            parts.swap(0, 1);
            parts[0].align = None;
            parts[1].align = None;
            let (_, report) = compose(&parts, &ComposeOptions::default()).expect("合成できる");
            let hidden = report.placements[0].covered;
            hidden_total += hidden;
            *counts.entry(equip).or_default() += usize::from(hidden > 0);
        }
    }
    assert!(
        hidden_total > 0,
        "胴体の下に敷いた装備は必ず一部隠れるはずである"
    );
    for equip in EQUIPS {
        assert!(
            counts.get(equip).copied().unwrap_or(0) > 0,
            "{equip} が 1 組も隠れていない — 被覆を数えていない見込みが高い"
        );
    }
}
