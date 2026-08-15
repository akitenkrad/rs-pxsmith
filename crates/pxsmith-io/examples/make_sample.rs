//! 自前で書き出した `.aseprite` の見本を作る．
//!
//! ```sh
//! cargo run -p pxsmith-io --example make_sample
//! ```
//!
//! 出力先は `testdata/generated/` である．**`testdata/aseprite/` には置かない** —
//! あちらは Aseprite 本体が書いた実ファイル専用で，未知チャンク・タイルマップ・
//! リンクセルを含むものでなければ M0 の往復検証にならない (R3)．自前で書いた
//! ファイルを混ぜると，検証が通ったように見えて実は何も確かめていないことになる．

use pxsmith_io::Document;
use pxsmith_io::aseprite::{AsepriteFile, Color, ColorMode, Pixels};

fn color(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color {
        r,
        g,
        b,
        a,
        name: None,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut raw = AsepriteFile::new(16, 16, ColorMode::Indexed);
    raw.set_palette(&[
        color(0, 0, 0, 0),
        color(0x1a, 0x1c, 0x2c, 255),
        color(0xb1, 0x3e, 0x53, 255),
        color(0xef, 0x7d, 0x57, 255),
        color(0xff, 0xcd, 0x75, 255),
    ])?;
    raw.set_transparent_index(0);

    let group = raw.add_group("hero");
    let body = raw.add_layer_in("body", group);
    let face = raw.add_layer_in("face", group);

    let f0 = raw.add_frame(83);
    let f1 = raw.add_frame(83);

    // 胴体: 8x8 の塗り
    let mut torso = vec![2u8; 8 * 8];
    for x in 0..8 {
        torso[x] = 1;
        torso[7 * 8 + x] = 1;
    }
    raw.set_cel(
        body,
        f0,
        Pixels::new(torso, 8, 8, ColorMode::Indexed)?,
        4,
        6,
    )?;
    // 2 コマ目は同じ絵なのでリンクセルにする
    raw.set_linked_cel(body, f1, f0)?;

    // 顔: 目だけ動かす
    for (frame, eye) in [(f0, 1usize), (f1, 2usize)] {
        let mut data = vec![0u8; 6 * 4];
        data[6 + eye] = 4;
        data[6 + eye + 3] = 4;
        raw.set_cel(
            face,
            frame,
            Pixels::new(data, 6, 4, ColorMode::Indexed)?,
            5,
            2,
        )?;
    }

    let doc = Document::from_raw(raw);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/generated");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("sample.aseprite");
    doc.write(&path)?;
    println!("{} を書き出した", path.display());
    Ok(())
}
