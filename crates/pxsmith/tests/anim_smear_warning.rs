//! `anim smear` の切断警告が `--align` と入力形状を反映することの回帰試験
//! (MYTASK-3579)．

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn scratch(name: &str) -> PathBuf {
    let root = repo("target/anim-smear-warning-tests").join(format!(
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

fn write_end(path: &Path, data: &str) {
    let text = format!(
        r#"[meta]
format = 1
name = "end"

[palette]
ref = "end.hex"

[palette.map]
"." = "transparent"
a = 0

[[frame]]
name = "f"
data = '''
{data}
'''
"#
    );
    std::fs::write(path, text).expect("端の形を書ける");
}

fn run_smear(align: &str) -> Output {
    let root = scratch(align);
    std::fs::write(root.join("end.hex"), "b13e53\n").expect("パレットを書ける");
    let from = root.join("from.px.toml");
    let to = root.join("to.px.toml");
    let output = root.join("out.px.toml");
    write_end(
        &from,
        "a.....a.......\n..............\n..............\n..............",
    );
    write_end(
        &to,
        "..............\n..............\n..............\n.......a.....a",
    );

    Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .args([
            "anim",
            "smear",
            "--from",
            from.to_str().unwrap(),
            "--to",
            to.to_str().unwrap(),
            "--base",
            "b13e53",
            "--align",
            align,
            output.to_str().unwrap(),
        ])
        .output()
        .expect("pxsmith を起こせる")
}

fn stdout_of(output: &Output) -> String {
    assert!(
        output.status.success(),
        "pxsmith が失敗した:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).expect("標準出力は UTF-8")
}

#[test]
fn centroid_disconnect_explains_input_components_and_recovery() {
    let stdout = stdout_of(&run_smear("centroid"));

    assert!(stdout.contains("重心を合わせても繋がらない"), "{stdout}");
    assert!(stdout.contains("入力の成分数が A=2・B=2"), "{stdout}");
    assert!(stdout.contains("離れた部品"), "{stdout}");
    assert!(
        stdout.contains("キーを足すか，部品ごとに伸ばす"),
        "{stdout}"
    );
    assert!(!stdout.contains("--align none"), "{stdout}");
    assert!(!stdout.contains("--align centroid にすること"), "{stdout}");
}

#[test]
fn none_disconnect_keeps_the_containment_theorem_guidance() {
    let stdout = stdout_of(&run_smear("none"));

    assert!(
        stdout.contains("--align none は包含定理から «2 枚の和集合» にしかならない"),
        "{stdout}"
    );
    assert!(stdout.contains("--align centroid にすること"), "{stdout}");
}
