use crate::types::Asset;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TradingPair {
    pub base: Asset,
    pub quote: Asset
}

impl TradingPair {
    pub fn new(base: Asset, quote: Asset) -> Self {
        TradingPair { base, quote }
    }
}