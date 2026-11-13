//! `pxsmith palette cvd` の端から端試験．

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pxsmith_core::cvd::{CvdOptions, check};

fn sweetie() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../palettes/sweetie-16.hex")
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .args(args)
        .output()
        .expect("pxsmith を起こせる")
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn sweetie_report_is_deterministic() {
    let input = sweetie();
    let args = ["palette", "cvd", input.to_str().unwrap()];
    let first = run(&args);
    let second = run(&args);
    assert!(first.status.success(), "{}", output_text(&first));
    assert!(second.status.success(), "{}", output_text(&second));
    assert_eq!(first.stdout, second.stdout);
    let text = String::from_utf8(first.stdout).unwrap();
    assert!(text.contains("protan"));
    assert!(text.contains("deutan"));
}

#[test]
fn a_named_confusion_is_blocking_but_an_unnamed_one_is_advisory() {
    let input = sweetie();
    let palette = pxsmith_io::hex::read(&input).expect("Sweetie 16 を読める");
    let report = check(palette.entries(), &CvdOptions::default());
    let pair = report
        .by_deficiency
        .iter()
        .flat_map(|r| &r.confusions)
        .next()
        .expect("Sweetie 16 に見分けられなくなる組がある");
    let pair_arg = format!("{},{}", pair.a.to_hex_string(), pair.b.to_hex_string());

    let advisory = run(&["palette", "cvd", input.to_str().unwrap()]);
    assert_eq!(
        advisory.status.code(),
        Some(0),
        "{}",
        output_text(&advisory)
    );

    let blocking = run(&[
        "palette",
        "cvd",
        input.to_str().unwrap(),
        "--pair",
        &pair_arg,
    ]);
    assert_eq!(
        blocking.status.code(),
        Some(1),
        "{}",
        output_text(&blocking)
    );
    assert!(output_text(&blocking).contains("blocking"));
}

#[test]
fn a_pair_colour_missing_from_the_palette_is_an_error() {
    let input = sweetie();
    let output = run(&[
        "palette",
        "cvd",
        input.to_str().unwrap(),
        "--pair",
        "000001,ffffff",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", output_text(&output));
    let text = output_text(&output);
    assert!(text.contains("000001"), "{text}");
    assert!(text.contains("パレットに無い"), "{text}");
}

#[test]
fn json_output_is_machine_readable() {
    let input = sweetie();
    let output = run(&["palette", "cvd", input.to_str().unwrap(), "--json"]);
    assert!(output.status.success(), "{}", output_text(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("正しい JSON");
    let threshold = value["threshold"].as_f64().expect("閾値が数値");
    assert!((threshold - 0.02).abs() < 1e-6);
    assert!(value["types"]["protan"].as_array().is_some());
    assert!(value["types"]["deutan"].as_array().is_some());
}
