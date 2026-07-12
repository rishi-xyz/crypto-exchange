use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type Price  = i32;
pub type Quantity = u32;
pub type OrderId = u64;
pub type TradeId = u64;
pub type UserId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OrderType {
    GoodTillCancel,
    GoodForDay,
    FillAndKill,
    FillOrKill,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OrderStatus {
    Cancelled,
    PartiallyFilled,
    Filled,
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash)]
pub enum Asset {
    ETH,
    SOL,
    BTC,
    USDC,
    USDT
}