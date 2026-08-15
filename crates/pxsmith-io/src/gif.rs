//! GIF の書き出し (設計書 2 章の «pxsmith-io — aseprite / PNG / GIF / palette»)．
//!
//! 用途は**生成過程の GIF** である — 設計書 3 章は «レシピの再現性は差分ビルドが
//! 担保し，生成過程の GIF は差分ビルドの中間成果物を連結して作る» としか
//! 決めていない．ここは «連結する» 側の道具だけを持つ．
//!
//! # 色は 1 つも作らない — これは指標付きモデルの帰結である
//!
//! GIF はフレームごとに**局所カラーテーブル**を持てる．こちらの絵は
//!
//! | こちらの不変条件 | GIF 側の受け皿 |
//! | --- | --- |
//! | 添字は `u8` = 1 パレット 256 色まで (D2) | 局所カラーテーブルは最大 256 色 |
//! | アルファは 2 値 (D4) | 透明は «透明添字 1 つ» で表す |
//!
//! と，**ちょうど噛み合う**．したがって減色も量子化も要らず，
//! **書いた絵がそのまま出る**．`image` クレートの `GifEncoder` は RGBA から
//! 量子化し直すので使わない — «並べ替えるだけの道具は色を作らない» (D94) は
//! 書き出しにも掛かる．
//!
//! カラーテーブルの長さは 2 の冪でなければならないので**足りない分は黒で埋める**
//! (埋めた分は絵が参照しないので見えない) ．
//!
//! # 表示時間は 10 ms 刻みしか持てない
//!
//! GIF の遅延は 1/100 秒単位である．**ms で頼まれた値は丸めるしかない**ので，
//! `pxsmith anim ease` が D40 ・D116 で表示周期を扱ったのと同じ作法で
//! **丸めたことを報告する** ([`GifReport::rounded`]) ．黙って丸めない．

use std::path::Path;

use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::palette::Palette;

use crate::error::{IoError, Result};

/// 書き出す 1 コマ．
#[derive(Clone, Debug)]
pub struct GifFrame {
    pub canvas: IndexedCanvas,
    pub palette: Palette,
    /// 表示時間 (ミリ秒)．**10 ms 刻みに丸められる**．
    pub delay_ms: u32,
    /// 報告に出す名前 (ファイル名など)．
    pub label: String,
}

/// 書き出した結果の素性．
#[derive(Clone, Debug, Default)]
pub struct GifReport {
    pub frames: usize,
    /// 揃えた画布の寸法．
    pub size: (u32, u32),
    /// 画布が足りずに継ぎ足したコマの数．
    pub padded: usize,
    /// 丸めた表示時間 (頼まれた ms, 実際の ms)．**同じ値は 1 度だけ**．
    pub rounded: Vec<(u32, u32)>,
    /// いちばん多い色数．
    pub max_colors: usize,
}

