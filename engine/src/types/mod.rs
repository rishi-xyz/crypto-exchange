pub type Price  = i32;
pub type Quantity = u32;
pub type OrderId = u64;

#[derive(Debug, Clone, Copy)]
pub enum OrderType {
    GoodTillCancel,
    GoodForDay,
    FillAndKill,
    FillOrKill,
}

#[derive(Debug, Clone, Copy)]
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