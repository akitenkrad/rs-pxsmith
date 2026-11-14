use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::Rgba8;

fn scratch() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-nineslice-cli-{}-{}",
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

fn write_fixture(root: &Path, stretch: &str, data: &str) -> PathBuf {
    std::fs::write(root.join("window.hex"), "1a1c2c\nb13e53\n38b764\nfee761\n")
        .expect("パレットを書ける");
    let input = root.join("window.px.toml");
    std::fs::write(
        &input,
        format!(
            r#"[meta]
format = 1
name = "window"
nine_slice = {{ left = 1, top = 1, right = 1, bottom = 1{stretch} }}

[palette]
ref = "window.hex"

[palette.map]
"." = "transparent"
0 = 0
1 = 1
2 = 2
3 = 3

[[frame]]
name = "window"
kind = "key"
duration_ms = 100
data = '''
{data}
'''
"#
        ),
    )
    .expect("L0 を書ける");
    input
}

#[test]
fn preview_writes_one_path_or_size_suffixed_paths_and_keeps_transparency() {
    let root = scratch();
    let input = write_fixture(&root, "", "0120\n1201\n2012\n012.");
    let single = root.join("single.png");
    let output = run(&[
        "nineslice".as_ref(),
        "preview".as_ref(),
        input.as_os_str(),
        single.as_os_str(),
        "--size".as_ref(),
        "7x7".as_ref(),
    ]);
    assert_success(&output);
    let image = pxsmith_io::png::read_rgba(&single).expect("単一出力を読める");
    assert_eq!((image.width(), image.height()), (7, 7));
    assert!(
        image.pixels().contains(&Rgba8::TRANSPARENT),
        "透明添字が PNG で透明になっていない"
    );

    let base = root.join("window-preview.png");
    let output = run(&[
        "nineslice".as_ref(),
        "preview".as_ref(),
        input.as_os_str(),
        base.as_os_str(),
        "--size".as_ref(),
        "7x7".as_ref(),
        "--size".as_ref(),
        "6x5".as_ref(),
    ]);
    assert_success(&output);
    assert!(root.join("window-preview-7x7.png").is_file());
    assert!(root.join("window-preview-6x5.png").is_file());
    assert!(!base.exists(), "複数指定で接尾辞なしの出力を作らない");
}

