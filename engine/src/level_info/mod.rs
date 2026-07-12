//! Orderbook depth snapshot types.
//!
//! Provides [`LevelInfo`] (a single price level) and [`OrderBookLevelInfo`]
//! (a full book snapshot) for read-only queries. These are cheap to construct
//! and are returned by [`OrderBook::get_order_info`](crate::orderbook::OrderBook::get_order_info).

use std::{collections::VecDeque, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::types::{Price, Quantity};

/// A single price level with its aggregated quantity.
///
/// Used in orderbook depth snapshots. `quantity` is the sum of
/// `remaining_quantity` across all orders at this price level.
///
/// # Examples
///
/// ```ignore
/// let level = LevelInfo::new(50000, 120);
/// assert_eq!(level.price, 50000);
/// assert_eq!(level.quantity, 120);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelInfo {
    /// Price in quote units
    pub price : Price,
    /// Total remaining quantity across all orders at this price
    pub quantity : Quantity
}

impl LevelInfo {
    /// Creates a new price level snapshot.
    pub fn new(price:Price, quantity:Quantity) ->Self {
        LevelInfo { price, quantity }
    }
}

/// Thread-safe, shared slice of price levels.
pub type LevelInfos = Arc<VecDeque<LevelInfo>>;

/// A full orderbook depth snapshot — bids and asks at each price level.
///
/// Returned by [`OrderBook::get_order_info`](crate::orderbook::OrderBook::get_order_info)
/// and [`Engine::get_order_info`](crate::matching_engine::Engine::get_order_info).
/// Bids are sorted ascending by price (best bid last); asks are sorted
/// ascending by price (best ask first).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookLevelInfo {
    /// Bid levels, sorted ascending by price (best bid is the last element)
    bids : LevelInfos,
    /// Ask levels, sorted ascending by price (best ask is the first element)
    asks : LevelInfos,
}

impl OrderBookLevelInfo {
    /// Creates a new orderbook level info snapshot.
    ///
    /// # Arguments
    ///
    /// * `bids_` — Bid price levels (ascending by price)
    /// * `asks_` — Ask price levels (ascending by price)
    pub fn new(bids_ :LevelInfos,asks_ :LevelInfos) ->Self {
        return OrderBookLevelInfo { 
            bids: bids_, 
            asks: asks_ 
        };
    }

    /// Returns the bid levels (ascending by price; best bid is last).
    pub fn get_bids(&self) ->LevelInfos {
        return self.bids.clone();
    }

    /// Returns the ask levels (ascending by price; best ask is first).
    pub fn get_asks(&self) ->LevelInfos {
        return  self.asks.clone();
    }
}
