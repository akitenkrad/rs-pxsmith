# コマンドライン

[English](cli.md) | **日本語**

[← README へ戻る](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.ja.md)

以下の例はインストール済みの `pxsmith` を実行する形で記載しています．リポジトリの
チェックアウトから実行する場合は，先頭に `cargo run -p pxsmith --` を付けてください．

`.px.toml` は L0 と呼ぶテキスト形式です．スプライトを文字の並びとして記述し，パレットは
別の `.hex` ファイルが保持します．`.aseprite` はバイト単位で往復するため，pxsmith は
既存の Aseprite による作業の途中に配置することができ，ファイルの所有権を奪うことは
ありません．

## 基本

```sh
# 端末が画素を正確に出せるか確かめる (Kitty / iTerm2 / Sixel)
pxsmith verify terminal

# スプライトのレイヤを編集できるテキストにして，戻す
pxsmith text export sprite.aseprite hero.px.toml --palette pal.hex
pxsmith text import hero.px.toml sprite.aseprite

# 保存を見張って描き直す
pxsmith watch hero.px.toml --zoom 8

# 2 枚の間でどの画素が変わったかを出す
pxsmith diff before.px.toml after.px.toml

# パレットを見る・変換する (`.hex` が正典の形式)
pxsmith palette info palettes/sweetie-16.hex
pxsmith palette convert input.gpl output.hex

# `.aseprite` の読み書きがバイト一致するか確かめる
pxsmith verify roundtrip sprite.aseprite --via-frame
```

## 絵を導出する

陰影は塗るのではなく，シルエットから導出します．入力の色は捨てるため，反転・中割り・
色替えを行っても光の向きが壊れることはありません．

```sh
# シルエットから陰影を導出する (入力の色は捨てる)
pxsmith shade hero.png hero.px.toml --base 8A6A4A --light dir:-0.6,0.8

# ジャギーを正規化し，アンチエイリアスを付け，縁取りを描く
pxsmith smooth hero.px.toml smoothed.px.toml
pxsmith aa smoothed.px.toml aa.px.toml
pxsmith outline aa.px.toml outlined.px.toml --style tinted

# アニメ: 中割り・タイミング・周期・スメア・スクワッシュ・サブピクセル・残像
pxsmith anim tween out.px.toml --from a.px.toml --to b.px.toml --base 8A6A4A
pxsmith anim ease walk.px.toml eased.px.toml --fps 30 --hold 2,1,1,1,2
pxsmith anim smear out.px.toml --from a.px.toml --to b.px.toml --base 8A6A4A
pxsmith anim squash in.px.toml out.px.toml --amount -0.3
pxsmith anim subpixel in.px.toml out.px.toml --method tangent

# 出力先の制約に照らす (違反があれば非零で終了する)
pxsmith lint out.px.toml
pxsmith validate out.px.toml --target gb
```

### 色数を減らす

```sh
pxsmith quantize photo.png indexed.png --colors 16 --method kmeans
pxsmith clean indexed.png cleaned.png
pxsmith conform upscaled.png native.png
```

`conform` は，拡大されさらに JPEG 圧縮を経ているかもしれない画像から元の格子を復元し，
等倍へ戻します．格子が一様でない場合は推測を行わず拒否します．非一様な格子は決定論的に
復元することができないため，そこは人の判断に委ねるという方針を採りました．

## 合成・タイルセット・投影

```sh
# パーツを合成し，反転と陰影の再導出で残り 7 方向を導出する
pxsmith compose out.px.toml --part body.px.toml --part head.px.toml
pxsmith direction 'out/${dir}.px.toml' --from s=hero_s.px.toml \
    --light dir:-0.6,0.8 --reshade

# シートをタイルに切り，重複を畳み，47 枚のオートタイルを組む
# (入力はインデックスカラーのみ — ここで量子化するとタイルの同一性に
#  こちらの都合が紛れ込む)
pxsmith tileset extract sheet.aseprite tiles.aseprite --tile 16 --map map.json
pxsmith tileset autotile quadrants.px.toml auto.aseprite

# 奥行きを作る: 遠景を空色へ寄せ，視差の速さを記録する
pxsmith atmos 'out/${name}.px.toml' --input fg.px.toml --input bg.px.toml \
    --sky 41a6f6 --haze background=0.6 --scroll-meta out/scene.scroll.json

# 真上から見た絵を等角の床へ投影し，合うガイドを引く
pxsmith project in.px.toml iso.px.toml --to iso --from top --facing right
pxsmith guide g.png --projection iso --from top --cell 16 --size 256x256
```

`project` は `--from` と `--facing` を必須の引数としています．どの面を倒すのか，また
どちらを向いているのかは画素からは読み取れないため，推測に任せると外れた場合にのみ
静かに壊れることになるからです．

## 拡縮と回転

```sh
pxsmith scale in.px.toml out.px.toml --factor 4          # 既定は nearest (厳密)
pxsmith rotate in.px.toml out.px.toml --degrees 30 --algo cleanedge
```

整数倍の拡大と 90 度の倍数の回転は，標本の丸めに任せず添字の置き換えとして実装して
います．そのため 4 回まわすと元の絵に完全に戻ります．`cleanedge` が効果を発揮するのは
拡大を伴う回転であり，等倍では既定の `nearest` の方が良い結果になります．この点は
CLI が実行時に説明します．

## 書き出す

```sh
pxsmith sheet pack out/sheet.png --input a.px.toml --input b.px.toml --layout out/sheet.json
pxsmith export tiled map.json map.tmx --sheet out/sheet.json
```

## 調べる

```sh
pxsmith view walk.px.toml --frame 2 --onion 2   # オニオンスキン．輪郭のみ
pxsmith palette report hero.px.toml --top 12    # どの色が面積を担っているか
```

`palette report` は単一の割合ではなく 4 通りの閾値を並べ，さらに「その添字の合計面積」と
「1 つながりの塊としての最大面積」を分けて報告します．広い範囲に撒かれた色は主要な色とは
言えませんが，合計面積だけを見ると主要な色として読めてしまうためです．

## チェックアウトからのビルド

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

`cargo-make` のタスクは `Makefile.toml` に定義しています．

```sh
cargo make format-all   # taplo + clippy + rustfmt
cargo make test
```