#[test]
fn preview_rejects_a_target_smaller_than_the_margins() {
    let root = scratch();
    let input = write_fixture(&root, "", "0000\n0000\n0000\n0000");
    let output = run(&[
        "nineslice".as_ref(),
        "preview".as_ref(),
        input.as_os_str(),
        root.join("too-small.png").as_os_str(),
        "--size".as_ref(),
        "1x4".as_ref(),
    ]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("描画先"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn seam_advice_is_emitted_per_repeating_axis_but_does_not_fail() {
    let root = scratch();
    let input = write_fixture(
        &root,
        ", stretch_x = \"tile\", stretch_y = \"tile_fit\"",
        "0120\n1201\n2012\n0123",
    );
    let output = run(&[
        "nineslice".as_ref(),
        "preview".as_ref(),
        input.as_os_str(),
        root.join("seams.png").as_os_str(),
        "--size".as_ref(),
        "7x7".as_ref(),
    ]);
    assert_success(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("横軸") && stderr.contains("縦軸"),
        "{stderr}"
    );
    assert!(stderr.contains("タイルの継ぎ目が見える"), "{stderr}");

    let seamless = write_fixture(&root, ", stretch = \"tile\"", "0000\n0000\n0000\n0000");
    let output = run(&[
        "nineslice".as_ref(),
        "preview".as_ref(),
        seamless.as_os_str(),
        root.join("seamless.png").as_os_str(),
        "--size".as_ref(),
        "7x7".as_ref(),
    ]);
    assert_success(&output);
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("継ぎ目"),
        "継ぎ目が一致する素材へ助言を出さない"
    );
}

#[test]
fn sheet_pack_copies_nine_slice_to_format_one_and_two_cells() {
    let root = scratch();
    let input = write_fixture(
        &root,
        ", stretch_x = \"tile\", stretch_y = \"tile_fit\"",
        "0120\n1201\n2012\n0123",
    );
    for (name, palette_swap) in [("plain", false), ("indexed", true)] {
        let image = root.join(format!("{name}.png"));
        let meta = root.join(format!("{name}.sheet.json"));
        let mut args = vec![
            "sheet".as_ref(),
            "pack".as_ref(),
            image.as_os_str(),
            "--input".as_ref(),
            input.as_os_str(),
            "--meta".as_ref(),
            meta.as_os_str(),
        ];
        if palette_swap {
            args.push("--palette-swap".as_ref());
        }
        assert_success(&run(&args));
        let text = std::fs::read_to_string(&meta).expect("シートメタを読める");
        let json: serde_json::Value = serde_json::from_str(&text).expect("JSON を読める");
        assert_eq!(json["format"], if palette_swap { 2 } else { 1 });
        assert_eq!(
            json["cells"][0]["nine_slice"],
            serde_json::json!({
                "left": 1,
                "top": 1,
                "right": 1,
                "bottom": 1,
                "stretch_x": "tile",
                "stretch_y": "tile_fit"
            })
        );
        let document = pxsmith_core::SheetDoc::from_json(&text).expect("シートメタを読み直せる");
        assert!(document.cells[0].nine_slice.is_some());
    }
}

#[test]
fn sheet_json_without_nine_slice_is_byte_identical_to_the_old_form() {
    let root = scratch();
    let input = write_fixture(&root, "", "0000\n0000\n0000\n0000");
    let text = std::fs::read_to_string(&input)
        .expect("L0 を読める")
        .replace(
            "nine_slice = { left = 1, top = 1, right = 1, bottom = 1 }\n",
            "",
        );
    std::fs::write(&input, text).expect("nine_slice 無しの L0 を書ける");
    let image = root.join("plain-sheet.png");
    let meta = root.join("plain-sheet.sheet.json");
    assert_success(&run(&[
        "sheet".as_ref(),
        "pack".as_ref(),
        image.as_os_str(),
        "--input".as_ref(),
        input.as_os_str(),
        "--meta".as_ref(),
        meta.as_os_str(),
    ]));
    let expected = r#"{
  "format": 1,
  "image": "plain-sheet.png",
  "width": 4,
  "height": 4,
  "columns": 1,
  "rows": 1,
  "cell_w": 4,
  "cell_h": 4,
  "cells": [
    {
      "index": 0,
      "name": "window",
      "x": 0,
      "y": 0,
      "w": 4,
      "h": 4,
      "duration_ms": 100
    }
  ]
}"#;
    assert_eq!(
        std::fs::read_to_string(meta).expect("JSON を読める"),
        expected
    );
}

#[test]
fn text_import_warns_only_when_the_l0_declares_nine_slice() {
    let root = scratch();
    let input = write_fixture(&root, "", "0000\n0000\n0000\n0000");
    let output = run(&[
        "text".as_ref(),
        "import".as_ref(),
        input.as_os_str(),
        root.join("window.aseprite").as_os_str(),
    ]);
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(".aseprite は nine_slice を持てない") && stdout.contains("失われる"),
        "注意表示が無い: {stdout}"
    );

    // 9-slice を宣言していない L0 では何も言わない (雑音にしない)．
    let plain = root.join("plain.px.toml");
    let text = std::fs::read_to_string(&input).expect("素材を読める");
    let without: String = text
        .lines()
        .filter(|line| !line.starts_with("nine_slice"))
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&plain, without).expect("素材を書ける");
    let output = run(&[
        "text".as_ref(),
        "import".as_ref(),
        plain.as_os_str(),
        root.join("plain.aseprite").as_os_str(),
    ]);
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("nine_slice を持てない"),
        "9-slice の無い入力で注意が出た: {stdout}"
    );
}
