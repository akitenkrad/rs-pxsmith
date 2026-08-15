//! 中間キャッシュ (設計書 6.15)．
//!
//! 置き場所は `.pxcache/` で，**gitignore する側**である
//! (コミットするのは `assets/generated/` の方) ．
//!
//! ```text
//! .pxcache/
//!   <key>/
//!     manifest.json     どの相対パスに何を戻すか
//!     files/0 1 2 ...   中身
//! ```
//!
//! # 鍵は «中身» を指し，マニフェストが «置き場所» を持つ
//!
//! キーには出力の**名前**も混ざっている ([`crate::key`]) ので，
//! 同じキーなら置き場所も同じである．マニフェストを別に持つのは，
//! **戻すときに «何個あったか» を数え直さないため**である．
//!
//! # `--no-cache` は «読まない» であって «書かない» ではない
//!
//! 読み書き両方を止めると，1 度 `--no-cache` を付けた後の実行まで遅くなる．
//! **参照だけを止めて，結果は貯める．**

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::{RecipeError, Result};

/// キャッシュの置き場所．
#[derive(Clone, Debug)]
pub struct Cache {
    root: PathBuf,
}

/// 1 つのキーに貯めてあるもの．
#[derive(Clone, Debug)]
pub struct Entry {
    pub key: String,
    /// 戻す先 (レシピからの相対パス)．
    pub outputs: Vec<PathBuf>,
}

impl Cache {
    /// `.pxcache/` を `root` の下に置く．
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.join(".pxcache"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.root
    }

    fn entry_dir(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }

    /// 貯めてあるか見る．**中身の欠けたものは «無い» とみなす**．
    pub fn lookup(&self, key: &str) -> Option<Entry> {
        let dir = self.entry_dir(key);
        let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
        let outputs: Vec<String> = serde_json::from_str(&text).ok()?;
        for (n, _) in outputs.iter().enumerate() {
            if !dir.join("files").join(n.to_string()).exists() {
                return None;
            }
        }
        Some(Entry {
            key: key.to_string(),
            outputs: outputs.into_iter().map(PathBuf::from).collect(),
        })
    }

    /// 出力を貯める．
    pub fn store(&self, key: &str, project: &Path, outputs: &[PathBuf]) -> Result<()> {
        let dir = self.entry_dir(key);
        let files = dir.join("files");
        std::fs::create_dir_all(&files).map_err(|source| RecipeError::CacheWrite {
            path: files.display().to_string(),
            source,
        })?;
        for (n, rel) in outputs.iter().enumerate() {
            let from = project.join(rel);
            let to = files.join(n.to_string());
            std::fs::copy(&from, &to).map_err(|source| RecipeError::CacheWrite {
                path: from.display().to_string(),
                source,
            })?;
        }
        let names: Vec<String> = outputs
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        let text = serde_json::to_string(&names).unwrap_or_default();
        std::fs::write(dir.join("manifest.json"), text).map_err(|source| RecipeError::CacheWrite {
            path: dir.join("manifest.json").display().to_string(),
            source,
        })
    }

