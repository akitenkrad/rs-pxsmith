# ユースケース

[English](usecases.md) | **日本語**

[← README へ戻る](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.ja.md)

pxsmith が対象としているのは，絵を描くこと自体ではなく，その周辺にある作業です．
以下はこのツールを作る動機になった場面と，それを実行するコマンドです．引数の詳細は
[コマンドライン](cli.ja.md)にあります．

## 1. プルリクエストでスプライトの変更をレビューする

バイナリである `.aseprite` ファイルは差分を取れないため，スプライトの変更はレビューの
場に「ファイルが変わった」という情報としてしか届きません．レイヤを L0 テキストへ変換
すれば変更内容が読めるようになり，往復はバイト単位で一致するので，テキストをレビュー
対象としながら `.aseprite` を作業用ファイルとして保つことができます．

```sh
pxsmith text export hero.aseprite hero.px.toml --palette pal.hex
pxsmith diff old.px.toml hero.px.toml
```

`diff` は変化した画素を数え，その位置を要約統計ではなく 1 件ずつ報告します．ドット絵
では 1 画素の違いが意味を持つためです．

## 2. 1 枚の絵から 8 方向を導出する

キャラクタを 1 方向だけ描いて残りを導出すれば，8 枚のスプライトの一貫性を保つ作業が
不要になります．陰影は塗るのではなく導出しているため，反転したスプライトに陰影を
付け直すことができ，光源が絵と一緒に反転してしまうことがありません．

```sh
pxsmith direction 'out/${dir}.px.toml' --from s=hero_s.px.toml \
    --light dir:-0.6,0.8 --reshade
```

出力先には `${dir}` を含める必要があります．方向ごとに 1 ファイルを書き出すためです．

## 3. 拡大された状態で届いた絵を復元する

Web から集めたドット絵，誤った倍率で書き出された素材，画像モデルが生成した絵は，
たいてい拡大された状態で届き，JPEG 圧縮を経ていることも多くあります．`conform` が
元の格子を復元して等倍へ戻し，`quantize` と `clean` が扱えるインデックスカラーへ
戻します．

```sh
pxsmith conform upscaled.png native.png
pxsmith quantize native.png indexed.png --colors 16 --method kmeans
pxsmith clean indexed.png cleaned.png
```

格子が一様でない場合，`conform` は推測せずに拒否します．非一様な格子は決定論的に
復元できないため，黙って壊すのではなく人の判断に返します．

## 4. 手描きのシートからタイルセットを組む

シートをタイルに切って重複を除く作業は機械的で，手で行うと退屈なうえ，細かいところを
取り違えやすいものです．`tileset extract` が同値なタイルを束ねてマップを書き出し，
`tileset autotile` が象限から 47 枚組を構成します．

```sh
pxsmith tileset extract sheet.aseprite tiles.aseprite --tile 16 --map map.json
pxsmith tileset autotile quadrants.px.toml auto.aseprite
pxsmith export tiled map.json map.tmx --sheet out/sheet.json
```

入力はインデックスカラーである必要があります．ここで量子化を行うと，どのタイルを
同一とみなすかをツール自身の色の選択が決めてしまうためです．

## 5. 2 枚のキーフレームの間を埋める

中割り，タイミング，およびその周辺の副次的な動きは，2 枚のキーから計算します．

```sh
pxsmith anim tween out.px.toml --from a.px.toml --to b.px.toml --base 8A6A4A
pxsmith anim ease walk.px.toml eased.px.toml --fps 30 --hold 2,1,1,1,2
pxsmith anim squash in.px.toml out.px.toml --amount -0.3
```

27 のルールのうち 6 つはコマの列に掛かります．コマ間でトポロジーが変化する列，線が
揺れている列，ディザが物体に付いてこず画布に貼り付いている列は，`lint` が違反として
報告します．

## 6. ハードウェア制約に照らす

レトロプラットフォーム向けの制作では，パレットやタイルの上限を超えやすく，しかも
気付くのが遅れがちです．

```sh
pxsmith validate hero.px.toml --target gb
pxsmith validate hero.px.toml --target nes --json
```

組み込みの出力先は `gb`・`nes`・`snes`・`gba`・`pico8` で，それ以外は TOML の
プロファイルを渡せます．違反があれば非ゼロで終了するため，そのまま CI に組み込めます．

## 7. CI でアセットパイプラインを回す

レシピは導出の全体をデータとして記述するため，同じビルドが開発者の機械でもビルド
サーバでも動き，再実行すれば変わった箇所だけが作り直されます．

```sh
pxsmith run build.toml --dry-run   # 順序だけ出す．何も走らせない
pxsmith run build.toml
```

64 枚に対する 128 ステップで，冷えた状態が 2.66 秒，温まった状態が 0.09 秒です．
スレッド数を変えても出力はバイト単位で同じであり，これは主張ではなく試験で確かめて
います．[レシピ](recipes.ja.md)を参照してください．

## 8. プロトタイプ中の仮素材を用意する

仮のスプライトはプロンプトとパレットから生成でき，手描きの絵と同じ lint で検証でき
ます．モデルが書くのは色ではなくパレットの添字なので，生成されたスプライトが
プロジェクトの宣言していない色を持ち込むことはありません．

```sh
export ANTHROPIC_API_KEY=...
pxsmith gen prog out/chest.px.toml --prompt "木の宝箱．正面から" \
    --palette 1a1c2c,566c86,8a6a4a,b13e53,f4f4f4 --size 16x16
```

作り直しのループが何を検証し，何を検証していないかは[生成](generation.ja.md)にあります．

## 9. 端末で確認しながら描く

`watch` は保存のたびに描き直すため，L0 テキストをエディタで直接編集する作業に向いて
います．

```sh
pxsmith verify terminal        # この端末は 1 画素の確認に耐えるか
pxsmith watch hero.px.toml --zoom 8
pxsmith view walk.px.toml --frame 2 --onion 2
```

`verify terminal` が答えるのは，画像を表示できるかどうかではなく，1 画素を判断する
用途に耐えるかどうかです．Kitty・iTerm2・Sixel は該当しますが，半ブロックによる
代替表示は垂直解像度が半分になるため該当しません．

## 10. CLI を使わずライブラリとして使う

上記の操作はすべてライブラリの関数として提供しているため，独自のビルドシステムを持つ
プロジェクトから直接呼び出せます．スプライトをコンパイル時に埋め込むこともでき，
その場合は行の長さの不一致がコンパイルエラーになります．

```rust
use pxsmith_macro::pixels;

let frames = pixels!("sprites/hero_body.px.toml");
```

クレートの分割と，`pxsmith-view` を端末表示の実装なしで取り込む方法は
[ライブラリ](library.ja.md)にあります．

## pxsmith が対象としないこと

描画のための UI もキャンバスも持たず，Aseprite をはじめとするエディタの代わりには
なりません．また，作者が決めるべき事柄を代わりに決めることもしません．`conform` は
非一様な格子を推測せずに拒否し，`project` は投影の指定を推測せず宣言させ，
`palette report` は 4 通りの割合を並べて 1 つに決めません．
