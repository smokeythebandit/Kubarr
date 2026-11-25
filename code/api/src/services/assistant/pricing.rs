//! Models the assistant may use and what their tokens cost.
//!
//! Prices are Anthropic's first-party list prices per million tokens. They are
//! kept here as nano-dollars per token (1 USD per MTok = 1 000 nano-dollars per
//! token) so every computation is exact integer math; the total is rounded up
//! to whole micro-dollars when it is stored.

use serde::{Deserialize, Serialize};

use super::client::Usage;

/// A model the owner can select in Settings → Assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum AssistantModel {
    #[serde(rename = "claude-opus-5-5")]
    Opus55,
    #[serde(rename = "claude-sonnet-5-5")]
    Sonnet55,
    #[serde(rename = "claude-haiku-5-5")]
    Haiku55,
}

/// Price of one token class, in nano-dollars per token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prices {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

/// Haiku 5.5 bills prompts above this many input tokens at its long-prompt rate.
const HAIKU_LONG_PROMPT_TOKENS: u64 = 100_000;

impl AssistantModel {
    pub const ALL: [AssistantModel; 3] = [Self::Opus55, Self::Sonnet55, Self::Haiku55];

    /// The model ID sent to the Messages API.
    pub fn id(self) -> &'static str {
        match self {
            Self::Opus55 => "claude-opus-5-5",
            Self::Sonnet55 => "claude-sonnet-5-5",
            Self::Haiku55 => "claude-haiku-5-5",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|model| model.id() == id)
    }

    /// Prices for a request whose prompt holds `prompt_tokens` input tokens
    /// (uncached + cache read + cache write).
    ///
    /// Cache reads cost 10 % of input and 5-minute cache writes 125 %.
    pub fn prices(self, prompt_tokens: u64) -> Prices {
        let (input, output, cache_read) = match self {
            // $4 / $20 / $0.20 per MTok
            Self::Opus55 => (4_000, 20_000, 200),
            // $2 / $10 / $0.10 per MTok
            Self::Sonnet55 => (2_000, 10_000, 100),
            // $0.50 / $2.50 per MTok above 100K prompt tokens
            Self::Haiku55 if prompt_tokens > HAIKU_LONG_PROMPT_TOKENS => (500, 2_500, 50),
            // $0.10 / $0.50 per MTok, cache read 10 % of input
            Self::Haiku55 => (100, 500, 10),
        };
        Prices {
            input,
            output,
            cache_read,
            cache_write: input * 5 / 4,
        }
    }

    /// Cost of one response's `usage`, in micro-dollars, rounded up.
    pub fn cost_usd_micros(self, usage: &Usage) -> i64 {
        let cache_read = usage.cache_read_input_tokens.unwrap_or(0);
        let cache_write = usage.cache_creation_input_tokens.unwrap_or(0);
        let prices = self.prices(usage.input_tokens + cache_read + cache_write);
        let nanos = u128::from(usage.input_tokens) * u128::from(prices.input)
            + u128::from(usage.output_tokens) * u128::from(prices.output)
            + u128::from(cache_read) * u128::from(prices.cache_read)
            + u128::from(cache_write) * u128::from(prices.cache_write);
        i64::try_from(nanos.div_ceil(1_000)).unwrap_or(i64::MAX)
    }
}

impl std::fmt::Display for AssistantModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, output: u64, read: u64, write: u64) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_input_tokens: Some(read),
            cache_creation_input_tokens: Some(write),
        }
    }

    const MTOK: u64 = 1_000_000;

    #[test]
    fn ids_round_trip() {
        for model in AssistantModel::ALL {
            assert_eq!(AssistantModel::parse(model.id()), Some(model));
            assert_eq!(
                serde_json::to_value(model).unwrap(),
                serde_json::json!(model.id())
            );
        }
        assert_eq!(AssistantModel::parse("claude-opus-5"), None);
    }

    #[test]
    fn opus_prices_per_million_tokens() {
        let m = AssistantModel::Opus55;
        assert_eq!(m.cost_usd_micros(&usage(MTOK, 0, 0, 0)), 4_000_000);
        assert_eq!(m.cost_usd_micros(&usage(0, MTOK, 0, 0)), 20_000_000);
        assert_eq!(m.cost_usd_micros(&usage(0, 0, MTOK, 0)), 200_000);
        assert_eq!(m.cost_usd_micros(&usage(0, 0, 0, MTOK)), 5_000_000);
    }

    #[test]
    fn sonnet_prices_per_million_tokens() {
        let m = AssistantModel::Sonnet55;
        assert_eq!(m.cost_usd_micros(&usage(MTOK, 0, 0, 0)), 2_000_000);
        assert_eq!(m.cost_usd_micros(&usage(0, MTOK, 0, 0)), 10_000_000);
        assert_eq!(m.cost_usd_micros(&usage(0, 0, MTOK, 0)), 100_000);
        assert_eq!(m.cost_usd_micros(&usage(0, 0, 0, MTOK)), 2_500_000);
    }

    #[test]
    fn haiku_prices_per_million_tokens() {
        let m = AssistantModel::Haiku55;
        // Short prompts: $0.10 in, $0.50 out, cache read 10 % of input.
        assert_eq!(m.cost_usd_micros(&usage(50_000, 0, 0, 0)), 5_000);
        assert_eq!(m.cost_usd_micros(&usage(1_000, MTOK, 0, 0)), 500_100);
        assert_eq!(m.cost_usd_micros(&usage(0, 0, 100_000, 0)), 1_000);
        assert_eq!(m.cost_usd_micros(&usage(0, 0, 0, 80_000)), 10_000);
        // Above 100K prompt tokens the long-prompt rate applies to everything.
        assert_eq!(m.cost_usd_micros(&usage(MTOK, 0, 0, 0)), 500_000);
        assert_eq!(m.cost_usd_micros(&usage(MTOK, MTOK, 0, 0)), 3_000_000);
    }

    #[test]
    fn mixed_usage_adds_every_token_class() {
        // 1 200 in, 300 out, 10 000 cache read, 2 000 cache write on Opus:
        // 1200*4000 + 300*20000 + 10000*200 + 2000*5000 nanos = 22.8 ms$.
        let cost = AssistantModel::Opus55.cost_usd_micros(&usage(1_200, 300, 10_000, 2_000));
        assert_eq!(cost, 22_800);
    }

    #[test]
    fn fractions_round_up_and_missing_cache_fields_are_zero() {
        let one_token = Usage {
            input_tokens: 1,
            output_tokens: 0,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        };
        // 0.1 µ$ for one Haiku input token still counts as 1 µ$.
        assert_eq!(AssistantModel::Haiku55.cost_usd_micros(&one_token), 1);
        assert_eq!(AssistantModel::Opus55.cost_usd_micros(&one_token), 4);
        assert_eq!(
            AssistantModel::Opus55.cost_usd_micros(&usage(0, 0, 0, 0)),
            0
        );
    }
}
