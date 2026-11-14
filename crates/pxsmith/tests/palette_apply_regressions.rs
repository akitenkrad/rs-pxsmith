//! `pxsmith palette apply` のフレーム列と透明の回帰試験 (MYTASK-3577・3578)．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::{Frame, Rgba8, ivec2};
use pxsmith_io::{Document, FrameId};

fn repo(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn scratch(name: &str) -> PathBuf {
    let root = repo("target/palette-apply-tests").join(format!(
        "{name}-{}-{}",
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
        "pxsmith が失敗した:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn fixture(root: &Path) -> (PathBuf, PathBuf) {
    let input = root.join("two.px.toml");
    let source_palette = root.join("source.hex");
    let target_palette = root.join("dark.hex");
    std::fs::write(&source_palette, "ff0000\n0000ff\n").expect("元パレットを書ける");
    std::fs::write(&target_palette, "1a1c2c\n5d275d\n29366f\n333c57\n")
        .expect("強制先パレットを書ける");
    std::fs::write(
        &input,
        r#"[meta]
format = 1
name = "two"
layer = "art"
anchors = { origin = [0, 0] }

[palette]
ref = "source.hex"

[palette.map]
"." = "transparent"
a = 0
b = 1

[[frame]]
name = "f0"
kind = "breakdown"
duration_ms = 83
anchors = { hand = [1, 0] }
data = '''
.a.
'''

[[frame]]
name = "f1"
kind = "inbetween"
duration_ms = 47
anchors = { hand = [2, 0] }
data = '''
.b.
'''
"#,
    )
    .expect("2 コマの L0 を書ける");
    (input, target_palette)
}

fn apply(input: &Path, output: &Path, palette: &Path, dither: Option<&str>) -> Output {
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "palette".as_ref(),
        "apply".as_ref(),
        input.as_os_str(),
        output.as_os_str(),
        "--palette".as_ref(),
        palette.as_os_str(),
    ];
    if let Some(dither) = dither {
        args.push(std::ffi::OsStr::new("--dither"));
        args.push(std::ffi::OsStr::new(dither));
    }
    run(&args)
}

fn opaque_mask(frame: &Frame) -> Vec<bool> {
    let canvas = frame.layers[0].surface.as_indexed().expect("添字の面");
    canvas
        .bounds()
        .iter()
        .map(|point| !canvas.is_transparent_at(point))
        .collect()
}

#[test]
fn palette_apply_preserves_every_l0_frame_and_its_metadata() {
    let root = scratch("l0-metadata");
    let (input, palette) = fixture(&root);
    let output = root.join("out.px.toml");
    apply(&input, &output, &palette, None);

    let document = pxsmith_io::l0::L0Document::read(&output).expect("出力 L0 を読める");
    assert_eq!(document.meta.name, "two");
    assert_eq!(document.meta.anchors["origin"], [0, 0]);
    assert_eq!(document.frames.len(), 2);
    assert_eq!(
        document
            .frames
            .iter()
            .map(|frame| frame.name.as_str())
            .collect::<Vec<_>>(),
        ["f0", "f1"]
    );
    assert_eq!(
        document
            .frames
            .iter()
            .map(|frame| frame.duration_ms)
            .collect::<Vec<_>>(),
        [83, 47]
    );
    assert_eq!(
        document
            .frames
            .iter()
            .map(|frame| frame.kind.as_str())
            .collect::<Vec<_>>(),
        ["breakdown", "inbetween"]
    );
    assert_eq!(document.frames[0].anchors["hand"], [1, 0]);
    assert_eq!(document.frames[1].anchors["hand"], [2, 0]);
}

#[test]
fn palette_apply_preserves_aseprite_frames_and_durations() {
    let root = scratch("aseprite-frames");
    let (input, palette) = fixture(&root);
    let source = root.join("two.aseprite");
    run(&[
        "text".as_ref(),
        "import".as_ref(),
        input.as_os_str(),
        source.as_os_str(),
    ]);
    let output = root.join("out.aseprite");
    apply(&source, &output, &palette, None);

    let document = Document::read(&output).expect("出力 Aseprite を読める");
    assert_eq!(document.frame_count(), 2);
    assert_eq!(
        (0..2)
            .map(|index| {
                document
                    .project(FrameId(index))
                    .expect("コマを射影できる")
                    .duration_ms
            })
            .collect::<Vec<_>>(),
        [83, 47]
    );
}

#[test]
fn palette_apply_keeps_transparency_for_every_dither_mode() {
    let root = scratch("transparency");
    let (input, palette) = fixture(&root);
    for dither in ["none", "ordered", "error"] {
        let output = root.join(format!("out-{dither}.px.toml"));
        apply(&input, &output, &palette, Some(dither));

        let document = pxsmith_io::l0::L0Document::read(&output).expect("出力 L0 を読める");
        assert!(
            document
                .palette
                .map
                .values()
                .any(|value| matches!(value, pxsmith_io::l0::RawColorKey::Name(name) if name == "transparent")),
            "{dither}: [palette.map] に透明が無い"
        );
        let frames = document
            .to_frames(&output)
            .expect("出力を作業層へ変換できる");
        assert_eq!(frames.len(), 2, "{dither}: コマ数");
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(
                opaque_mask(frame),
                [false, true, false],
                "{dither}: フレーム {index} の透明マスク"
            );
            assert_eq!(
                frame.palette.entries().last(),
                Some(&Rgba8::TRANSPARENT),
                "{dither}: パレット末尾の透明"
            );
            let canvas = frame.layers[0].surface.as_indexed().expect("添字の面");
            assert!(canvas.is_transparent_at(ivec2(0, 0)));
            assert!(canvas.is_transparent_at(ivec2(2, 0)));
        }
    }
}

#[test]
fn fixture_has_distinct_meta_and_frame_anchors() {
    let root = scratch("fixture");
    let (input, _) = fixture(&root);
    let document = pxsmith_io::l0::L0Document::read(&input).expect("fixture を読める");
    assert_eq!(
        document.meta.anchors,
        BTreeMap::from([("origin".to_string(), [0, 0])])
    );
    assert_eq!(document.frames[0].anchors["hand"], [1, 0]);
    assert_eq!(document.frames[1].anchors["hand"], [2, 0]);
}
