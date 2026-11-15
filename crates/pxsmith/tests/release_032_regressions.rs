//! 0.3.2 で直す CLI 回帰群 (MYTASK-3581〜3588)．

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("target/release-032-regressions")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("作業場所を作れる");
    root
}

fn px<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
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

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "pxsmith が失敗した:\n{}",
        text(output)
    );
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

fn tally(output: &Output) -> (usize, usize) {
    assert_ok(output);
    let report = text(output);
    let line = report
        .lines()
        .find(|line| line.contains("キャッシュ当たり"))
        .unwrap_or_else(|| panic!("集計が無い:\n{report}"));
    let numbers: Vec<usize> = line
        .split_whitespace()
        .filter_map(|word| {
            word.trim_matches(|c: char| !c.is_ascii_digit())
                .parse()
                .ok()
        })
        .collect();
    (numbers[1], numbers[2])
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

#[test]
fn mytask_3581_l0_round_trips_do_not_grow_the_palette() {
    let root = scratch("3581");
    let input = write_l0(&root, "source", SWEETIE_16, "a = 0\nb = 1", ".ab.");
    let once = root.join("once.px.toml");
    let twice = root.join("twice.px.toml");

    assert_ok(&px([
        "scale".as_ref(),
        "--factor".as_ref(),
        "1".as_ref(),
        input.as_os_str(),
        once.as_os_str(),
    ]));
    assert_ok(&px([
        "scale".as_ref(),
        "--factor".as_ref(),
        "1".as_ref(),
        once.as_os_str(),
        twice.as_os_str(),
    ]));

    let expected = std::fs::read(root.join("source.hex")).expect("元を読める");
    assert_eq!(std::fs::read(root.join("once.px.hex")).unwrap(), expected);
    assert_eq!(std::fs::read(root.join("twice.px.hex")).unwrap(), expected);
    let lines = String::from_utf8(expected).unwrap();
    assert_eq!(lines.lines().count(), 16);
    assert!(!lines.lines().any(|line| line == "000000"));
}

#[test]
fn mytask_3581_shading_chain_keeps_opaque_palette_entries() {
    let root = scratch("3581-chain");
    let input = write_l0(&root, "mage", SWEETIE_16, MAGE_MAP, MAGE_DATA);
    let smoothed = root.join("smoothed.px.toml");
    let aa = root.join("aa.px.toml");
    let outlined = root.join("outlined.px.toml");

    assert_ok(&px([
        "smooth".as_ref(),
        input.as_os_str(),
        smoothed.as_os_str(),
    ]));
    assert_ok(&px([
        "aa".as_ref(),
        smoothed.as_os_str(),
        aa.as_os_str(),
        "--max-new-colors".as_ref(),
        "2".as_ref(),
    ]));
    assert_ok(&px([
        "outline".as_ref(),
        aa.as_os_str(),
        outlined.as_os_str(),
        "--style".as_ref(),
        "shaded".as_ref(),
        "--light".as_ref(),
        "dir:-0.6,0.8".as_ref(),
        "--max-new-colors".as_ref(),
        "2".as_ref(),
    ]));

    let smoothed_hex = std::fs::read_to_string(root.join("smoothed.px.hex")).unwrap();
    let aa_hex = std::fs::read_to_string(root.join("aa.px.hex")).unwrap();
    let outlined_hex = std::fs::read_to_string(root.join("outlined.px.hex")).unwrap();
    assert_eq!(smoothed_hex, SWEETIE_16);
    assert_eq!(aa_hex, SWEETIE_16);
    assert_eq!(outlined_hex, format!("{SWEETIE_16}0f1735\n"));
    assert!(!outlined_hex.lines().any(|line| line == "000000"));
}

#[test]
fn mytask_3581_compose_l0_omits_transparent_palette_entry() {
    let root = scratch("3581-compose");
    let body = write_l0(&root, "body", "112233\n445566\n", "a = 0", "a.");
    let hat = write_l0(&root, "hat", "aabbcc\nddeeff\n", "a = 0", ".a");
    let output = root.join("composed.px.toml");

    assert_ok(&px([
        "compose".as_ref(),
        output.as_os_str(),
        "--clip".as_ref(),
        "--part".as_ref(),
        body.as_os_str(),
        "--part".as_ref(),
        hat.as_os_str(),
    ]));

    let palette = std::fs::read_to_string(root.join("composed.px.hex")).unwrap();
    assert_eq!(palette.lines().count(), 2, "{palette}");
    assert!(palette.lines().any(|line| line == "112233"), "{palette}");
    assert!(palette.lines().any(|line| line == "aabbcc"), "{palette}");
    assert!(!palette.lines().any(|line| line == "000000"), "{palette}");
}

#[test]
fn mytask_3582_non_input_fields_and_l0_palettes_invalidate_the_cache() {
    let root = scratch("3582");
    let body = write_l0(&root, "body", "ff0000\n", "a = 0", "a.");
    let hat = write_l0(&root, "hat", "0000ff\n", "a = 0", ".a");
    let recipe = root.join("build.toml");
    std::fs::write(
        &recipe,
        format!(
            r#"[project]
format = 1
[[step]]
op = "compose"
output = "out.px.toml"
part = ["{}", "{}"]
clip = true
"#,
            body.file_name().unwrap().to_string_lossy(),
            hat.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();

    assert_eq!(tally(&px(["run", recipe.to_str().unwrap()])), (0, 1));
    assert_eq!(tally(&px(["run", recipe.to_str().unwrap()])), (1, 0));

    let hat_text = std::fs::read_to_string(&hat).unwrap().replace(".a", "a.");
    std::fs::write(&hat, hat_text).unwrap();
    assert_eq!(tally(&px(["run", recipe.to_str().unwrap()])), (0, 1));
    assert_eq!(tally(&px(["run", recipe.to_str().unwrap()])), (1, 0));

    std::fs::write(root.join("hat.hex"), "00ff00\n").unwrap();
    assert_eq!(tally(&px(["run", recipe.to_str().unwrap()])), (0, 1));
}

#[test]
fn mytask_3582_secondary_outputs_form_edges_and_are_restored() {
    let root = scratch("3582-secondary-outputs");
    let sprite = write_l0(&root, "sprite", "ff0000\n00ff00\n", "a = 0\nb = 1", "ab");
    let recipe = root.join("build.toml");
    std::fs::write(
        &recipe,
        format!(
            r#"[project]
format = 1

[[step]]
op = "sheet.pack"
input = "{}"
output = "out/sheet.png"
meta = "out/sheet.sheet.json"
palette_swap = true

[[step]]
op = "sheet.resolve"
index = "out/sheet.png"
meta = "out/sheet.sheet.json"
output = "out/resolved.png"
"#,
            sprite.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();

    let first = px(["run", recipe.to_str().unwrap()]);
    assert_eq!(tally(&first), (0, 2));
    assert!(
        !text(&first).contains("output に書いていない"),
        "{}",
        text(&first)
    );

    let meta = root.join("out/sheet.sheet.json");
    let atlas = root.join("out/sheet.palettes.png");
    assert!(meta.is_file());
    assert!(atlas.is_file());
    std::fs::remove_file(&meta).unwrap();
    std::fs::remove_file(&atlas).unwrap();

    let second = px(["run", recipe.to_str().unwrap()]);
    assert_eq!(tally(&second), (2, 0));
    assert!(meta.is_file(), "meta がキャッシュから戻らなかった");
    assert!(
        atlas.is_file(),
        "パレットアトラスがキャッシュから戻らなかった"
    );
    assert!(
        !text(&second).contains("output に書いていない"),
        "{}",
        text(&second)
    );
}

#[test]
fn mytask_3583_palette_convert_writes_gpl_and_round_trips() {
    let root = scratch("3583");
    let source = root.join("aurora.hex");
    let gpl = root.join("aurora.gpl");
    let back = root.join("back.hex");
    std::fs::write(&source, "1a1c2c\n5d275d\nb13e53\n").unwrap();

    assert_ok(&px([
        "palette",
        "convert",
        source.to_str().unwrap(),
        gpl.to_str().unwrap(),
    ]));
    let body = std::fs::read_to_string(&gpl).unwrap();
    assert!(
        body.starts_with("GIMP Palette\nName: aurora\nColumns: 0\n#\n"),
        "{body}"
    );
    assert!(body.contains("26 28 44\t#1a1c2c"), "{body}");
    assert_ok(&px([
        "palette",
        "convert",
        gpl.to_str().unwrap(),
        back.to_str().unwrap(),
    ]));
    assert_eq!(
        std::fs::read(&back).unwrap(),
        std::fs::read(&source).unwrap()
    );

    let unsupported = px([
        "palette",
        "convert",
        source.to_str().unwrap(),
        root.join("out.pal").to_str().unwrap(),
    ]);
    assert!(!unsupported.status.success());
}

#[test]
fn mytask_3584_gpl_requires_a_header_and_valid_colour_rows() {
    let root = scratch("3584");
    for (name, body) in [
        ("headerless.gpl", "1 2 3 colour\n"),
        ("garbage.gpl", "GIMP Palette\nName: bad\nnot a colour\n"),
        ("empty.gpl", "GIMP Palette\nName: empty\n# none\n"),
    ] {
        let input = root.join(name);
        std::fs::write(&input, body).unwrap();
        let output = px([
            "palette",
            "convert",
            input.to_str().unwrap(),
            root.join(format!("{name}.hex")).to_str().unwrap(),
        ]);
        assert!(
            !output.status.success(),
            "{name} が通った:\n{}",
            text(&output)
        );
        let report = text(&output);
        assert!(
            report.contains("先頭") || report.contains(":3:") || report.contains("1 つも"),
            "{report}"
        );
    }
}

#[test]
fn mytask_3585_png_input_gets_actionable_guidance() {
    let root = scratch("3585-png");
    let png = root.join("input.png");
    let image = pxsmith_core::RgbaCanvas::filled(1, 1, pxsmith_core::Rgba8::rgb(1, 2, 3));
    pxsmith_io::png::write_rgba(&png, &image).unwrap();
    let output = px([
        "scale",
        "--factor",
        "2",
        png.to_str().unwrap(),
        root.join("out.png").to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let report = text(&output);
    assert!(
        report.contains("PNG なのでインデックスカラーの絵として直接読めない"),
        "{report}"
    );
    assert!(
        report.contains("quantize") && report.contains("palette apply"),
        "{report}"
    );
}

#[test]
fn mytask_3585_progress_captures_png_outputs() {
    let root = scratch("3585-progress");
    write_l0(&root, "source", "112233\n", "a = 0", "aa");
    std::fs::write(
        root.join("build.toml"),
        r#"[project]
format = 1
[[step]]
op = "scale"
input = "source.px.toml"
output = "out.png"
factor = 2
"#,
    )
    .unwrap();
    let output = px([
        "run",
        root.join("build.toml").to_str().unwrap(),
        "--progress",
        root.join("progress.gif").to_str().unwrap(),
    ]);
    assert_ok(&output);
    assert!(text(&output).contains("1 コマ"), "{}", text(&output));
}

#[test]
fn mytask_3586_centroid_smear_stays_inside_the_canvas() {
    let root = scratch("3586");
    let from = write_l0(
        &root,
        "from",
        "b13e53\n",
        "a = 0",
        "................\n.aaa............\n.aaa............\n.aaa............\n................",
    );
    let to = write_l0(
        &root,
        "to",
        "b13e53\n",
        "a = 0",
        "................\n.aaa........aaa.\n.aaa........aaa.\n.aaa........aaa.\n................",
    );
    let output = px([
        "anim".as_ref(),
        "smear".as_ref(),
        "--from".as_ref(),
        from.as_os_str(),
        "--to".as_ref(),
        to.as_os_str(),
        "--base".as_ref(),
        "b13e53".as_ref(),
        "--samples".as_ref(),
        "3".as_ref(),
        root.join("out.px.toml").as_os_str(),
    ]);
    assert_ok(&output);
}

#[test]
fn mytask_3587_reports_when_rule_7_has_too_few_samples() {
    let root = scratch("3587");
    let source = write_l0(
        &root,
        "east",
        "202020\ne0e0e0\n",
        "a = 0\nb = 1",
        "aaaaaa\naabbaa\naabbaa\naabbaa\naabbaa\naaaaaa",
    );
    let output = px([
        "direction".as_ref(),
        "--from".as_ref(),
        OsStr::new(&format!("e={}", source.display())),
        "--light".as_ref(),
        "dir:-0.6,0.8".as_ref(),
        root.join("${dir}.px.toml").as_os_str(),
    ]);
    assert_ok(&output);
    let report = text(&output);
    assert!(report.contains("ルール 7 を検査していない: n="), "{report}");
    assert!(report.contains("<64"), "{report}");

    let help = px(["lint", "--help"]);
    assert_ok(&help);
    assert!(text(&help).contains("--light"), "{}", text(&help));
}

#[test]
fn mytask_3588_dry_run_validates_operations_and_field_names() {
    let root = scratch("3588");
    write_l0(&root, "source", "112233\n", "a = 0", "a");
    for (name, body, needle) in [
        (
            "field.toml",
            "[project]\nformat=1\n[[step]]\nop='scale'\ninput='source.px.toml'\noutput='out.png'\nfactr=2\n",
            "factr",
        ),
        (
            "op.toml",
            "[project]\nformat=1\n[[step]]\nop='scael'\ninput='source.px.toml'\noutput='out.png'\nfactor=2\n",
            "scael",
        ),
    ] {
        let recipe = root.join(name);
        std::fs::write(&recipe, body).unwrap();
        let output = px(["run", recipe.to_str().unwrap(), "--dry-run"]);
        assert!(
            !output.status.success(),
            "{name} が通った:\n{}",
            text(&output)
        );
        assert!(text(&output).contains(needle), "{}", text(&output));
    }
}

#[test]
fn mytask_3588_light_help_explains_direction_and_coordinates() {
    for args in [
        &["shade", "--help"][..],
        &["outline", "--help"][..],
        &["direction", "--help"][..],
        &["anim", "tween", "--help"][..],
        &["anim", "smear", "--help"][..],
        &["lint", "--help"][..],
        &["tileset", "extract", "--help"][..],
        &["tileset", "autotile", "--help"][..],
    ] {
        let output = px(args);
        assert_ok(&output);
        let help = text(&output);
        for needle in [
            "光が進む向き",
            "x は右",
            "y は下",
            "右上から",
            "左上から",
            "point:x,y",
        ] {
            assert!(
                help.contains(needle),
                "{} の help に '{needle}' が無い:\n{help}",
                args.join(" ")
            );
        }
    }
}