/// 生成過程の GIF を書く．
///
/// 画布の寸法が違うコマは**左上を合わせて継ぎ足す**．
///
/// > [!note] **なぜ左上か．**
/// > 途中で画布が広がる道具がある (`pxsmith anim squash` は 32x32 を 42x32 にする) ．
/// > 中央で合わせると，**広がっていない他のコマまで動いて見える** — 動いて
/// > いないものは動かさない方がよい．左上は座標の原点でもある．
pub fn write_progress(path: &Path, frames: &[GifFrame]) -> Result<GifReport> {
    if frames.is_empty() {
        return Err(IoError::GifNoFrames);
    }
    let width = frames.iter().map(|f| f.canvas.width()).max().unwrap_or(1);
    let height = frames.iter().map(|f| f.canvas.height()).max().unwrap_or(1);

    let mut report = GifReport {
        frames: frames.len(),
        size: (width, height),
        ..Default::default()
    };

    let file = std::fs::File::create(path).map_err(|source| IoError::File {
        path: path.to_path_buf(),
        source,
    })?;
    let mut encoder = gif::Encoder::new(
        std::io::BufWriter::new(file),
        width as u16,
        height as u16,
        &[],
    )
    .map_err(|e: gif::EncodingError| IoError::GifEncode {
        detail: e.to_string(),
    })?;
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|e: gif::EncodingError| IoError::GifEncode {
            detail: e.to_string(),
        })?;

    for f in frames {
        if f.canvas.width() != width || f.canvas.height() != height {
            report.padded += 1;
        }
        report.max_colors = report.max_colors.max(f.palette.len());

        // **局所カラーテーブル** — 2 の冪へ切り上げる
        let mut table: Vec<u8> = Vec::with_capacity(f.palette.len() * 3);
        for i in 0..f.palette.len() {
            let c = f
                .palette
                .get(i as u8)
                .unwrap_or(pxsmith_core::color::Rgba8::TRANSPARENT);
            table.extend_from_slice(&[c.r, c.g, c.b]);
        }
        let want = f.palette.len().next_power_of_two().clamp(2, 256);
        table.resize(want * 3, 0);

        // 透明は «透明添字» で表す (アルファが 2 値だから 1 つで足りる．D4)
        let transparent = f.canvas.transparent();
        let fill = transparent.unwrap_or(0);
        let mut buffer = vec![fill; (width * height) as usize];
        for y in 0..f.canvas.height() {
            for x in 0..f.canvas.width() {
                let index = f.canvas.get(x as i32, y as i32).unwrap_or(fill);
                buffer[(y * width + x) as usize] = index;
            }
        }

        let delay = (f.delay_ms as f32 / 10.0).round().max(1.0) as u16;
        let actual = delay as u32 * 10;
        if actual != f.delay_ms && !report.rounded.iter().any(|(w, _)| *w == f.delay_ms) {
            report.rounded.push((f.delay_ms, actual));
        }

        let mut frame = gif::Frame {
            width: width as u16,
            height: height as u16,
            buffer: std::borrow::Cow::Owned(buffer),
            palette: Some(table),
            transparent,
            delay,
            dispose: gif::DisposalMethod::Background,
            ..Default::default()
        };
        frame.make_lzw_pre_encoded();
        encoder
            .write_lzw_pre_encoded_frame(&frame)
            .map_err(|e: gif::EncodingError| IoError::GifEncode {
                detail: e.to_string(),
            })?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxsmith_core::color::Rgba8;

    fn palette(n: u8) -> Palette {
        let mut colors = vec![Rgba8::TRANSPARENT];
        for i in 0..n {
            colors.push(Rgba8::new(10 + i * 8, 20, 30, 255));
        }
        Palette::new(colors).expect("パレット")
    }

    fn frame(w: u32, h: u32, colors: u8, delay_ms: u32) -> GifFrame {
        let mut canvas = IndexedCanvas::filled(w, h, 0).with_transparent(Some(0));
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                canvas.set(x, y, 1 + (x as u8 % colors));
            }
        }
        GifFrame {
            canvas,
            palette: palette(colors),
            delay_ms,
            label: format!("{w}x{h}"),
        }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pxsmith-gif-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("作れる");
        dir.join(name)
    }

    /// **壊れると: 生成過程の GIF が «見た絵と違う色» で出る．**
    ///
    /// 書いた添字がそのまま出ることを，読み直して確かめる．
    #[test]
    fn what_we_wrote_is_what_comes_back_with_no_requantisation() {
        let path = tmp("exact.gif");
        let f = frame(8, 4, 5, 100);
        let want: Vec<u8> = f.canvas.pixels().to_vec();
        let table: Vec<Rgba8> = (0..f.palette.len())
            .map(|i| f.palette.get(i as u8).expect("ある"))
            .collect();
        write_progress(&path, &[f]).expect("書ける");

        let file = std::fs::File::open(&path).expect("開ける");
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options.read_info(file).expect("読める");
        let got = decoder.read_next_frame().expect("読める").expect("1 コマ");
        assert_eq!(got.buffer.as_ref(), want.as_slice(), "添字が変わった");

        // 色そのものも変わっていないこと
        let read_table = decoder.palette().expect("局所表がある");
        for (at, c) in table.iter().enumerate() {
            if c.a == 0 {
                continue;
            }
            assert_eq!(
                (
                    read_table[at * 3],
                    read_table[at * 3 + 1],
                    read_table[at * 3 + 2]
                ),
                (c.r, c.g, c.b),
                "添字 {at} の色が変わった"
            );
        }
    }

    /// **壊れると: 画布の広がったコマで，広がっていないコマまで動いて見える．**
    #[test]
    fn frames_of_different_sizes_line_up_at_the_top_left() {
        let path = tmp("pad.gif");
        let small = frame(8, 4, 3, 100);
        let large = frame(12, 4, 3, 100);
        let report = write_progress(&path, &[small.clone(), large]).expect("書ける");
        assert_eq!(report.size, (12, 4));
        assert_eq!(report.padded, 1, "継ぎ足した数を数えていない");

        let file = std::fs::File::open(&path).expect("開ける");
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options.read_info(file).expect("読める");
        let got = decoder.read_next_frame().expect("読める").expect("1 コマ");
        // 左上 8 画素は元のまま，右の 4 画素は透明
        for x in 0..8usize {
            assert_eq!(got.buffer[x], small.canvas.pixels()[x], "x={x}");
        }
        for x in 8..12usize {
            assert_eq!(got.buffer[x], 0, "継ぎ足した先が透明でない x={x}");
        }
    }

    /// **壊れると: 頼んだ表示時間と実際が黙ってずれる (D40 ・D116 と同じ作法)．**
    #[test]
    fn a_delay_that_does_not_fit_the_ten_millisecond_grid_is_reported() {
        let path = tmp("delay.gif");
        let report = write_progress(
            &path,
            &[frame(4, 4, 2, 100), frame(4, 4, 2, 33), frame(4, 4, 2, 500)],
        )
        .expect("書ける");
        // 100 と 500 は 10 で割り切れるので丸めていない．33 だけが動く
        assert_eq!(report.rounded, vec![(33, 30)]);
    }

    /// **壊れると: 1 コマも無い GIF を «成功» として書く．**
    #[test]
    fn an_empty_sequence_is_an_error() {
        assert!(matches!(
            write_progress(&tmp("empty.gif"), &[]),
            Err(IoError::GifNoFrames)
        ));
    }

    /// **壊れると: 色数がカラーテーブルの 2 の冪に収まらず GIF が壊れる．**
    ///
    /// 添字は `u8` (D2) なので 256 色までしか無い — **構造として収まる**．
    #[test]
    fn a_full_palette_still_fits_the_colour_table() {
        let path = tmp("full.gif");
        let mut colors = vec![Rgba8::TRANSPARENT];
        for i in 0..255u16 {
            colors.push(Rgba8::new(i as u8, (255 - i) as u8, 128, 255));
        }
        let palette = Palette::new(colors).expect("パレット");
        let mut canvas = IndexedCanvas::filled(16, 16, 0).with_transparent(Some(0));
        for y in 0..16i32 {
            for x in 0..16i32 {
                canvas.set(x, y, (y * 16 + x) as u8);
            }
        }
        let report = write_progress(
            &path,
            &[GifFrame {
                canvas,
                palette,
                delay_ms: 100,
                label: "full".into(),
            }],
        )
        .expect("書ける");
        assert_eq!(report.max_colors, 256);
    }
}
