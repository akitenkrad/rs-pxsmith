//! **`aseprite-io` の読みを «仕様から独立に» 突き合わせる** (R3 の残り．D167)．
//!
//! # バイト一致は «理解した» の証明ではない
//!
//! `aseprite_roundtrip.rs` は読み込み → 書き出しがバイト一致することを確かめる．
//! これは保持層が汚れないことの証明だが，**`aseprite-io` が中身を正しく
//! 解釈したことの証明にはならない** — 未知のチャンクを不透明な塊として
//! 持ち回って書き戻すだけでもバイトは一致するからである．
//!
//! **そして誤読は静かに絵を壊す．** `Document::project` は «解釈した側» を
//! 読むので，レイヤの取り違えやセルの位置ずれは往復試験を素通りする．
//!
//! # だから仕様から 2 つ目の読み手を書く
//!
//! ここにあるのは公開仕様 (`ase-file-specs.md`) だけから書いた最小の
//! パーサである．**`aseprite-io` の実装は見ずに，仕様の欄の並びから書く** —
//! 実装を写したら «同じ誤りを 2 回書く» ことになり，突き合わせる意味が消える．
//!
//! 比べるのは «絵を組み立てるのに要る事実» に絞る:
//! 画布 ・コマ数 ・表示時間 ・レイヤの並びと種別と名前 ・パレットの長さ ・
//! 透明添字 ・コマごとのセルの有無．**ここが食い違えば `project` は違う絵を出す．**
//!
//! # これは R3 を閉じない
//!
//! 独立なのは**読み手**であって**素材**ではない．素材は依然 19 件
//! (Aseprite 公式のテストスプライト) しかなく，**そこに現れない書き方は
//! どちらの読み手も試していない** — 実際 `cel extra` (0x2006) ・
//! `old palette` (0x0011) ・生のセル (type 0) は 1 件も無い．
//! 残りは `testdata/aseprite/independent/README.md` に書いてある．

use std::path::{Path, PathBuf};

use pxsmith_io::Document;

// ---------------------------------------------------------------- 仕様の読み手

/// 仕様 (`ase-file-specs.md`) の欄の並びだけから読んだ事実．
#[derive(Debug, Default)]
struct SpecFile {
    width: u16,
    height: u16,
    frames: u16,
    /// 32 = RGBA ・16 = grayscale ・8 = indexed．
    color_depth: u16,
    transparent_index: u8,
    /// ヘッダが宣言する色数．
    header_colors: u16,
    /// フレームごとの表示時間 (ms)．
    durations: Vec<u16>,
    layers: Vec<SpecLayer>,
    /// `(フレーム, レイヤ添字, セル種別)`．
    cels: Vec<(usize, u16, u16)>,
    /// パレットチャンク (0x2019) が宣言する最大の色数．
    palette_size: u32,
    /// 出てきたチャンク種別．
    chunk_types: Vec<u16>,
}

#[derive(Debug, PartialEq, Eq)]
struct SpecLayer {
    /// 0 = 通常 ・1 = グループ ・2 = タイルマップ．
    kind: u16,
    child_level: u16,
    name: String,
}

/// 読み進める位置を持つだけの読み手 (`byteorder` も使わない — 仕様の欄を
/// そのまま数えるため)．
struct Cursor<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cursor<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn seek(&mut self, p: usize) {
        self.p = p;
    }
    fn skip(&mut self, n: usize) {
        self.p += n;
    }
    fn byte(&mut self) -> u8 {
        let v = self.b[self.p];
        self.p += 1;
        v
    }
    fn word(&mut self) -> u16 {
        let v = u16::from_le_bytes([self.b[self.p], self.b[self.p + 1]]);
        self.p += 2;
        v
    }
    fn dword(&mut self) -> u32 {
        let v = u32::from_le_bytes([
            self.b[self.p],
            self.b[self.p + 1],
            self.b[self.p + 2],
            self.b[self.p + 3],
        ]);
        self.p += 4;
        v
    }
    /// STRING = WORD の長さ + UTF-8 のバイト列．
    fn string(&mut self) -> String {
        let n = self.word() as usize;
        let s = String::from_utf8_lossy(&self.b[self.p..self.p + n]).into_owned();
        self.p += n;
        s
    }
}

