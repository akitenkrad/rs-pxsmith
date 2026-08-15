# `.aseprite` 往復検証用の実ファイル

**Aseprite 本体が書き出したファイルだけを置く．** 自前で書いたファイルは
`testdata/generated/` に置くこと．

## なぜ実ファイルでなければならないか

二層構造 (設計書 3.1) と R3 の対策は，`aseprite-io` のバイト一致往復に全面的に
依存している．自前で書き出したファイルを往復させても，`aseprite-io` が読める形しか
現れないので**何も確かめていないことになる**．Aseprite の非公開挙動を踏むには，
Aseprite 本体が書いたバイト列が要る．

## 置き場所

| ディレクトリ | 内容 |
| --- | --- |
| `aseprite-tests/` | Aseprite 公式テストスプライト 19 件 (MIT)．出典は `testdata/SOURCES.md` |
| `independent/` | **未調達**．最新版 Aseprite で自作したファイル (下記) |

### 性質の網羅状況

| 性質 | 状態 |
| --- | --- |
| タイルマップ + タイルセット | 済 (`2x2tilemap2x2tile` / `2x3tilemap-indexed` / `3x2tilemap-grayscale`) |
| リンクセル | 済 (`link.aseprite`) |
| レイヤグループ (入れ子) | 済 (`groups2` / `groups3abc`) |
| インデックスカラー / グレースケール / 背景レイヤ | 済 (`bg-index-3` / `2f-index-3x3` / `4f-index-4x4`) |
| タグ・スライス・ユーザデータ props | 済 (`tags3` / `slices-moving` / `file-tests-props`) |
| RGBA モード | 済 (`abcd` / `1empty3` ほか) |
| **未知チャンク** | **未**．`aseprite-tests/` は `aseprite-io` が既知の範囲なので現れない |
| 半透明パレット | 未 (作業層の不変条件に引っかかることの確認用) |

`independent/` が必要なのはこの表の下 2 行のためである — `aseprite-tests/` は
`aseprite-io` 自身の fixtures でもあるので，**あちらの CI が通している範囲を出ない**．
別系統の素材で初めて R3 を独立に検証したことになる．

## 検証のしかた

```sh
# ここに置いたファイル全部を検査する
cargo test -p pxsmith-io --test aseprite_roundtrip

# 素材が無いことを失敗として扱う (M0 の完了判定)
PXFORGE_REQUIRE_ASEPRITE_CORPUS=1 cargo test -p pxsmith-io --test aseprite_roundtrip

# 個別のファイルを CLI で確かめる
cargo run -p pxsmith -- verify roundtrip path/to/file.aseprite --via-frame
```

## ライセンス

CC0 を原則とし，**著作権表示を同梱すれば再配布できるもの (MIT) は受け入れる**
(`testdata/SOURCES.md` の但し書き)．受け入れる場合は原文の LICENSE を素材と
同じディレクトリに置き，`SOURCES.md` に記録すること．
