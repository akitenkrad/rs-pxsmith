<p align="center">
  <img src="https://raw.githubusercontent.com/akitenkrad/rs-pxsmith/main/docs/assets/logo.png" width="180" alt="pxsmith">
</p>

<h1 align="center">pxsmith</h1>

<p align="center"><em>ドット絵のための Makefile．</em></p>

<!-- Restore after `cargo publish --workspace`:
  <a href="https://crates.io/crates/pxsmith-core"><img src="https://img.shields.io/crates/v/pxsmith-core.svg" alt="crates.io"></a>
  <a href="https://docs.rs/pxsmith-core"><img src="https://docs.rs/pxsmith-core/badge.svg" alt="docs.rs"></a>
-->
<p align="center">
  <a href="https://github.com/akitenkrad/rs-pxsmith/actions/workflows/ci.yml"><img src="https://github.com/akitenkrad/rs-pxsmith/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License">
  <img src="https://img.shields.io/badge/rust-2024%20edition-orange.svg" alt="Rust 2024">
</p>

[English](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.md) | **日本語**

---

pxsmith は，ドット絵アセットの導出・突き合わせ・検証を**宣言的なパイプライン**
として行う道具です．描画 UI は持ちません — 元絵は人（または生成モデル）が描き，
そこから先はすべてコードとして走ります．陰影はシルエットから**導出**し，
中割りは計算し，タイルセットは切り出して重複を畳み，出来上がったものは
**27 のルール**に照らしてから出荷します．

色は端から端までインデックスカラーです．どの変換も「すでに在る添字から選ぶ」
形になるので，**パレットにない色が生まれることは検査で防ぐのではなく，
構造として起こりません**．

この道具の閾値は**すべて実素材で何かを測って決めています**．測った結果が
悪ければ，その数字を書き残して機能を出しません —
[測って，直さなかったもの](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/architecture.ja.md#測って直さなかったもの)を見てください．

## インストール

ライブラリは crates.io にあります．

```sh
cargo add pxsmith-core pxsmith-io pxsmith-lint
```

`pxsmith` コマンドは**公開していません** — `viuer` 経由で `ansi_colours`
(LGPL-3.0-or-later) を静的リンクするためです．ソースから入れてください．

```sh
cargo install --git https://github.com/akitenkrad/rs-pxsmith pxsmith
```

## まず動かす

```sh
# スプライトのレイヤを編集できるテキストにして，戻す
pxsmith text export sprite.aseprite hero.px.toml --palette pal.hex
pxsmith text import hero.px.toml sprite.aseprite

# シルエットから陰影を導出し，結果を検査する
pxsmith shade hero.png hero.px.toml --base 8A6A4A --light dir:-0.6,0.8
pxsmith lint hero.px.toml

# 保存のたびに端末へ描き直す
pxsmith watch hero.px.toml --zoom 8
```

`lint` は「**鳴らなかった**ルール」と「**検査できなかった**ルール」を区別して
報告します．検査が落ちうる位置にあって初めて，静かな報告が「きれいな絵」の
証拠になります．

## ドキュメント

| | |
| --- | --- |
| [コマンドライン](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/cli.ja.md) | 全サブコマンドと，効く引数 |
| [レシピ](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/recipes.ja.md) | 宣言的なビルド形式とキャッシュ |
| [生成](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/generation.ja.md) | 言語モデルに絵を頼み，返ってきたものを検証する |
| [ライブラリ](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/library.ja.md) | Rust から使う．`pixels!` マクロ |
| [設計](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/architecture.ja.md) | クレートの分け方，設計判断，閾値の決め方 |
| [どう作ったか](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/engineering.ja.md) | 開発思想と，それを生んだ失敗の記録 |

測定の記録は [`docs/status.md`](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/status.md) と
[`docs/investigations/`](https://github.com/akitenkrad/rs-pxsmith/tree/main/docs/investigations) にあります — 何を測り，
数字がいくつで，その結果どの機能をやめたかが書いてあります．

## ライセンス

[Apache License 2.0](https://github.com/akitenkrad/rs-pxsmith/blob/main/LICENSE-APACHE) または [MIT license](https://github.com/akitenkrad/rs-pxsmith/blob/main/LICENSE-MIT) の
どちらかを選べます — Rust クレートの通例の二重ライセンスで，
どちら側の生態系からでも使えるようにするためです．

`crates/pxsmith-core/src/cleanedge.rs` は torcado の cleanEdge シェーダの移植で，
その条件のもとで使っています．要求される著作権表示は [NOTICE](https://github.com/akitenkrad/rs-pxsmith/blob/main/NOTICE) にあります．

`testdata/` の素材は CC0 か MIT で，出所は `testdata/SOURCES.md` に記録して
います．再配布できない素材はコミットしていません．
