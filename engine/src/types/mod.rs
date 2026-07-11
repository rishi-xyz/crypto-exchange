use uuid::Uuid;

pub type Price  = i32;
pub type Quantity = u32;
pub type OrderId = u64;
pub type UserId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderType {
    GoodTillCancel,
    GoodForDay,
    FillAndKill,
    FillOrKill,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy)]
pub enum OrderStatus {
    Cancelled,
    PartiallyFilled,
    Filled,
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Asset {
    ETH,
    SOL,
    BTC,
    USDC,
    USDT
}