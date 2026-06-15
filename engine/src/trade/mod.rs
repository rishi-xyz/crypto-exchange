use std::{collections::VecDeque, sync::Arc};

use crate::types::{OrderId, Price, Quantity};

pub struct TradeInfo {
    order_id: OrderId,
    price: Price,
    quantity: Quantity,
}

impl TradeInfo {
    pub fn new(
        order_id: OrderId,
        price: Price,
        quantity: Quantity
    ) -> Self {
        TradeInfo { order_id, price, quantity }
    }
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

pub type Trades = VecDeque<Trade>; 