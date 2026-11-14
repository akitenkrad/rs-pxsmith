use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pxsmith-export-godot-{}-{}",
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

fn write_l0(path: &Path, name: &str, size: (u32, u32), nine_slice: bool) {
    let palette = path.with_extension("hex");
    std::fs::write(&palette, "1a1c2c\nb13e53\n").expect("パレットを書ける");
    let nine_slice = if nine_slice {
        "nine_slice = { left = 1, top = 1, right = 1, bottom = 1 }\n"
    } else {
        ""
    };
    let row = "0".repeat(size.0 as usize);
    let data = std::iter::repeat_n(row, size.1 as usize)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        path,
        format!(
            "[meta]\nformat = 1\nname = \"{name}\"\n{nine_slice}\n\
             [palette]\nref = \"{}\"\n\n\
             [palette.map]\n0 = 0\n\n\
             [[frame]]\nname = \"{name}\"\nkind = \"key\"\nduration_ms = 100\n\
             data = '''\n{data}\n'''\n",
            palette.file_name().expect("名前").to_string_lossy()
        ),
    )
    .expect("L0 を書ける");
}

#[test]
fn sheet_pack_then_godot_export_uses_source_rect_and_reports_omissions() {
    let root = scratch();
    let window = root.join("window.px.toml");
    let plain = root.join("plain.px.toml");
    write_l0(&window, "window", (4, 4), true);
    write_l0(&plain, "plain", (6, 5), false);

    let image = root.join("ui.png");
    let meta = root.join("ui.sheet.json");
    let packed = run(&[
        "sheet".as_ref(),
        "pack".as_ref(),
        image.as_os_str(),
        "--input".as_ref(),
        window.as_os_str(),
        plain.as_os_str(),
        "--meta".as_ref(),
        meta.as_os_str(),
    ]);
    assert_success(&packed);

    let text = std::fs::read_to_string(&meta).expect("シートメタを読める");
    let sheet = pxsmith_core::SheetDoc::from_json(&text).expect("シートメタを解釈できる");
    let window_cell = sheet
        .cells
        .iter()
        .find(|cell| cell.name == "window")
        .expect("窓の升がある");
    assert_ne!((window_cell.w, window_cell.h), (sheet.cell_w, sheet.cell_h));

    let out_dir = root.join("styleboxes");
    let exported = run(&[
        "export".as_ref(),
        "godot".as_ref(),
        "--sheet".as_ref(),
        meta.as_os_str(),
        "--out-dir".as_ref(),
        out_dir.as_os_str(),
        "--res-dir".as_ref(),
        "res://ui/".as_ref(),
    ]);
    assert_success(&exported);

    let stylebox = std::fs::read_to_string(out_dir.join("window.stylebox.tres"))
        .expect("9-slice の StyleBox がある");
    assert!(
        stylebox.contains(&format!(
            "region_rect = Rect2({}, {}, {}, {})",
            window_cell.x, window_cell.y, window_cell.w, window_cell.h
        )),
        "元画像の矩形を使っていない:\n{stylebox}"
    );
    assert!(stylebox.contains("path=\"res://ui/ui.png\""));
    assert!(!out_dir.join("plain.stylebox.tres").exists());
    let stdout = String::from_utf8_lossy(&exported.stdout);
    assert!(
        stdout.contains("nine_slice を持たない升 (1 件)"),
        "{stdout}"
    );
    assert!(stdout.contains("Theme (1 件)"), "{stdout}");
}
