use std::sync::Arc;

use crate::types::{OrderId, Price, Quantity};

pub struct TradeInfo {
    order_id: OrderId,
    price: Price,
    qunatity: Quantity,
}

type TradeInfoPointer = Arc<TradeInfo>;

pub struct Trade {
    bid_trade: TradeInfoPointer,
    ask_trade: TradeInfoPointer,
}

impl Trade {
    pub fn new(bid_trade: TradeInfoPointer, ask_trade : TradeInfoPointer) 
    ->Self {
        return Trade { bid_trade, ask_trade };
    }

    pub fn get_bid_trade_info(&self)->TradeInfoPointer {
        return self.bid_trade.clone();
    }

    pub fn get_ask_trade_info(&self)->TradeInfoPointer {
        return self.ask_trade.clone();
    }
}