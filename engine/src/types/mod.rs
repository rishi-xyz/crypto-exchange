pub type Price  = i32;
pub type Quantity = u32;
pub type OrderId = u64;

pub enum OrderType {
    GoodTillCancel,
    GoodForDay,
    FillAndKill,
    FillOrKill,
}

pub enum Side {
    Buy,
    Sell,
}

pub enum OrderStatus {
    Cancelled,
    PartiallyFilled,
    Filled,
    Empty,
}