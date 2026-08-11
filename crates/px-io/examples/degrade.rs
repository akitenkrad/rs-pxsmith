//! 格子推定の評価用に，きれいなドット絵を「劣化」させる (実装計画書 M2)．
//!
//! ```sh
//! cargo run -p px-io --example degrade -- <入力> <出力.png> \
//!     --scale 6 --filter bilinear --phase 2,3 --resize 1.3
//! ```
//!
//! 評価データセット (合成 500 件) の劣化要因は次の 5 つである．
//!
//! | 要因 | 水準 |
//! | --- | --- |
//! | 拡大率 $s$ | 2, 3, 4, 6, 8, 12 |
//! | 補間法 | nearest, bilinear, bicubic, lanczos |
//! | 非整数倍リサイズ | なし / 1.3 倍 / 0.85 倍 |
//! | 圧縮 | PNG / JPEG 品質 95, 80, 60 |
//! | 位相 | $d_x, d_y$ をランダムにずらしてクロップ |
//!
//! ここで扱うのは前 3 つと位相である．JPEG は `image` の JPEG エンコーダを通す
//! 別の手順になるので，データセット生成器側で行う．

use std::path::PathBuf;

use image::imageops::FilterType;
use px_io::png;

fn usage() -> ! {
    eprintln!(
        "使い方: degrade <入力> <出力.png> [--scale N] [--filter nearest|bilinear|bicubic|lanczos] \
         [--phase DX,DY] [--resize 倍率]"
    );
    std::process::exit(2)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        usage();
    }
    let input = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[1]);

    let mut scale = 4u32;
    let mut filter = FilterType::Nearest;
    let mut phase = (0u32, 0u32);
    let mut resize = 1.0f32;

    let mut i = 2;
    while i < args.len() {
        let value = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--scale" => scale = value.parse()?,
            "--filter" => {
                filter = match value.as_str() {
                    "nearest" => FilterType::Nearest,
                    "bilinear" => FilterType::Triangle,
                    "bicubic" => FilterType::CatmullRom,
                    "lanczos" => FilterType::Lanczos3,
                    other => {
                        eprintln!("未知の補間法: {other}");
                        usage()
                    }
                }
            }
            "--phase" => {
                let (a, b) = value.split_once(',').unwrap_or(("0", "0"));
                phase = (a.parse()?, b.parse()?);
            }
            "--resize" => resize = value.parse()?,
            other => {
                eprintln!("未知の引数: {other}");
                usage()
            }
        }
        i += 2;
    }

    // 入力を RGBA として読む (.aseprite / .px.toml も通す)
    let src = if input.extension().and_then(|e| e.to_str()) == Some("png") {
        png::read_rgba(&input)?
    } else {
        let doc = px_io::Document::read(&input)?;
        let frame = doc.project(px_io::FrameId(0))?;
        let canvas = frame
            .layers
            .iter()
            .find_map(|l| l.surface.as_indexed())
            .ok_or("インデックスカラーのレイヤが無い")?;
        png::resolve(canvas, &frame.palette)
    };

    let mut buf = image::RgbaImage::new(src.width(), src.height());
    for (i, p) in buf.pixels_mut().enumerate() {
        let c = src.pixels()[i];
        *p = image::Rgba([c.r, c.g, c.b, c.a]);
    }

    // 1. 整数倍に拡大
    let big = image::imageops::resize(&buf, src.width() * scale, src.height() * scale, filter);
    // 2. 非整数倍のリサイズ (格子を壊す要因)
    let resized = if (resize - 1.0).abs() > f32::EPSILON {
        let w = ((big.width() as f32) * resize).round().max(1.0) as u32;
        let h = ((big.height() as f32) * resize).round().max(1.0) as u32;
        image::imageops::resize(&big, w, h, filter)
    } else {
        big
    };
    // 3. 位相をずらしてクロップ
    let (dx, dy) = (
        phase.0.min(resized.width() - 1),
        phase.1.min(resized.height() - 1),
    );
    let cropped = image::imageops::crop_imm(
        &resized,
        dx,
        dy,
        resized.width() - dx,
        resized.height() - dy,
    )
    .to_image();

    let pixels: Vec<px_io::px_core::Rgba8> = cropped
        .pixels()
        .map(|p| px_io::px_core::Rgba8::new(p.0[0], p.0[1], p.0[2], p.0[3]))
        .collect();
    let out = px_io::px_core::RgbaCanvas::from_pixels(cropped.width(), cropped.height(), pixels)?;
    png::write_rgba(&output, &out)?;

    println!(
        "{} -> {} ({}x{}，{scale} 倍 / 位相 ({dx},{dy}) / リサイズ {resize})",
        input.display(),
        output.display(),
        out.width(),
        out.height()
    );
    Ok(())
}
