//! 0.3.3 で直す CLI 回帰群 (MYTASK-3590・3591)．

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::Rgba8;

fn scratch(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("target/release-033-regressions")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("作業場所を作れる");
    root
}

fn px(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .args(args)
        .output()
        .expect("pxsmith を起こせる")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn ok(args: &[&str]) {
    let output = px(args);
    assert!(
        output.status.success(),
        "pxsmith {args:?} が失敗した:\n{}",
        text(&output)
    );
}

fn s(path: &Path) -> &str {
    path.to_str().expect("UTF-8 のパス")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{} を読めない: {e}", path.display()))
}

fn write_l0(root: &Path, name: &str, palette: &str, map: &str, data: &str) -> PathBuf {
    std::fs::write(root.join(format!("{name}.hex")), palette).expect("パレットを書ける");
    let path = root.join(format!("{name}.px.toml"));
    std::fs::write(
        &path,
        format!(
            r#"[meta]
format = 1
name = "{name}"

[palette]
ref = "{name}.hex"

[palette.map]
"." = "transparent"
{map}

[[frame]]
name = "f"
data = '''
{data}
'''
"#
        ),
    )
    .expect("L0 を書ける");
    path
}

/// L0 を読んで，画素ごとの色 (透明は `None`) を返す．
fn pixels(path: &Path) -> (u32, u32, Vec<Option<Rgba8>>) {
    let doc = pxsmith_io::l0::L0Document::read(path).expect("L0 を読める");
    let frames = doc.to_frames(path).expect("作業層へ変換できる");
    let frame = &frames[0];
    let canvas = frame.layers[0]
        .surface
        .as_indexed()
        .expect("インデックスカラー");
    let mut out = Vec::new();
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            let color = canvas
                .get(x, y)
                .and_then(|index| frame.palette.get(index))
                .filter(|color| color.a != 0);
            out.push(color);
        }
    }
    (canvas.width(), canvas.height(), out)
}

const SWEETIE_16: &str = "1a1c2c\n5d275d\nb13e53\nef7d57\nffcd75\na7f070\n38b764\n257179\n29366f\n3b5dc9\n41a6f6\n73eff7\nf4f4f4\n94b0c2\n566c86\n333c57\n";

const MAGE_MAP: &str = "O = 0\nH = 1\nR = 2\ns = 3\nS = 4\nD = 8\nB = 9\nc = 10\nC = 11\nW = 15";

const MAGE_DATA: &str = "..OH............
...OHH......CC..
....OHHH...CCCc.
....OHHHO..cCcc.
...ORRRRRO..cc..
.OHHHHHHHHHO.W..
.....SOSO....W..
.....sSSs....W..
....OBBBBD...W..
...OBBBBBDDSSW..
...OBBBBDD...W..
..OBBBBBDD...W..
..OBBBBBBDD..W..
.OBBBBBBBDD..W..
.OBBBBBBBBDD.W..
..ODDO.ODDO..W..";

/// `--emit-palette` の `.hex` が同じ実行の `.px.hex` とバイト一致することを確かめ，
/// その中身を返す．
fn assert_emitted_matches_l0(root: &Path, stem: &str) -> String {
    let emitted = read(&root.join(format!("{stem}.emit.hex")));
    let l0 = read(&root.join(format!("{stem}.px.hex")));
    assert_eq!(emitted, l0, "{stem}: --emit-palette と .px.hex が食い違う");
    emitted
}

fn count_black(hex: &str) -> usize {
    hex.lines().filter(|line| *line == "000000").count()
}

