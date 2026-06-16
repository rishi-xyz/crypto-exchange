use std::{collections::HashMap, fs::TryLockError::Error};

use crate::{
    level_info::OrderBookLevelInfo, order::OrderPointer, order_modify::OrderModify, orderbook::OrderBook, trade::Trades, trading_pair::TradingPair, types::OrderId
};

pub struct Engine {
    orderbooks: HashMap<TradingPair,OrderBook>
}

impl Engine {
    pub fn new() ->Self {
        let order_book:HashMap<TradingPair,OrderBook>  = HashMap::new();
        Engine { orderbooks: order_book  }
    }
    // create orderbook
    pub fn add_trading_pair(&mut self, pair:TradingPair) {
        self.orderbooks.entry(pair).or_insert(OrderBook::new());    
    }

    pub fn remove_trading_pair(&mut self,pair:&TradingPair) ->Option<OrderBook> {
        self.orderbooks.remove(pair)
    }

    pub fn add_order(&mut self, pair:&TradingPair, order: OrderPointer) -> Option<Trades> {
        self.orderbooks.get_mut(pair)?.add_order(order)
    }

    pub fn cancel_order(&mut self, pair:&TradingPair, order_id: &OrderId) ->bool {
        if let Some(book) = self.orderbooks.get_mut(pair) {
            book.cancel_order(order_id);
            return true
        };
        return false
    }

    pub fn modify_order(&mut self, pair:&TradingPair, modify_order:OrderModify) ->Option<Trades> {
        self.orderbooks.get_mut(pair)?.modify_orders(modify_order)
    }

    pub fn get_order_info(&self, pair:&TradingPair) -> Option<OrderBookLevelInfo> {
        self.orderbooks.get(pair).map(|book: &OrderBook| book.get_order_info())
    }

    pub fn size(&self,pair:&TradingPair) ->Option<usize> {
        self.orderbooks.get(pair).map(|book| book.size())
    }

}