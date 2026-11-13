//! 透明を使わない L0 が `.aseprite` 往復で消えないことを実プロセスで確かめる．

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn scratch() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-text-opaque-roundtrip-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("作業場所を作れる");
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

#[test]
fn opaque_l0_data_survives_import_and_export_through_aseprite() {
    let root = scratch();
    let input = repo("testdata/lint-cases/negative/jaggy-00.png");
    let l0 = root.join("source.px.toml");
    let aseprite = root.join("middle.aseprite");
    let back = root.join("back.px.toml");
    let back_palette = root.join("back.hex");

    assert_success(&run(&[
        "quantize".as_ref(),
        input.as_os_str(),
        l0.as_os_str(),
        "--colors".as_ref(),
        "62".as_ref(),
    ]));
    let source = pxsmith_io::l0::L0Document::read(&l0).expect("量子化した L0 を読める");
    assert!(!source.frames[0].data.contains('.'), "入力は全画素不透明");

    assert_success(&run(&[
        "text".as_ref(),
        "import".as_ref(),
        l0.as_os_str(),
        aseprite.as_os_str(),
    ]));
    assert_success(&run(&[
        "text".as_ref(),
        "export".as_ref(),
        aseprite.as_os_str(),
        back.as_os_str(),
        "--palette".as_ref(),
        back_palette.as_os_str(),
    ]));

    let back = pxsmith_io::l0::L0Document::read(&back).expect("往復後の L0 を読める");
    assert_eq!(back.frames[0].data, source.frames[0].data);
}
