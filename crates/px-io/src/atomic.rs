//! 原子的な出力 (設計書 3.7 `AtomicOutput`)．
//!
//! 途中で失敗した出力ファイルを残さない．同じディレクトリに一時ファイルを作って
//! から `rename` するので，置換はファイルシステム上で原子的になる．

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{IoError, Result};

/// 書き込み中の一時ファイルと最終的な置き場所．
#[derive(Debug)]
pub struct AtomicOutput {
    tmp: PathBuf,
    final_path: PathBuf,
    committed: bool,
}

impl AtomicOutput {
    /// 一時ファイルを作る．`final_path` と同じディレクトリに置くので，
    /// `commit` の `rename` が同一ファイルシステム内に収まる．
    pub fn new(final_path: impl AsRef<Path>) -> Result<Self> {
        let final_path = final_path.as_ref().to_path_buf();
        let dir = final_path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(|source| IoError::File {
            path: dir.to_path_buf(),
            source,
        })?;

        let stem = final_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "out".to_string());
        // 同じ出力先へ並行して書く可能性を考え，プロセス ID で分ける．
        let tmp = dir.join(format!(".{stem}.{}.tmp", std::process::id()));

        Ok(Self {
            tmp,
            final_path,
            committed: false,
        })
    }

    pub fn tmp_path(&self) -> &Path {
        &self.tmp
    }

    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    /// 一時ファイルへ書き出す．
    pub fn write_all(&self, bytes: &[u8]) -> Result<()> {
        let mut f = std::fs::File::create(&self.tmp).map_err(|source| IoError::File {
            path: self.tmp.clone(),
            source,
        })?;
        f.write_all(bytes).map_err(|source| IoError::File {
            path: self.tmp.clone(),
            source,
        })?;
        f.sync_all().map_err(|source| IoError::File {
            path: self.tmp.clone(),
            source,
        })?;
        Ok(())
    }

    /// 一時ファイルを最終的な置き場所へ移す．
    pub fn commit(mut self) -> Result<()> {
        std::fs::rename(&self.tmp, &self.final_path).map_err(|source| IoError::File {
            path: self.final_path.clone(),
            source,
        })?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for AtomicOutput {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.tmp);
        }
    }
}

/// バイト列を原子的に書き出す．
pub fn write(path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
    let out = AtomicOutput::new(path)?;
    out.write_all(bytes)?;
    out.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("pxforge-atomic-test");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(name)
    }

    #[test]
    fn write_creates_file_with_contents() {
        let p = scratch("basic.bin");
        let _ = std::fs::remove_file(&p);
        write(&p, b"hello").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hello");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn write_creates_missing_directories() {
        let p = scratch("nested/deep/out.bin");
        let _ = std::fs::remove_dir_all(scratch("nested"));
        write(&p, b"x").unwrap();
        assert!(p.exists());
        let _ = std::fs::remove_dir_all(scratch("nested"));
    }

    #[test]
    fn dropping_without_commit_leaves_no_temp_file() {
        let p = scratch("dropped.bin");
        let tmp;
        {
            let out = AtomicOutput::new(&p).unwrap();
            out.write_all(b"partial").unwrap();
            tmp = out.tmp_path().to_path_buf();
            assert!(tmp.exists());
        }
        assert!(!tmp.exists(), "一時ファイルが残っている");
        assert!(!p.exists(), "未 commit なのに最終ファイルができている");
    }

    #[test]
    fn commit_replaces_existing_file() {
        let p = scratch("replace.bin");
        write(&p, b"old").unwrap();
        write(&p, b"new").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        let _ = std::fs::remove_file(&p);
    }
}
