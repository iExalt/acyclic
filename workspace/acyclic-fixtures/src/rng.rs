//! A small, self-contained random number generator, so a seed gives the same plan across dependency upgrades.

/// `SplitMix64`: fast, and stable by construction.
#[derive(Debug, Clone)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    /// A generator from a seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A uniform value in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A uniform index below `n`.
    ///
    /// # Panics
    ///
    /// Panics when `n` is 0.
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "below(0) has no values");
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }

    /// `true` with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// A count with a Poisson distribution whose mean `m` is given as `limit` = e^-m, so callers can pass a
    /// constant and the result never depends on the platform's `exp`.
    pub fn poisson(&mut self, limit: f64) -> usize {
        let mut product = self.unit();
        let mut count = 0;
        while product > limit {
            product *= self.unit();
            count += 1;
        }
        count
    }

    /// Shuffles a slice in place (Fisher–Yates).
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_gives_a_fixed_sequence() {
        // The reference values of SplitMix64 for seed 0; a change here changes every generated plan.
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(rng.next_u64(), 0x6e78_9e6a_a1b9_65f4);
    }

    #[test]
    fn helpers_stay_in_range() {
        let mut rng = SplitMix64::new(1);
        for _ in 0..1000 {
            assert!(rng.below(7) < 7);
            assert!((0.0..1.0).contains(&rng.unit()));
        }
        let mean =
            (0..10_000).map(|_| rng.poisson((-1.3f64).exp())).sum::<usize>() as f64 / 10_000.0;
        assert!((1.2..1.4).contains(&mean), "mean {mean}");
    }
}
