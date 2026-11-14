//! L0 テキスト形式の往復 (M1 の完了条件)．
//!
//! 「`pxsmith text` は L0 制約内で双方向変換が画素一致」と「既存 `.aseprite` 1 枚を
//! テキストで再現できるか」を，実ファイルに対して確かめる．
//!
//! 合成した例ではなく `testdata/aseprite/` の実ファイルを通すのが要点である．
//! L0 は制約の強い形式なので，**通らない素材がどれで，なぜ通らないか**が
//! 分かることに意味がある．

use std::path::{Path, PathBuf};

use pxsmith_core::AxisStretch;
use pxsmith_io::l0::L0Document;
use pxsmith_io::{Document, FrameId, hex};

const CANONICAL_L0_WITHOUT_FRAME_ANCHORS: &str = r#"[meta]
format = 1
name = "hero"

[palette]
ref = "hero.hex"

[palette.map]
"." = "transparent"
0 = 0

[[frame]]
name = "idle"
kind = "key"
duration_ms = 100
data = '''
.0.
000
'''
"#;

const L0_WITH_FRAME_ANCHORS: &str = r#"[meta]
format = 1
name = "hero"
anchors = { head = [1, 0], feet = [1, 2] }

[palette]
ref = "hero.hex"

[palette.map]
"." = "transparent"
0 = 0

[[frame]]
name = "idle"
kind = "key"
duration_ms = 100
anchors = { head = [2, 1], ankle = [0, 2] }
data = '''
.0.
000
'''
"#;

const L0_WITH_DEFAULT_NINE_SLICE: &str = r#"[meta]
format = 1
name = "window"
nine_slice = { left = 1, top = 1, right = 1, bottom = 1 }

[palette]
ref = "window.hex"

[palette.map]
0 = 0

[[frame]]
name = "window"
kind = "key"
duration_ms = 100
data = '''
000
000
000
'''
"#;

const L0_WITH_COMMON_NINE_SLICE: &str = r#"[meta]
format = 1
name = "window"
nine_slice = { left = 1, top = 1, right = 1, bottom = 1, stretch = "tile" }

[palette]
ref = "window.hex"

[palette.map]
0 = 0

[[frame]]
name = "window"
kind = "key"
duration_ms = 100
data = '''
000
000
000
'''
"#;

const L0_WITH_AXIS_NINE_SLICE: &str = r#"[meta]
format = 1
name = "window"
nine_slice = { left = 1, top = 1, right = 1, bottom = 1, stretch_x = "tile", stretch_y = "tile_fit" }

[palette]
ref = "window.hex"

[palette.map]
0 = 0

[[frame]]
name = "window"
kind = "key"
duration_ms = 100
data = '''
000
000
000
'''
"#;

fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/aseprite");
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                walk(&p, out);
            } else if matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("aseprite") | Some("ase")
            ) {
                out.push(p);
            }
        }
    }
    walk(&dir, &mut out);
    out
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("pxforge-l0-roundtrip");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// `.aseprite` → L0 → `.aseprite` で画素が一致すること．
#[test]
fn indexed_sprites_survive_the_text_round_trip() {
    let mut checked = 0usize;
    let mut skipped = Vec::new();

    for path in corpus() {
        let doc = match Document::read(&path) {
            Ok(d) => d,
            Err(e) => {
                skipped.push(format!("{}: 読めない ({e})", path.display()));
                continue;
            }
        };
        let Ok(frames) = (0..doc.frame_count())
            .map(|i| doc.project(FrameId(i)))
            .collect::<Result<Vec<_>, _>>()
        else {
            skipped.push(format!("{}: 射影できない", path.display()));
            continue;
        };
        let Some(first) = frames.first() else {
            continue;
        };
        // L0 はインデックスカラーの 1 レイヤだけを扱う (D9)
        let Some(layer) = first
            .layers
            .iter()
            .position(|l| l.surface.as_indexed().is_some())
        else {
            skipped.push(format!(
                "{}: インデックスカラーのレイヤが無い",
                path.display()
            ));
            continue;
        };

        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let hex_path = scratch(&format!("{stem}.hex"));
        let l0_path = scratch(&format!("{stem}.px.toml"));
        hex::write(&hex_path, &first.palette).unwrap();

        let exported = match L0Document::from_frames(&stem, format!("{stem}.hex"), &frames, layer) {
            Ok(e) => e,
            Err(v) => {
                assert!(v.is_blocking(), "助言レベルの違反で失敗している: {v}");
                skipped.push(format!("{}: {v}", path.display()));
                continue;
            }
        };
        exported.document.write(&l0_path).unwrap();

        // 読み直して画素が一致するか
        let reloaded = L0Document::read(&l0_path).unwrap();
        let back = reloaded.to_frames(&l0_path).unwrap();

        assert_eq!(back.len(), frames.len(), "{}: フレーム数", path.display());
        for (i, (a, b)) in frames.iter().zip(&back).enumerate() {
            let ca = a.layers[layer].surface.as_indexed().unwrap();
            let cb = b.layers[0].surface.as_indexed().unwrap();
            assert_eq!(
                (ca.width(), ca.height()),
                (cb.width(), cb.height()),
                "{} のフレーム {i}: 大きさ",
                path.display()
            );

            // 添字そのものではなく**画素が指す色**を比べる．L0 は使われている色だけを
            // 62 文字へ詰め直すので添字は変わりうるが，見える絵は変わってはいけない
            for (pa, pb) in ca.pixels().iter().zip(cb.pixels()) {
                let ta = ca.transparent() == Some(*pa);
                let tb = cb.transparent() == Some(*pb);
                assert_eq!(ta, tb, "{} のフレーム {i}: 透明の位置", path.display());
                if !ta {
                    assert_eq!(
                        a.palette.get(*pa),
                        b.palette.get(*pb),
                        "{} のフレーム {i}: 色",
                        path.display()
                    );
                }
            }
            assert_eq!(
                a.duration_ms,
                b.duration_ms,
                "{}: フレーム長",
                path.display()
            );
            assert_eq!(a.kind, b.kind, "{}: フレームの役割", path.display());
        }
        checked += 1;
    }

    eprintln!("{checked} 件が L0 の往復を通った");
    for s in &skipped {
        eprintln!("  対象外: {s}");
    }
    assert!(checked > 0, "L0 の往復を試せる素材が 1 つも無い");
}

