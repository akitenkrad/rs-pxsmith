//! 決定論的な擬似乱数．
//!
//! 校正は「同じ種から同じ評価データセットが出る」ことが前提なので，標準ライブラリの
//! `RandomState` も外部 crate も使わず splitmix64 を自前で持つ．外部 crate の実装が
//! 版で変わると，同じ種から違うデータセットが出て過去の校正結果と比べられなくなる
//! (設計書 6.15 の決定論性)．

/// splitmix64．状態 1 語だけの生成器で，種の分配にもそのまま使える．
#[derive(Copy, Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// `0..n` の一様乱数．`n = 0` は呼び出し側の誤りなので 0 を返す．
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        // 素朴な剰余で入る偏りは 2^64 / n に対して無視できる大きさである．
        (self.next_u64() % u64::from(n)) as u32
    }

    /// `lo..=hi` の一様乱数．
    pub fn range(&mut self, lo: u32, hi: u32) -> u32 {
        debug_assert!(lo <= hi);
        lo + self.below(hi - lo + 1)
    }

    /// 候補から 1 つ選ぶ．
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u32) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_sequence() {
        let a: Vec<u64> = (0..8)
            .scan(Rng::new(42), |r, _| Some(r.next_u64()))
            .collect();
        let b: Vec<u64> = (0..8)
            .scan(Rng::new(42), |r, _| Some(r.next_u64()))
            .collect();
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn below_stays_in_range_and_covers_it() {
        let mut r = Rng::new(7);
        let mut seen = [false; 5];
        for _ in 0..200 {
            let v = r.below(5);
            assert!(v < 5);
            seen[v as usize] = true;
        }
        assert!(seen.iter().all(|s| *s), "5 面の目が全部は出ていない");
    }

    #[test]
    fn range_includes_both_ends() {
        let mut r = Rng::new(9);
        let mut lo = false;
        let mut hi = false;
        for _ in 0..200 {
            let v = r.range(3, 6);
            assert!((3..=6).contains(&v));
            lo |= v == 3;
            hi |= v == 6;
        }
        assert!(lo && hi, "両端が出ていない");
    }
}