/// shade ・aa ・outline を順に掛け，それぞれ `--emit-palette` を出す．
fn run_shape_chain(root: &Path, input: &Path) {
    let shaded = root.join("shaded.px.toml");
    let smoothed = root.join("smoothed.px.toml");
    let aa = root.join("aa.px.toml");
    let outlined = root.join("outlined.px.toml");
    ok(&[
        "shade",
        s(input),
        s(&shaded),
        "--base",
        "3b5dc9",
        "--steps",
        "4",
        "--light",
        "dir:0.6,0.8",
        "--ao",
        "--emit-palette",
        s(&root.join("shaded.emit.hex")),
    ]);
    ok(&["smooth", s(input), s(&smoothed)]);
    ok(&[
        "aa",
        s(&smoothed),
        s(&aa),
        "--max-new-colors",
        "2",
        "--emit-palette",
        s(&root.join("aa.emit.hex")),
    ]);
    ok(&[
        "outline",
        s(&aa),
        s(&outlined),
        "--style",
        "shaded",
        "--light",
        "dir:0.6,0.8",
        "--max-new-colors",
        "2",
        "--emit-palette",
        s(&root.join("outlined.emit.hex")),
    ]);
}

#[test]
fn mytask_3590_emit_palette_matches_l0_palette_without_transparent_rows() {
    let root = scratch("3590");
    let input = write_l0(&root, "mage", SWEETIE_16, MAGE_MAP, MAGE_DATA);
    run_shape_chain(&root, &input);

    for stem in ["shaded", "aa", "outlined"] {
        let hex = assert_emitted_matches_l0(&root, stem);
        assert_eq!(
            count_black(&hex),
            0,
            "{stem}: 透明の枠が 000000 に化けた\n{hex}"
        );
    }
    // outline は既存色の直後に新しい色が続く (間に 000000 が挟まらない)
    let outlined = read(&root.join("outlined.emit.hex"));
    assert!(outlined.starts_with(SWEETIE_16), "{outlined}");
}

#[test]
fn mytask_3590_genuine_black_survives_emit_palette() {
    let root = scratch("3590-black");
    // 本物の 000000 を添字 0 (輪郭) に置く
    let palette = format!("000000\n{}", SWEETIE_16.split_once('\n').unwrap().1);
    let input = write_l0(&root, "mage", &palette, MAGE_MAP, MAGE_DATA);
    run_shape_chain(&root, &input);

    for stem in ["aa", "outlined"] {
        let hex = assert_emitted_matches_l0(&root, stem);
        assert_eq!(
            count_black(&hex),
            1,
            "{stem}: 本物の黒が 1 行だけ残る\n{hex}"
        );
        assert_eq!(
            hex.lines().next(),
            Some("000000"),
            "{stem}: 添字 0 のまま\n{hex}"
        );
    }
    // shade は入力の色を捨てるので黒は来ない — それでも一致はする
    assert_emitted_matches_l0(&root, "shaded");
}

#[test]
fn mytask_3590_palette_extract_drops_transparent_slots_but_keeps_black() {
    let root = scratch("3590-extract");
    let clear = write_l0(&root, "clear", "112233\n445566\n", "a = 0\nb = 1", ".ab.");
    let output = root.join("clear-out.hex");
    ok(&["palette", "extract", "--output", s(&output), s(&clear)]);
    assert_eq!(read(&output), "112233\n445566\n");

    let black = write_l0(&root, "black", "000000\n445566\n", "a = 0\nb = 1", ".ab.");
    let output = root.join("black-out.hex");
    ok(&["palette", "extract", "--output", s(&output), s(&black)]);
    assert_eq!(count_black(&read(&output)), 1);
}

#[test]
fn mytask_3590_text_export_from_aseprite_writes_opaque_palette() {
    let root = scratch("3590-text");
    let input = write_l0(&root, "mage", SWEETIE_16, MAGE_MAP, MAGE_DATA);
    run_shape_chain(&root, &input);
    // 透明の枠がパレットの途中にある .aseprite を作る (outline の出力)
    let aseprite = root.join("outlined.aseprite");
    ok(&[
        "outline",
        s(&root.join("aa.px.toml")),
        s(&aseprite),
        "--style",
        "shaded",
        "--light",
        "dir:0.6,0.8",
        "--max-new-colors",
        "2",
    ]);
    let exported = root.join("exported.px.toml");
    let hex = root.join("exported.hex");
    ok(&[
        "text",
        "export",
        s(&aseprite),
        s(&exported),
        "--palette",
        s(&hex),
    ]);

    let written = read(&hex);
    assert_eq!(count_black(&written), 0, "{written}");
    assert_eq!(written, read(&root.join("outlined.px.hex")));
    assert_eq!(
        pixels(&exported),
        pixels(&root.join("outlined.px.toml")),
        "書き出した L0 の見た目が変わった"
    );
}

