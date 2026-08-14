//! 生成 1 回ぶんの場所づくりと後始末．
//!
//! # 道具がパレットを書き，モデルは添字だけを書く
//!
//! L0 のパレットは外の `.hex` への参照なので (`px_io::l0::L0PaletteSpec`)，
//! **宣言されたパレットから `.hex` を先に書く**．モデルが触れるのは添字の
//! 文字だけで，**パレットに無い色を出しようがない** — D2 ・D4 ・D94 の
//! «色を作らない» が検査ではなく構造で成り立つ．
//!
//! # 素性は生成物の隣に置く
//!
//! `<name>.gen.json` に，何をどのモデルへ頼んだかを全部残す (設計書 8.3) ．
//! **鍵は書かない** — 素性はコミットするものである．

use std::path::{Path, PathBuf};

use px_core::palette::Palette;
use px_core::{Frame, Rgba8};

use crate::error::{GenError, Result};
use crate::repair::{Generator, Report, generate_with_repair};
use crate::request::{DETERMINISM_NOTE, GenRequest, Provenance};

/// 生成の置き場所．
pub struct Session {
    dir: PathBuf,
    stem: String,
}

impl Session {
    /// `<dir>/<stem>.px.hex` を宣言されたパレットから書き，場所を用意する．
    pub fn prepare(dir: &Path, stem: &str, req: &GenRequest) -> Result<Self> {
        std::fs::create_dir_all(dir).map_err(|source| GenError::Write {
            path: dir.display().to_string(),
            source,
        })?;
        let palette = build_palette(&req.constraints.palette)?;
        let hex = dir.join(format!("{stem}.px.hex"));
        px_io::hex::write(&hex, &palette)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            stem: stem.to_string(),
        })
    }

    /// L0 の置き場所 (まだ書かれていなくてよい)．
    pub fn l0_path(&self) -> PathBuf {
        self.dir.join(format!("{}.px.toml", self.stem))
    }

    /// L0 が書くべきパレット参照の名前．
    pub fn palette_ref(&self) -> String {
        format!("{}.px.hex", self.stem)
    }

    /// 素性の置き場所．
    pub fn provenance_path(&self) -> PathBuf {
        self.dir.join(format!("{}.gen.json", self.stem))
    }

    /// 輪を回し，通ったら L0 と素性を書く．
    pub fn run(
        &self,
        backend: &dyn Generator,
        req: &GenRequest,
        created_at: &str,
    ) -> Result<Report> {
        let report = generate_with_repair(backend, req, &self.l0_path())?;
        if let Some(v) = &report.verified {
            let path = self.l0_path();
            std::fs::write(&path, &v.l0).map_err(|source| GenError::Write {
                path: path.display().to_string(),
                source,
            })?;
            self.write_provenance(req, backend, v.attempts, v.advisory, created_at)?;
        }
        Ok(report)
    }

    fn write_provenance(
        &self,
        req: &GenRequest,
        backend: &dyn Generator,
        attempts: u32,
        advisory: usize,
        created_at: &str,
    ) -> Result<()> {
        let p = Provenance {
            tool: format!("px-gen {}", env!("CARGO_PKG_VERSION")),
            backend: backend.describe(),
            endpoint: req.backend.endpoint.clone(),
            model: req.backend.model.clone(),
            effort: req.effort.as_str().to_string(),
            request_key: req.key(),
            prompt: req.prompt.clone(),
            constraints: req.constraints.clone(),
            attempts,
            advisory,
            determinism: DETERMINISM_NOTE.to_string(),
            created_at: created_at.to_string(),
        };
        let text = serde_json::to_string_pretty(&p).map_err(|e| GenError::BadResponse {
            message: e.to_string(),
        })?;
        let path = self.provenance_path();
        std::fs::write(&path, text).map_err(|source| GenError::Write {
            path: path.display().to_string(),
            source,
        })
    }
}

/// 宣言された `RRGGBB` の並びからパレットを作る．
pub fn build_palette(colors: &[String]) -> Result<Palette> {
    let mut entries = Vec::with_capacity(colors.len());
    for c in colors {
        let rgba = Rgba8::from_hex_str(c).map_err(|_| GenError::BadResponse {
            message: format!("色 '{c}' を読めない (RRGGBB のはず)"),
        })?;
        entries.push(rgba);
    }
    Palette::new(entries).map_err(|e| GenError::BadResponse {
        message: e.to_string(),
    })
}

/// いまの時刻を ISO 8601 (UTC) で．**素性にだけ使い，鍵には混ぜない**．
///
/// 暦の計算は Howard Hinnant の `civil_from_days` — 日付だけのために依存を
/// 1 つ増やすより短い．
pub fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso8601_from_epoch(secs)
}

/// UNIX 秒 → `YYYY-MM-DDTHH:MM:SSZ`．
pub fn iso8601_from_epoch(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 通ったフレームの要約 (報告用)．
pub fn describe_frames(frames: &[Frame]) -> String {
    match frames.first() {
        Some(f) => format!("{} コマ ・{}x{}", frames.len(), f.size.x, f.size.y),
        None => "0 コマ".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Backend, Constraints, Effort, GenKind};

    fn req() -> GenRequest {
        GenRequest {
            kind: GenKind::Prog,
            backend: Backend::anthropic("test"),
            effort: Effort::High,
            prompt: "t".to_string(),
            constraints: Constraints {
                width: 4,
                height: 4,
                palette: vec!["1a1c2c".to_string(), "f4f4f4".to_string()],
                frames: 1,
            },
            max_attempts: 1,
        }
    }

    /// **壊れると: モデルがパレットを書く側になり，色を作れてしまう** (D94)．
    #[test]
    fn the_tool_writes_the_palette_before_the_model_runs() {
        let dir = std::env::temp_dir().join(format!("pxgen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Session::prepare(&dir, "t", &req()).unwrap();
        let hex = dir.join("t.px.hex");
        assert!(hex.exists(), "モデルを呼ぶ前に .hex が無い");
        let text = std::fs::read_to_string(&hex).unwrap();
        assert!(text.contains("1a1c2c"), "宣言した色が入っていない");
        assert_eq!(s.palette_ref(), "t.px.hex");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **壊れると: 読めない色を黙って通し，後段で意味不明に落ちる．**
    #[test]
    fn an_unreadable_colour_is_named() {
        assert!(build_palette(&["ZZZZZZ".to_string()]).is_err());
    }

    /// **壊れると: 素性の日付が狂う (いつ生成したか読めなくなる)．**
    ///
    /// 真値のある場面で固定する — 既知の UNIX 秒と暦の対応は数え上げである．
    #[test]
    fn the_timestamp_matches_known_epoch_values() {
        assert_eq!(iso8601_from_epoch(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_from_epoch(1), "1970-01-01T00:00:01Z");
        // 2000-03-01 — うるう年の境目の翌日
        assert_eq!(iso8601_from_epoch(951_868_800), "2000-03-01T00:00:00Z");
        // 2024-02-29 — うるう日そのもの
        assert_eq!(iso8601_from_epoch(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(iso8601_from_epoch(1_700_000_000), "2023-11-14T22:13:20Z");
    }
}
