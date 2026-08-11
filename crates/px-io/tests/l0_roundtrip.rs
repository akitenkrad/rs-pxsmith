//! L0 テキスト形式の往復 (M1 の完了条件)．
//!
//! 「`px text` は L0 制約内で双方向変換が画素一致」と「既存 `.aseprite` 1 枚を
//! テキストで再現できるか」を，実ファイルに対して確かめる．
//!
//! 合成した例ではなく `testdata/aseprite/` の実ファイルを通すのが要点である．
//! L0 は制約の強い形式なので，**通らない素材がどれで，なぜ通らないか**が
//! 分かることに意味がある．

use std::path::{Path, PathBuf};

use px_io::l0::L0Document;
use px_io::{Document, FrameId, hex};

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

/// L0 から作った `.aseprite` を読み直しても絵が変わらないこと (`px text import`)．
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