/// ファイル全体を仕様どおりに歩く．
///
/// **チャンクの中身は必要な欄だけ読み，残りは «チャンクの長さ» で飛ばす** —
/// 長さは各チャンクの先頭にあるので，知らないチャンクでも位置を見失わない
/// (これは仕様が «未知のチャンクは飛ばせ» と定めている歩き方そのものである)．
fn parse_spec(bytes: &[u8]) -> SpecFile {
    let mut c = Cursor::new(bytes);
    let mut out = SpecFile::default();

    // --- ヘッダ (128 バイト)
    c.skip(4); // DWORD ファイル長
    let magic = c.word();
    assert_eq!(magic, 0xA5E0, "ヘッダの magic が違う");
    out.frames = c.word();
    out.width = c.word();
    out.height = c.word();
    out.color_depth = c.word();
    c.skip(4); // DWORD flags
    c.skip(2); // WORD speed (非推奨)
    c.skip(4 + 4); // DWORD 0 が 2 つ
    out.transparent_index = c.byte();
    c.skip(3); // 無視する 3 バイト
    out.header_colors = c.word();
    c.seek(128);

    // --- フレーム
    for frame in 0..out.frames as usize {
        let frame_start = c.p;
        let frame_size = c.dword() as usize;
        let fmagic = c.word();
        assert_eq!(fmagic, 0xF1FA, "フレーム {frame} の magic が違う");
        let old_chunks = c.word();
        out.durations.push(c.word());
        c.skip(2); // 予約
        let new_chunks = c.dword();
        // **新しい欄が 0 のときだけ古い欄を使う** (仕様の但し書き)
        let chunks = if new_chunks != 0 {
            new_chunks
        } else {
            old_chunks as u32
        };

        for _ in 0..chunks {
            let chunk_start = c.p;
            let size = c.dword() as usize;
            let kind = c.word();
            out.chunk_types.push(kind);

            match kind {
                // レイヤ
                0x2004 => {
                    c.skip(2); // WORD flags
                    let layer_kind = c.word();
                    let child_level = c.word();
                    c.skip(2 + 2); // 既定の幅 ・高さ (無視される欄)
                    c.skip(2); // WORD ブレンドモード
                    c.skip(1); // BYTE 不透明度
                    c.skip(3); // 予約
                    let name = c.string();
                    out.layers.push(SpecLayer {
                        kind: layer_kind,
                        child_level,
                        name,
                    });
                }
                // セル
                0x2005 => {
                    let layer = c.word();
                    c.skip(2 + 2); // SHORT x ・y
                    c.skip(1); // BYTE 不透明度
                    let cel_kind = c.word();
                    out.cels.push((frame, layer, cel_kind));
                }
                // パレット
                0x2019 => {
                    out.palette_size = out.palette_size.max(c.dword());
                }
                _ => {}
            }
            // **必ずチャンクの長さで進む** — 途中まで読んでも位置がずれない
            c.seek(chunk_start + size);
        }
        c.seek(frame_start + frame_size);
    }
    out
}

// ---------------------------------------------------------------- 素材

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/aseprite")
}

fn corpus() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                walk(&p, out);
            } else if matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("aseprite") | Some("ase")
            ) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(&corpus_dir(), &mut out);
    out
}

// ---------------------------------------------------------------- 突き合わせ

