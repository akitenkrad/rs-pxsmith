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

pxsmith は，ドット絵アセットの導出・突き合わせ・検証を宣言的なパイプラインとして
実行するツールです．描画のための UI は持っておらず，元絵は人あるいは生成モデルが
描くことを前提としています．そこから先の工程，すなわち陰影の導出，中割りの生成，
タイルセットの切り出しと重複の統合はすべてコードとして実行され，出来上がった成果物は
27 個の品質ルールに照らしてから出荷されます．

色の表現には一貫してインデックスカラーを用いています．あらゆる変換が「すでにパレットに
存在する添字を選ぶ」という形をとるため，パレットにない色が生成される事態は検査によって
防いでいるのではなく，構造上そもそも起こりません．

このツールに現れる閾値は，すべて実際のドット絵素材に対して何かを測定した結果として
決められています．測定に用いた実行口は残してあるため，数値は信じるものではなく作り直して
確かめられるものになっています．

## インストール

ライブラリは crates.io で公開しています．

```sh
cargo add pxsmith-core pxsmith-io pxsmith-lint
```

コマンドライン本体も同じ登録所から入ります．

```sh
cargo install pxsmith
```

`cargo install` は利用者自身の機械でビルドします．端末プレビューが `viuer` を経由して
`ansi_colours` (LGPL-3.0-or-later) に届くため，この点は重要です．ビルド済みバイナリは
配布していません．この依存を避けたいライブラリ利用者は，`pxsmith-view` を
`--no-default-features` で取り込めば依存ツリーから完全に外せます．

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

`lint` は「鳴らなかったルール」と「検査できなかったルール」を区別して報告します．
検査が落ちうる位置にあって初めて，違反が報告されなかったという事実が「きれいな絵で
ある」ことの証拠になるためです．

## ドキュメント

| | |
| --- | --- |
| [コマンドライン](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/cli.ja.md) | 全サブコマンドと，主要な引数 |
| [レシピ](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/recipes.ja.md) | 宣言的なビルド形式とキャッシュ |
| [生成](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/generation.ja.md) | 言語モデルへの依頼と，返ってきた成果物の検証 |
| [ライブラリ](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/library.ja.md) | Rust から利用する方法と `pixels!` マクロ |
| [設計](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/architecture.ja.md) | クレートの分割，設計判断，閾値の決定方法 |
| [どう作ったか](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/engineering.ja.md) | 開発の思想と，それを生んだ失敗の記録 |

測定そのものの記録は
[`docs/status.md`](https://github.com/akitenkrad/rs-pxsmith/blob/main/docs/status.md)
と
[`docs/investigations/`](https://github.com/akitenkrad/rs-pxsmith/tree/main/docs/investigations)
にあります．何を測定し，数値がいくつであったのかを記録しました．

## ライセンス

[Apache License 2.0](https://github.com/akitenkrad/rs-pxsmith/blob/main/LICENSE-APACHE)
と [MIT license](https://github.com/akitenkrad/rs-pxsmith/blob/main/LICENSE-MIT)
のいずれかを選択できます．Rust クレートで慣例となっている二重ライセンスであり，
どちらの生態系からでも利用できるようにするための措置です．

`crates/pxsmith-core/src/cleanedge.rs` は torcado による cleanEdge シェーダの移植で
あり，その条件のもとで利用しています．要求される著作権表示は
[NOTICE](https://github.com/akitenkrad/rs-pxsmith/blob/main/NOTICE) に記載しました．

`testdata/` に置いた素材は CC0 または MIT であり，出所は `testdata/SOURCES.md` に
記録しています．再配布できない素材はコミットしていません．
