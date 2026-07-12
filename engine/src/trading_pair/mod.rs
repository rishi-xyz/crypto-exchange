//! Trading pair identifier — a base/quote asset combination.
//!
//! Each [`OrderBook`](crate::orderbook::OrderBook) is keyed by a [`TradingPair`].
//! For example, `ETH-USDC` means "trade ETH (base) for USDC (quote)".

use serde::{Deserialize, Serialize};

use crate::types::Asset;

/// A trading pair represented as base and quote assets.
///
/// For example, `ETH-USDC` means "buy/sell ETH priced in USDC".
/// The base asset is what is being traded; the quote asset is what it is priced in.
///
/// Implements `Hash` and `Eq` so it can be used as a `HashMap` key
/// in [`Engine::orderbooks`](crate::matching_engine::Engine).
///
/// # Examples
///
/// ```ignore
/// let eth_usdc = TradingPair::new(Asset::ETH, Asset::USDC);
/// assert_eq!(eth_usdc.base, Asset::ETH);
/// assert_eq!(eth_usdc.quote, Asset::USDC);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TradingPair {
    /// The asset being traded (e.g. ETH, SOL, BTC)
    pub base: Asset,
    /// The asset used to price the base (e.g. USDC, USDT)
    pub quote: Asset
}

impl TradingPair {
    /// Creates a new trading pair.
    ///
    /// # Arguments
    ///
    /// * `base` — The asset being traded
    /// * `quote` — The pricing currency
    pub fn new(base: Asset, quote: Asset) -> Self {
        TradingPair { base, quote }
    }
}