    /// 貯めてあるものを置き直す．
    pub fn restore(&self, entry: &Entry, project: &Path) -> Result<()> {
        let files = self.entry_dir(&entry.key).join("files");
        for (n, rel) in entry.outputs.iter().enumerate() {
            let to = project.join(rel);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|source| RecipeError::CacheWrite {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
            std::fs::copy(files.join(n.to_string()), &to).map_err(|source| {
                RecipeError::CacheWrite {
                    path: to.display().to_string(),
                    source,
                }
            })?;
        }
        Ok(())
    }

    /// 今のレシピが使わないものを捨てる (`--gc`)．
    ///
    /// 返すのは捨てた数．**`keep` に無いものだけ**を消すので，
    /// 何度走らせても同じ結果になる (冪等) ．
    pub fn gc(&self, keep: &BTreeSet<String>) -> Result<usize> {
        if !self.root.exists() {
            return Ok(0);
        }
        let mut dropped = 0usize;
        let mut names: Vec<PathBuf> = std::fs::read_dir(&self.root)
            .map_err(|source| RecipeError::CacheRead {
                path: self.root.display().to_string(),
                source,
            })?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        // **決定論的な順で消す** — 途中で失敗したときに «どこまで消えたか» が決まる
        names.sort();
        for path in names {
            let Some(name) = path.file_name().map(|s| s.to_string_lossy().to_string()) else {
                continue;
            };
            if keep.contains(&name) {
                continue;
            }
            std::fs::remove_dir_all(&path).map_err(|source| RecipeError::CacheWrite {
                path: path.display().to_string(),
                source,
            })?;
            dropped += 1;
        }
        Ok(dropped)
    }

    /// 貯めてある数．
    pub fn len(&self) -> usize {
        std::fs::read_dir(&self.root)
            .map(|d| d.filter_map(|e| e.ok()).count())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pxsmith-cache-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("作れる");
        dir
    }

    /// **壊れると: 差分ビルドが «古い中身» か «空» を返す．**
    #[test]
    fn what_goes_in_comes_back_out_byte_for_byte() {
        let project = tmp("roundtrip");
        let cache = Cache::new(&project);
        std::fs::create_dir_all(project.join("out")).expect("作れる");
        std::fs::write(project.join("out/a.bin"), b"first").expect("書ける");
        std::fs::write(project.join("out/b.bin"), b"second").expect("書ける");
        let outputs = vec![PathBuf::from("out/a.bin"), PathBuf::from("out/b.bin")];

        cache.store("k1", &project, &outputs).expect("貯める");
        std::fs::remove_dir_all(project.join("out")).expect("消せる");

        let entry = cache.lookup("k1").expect("ある");
        cache.restore(&entry, &project).expect("戻せる");
        assert_eq!(
            std::fs::read(project.join("out/a.bin")).unwrap(),
            b"first".to_vec()
        );
        assert_eq!(
            std::fs::read(project.join("out/b.bin")).unwrap(),
            b"second".to_vec()
        );
    }

    /// **壊れると: 中身が欠けたキャッシュを «当たり» と読み，空のファイルが出る．**
    #[test]
    fn a_half_written_entry_counts_as_a_miss() {
        let project = tmp("partial");
        let cache = Cache::new(&project);
        std::fs::write(project.join("a.bin"), b"x").expect("書ける");
        let outputs = vec![PathBuf::from("a.bin")];
        cache.store("k1", &project, &outputs).expect("貯める");
        std::fs::remove_file(cache.dir().join("k1/files/0")).expect("消せる");
        assert!(cache.lookup("k1").is_none());
    }

    /// **壊れると: `--gc` が使っているものまで消す，または何度も違う結果になる．**
    #[test]
    fn gc_only_drops_what_the_recipe_no_longer_uses_and_is_idempotent() {
        let project = tmp("gc");
        let cache = Cache::new(&project);
        std::fs::write(project.join("a.bin"), b"x").expect("書ける");
        for key in ["k1", "k2", "k3"] {
            cache
                .store(key, &project, &[PathBuf::from("a.bin")])
                .expect("貯める");
        }
        let keep: BTreeSet<String> = ["k2".to_string()].into_iter().collect();
        assert_eq!(cache.gc(&keep).expect("掃除"), 2);
        assert_eq!(cache.len(), 1);
        assert!(cache.lookup("k2").is_some());
        // 2 度目は何も消さない
        assert_eq!(cache.gc(&keep).expect("掃除"), 0);
        assert_eq!(cache.len(), 1);
    }

    /// **壊れると: 空のキャッシュに `--gc` を掛けて落ちる．**
    #[test]
    fn gc_on_an_empty_cache_is_fine() {
        let project = tmp("gc-empty");
        let cache = Cache::new(&project);
        assert_eq!(cache.gc(&BTreeSet::new()).expect("掃除"), 0);
    }
}