/// L0 から作った `.aseprite` を読み直しても絵が変わらないこと (`pxsmith text import`)．
#[test]
fn l0_can_be_imported_into_a_new_aseprite() {
    let hex_path = scratch("import.hex");
    let l0_path = scratch("import.px.toml");
    std::fs::write(&hex_path, "1a1c2c\nb13e53\nffcd75\n").unwrap();
    std::fs::write(
        &l0_path,
        r#"
[meta]
format = 1
name = "sprite"
layer = "body"

[palette]
ref = "import.hex"
map = { "." = "transparent", "k" = 0, "r" = 1, "y" = 2 }

[[frame]]
name = "a"
duration_ms = 83
data = '''
.kkk.
kryrk
.kkk.
'''

[[frame]]
name = "b"
kind = "inbetween"
duration_ms = 42
data = '''
.kkk.
kyryk
.kkk.
'''
"#,
    )
    .unwrap();

    let frames = L0Document::read(&l0_path)
        .unwrap()
        .to_frames(&l0_path)
        .unwrap();
    let doc = Document::from_frames(&frames).unwrap();
    let bytes = doc.to_bytes().unwrap();
    let reloaded = Document::from_bytes(&bytes).unwrap();

    assert_eq!(reloaded.frame_count(), 2);
    for (i, expected) in frames.iter().enumerate() {
        let got = reloaded.project(FrameId(i)).unwrap();
        assert_eq!(got.size, expected.size);
        assert_eq!(
            got.layers[0].surface.as_indexed().unwrap().pixels(),
            expected.layers[0].surface.as_indexed().unwrap().pixels(),
            "フレーム {i}"
        );
        assert_eq!(got.duration_ms, expected.duration_ms);
        assert_eq!(got.layers[0].meta.name, "body");
    }

    // 書き出した .aseprite 自体もバイト一致で往復する
    assert_eq!(reloaded.to_bytes().unwrap(), bytes);
}

#[test]
fn frame_anchors_survive_the_l0_round_trip() {
    let path = Path::new("frame-anchors.px.toml");
    let doc = L0Document::parse(L0_WITH_FRAME_ANCHORS, path).expect("フレームアンカーを読める");
    assert_eq!(doc.frames[0].anchors["head"], [2, 1]);

    let written = doc.to_toml().expect("書ける");
    assert!(
        written.contains("anchors = { feet = [1, 2], head = [1, 0] }"),
        "[meta] のアンカーが昇順のインライン表でない:\n{written}"
    );
    let frame_anchors = "anchors = { ankle = [0, 2], head = [2, 1] }";
    assert!(
        written.contains(frame_anchors),
        "フレームのアンカーが昇順のインライン表でない:\n{written}"
    );
    assert!(
        written.find(frame_anchors) < written.find("data = '''"),
        "フレームのアンカーが data より前にない:\n{written}"
    );
    assert!(!written.contains("[meta.anchors]"), "{written}");
    assert!(!written.contains("[frame.anchors]"), "{written}");

    let reparsed = L0Document::parse(&written, path).expect("書いたものを読める");
    assert_eq!(reparsed.meta.anchors["head"], [1, 0]);
    assert_eq!(reparsed.frames[0].anchors["head"], [2, 1]);
    assert_eq!(reparsed.frames[0].anchors["ankle"], [0, 2]);
}