/// **壊れると: `aseprite-io` が中身を取り違えても往復試験は通り続ける** (D167)．
///
/// 往復はバイトしか見ないので，**解釈の誤りは `project` が違う絵を出すまで
/// 気付けない**．ここは «絵を組み立てるのに要る事実» を仕様から独立に読み，
/// `aseprite-io` の言い分と突き合わせる．
#[test]
fn the_independent_reader_agrees_with_aseprite_io() {
    let files = corpus();
    assert!(!files.is_empty(), "素材が 1 つも無い");

    for path in &files {
        let bytes = std::fs::read(path).expect("読み込みに失敗");
        let spec = parse_spec(&bytes);
        let doc = Document::from_bytes(&bytes)
            .unwrap_or_else(|e| panic!("{} の解釈に失敗した: {e}", path.display()));
        let name = path.file_name().unwrap().to_string_lossy();

        // 画布とコマ数
        assert_eq!(
            doc.size().x,
            spec.width as u32,
            "{name}: 幅が食い違う (道具 {} 対 仕様 {})",
            doc.size().x,
            spec.width
        );
        assert_eq!(doc.size().y, spec.height as u32, "{name}: 高さが食い違う");
        assert_eq!(
            doc.frame_count(),
            spec.frames as usize,
            "{name}: コマ数が食い違う"
        );

        // 表示時間 — **コマごとに見る** (合計が合っていても並びが違えば動きが変わる)
        for (i, &d) in spec.durations.iter().enumerate() {
            assert_eq!(
                doc.raw().frames()[i].duration_ms,
                d,
                "{name}: コマ {i} の表示時間が食い違う"
            );
        }

        // レイヤの並び ・種別 ・名前
        let raw_layers = doc.raw().layers();
        assert_eq!(
            raw_layers.len(),
            spec.layers.len(),
            "{name}: レイヤ数が食い違う"
        );
        for (i, sl) in spec.layers.iter().enumerate() {
            assert_eq!(
                raw_layers[i].name, sl.name,
                "{name}: レイヤ {i} の名前が食い違う"
            );
        }

        // **グループがどれかが食い違うと，作業層の添字が丸ごとずれる**
        let spec_content: Vec<usize> = spec
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.kind != 1)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            doc.content_layers(),
            spec_content,
            "{name}: 中身を持つレイヤの並びが食い違う"
        );

        // 透明添字 — インデックスカラーのときだけ意味を持つ
        if spec.color_depth == 8 {
            assert_eq!(
                doc.raw().transparent_index(),
                spec.transparent_index,
                "{name}: 透明添字が食い違う"
            );
        }

        // パレットの長さ
        if spec.palette_size > 0 {
            assert_eq!(
                doc.raw().palette().len() as u32,
                spec.palette_size,
                "{name}: パレットの色数が食い違う"
            );
        }
    }

    eprintln!("{} 件を仕様から独立に読み直して突き合わせた", files.len());
}

/// **突き合わせが «空振り» でないことを機械で確かめる** (D163 の作法．D167)．
///
/// 上の試験は «食い違わないこと» を見る．だが素材にグループが 1 つも無ければ
/// «グループを取り違えない» は**何も検査していない**のと同じである．
/// **区別が要る場面が素材に実在するか**をここで数える．
///
/// **壊れると: 覆っていない性質を «確かめた» と読む．**
#[test]
fn the_corpus_actually_exercises_what_the_comparison_checks() {
    let (mut groups, mut tilemaps, mut linked, mut multi_frame, mut depths) = (
        0usize,
        0usize,
        0usize,
        0usize,
        std::collections::BTreeSet::new(),
    );

    for path in &corpus() {
        let spec = parse_spec(&std::fs::read(path).expect("読み込みに失敗"));
        groups += spec.layers.iter().filter(|l| l.kind == 1).count();
        tilemaps += spec.layers.iter().filter(|l| l.kind == 2).count();
        linked += spec.cels.iter().filter(|(_, _, k)| *k == 1).count();
        if spec.frames > 1 {
            multi_frame += 1;
        }
        depths.insert(spec.color_depth);
    }

    assert!(
        groups > 0,
        "グループレイヤが 1 つも無い — 添字のずれを検査できない"
    );
    assert!(tilemaps > 0, "タイルマップレイヤが 1 つも無い");
    assert!(linked > 0, "リンクセルが 1 つも無い — 解決を検査できない");
    assert!(
        multi_frame > 0,
        "複数コマの素材が無い — 表示時間の並びを検査できない"
    );
    assert!(
        depths.len() >= 3,
        "色深度が {} 種しか無い (RGBA ・grayscale ・indexed が要る)",
        depths.len()
    );

    eprintln!(
        "区別が要る場面: グループ {groups} ・タイルマップ {tilemaps} ・\
         リンクセル {linked} ・複数コマ {multi_frame} 件 ・色深度 {} 種",
        depths.len()
    );
}

