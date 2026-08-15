# レシピ

[English](recipes.md) | **日本語**

[← README へ戻る](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.ja.md)

レシピは TOML ファイルとして記述します．変数，制限された式評価器，直積を展開する
`for_each` を備えていますが，ループも関数定義も I/O も持ちません．この制限は意図的な
もので，ステップキーが逐次的に決定されるため，変わっていないステップを実行することなく
「変わっていない」と判定できるようになります．

```toml
[project]
format = 1

[vars]
seeds = ["hero", "slime"]

[[step]]
op = "shade"
input = "src/${s}.png"
output = "out/${s}.px.toml"
base = "8A6A4A"
light = "dir:-0.6,0.8"
for_each = { s = "${seeds}" }

[[step]]
op = "anim.squash"
input = "out/hero.px.toml"
output = "out/squashed.px.toml"
amount = -0.3
```

`op` は CLI のサブコマンドと 1 対 1 に対応しており，`op = "anim.squash"` は
`pxsmith anim squash` を意味します．引数の名前と順序は手書きの対応表からではなく，
コマンドライン parser から読み出しています．対応表を二重に持てば必ずずれが生じますが，
parser を直接読む限りずれようがないためです．

## 走らせる

```sh
pxsmith run build.toml --dry-run   # 順序だけ出す．何も走らせない
pxsmith run build.toml --explain   # 各ステップのキーと argv を出す
pxsmith run build.toml --gc        # このレシピが使わなくなったキャッシュを落とす

# ある成果物が «どう出来たか» を GIF にする (系譜をビルド順に)
pxsmith run build.toml --progress how.gif --progress-of out/hero.px.toml

# 外部データからレシピを起こす (1 行につき 1 つの [[step]]．対応関係が保たれる)
pxsmith recipe expand template.toml build.toml --data rows.csv
```

生成過程を記録した GIF は，コマごとに局所カラーテーブルを書き出すため，色は入力した
とおりに出力されます．添字は `u8` でありアルファは 2 値であって，これはちょうど GIF の
コマが保持できる情報そのものであるため，量子化をやり直す必要がありません．

## キャッシュ

変わっていないレシピを再実行すると，すべて `.pxcache/` から復元されます．64 枚の
スプライトに対する 128 ステップの構成では，冷えた状態で 2.66 秒，温まった状態で
0.09 秒でした．入力を 1 つ変更した場合には，それに依存する 2 ステップだけが再ビルド
されます．

生成物を含むビルドが再現するのも，このキャッシュによるものです．生成のステップ自体は
決定論的ではありません．採用したモデルは seed を受け付けないためです．ビルドが繰り返し
可能であるのは結果をキャッシュしてコミットするからであって，モデルが 2 度同じ答えを
返すからではありません．詳しくは[生成](generation.ja.md)を参照してください．

## 決定論性

スレッド数を変更しても出力はバイト単位で一致します．`RAYON_NUM_THREADS` を変えても
成果物は変化しません．これは主張として書いているのではなく，試験によって確かめて
います．
