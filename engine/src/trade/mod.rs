use std::{collections::VecDeque, sync::Arc};

use crate::types::{OrderId, Price, Quantity, UserId};

#[derive(Debug)]
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

#[derive(Debug)]
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