use std::ops::AddAssign;

use serde::{Deserialize, Serialize};

/// Token counts normalized across providers. Missing fields are zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

impl AddAssign for Usage {
    fn add_assign(&mut self, o: Self) {
        self.input = self.input.saturating_add(o.input);
        self.output = self.output.saturating_add(o.output);
        self.cache_read = self.cache_read.saturating_add(o.cache_read);
        self.cache_write = self.cache_write.saturating_add(o.cache_write);
        self.reasoning = self.reasoning.saturating_add(o.reasoning);
    }
}

/// Money in micro-units of USD (1 USD = `1_000_000`) to avoid float drift in sums.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cost {
    pub micro_usd: u64,
}

impl Cost {
    /// Cost of `tokens` at a price given in micro-USD per million tokens.
    pub fn for_tokens(tokens: u64, micro_usd_per_million: u64) -> Self {
        let v = u128::from(tokens) * u128::from(micro_usd_per_million) / 1_000_000;
        Self {
            micro_usd: u64::try_from(v).unwrap_or(u64::MAX),
        }
    }

    pub fn usd(self) -> f64 {
        #[allow(clippy::cast_precision_loss)] // display only
        let v = self.micro_usd as f64;
        v / 1_000_000.0
    }
}

impl AddAssign for Cost {
    fn add_assign(&mut self, o: Self) {
        self.micro_usd = self.micro_usd.saturating_add(o.micro_usd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_math_is_exact_and_saturating() {
        // 1500 tokens at $3 / 1M = $0.0045
        assert_eq!(Cost::for_tokens(1500, 3_000_000).micro_usd, 4500);
        assert_eq!(Cost::for_tokens(u64::MAX, u64::MAX).micro_usd, u64::MAX);
        let mut c = Cost {
            micro_usd: u64::MAX,
        };
        c += Cost { micro_usd: 1 };
        assert_eq!(c.micro_usd, u64::MAX);
    }

    #[test]
    fn usage_accumulates() {
        let mut u = Usage::default();
        u += Usage {
            input: 10,
            output: 5,
            ..Usage::default()
        };
        u += Usage {
            input: 1,
            reasoning: 2,
            ..Usage::default()
        };
        assert_eq!((u.input, u.output, u.reasoning), (11, 5, 2));
    }
}
