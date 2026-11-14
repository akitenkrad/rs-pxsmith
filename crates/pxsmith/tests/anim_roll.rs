use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::{Frame, FrameKind, IndexedCanvas};

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fonts")
        .join(path)
}

fn scratch() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-anim-roll-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .args(args)
        .output()
        .expect("pxsmith を起こせる")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "pxsmith が失敗した:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn read_frames(path: &Path) -> Vec<Frame> {
    pxsmith_io::l0::L0Document::read(path)
        .unwrap()
        .to_frames(path)
        .unwrap()
}

fn first_canvas(frames: &[Frame]) -> &IndexedCanvas {
    frames[0].layers[0].surface.as_indexed().unwrap()
}

fn rgba_pixels(frame: &Frame) -> Vec<u8> {
    let canvas = frame.layers[0].surface.as_indexed().unwrap();
    pxsmith_io::png::resolve(canvas, &frame.palette)
        .pixels()
        .iter()
        .flat_map(|color| [color.r, color.g, color.b, color.a])
        .collect()
}

fn read_gif(path: &Path) -> (Vec<u16>, Vec<Vec<u8>>) {
    let file = std::fs::File::open(path).expect("GIF を開ける");
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(file).expect("GIF を読める");
    let mut delays = Vec::new();
    let mut pixels = Vec::new();
    while let Some(frame) = decoder.read_next_frame().expect("GIF のコマを読める") {
        assert_eq!((frame.width, frame.height), (16, 8));
        delays.push(frame.delay);
        pixels.push(frame.buffer.to_vec());
    }
    (delays, pixels)
}

#[test]
fn misaki_roll_runs_through_ease_and_matches_font_render_at_both_ends() {
    let root = scratch();
    let font = root.join("misaki.pxfont.toml");
    let roll_output = root.join("roll.px.toml");
    let eased_output = root.join("eased.px.toml");
    let gif_output = root.join("eased.gif");
    let from_output = root.join("from.px.toml");
    let to_output = root.join("to.px.toml");
    let palette = fixture("handmade.hex");

    let imported = run(&[
        "font".as_ref(),
        "import".as_ref(),
        fixture("misaki/misaki_gothic.bdf").as_os_str(),
        font.as_os_str(),
    ]);
    assert_success(&imported);

    for (text, output) in [("0129", &from_output), ("0131", &to_output)] {
        let rendered = run(&[
            "font".as_ref(),
            "render".as_ref(),
            font.as_os_str(),
            text.as_ref(),
            output.as_os_str(),
            "--palette".as_ref(),
            palette.as_os_str(),
            "--ink".as_ref(),
            "1".as_ref(),
        ]);
        assert_success(&rendered);
    }

    let rolled = run(&[
        "anim".as_ref(),
        "roll".as_ref(),
        font.as_os_str(),
        "--from".as_ref(),
        "0129".as_ref(),
        "--to".as_ref(),
        "0131".as_ref(),
        "--frames".as_ref(),
        "4".as_ref(),
        roll_output.as_os_str(),
        "--palette".as_ref(),
        palette.as_os_str(),
        "--ink".as_ref(),
        "1".as_ref(),
    ]);
    assert_success(&rolled);

    let frames = read_frames(&roll_output);
    assert_eq!(frames.len(), 9, "2 回の切り替え × 4 区間 + 始点");
    assert_eq!(frames.first().unwrap().kind, FrameKind::Key);
    assert_eq!(frames.last().unwrap().kind, FrameKind::Key);
    assert!(
        frames[1..8]
            .iter()
            .all(|frame| frame.kind == FrameKind::Inbetween)
    );
    assert_eq!(
        frames[0].layers[0].surface.as_indexed().unwrap(),
        first_canvas(&read_frames(&from_output))
    );
    assert_eq!(
        frames[8].layers[0].surface.as_indexed().unwrap(),
        first_canvas(&read_frames(&to_output))
    );

    let eased = run(&[
        "anim".as_ref(),
        "ease".as_ref(),
        roll_output.as_os_str(),
        eased_output.as_os_str(),
        "--fps".as_ref(),
        "60".as_ref(),
        "--hold".as_ref(),
        "1,1,1,2,2,3,3,4,6".as_ref(),
    ]);
    assert_success(&eased);
    let eased_frames = read_frames(&eased_output);
    assert_eq!(eased_frames.len(), 9);
    assert_eq!(
        eased_frames
            .iter()
            .map(|frame| frame.duration_ms)
            .collect::<Vec<_>>(),
        [17, 17, 17, 33, 33, 50, 50, 67, 100]
    );

    let gif = run(&[
        "anim".as_ref(),
        "ease".as_ref(),
        roll_output.as_os_str(),
        gif_output.as_os_str(),
        "--fps".as_ref(),
        "60".as_ref(),
        "--hold".as_ref(),
        "1,1,1,2,2,3,3,4,6".as_ref(),
    ]);
    assert_success(&gif);
    let stdout = String::from_utf8_lossy(&gif.stdout);
    assert!(
        stdout.contains("6 フレーム"),
        "GIF の表示時間を丸めた件数が無い:\n{stdout}"
    );

    let (delays, gif_pixels) = read_gif(&gif_output);
    assert_eq!(delays, [2, 2, 2, 3, 3, 5, 5, 7, 10]);
    assert_eq!(gif_pixels.len(), eased_frames.len());
    assert_eq!(gif_pixels[0], rgba_pixels(&eased_frames[0]));
    assert_eq!(gif_pixels[8], rgba_pixels(&eased_frames[8]));
}
