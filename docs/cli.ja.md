# コマンドライン

[English](cli.md) | **日本語**

[← README へ戻る](../README.ja.md)

以下の例はインストール済みの `pxsmith` を叩く形です．チェックアウトから走らせる
ときは `cargo run -p pxsmith --` を頭に付けてください．

`.px.toml` は L0 テキスト形式です — スプライトを文字として書き，パレットは
別の `.hex` ファイルが持ちます．`.aseprite` はバイト単位で往復するので，
既存の Aseprite の作業の**途中に**置けます（ファイルの所有権を奪いません）．

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

陰影は**シルエットから導出**します．塗るのではありません．入力の色は捨てるので，
反転・中割り・色替えをしても光の向きが壊れません．

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

`conform` は，拡大された（さらに JPEG を通ったかもしれない）画像から元の格子を
復元して等倍へ戻します．格子が一様でないときは**推測せず拒否**します —
非一様な格子は決定論的には戻せないので，そこは人に返します．

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

`project` は `--from` と `--facing` を**必須**にしています．どの面を倒すのか，
どちらを向いているのかは画素からは読めないので，推測すると**外れたときだけ
静かに壊れます**．

## 拡縮と回転

```sh
pxsmith scale in.px.toml out.px.toml --factor 4          # 厳密．添字の置き換え
pxsmith rotate in.px.toml out.px.toml --degrees 30 --algo cleanedge
```

整数倍の拡大と 90 度の倍数の回転は，標本ではなく**添字の置き換え**として書いて
あります．だから 4 回まわすと元の絵にきっちり戻ります．`cleanedge` が効くのは
**拡大を伴う回転**で，等倍では既定の `nearest` の方が良い — CLI がそう言います．

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

`palette report` は 1 つの割合ではなく**4 通り**を並べ，さらに
「その添字の合計面積」と「1 つながりの塊として最大」を**分けて**出します —
撒かれた色は主な色ではないのに，合計だけ見るとそう読めてしまうためです．

## チェックアウトからのビルド

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

`cargo-make` のタスクは `Makefile.toml` にあります．

```sh
cargo make format-all   # taplo + clippy + rustfmt
cargo make test
```
