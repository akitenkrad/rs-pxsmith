//! 複数レイヤの `compose` を単一レイヤ形式へ書くときの回帰試験 (MYTASK-3576)．

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::Frame;
use pxsmith_io::{Document, FrameId};
use pxsmith_view::render::RenderOptions;

fn repo(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn scratch() -> PathBuf {
    let root = repo("target/compose-flatten-tests").join(format!(
        "{}-{}",
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

fn write_part(root: &Path, name: &str, color: u8, data: &str) -> PathBuf {
    let path = root.join(format!("{name}.px.toml"));
    let text = format!(
        r#"[meta]
format = 1
name = "{name}"
layer = "{name}"

[palette]
ref = "parts.hex"

[palette.map]
"." = "transparent"
0 = {color}

[[frame]]
name = "{name}_0"
kind = "key"
duration_ms = 70
data = '''
{data}
'''
"#
    );
    std::fs::write(&path, text).expect("パーツを書ける");
    path
}

fn rgba(frame: &Frame) -> Vec<u8> {
    pxsmith_view::render::to_rgba_image(
        frame,
        &RenderOptions {
            zoom: 1,
            checkerboard: false,
            ..RenderOptions::default()
        },
    )
    .pixels()
    .flat_map(|pixel| pixel.0)
    .collect()
}

fn gif_rgba(path: &Path) -> Vec<u8> {
    let file = std::fs::File::open(path).expect("GIF を開ける");
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(file).expect("GIF を読める");
    let frame = decoder
        .read_next_frame()
        .expect("GIF のコマを読める")
        .expect("GIF にコマがある");
    let pixels = frame.buffer.to_vec();
    assert!(
        decoder
            .read_next_frame()
            .expect("GIF の末尾を読める")
            .is_none(),
        "入力は 1 コマなので GIF も 1 コマ"
    );
    pixels
}

#[test]
fn compose_single_layer_outputs_match_the_layered_aseprite_pixel_for_pixel() {
    let root = scratch();
    std::fs::write(root.join("parts.hex"), "ff0000\n0000ff\n").expect("パレットを書ける");
    let body = write_part(&root, "body", 0, "0..");
    let hat = write_part(&root, "hat", 1, ".0.");

    let aseprite = root.join("out.aseprite");
    let layered = run(&[
        "compose".as_ref(),
        aseprite.as_os_str(),
        "--part".as_ref(),
        body.as_os_str(),
        "--part".as_ref(),
        hat.as_os_str(),
    ]);
    assert!(
        !String::from_utf8_lossy(&layered.stdout).contains("平坦化"),
        ".aseprite はレイヤを保つ"
    );
    let expected = rgba(
        &Document::read(&aseprite)
            .expect("Aseprite を読める")
            .project(FrameId(0))
            .expect("Aseprite のコマを射影できる"),
    );
    assert_eq!(
        expected,
        [255, 0, 0, 255, 0, 0, 255, 255, 0, 0, 0, 0],
        "下の赤・上の青・透明の重ね順"
    );

    let l0 = root.join("out.px.toml");
    let png = root.join("out.png");
    let gif = root.join("out.gif");
    for output in [&l0, &png, &gif] {
        let result = run(&[
            "compose".as_ref(),
            output.as_os_str(),
            "--part".as_ref(),
            body.as_os_str(),
            "--part".as_ref(),
            hat.as_os_str(),
        ]);
        let report = String::from_utf8_lossy(&result.stdout);
        assert!(
            report.contains("複数レイヤを平坦化"),
            "{} の平坦化報告が無い:\n{report}",
            output.display()
        );
    }

    let l0_doc = pxsmith_io::l0::L0Document::read(&l0).expect("L0 を読める");
    let l0_frames = l0_doc.to_frames(&l0).expect("L0 を作業層へ変換できる");
    assert_eq!(rgba(&l0_frames[0]), expected, "L0 の平坦化結果");

    let png_pixels: Vec<u8> = pxsmith_io::png::read_rgba(&png)
        .expect("PNG を読める")
        .pixels()
        .iter()
        .flat_map(|color| [color.r, color.g, color.b, color.a])
        .collect();
    assert_eq!(png_pixels, expected, "PNG の平坦化結果");
    assert_eq!(gif_rgba(&gif), expected, "GIF の平坦化結果");
}
