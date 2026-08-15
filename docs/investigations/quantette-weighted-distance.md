# quantette で重み付き色距離を指定できるか

- 対象: `quantette` 0.6.0
- 調査時期: M0
- 関連: 設計書 6.6 (色距離と量子化)，実装計画書 M0「`quantette` の API 確認」

## 問い

設計書 6.6 の色距離

$$ d^2 = w_L (L_1 - L_2)^2 + (a_1 - a_2)^2 + (b_1 - b_2)^2 $$

の $w_L$ を `quantette` に渡せるか．渡せないなら `pxsmith quantize` の $w_L$ は 1.0 固定でよいか．

## 結論

**重みを渡す API は無い．ただし OKLab の $L$ 成分を $\sqrt{w_L}$ 倍して入力すれば，同じ距離を厳密に再現できる．**

`quantette` の距離計算は成分ごとの**重みなし二乗ユークリッド距離**である
(`color_map/nearest_neighbor.rs` の `squared_euclidean_distance` および SIMD 版)．
重みの入る余地は無い．一方で成分を事前に定数倍すれば

$$ (\sqrt{w_L} L_1 - \sqrt{w_L} L_2)^2 + (\Delta a)^2 + (\Delta b)^2 = w_L (\Delta L)^2 + (\Delta a)^2 + (\Delta b)^2 $$

となり，求める距離に一致する．線形変換なので k-means の重心 (成分ごとの平均) とも
整合する — 尺度変換した値の平均は，平均の尺度変換に等しい．

### 手順

1. 入力の `Oklab` を作り，`l *= w_l.sqrt()` する．
2. Wu を使う場合は `BinnerF32x3::new` の第 1 成分の範囲を `(0.0, sqrt(w_l))` にする
   (既定の `oklab_from_srgb8()` は $(0, 1)$ 固定なので，そのままでは全画素が最上位ビンへ潰れる)．
3. 出力パレットの `l` を $\sqrt{w_L}$ で割って元の尺度へ戻してから sRGB へ変換する．

`quantette::color_space::oklab_to_srgb8` は尺度を戻した後にしか使えない．

### 検証

$w_L = 1$ と $w_L = 64$ で，明度がほぼ等しく色相が離れた 2 色 (赤 `c80000` /
青 `0000cd`) と白黒を含む入力を 3 色へ落とした結果．

| $w_L$ | 得られた 3 色 (OKLab，$L$ は元の尺度へ戻したもの) |
| --- | --- |
| 1 | `(0.19, -0.01, -0.13)`，`(0.52, 0.19, 0.10)`，`(1.00, 0.00, 0.00)` |
| 64 | `(0.00, 0.00, 0.00)`，`(0.45, 0.08, -0.08)`，`(1.00, 0.00, 0.00)` |

$w_L = 1$ では黒と青が 1 色へ統合され (明度差より色相差を重く見るため)，
$w_L = 64$ では逆に赤と青が 1 色へ統合されて黒が独立した．**重みが効いている**．

### 型の制約

`ColorComponents` はブランケット実装であり封印されていない．

```rust
pub trait ColorComponents<Component, const N: usize>:
    ArrayCast<Array = [Component; N]> + Copy + Send + Sync + 'static {}

impl<Color, Component, const N: usize> ColorComponents<Component, N> for Color
where Color: ArrayCast<Array = [Component; N]> + Copy + Send + Sync + 'static {}
```

独自の色型を渡すこともできるが，`palette::cast::ArrayCast` の実装に `unsafe` が要る．
本リポジトリは `unsafe_code = "forbid"` なので，**`Oklab` の値を直接スケールする方法を採る**．
新しい型は要らない．

## 設計への反映

設計書 6.6 の割り当て — `pxsmith quantize` は $w_L = 1.0$ 固定 (quantette に委譲)，
`pxsmith palette apply` は $w_L$ 可変 (自前実装) — は**そのまま維持する**．

- 上の手順により将来 `pxsmith quantize --weight-l` を足す余地はあるが，M2 の完了条件には含めない．
- ビナー範囲の調整 (手順 2) を忘れると**黙って結果が壊れる**種類の落とし穴なので，
  実装するときは $w_L \ne 1$ の回帰テストを必ず付ける．