/// 陰影の付いた e を描き，`direction` で w を作る．
fn direction(root: &Path, out: &str, extra: &[&str]) -> PathBuf {
    let input = write_l0(root, "mage", SWEETIE_16, MAGE_MAP, MAGE_DATA);
    let east = root.join("east.px.toml");
    if !east.exists() {
        ok(&[
            "shade",
            s(&input),
            s(&east),
            "--base",
            "3b5dc9",
            "--light",
            "dir:-0.6,0.8",
        ]);
    }
    let dir = root.join(out);
    std::fs::create_dir_all(&dir).unwrap();
    let template = format!("{}/${{dir}}.px.toml", s(&dir));
    let from = format!("e={}", s(&east));
    let mut args = vec!["direction", template.as_str(), "--from", from.as_str()];
    args.extend_from_slice(extra);
    args.push("--allow-inconsistent");
    ok(&args);
    dir.join("w.px.toml")
}

/// 左半分と右半分の OKLab 明度の平均．
fn left_right_lightness(path: &Path) -> (f32, f32) {
    let (w, _, pixels) = pixels(path);
    let (mut left, mut right) = ((0.0f32, 0usize), (0.0f32, 0usize));
    for (i, color) in pixels.iter().enumerate() {
        let Some(color) = color else { continue };
        let l = color.to_oklab().l;
        if (i as u32 % w) < w / 2 {
            left.0 += l;
            left.1 += 1;
        } else {
            right.0 += l;
            right.1 += 1;
        }
    }
    (left.0 / left.1 as f32, right.0 / right.1 as f32)
}

#[test]
fn mytask_3591_reshade_uses_the_declared_light() {
    let root = scratch("3591");
    let reshade = ["--reshade", "--base", "3b5dc9"];
    let from_left = direction(
        &root,
        "left",
        &[reshade[0], reshade[1], reshade[2], "--light", "dir:0.6,0.8"],
    );
    let from_right = direction(
        &root,
        "right",
        &[
            reshade[0],
            reshade[1],
            reshade[2],
            "--light",
            "dir:-0.6,0.8",
        ],
    );
    let preset = direction(&root, "preset", &reshade);

    assert_ne!(
        pixels(&from_left),
        pixels(&from_right),
        "--light を変えても塗り直しが変わらない"
    );
    // --light 無しは従来どおりプリセットの既定 (右上から = dir:-0.6,0.8)
    assert_eq!(pixels(&preset), pixels(&from_right));

    // 左上からの光なら左側が明るい
    let (left, right) = left_right_lightness(&from_left);
    assert!(left > right, "左 {left} / 右 {right}");
    let (left, right) = left_right_lightness(&from_right);
    assert!(left < right, "左 {left} / 右 {right}");
}

#[test]
fn mytask_3591_reshade_matches_shade_with_the_same_light() {
    let root = scratch("3591-shade");
    let mirrored = direction(&root, "mirror", &[]);
    for (name, light) in [("left", "dir:0.6,0.8"), ("point", "point:2,2")] {
        let reshaded = direction(
            &root,
            name,
            &["--reshade", "--base", "3b5dc9", "--light", light],
        );
        let shaded = root.join(format!("shaded-{name}.px.toml"));
        ok(&[
            "shade",
            s(&mirrored),
            s(&shaded),
            "--base",
            "3b5dc9",
            "--light",
            light,
        ]);
        assert_eq!(
            pixels(&reshaded),
            pixels(&shaded),
            "{light}: direction --reshade と shade の結果が食い違う"
        );
    }
}
