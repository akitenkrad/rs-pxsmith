//! `pxsmith watch` — 保存を検知して描き直す (M1)．
//!
//! 完了条件は「**保存から端末表示まで 1 秒未満**」なので，毎回かかった時間を
//! 表示して測れるようにしてある．
//!
//! ディレクトリを監視するのが要点である．`pxsmith` の書き出しは一時ファイルを作って
//! `rename` する (設計書 3.7) ので，ファイル自体を監視すると inode が入れ替わって
//! 通知が来なくなる．エディタの保存も多くが同じ方式を採る．

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use pxsmith_view::render::RenderOptions;
use pxsmith_view::term::Placement;

/// 保存が落ち着くまで待つ時間．エディタは 1 回の保存で複数のイベントを出す．
const DEBOUNCE: Duration = Duration::from_millis(60);

pub fn run(path: &Path, opts: &RenderOptions, frame_index: usize) -> Result<()> {
    let kind = pxsmith_view::detect();
    eprintln!("{}", kind.report());
    if !kind.is_pixel_accurate() {
        eprintln!("警告: このまま続けるが，1 画素の確認には使えない");
    }

    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let target = path
        .canonicalize()
        .with_context(|| format!("{} が見つからない", path.display()))?;

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        let _ = tx.send(res);
    })
    .context("ファイル監視を開始できない")?;
    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("{} を監視できない", dir.display()))?;

    draw(path, opts, frame_index, None);
    eprintln!("監視中: {} (Ctrl-C で終了)", path.display());

    loop {
        let event = match rx.recv() {
            Ok(Ok(e)) => e,
            Ok(Err(e)) => {
                eprintln!("監視エラー: {e}");
                continue;
            }
            // 送信側が落ちた = watcher が死んだ
            Err(_) => return Ok(()),
        };
        if !touches(&event, &target) {
            continue;
        }
        let started = Instant::now();

        // 同じ保存から続けて来るイベントを吸収する
        while rx.recv_timeout(DEBOUNCE).is_ok() {}

        draw(path, opts, frame_index, Some(started));
    }
}

/// このイベントが監視対象に関わるか．
fn touches(event: &Event, target: &Path) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.paths.iter().any(|p| {
        // 書き出し前は存在しないので canonicalize できない．素のパスでも比べる
        p == target || p.canonicalize().is_ok_and(|c| c == *target)
    })
}

fn draw(path: &Path, opts: &RenderOptions, frame_index: usize, since: Option<Instant>) {
    pxsmith_view::term::clear();
    match crate::load_frames(path) {
        Ok(frames) => match frames.get(frame_index) {
            Some(frame) => {
                let img = pxsmith_view::render::to_rgba_image(frame, opts);
                if let Err(e) = pxsmith_view::show(&img, Placement::default()) {
                    eprintln!("表示に失敗: {e}");
                    return;
                }
                let elapsed = since.map(|s| s.elapsed());
                println!(
                    "{} フレーム {} / {} — {}x{}，{} ms，{}{}",
                    path.display(),
                    frame_index,
                    frames.len(),
                    frame.size.x,
                    frame.size.y,
                    frame.duration_ms,
                    frame.kind.as_str(),
                    match elapsed {
                        // 検知から表示まで．デバウンス分を含む
                        Some(d) => format!("  [{} ms で反映]", d.as_millis()),
                        None => String::new(),
                    }
                );
            }
            None => eprintln!("フレーム {frame_index} が無い (全 {} 件)", frames.len()),
        },
        // 編集の途中は構文誤りになる．監視は続ける
        Err(e) => eprintln!("読み込めない: {e:#}"),
    }
}
