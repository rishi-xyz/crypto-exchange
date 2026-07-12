use std::{collections::VecDeque, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::types::{OrderId, Price, Quantity, TradeId, UserId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeInfo {
    order_id: OrderId,
    price: Price,
    quantity: Quantity,
    user_id: UserId
}

impl TradeInfo {
    pub fn new(
        order_id: OrderId,
        price: Price,
        quantity: Quantity,
        user_id: UserId
    ) -> Self {
        TradeInfo { order_id, price, quantity, user_id }
    }

    pub fn get_order_id(&self) -> OrderId { self.order_id }

    pub fn get_price(&self) -> Price { self.price }

    pub fn get_quantity(&self) -> Quantity { self.quantity }

    pub fn get_user_id(&self) -> UserId { self.user_id }
}

type TradeInfoPointer = Arc<TradeInfo>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    trade_id: TradeId,
    timestamp: u64,
    bid_trade: TradeInfoPointer,
    ask_trade: TradeInfoPointer,
}

impl Trade {
    pub fn new(trade_id: TradeId, timestamp: u64, bid_trade: TradeInfoPointer, ask_trade: TradeInfoPointer) 
    ->Self {
        return Trade { trade_id, timestamp, bid_trade, ask_trade };
    }

    pub fn get_trade_id(&self) -> TradeId {
        self.trade_id
    }

    pub fn get_timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn set_trade_id(&mut self, id: TradeId) {
        self.trade_id = id;
    }

    pub fn set_timestamp(&mut self, ts: u64) {
        self.timestamp = ts;
    }

    pub fn get_bid_trade_info(&self)->TradeInfoPointer {
        return self.bid_trade.clone();
    }

    pub fn get_ask_trade_info(&self)->TradeInfoPointer {
        return self.ask_trade.clone();
    }
}

pub type Trades = VecDeque<Trade>; 