/// **壊れると: セルの有無を取り違え，空でないコマが空になる (またはその逆)．**
///
/// セルは «そのコマのそのレイヤに絵があるか» を決める．リンクセルは
/// **別のコマを指す**ので，指した先に絵があれば «ある» と数える．
#[test]
fn the_two_readers_find_the_same_cels() {
    for path in &corpus() {
        let bytes = std::fs::read(path).expect("読み込みに失敗");
        let spec = parse_spec(&bytes);
        let doc = Document::from_bytes(&bytes).expect("解釈できる");
        let name = path.file_name().unwrap().to_string_lossy();

        // 仕様側: 直接置かれたセル (リンク以外) の集合
        let direct: std::collections::BTreeSet<(usize, u16)> = spec
            .cels
            .iter()
            .filter(|(_, _, kind)| *kind != 1)
            .map(|(f, l, _)| (*f, *l))
            .collect();
        // リンクセルはそれ自体もセルである (指す先に絵がある)
        let all: std::collections::BTreeSet<(usize, u16)> =
            spec.cels.iter().map(|(f, l, _)| (*f, *l)).collect();

        for frame in 0..doc.frame_count() {
            for &layer in &doc.content_layers() {
                let layer_ref = doc.raw().layer_ref(layer).expect("中身を持つレイヤ");
                let has = doc.raw().resolve_cel(layer_ref, frame).is_some();
                let spec_has = all.contains(&(frame, layer as u16));
                assert_eq!(
                    has, spec_has,
                    "{name}: コマ {frame} ・レイヤ {layer} のセルの有無が食い違う \
                     (道具 {has} 対 仕様 {spec_has})"
                );
            }
        }

        // **リンクセルが実際に使われている素材であること**を確かめておく —
        // 使われていなければ，この試験はリンクの解決を 1 度も見ていない
        if all.len() > direct.len() {
            eprintln!(
                "{name}: リンクセル {} 件を解決して突き合わせた",
                all.len() - direct.len()
            );
        }
    }
}

/// **素材が覆っていない書き方を «覆った» と読まないための記録** (D167)．
///
/// 仕様にあるのにこのコーパスに 1 件も無いチャンクを数え上げる．
/// **どちらの読み手も試していない**ので，ここが空でないうちは R3 は閉じない．
#[test]
fn the_corpus_records_what_it_does_not_cover() {
    let mut seen = std::collections::BTreeSet::new();
    let mut cel_kinds = std::collections::BTreeSet::new();
    for path in &corpus() {
        let spec = parse_spec(&std::fs::read(path).expect("読み込みに失敗"));
        seen.extend(spec.chunk_types.iter().copied());
        cel_kinds.extend(spec.cels.iter().map(|(_, _, k)| *k));
    }

    // 仕様が定めるチャンク (非推奨も含む)
    let known: [(u16, &str); 14] = [
        (0x0004, "old palette 4"),
        (0x0011, "old palette 11"),
        (0x2004, "layer"),
        (0x2005, "cel"),
        (0x2006, "cel extra"),
        (0x2007, "color profile"),
        (0x2008, "external files"),
        (0x2016, "mask (deprecated)"),
        (0x2017, "path"),
        (0x2018, "tags"),
        (0x2019, "palette"),
        (0x2020, "user data"),
        (0x2022, "slice"),
        (0x2023, "tileset"),
    ];
    let missing: Vec<&str> = known
        .iter()
        .filter(|(t, _)| !seen.contains(t))
        .map(|(_, n)| *n)
        .collect();
    let missing_cels: Vec<&str> = [
        (0u16, "raw"),
        (1, "linked"),
        (2, "compressed image"),
        (3, "compressed tilemap"),
    ]
    .iter()
    .filter(|(k, _)| !cel_kinds.contains(k))
    .map(|(_, n)| *n)
    .collect();

    eprintln!("コーパスに現れたチャンク: {} 種", seen.len());
    eprintln!("**現れなかったチャンク**: {}", missing.join(" ・"));
    eprintln!("**現れなかったセル種別**: {}", missing_cels.join(" ・"));

    // **知らないチャンクが出たら知らせる** — 仕様が増えたか，読み違えている
    let unknown: Vec<u16> = seen
        .iter()
        .filter(|t| !known.iter().any(|(k, _)| k == *t))
        .copied()
        .collect();
    assert!(
        unknown.is_empty(),
        "仕様の一覧に無いチャンクが出た: {unknown:04x?} — 仕様を引き直すこと"
    );
}