#[test]
fn l0_without_frame_anchors_is_byte_identical_after_read_and_write() {
    let doc = L0Document::parse(
        CANONICAL_L0_WITHOUT_FRAME_ANCHORS,
        Path::new("legacy.px.toml"),
    )
    .expect("既存 L0 を読める");
    assert_eq!(
        doc.to_toml().expect("書ける"),
        CANONICAL_L0_WITHOUT_FRAME_ANCHORS
    );
}

#[test]
fn nine_slice_forms_are_byte_identical_after_read_and_write() {
    let cases = [
        (
            L0_WITH_DEFAULT_NINE_SLICE,
            AxisStretch::Stretch,
            AxisStretch::Stretch,
        ),
        (
            L0_WITH_COMMON_NINE_SLICE,
            AxisStretch::Tile,
            AxisStretch::Tile,
        ),
        (
            L0_WITH_AXIS_NINE_SLICE,
            AxisStretch::Tile,
            AxisStretch::TileFit,
        ),
    ];
    for (text, stretch_x, stretch_y) in cases {
        let doc = L0Document::parse(text, Path::new("window.px.toml")).expect("9-slice を読める");
        let nine_slice = doc.meta.nine_slice.expect("宣言が残る");
        assert_eq!(nine_slice.stretch_x, stretch_x);
        assert_eq!(nine_slice.stretch_y, stretch_y);
        assert_eq!(doc.to_toml().expect("書ける"), text);
    }
}

#[test]
fn common_and_axis_stretch_cannot_be_mixed() {
    let text = L0_WITH_COMMON_NINE_SLICE.replace(
        "stretch = \"tile\"",
        "stretch = \"tile\", stretch_x = \"tile_fit\"",
    );
    let error =
        L0Document::parse(&text, Path::new("conflict.px.toml")).expect_err("競合する指定はエラー");
    assert!(
        error.to_string().contains("同時に指定できない"),
        "予期しないエラー: {error}"
    );
}

#[test]
fn nine_slice_follows_anchors_before_the_palette() {
    let text = L0_WITH_DEFAULT_NINE_SLICE.replacen(
        "name = \"window\"\n",
        "name = \"window\"\nanchors = { origin = [1, 2] }\n",
        1,
    );
    let doc = L0Document::parse(&text, Path::new("anchored-window.px.toml"))
        .expect("アンカーと 9-slice を読める");
    let written = doc.to_toml().expect("書ける");
    assert_eq!(written, text);
    assert!(written.find("anchors =").unwrap() < written.find("nine_slice =").unwrap());
    assert!(written.find("nine_slice =").unwrap() < written.find("[palette]").unwrap());
}

#[test]
fn nine_slice_margins_must_leave_a_source_centre() {
    let root = scratch("nine-slice-validation");
    std::fs::create_dir_all(&root).expect("作業場所を作れる");
    std::fs::write(root.join("window.hex"), "1a1c2c\n").expect("パレットを書ける");
    let text = L0_WITH_DEFAULT_NINE_SLICE.replace("right = 1", "right = 2");
    let doc = L0Document::parse(&text, &root.join("window.px.toml")).expect("構文は読める");
    let error = doc
        .to_frames(&root.join("window.px.toml"))
        .expect_err("中央が残らない余白はエラー");
    assert!(error.to_string().contains("中央を 1 px 以上"), "{error}");
}

#[test]
fn every_repository_l0_is_byte_identical_after_read_and_write() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut paths = Vec::new();
    for subtree in ["testdata", "examples"] {
        collect_l0(&repository.join(subtree), &mut paths);
    }
    paths.sort();
    for path in &paths {
        let original = std::fs::read_to_string(path).expect("L0 を読める");
        let document = L0Document::parse(&original, path).expect("L0 を解釈できる");
        assert_eq!(
            document.to_toml().expect("L0 を書ける"),
            original,
            "{} の正規形が変わった",
            path.display()
        );
    }
    eprintln!("{} 件の .px.toml がバイト一致で往復した", paths.len());
    assert!(!paths.is_empty(), "往復対象の .px.toml が無い");
}

fn collect_l0(directory: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_l0(&path, paths);
        } else if path.to_string_lossy().ends_with(".px.toml") {
            paths.push(path);
        }
    }
}
