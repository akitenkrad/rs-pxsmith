//! フレームアンカーを `track` → `apply` → `compose` まで実プロセスで通す．

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_io::{Document, FrameId};
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/anchor-track")
        .join(name)
}

fn scratch() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-anchor-track-{}-{}",
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("test")
            .replace("::", "-")
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("作業場所を作れる");
    root
}

fn run(args: &[&std::ffi::OsStr]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .args(args)
        .output()
        .expect("pxsmith を起こせる");
    assert!(
        output.status.success(),
        "pxsmith が失敗した:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn track_apply_and_compose_keep_the_cap_on_a_fixed_canvas() {
    let root = scratch();
    let body = fixture("body.px.toml");
    let cap = root.join("cap.px.toml");
    std::fs::copy(fixture("cap.px.toml"), &cap).expect("帽子を作業場所へ置ける");
    std::fs::copy(fixture("anchor.hex"), root.join("anchor.hex"))
        .expect("パレットを作業場所へ置ける");
    let body_track = root.join("body.json");
    let cap_track = root.join("cap.json");
    let applied = root.join("cap-applied.px.toml");
    let output = root.join("composed.aseprite");

    run(&[
        "anchor".as_ref(),
        "track".as_ref(),
        body.as_os_str(),
        "--anchor".as_ref(),
        "head".as_ref(),
        "-o".as_ref(),
        body_track.as_os_str(),
    ]);
    let tracked: Value =
        serde_json::from_slice(&std::fs::read(&body_track).expect("トラックを読める"))
            .expect("JSON である");
    assert_eq!(tracked["format"], 1);
    assert_eq!(tracked["frames"], 3);
    assert_eq!(
        tracked["anchors"]["head"],
        serde_json::json!([[1, 2], [1, 3], [1, 1]])
    );

    run(&[
        "anchor".as_ref(),
        "track".as_ref(),
        cap.as_os_str(),
        "-o".as_ref(),
        cap_track.as_os_str(),
    ]);
    run(&[
        "anchor".as_ref(),
        "apply".as_ref(),
        cap_track.as_os_str(),
        cap.as_os_str(),
        "-o".as_ref(),
        applied.as_os_str(),
        "--anchor".as_ref(),
        "head".as_ref(),
    ]);
    let applied_text = std::fs::read_to_string(&applied).expect("適用後の本文を読める");
    assert_eq!(
        applied_text
            .lines()
            .filter(|line| line.starts_with("anchors = { head = ["))
            .count(),
        4,
        "[meta] と 3 フレームのアンカーがインライン表でない:\n{applied_text}"
    );
    let applied_doc = pxsmith_io::l0::L0Document::read(&applied).expect("適用後を読める");
    assert_eq!(applied_doc.meta.anchors["head"], [0, 0]);
    assert!(
        applied_doc
            .frames
            .iter()
            .all(|frame| frame.anchors["head"] == [0, 0])
    );

    let cap_spec = format!("{}@head", applied.display());
    let composed = run(&[
        "compose".as_ref(),
        output.as_os_str(),
        "--part".as_ref(),
        body.as_os_str(),
        "--part".as_ref(),
        cap_spec.as_ref(),
    ]);
    let report = String::from_utf8_lossy(&composed.stdout);
    assert!(report.contains("x=1..1"), "x の範囲が無い:\n{report}");
    assert!(report.contains("y=1..3"), "y の範囲が無い:\n{report}");

    let doc = Document::read(&output).expect("合成結果を読める");
    assert_eq!(doc.frame_count(), 3);
    for (i, expected) in [(1, 2), (1, 3), (1, 1)].into_iter().enumerate() {
        let frame = doc.project(FrameId(i)).expect("フレームを射影できる");
        assert_eq!((frame.size.x, frame.size.y), (3, 4));
        let cap_layer = frame
            .layers
            .iter()
            .find(|layer| layer.meta.name.ends_with("/cap"))
            .expect("帽子の層がある")
            .surface
            .as_indexed()
            .expect("添字の層");
        let colored: Vec<_> = cap_layer
            .bounds()
            .iter()
            .filter(|p| !cap_layer.is_transparent_at(*p))
            .map(|p| (p.x, p.y))
            .collect();
        assert_eq!(colored, [expected], "フレーム {i}");
    }
}
