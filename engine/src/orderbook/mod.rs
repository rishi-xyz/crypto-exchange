use std::{
    cmp::min, 
    collections::{
        BTreeMap, 
        HashMap, 
        VecDeque
    }, 
    sync::{
        Arc, 
        Mutex, 
        MutexGuard
    }
};

use crate::{ 
    level_info::{LevelInfo, OrderBookLevelInfo}, order::{
        Order, OrderPointer, OrderPointers
    }, order_modify::OrderModify, trade::{
        Trade, 
        TradeInfo, 
        Trades
    }, types::{
        OrderId, OrderType, Price, Quantity, Side, UserId
    }
};

#[derive(Debug)]
pub struct OrderBook {
    bids_map: BTreeMap<Price, OrderPointers>,
    asks_map: BTreeMap<Price,OrderPointers>,
    orders_map: HashMap<OrderId, Order>
}

impl OrderBook {
    pub fn new() ->Self {
        let bids_map: BTreeMap<Price, OrderPointers> = BTreeMap::new();
        let asks_map: BTreeMap<Price, OrderPointers> = BTreeMap::new();
        let orders_map: HashMap<OrderId, Order> = HashMap::new();
        OrderBook { bids_map , asks_map, orders_map }
    }

    fn can_match(&self, side: Side, price: Price) ->bool {
        match side {
            Side::Buy => {
                if let Some((&best_ask, _)) = self.asks_map.first_key_value() {
                    price >= best_ask
                } else {
                    false
                }
            }
            Side::Sell => {
                if let Some((&best_bid, _)) = self.bids_map.last_key_value() {
                    price <= best_bid
                } else {
                    false
                }
            }
        }
    }

    fn match_order(&mut self) ->Trades {
        let mut trades:Trades = Trades::new();
        trades.reserve(self.orders_map.len()/2);
        loop {
            if self.bids_map.is_empty() || self.asks_map.is_empty(){ break; }
            let bid_price:Price = *self.bids_map.last_key_value().unwrap().0;
            let ask_price:Price = *self.asks_map.first_key_value().unwrap().0;
            if bid_price < ask_price { break; }
            let bids:&mut VecDeque<Arc<Mutex<Order>>> = self.bids_map.get_mut(&bid_price).unwrap();
            let asks:&mut VecDeque<Arc<Mutex<Order>>> = self.asks_map.get_mut(&ask_price).unwrap();
            while bids.len() != 0 && asks.len() != 0 {
                let mut bid: MutexGuard<'_, Order> = bids.front().unwrap().lock().unwrap();
                let mut ask: MutexGuard<'_, Order> = asks.front().unwrap().lock().unwrap();
                let quantity: Quantity = min(
                    bid.get_remaining_quantity(),
                    ask.get_remaining_quantity()
                );
                let _ = bid.fills(quantity);
                let _ = ask.fills(quantity);

                let bid_filled: bool = bid.is_filled();
                let ask_filled: bool = ask.is_filled();
                let bid_id: OrderId = bid.get_order_id();
                let ask_id: OrderId = ask.get_order_id();
                let bid_price: Price = bid.get_price();
                let ask_price: Price = ask.get_price();
                let bid_user_id: UserId = bid.get_user_id();
                let ask_user_id: UserId = ask.get_user_id();

                drop(bid);
                drop(ask);

                if bid_filled {
                    bids.pop_front();
                    self.orders_map.remove(&bid_id);
                }
                if ask_filled {
                    asks.pop_front();
                    self.orders_map.remove(&ask_id);
                }
                trades.push_back(Trade::new(
                    0,
                    0,
                    Arc::new(TradeInfo::new(bid_id, bid_price, quantity,bid_user_id)),
                    Arc::new(TradeInfo::new(ask_id, ask_price, quantity,ask_user_id)),
                ));
            }
            if bids.is_empty(){
                self.bids_map.remove(&bid_price);
            }
            if asks.is_empty(){
                self.asks_map.remove(&ask_price);
            }
        }
        if !self.bids_map.is_empty()  {
            let bid_price:Price = *self.bids_map.last_key_value().unwrap().0;
            let (is_fak, order_id) = {
                let bids = self.bids_map.get_mut(&bid_price).unwrap();
                let order = bids.front().unwrap().lock().unwrap();
                (order.get_type() == OrderType::FillAndKill,order.get_order_id())
            };
            if is_fak { let _ = self.cancel_order(&order_id); }
        }
        if !self.asks_map.is_empty() {
            let ask_price:Price = *self.asks_map.first_key_value().unwrap().0;
            let (is_fak, order_id) = {
                let asks = self.asks_map.get_mut(&ask_price).unwrap();
                let order = asks.front().unwrap().lock().unwrap();
                (order.get_type() == OrderType::FillAndKill,order.get_order_id())
            };
            if is_fak { let _ = self.cancel_order(&order_id); }
        }
        return trades
    }
    
    pub fn cancel_order(&mut self,order_id: &OrderId) ->Option<Order> {
        let order = self.orders_map.remove(order_id)?;
        let price:Price = order.get_price();
        let side:Side = order.get_side();

        let level = match side {
            Side::Buy => self.bids_map.get_mut(&price),
            Side::Sell => self.asks_map.get_mut(&price),
        };
        if let Some(orders) = level {
            orders.retain(|arc| arc.lock().unwrap().get_order_id() != *order_id);
            if orders.is_empty() {
                match side {
                    Side::Buy => self.bids_map.remove(&price),
                    Side::Sell => self.asks_map.remove(&price),
                };
            }
        }
        Some(order)
    }
    
    pub fn add_order(&mut self,order: OrderPointer) ->Option<Trades> {
        let (order_id, order_type,order_side,order_price) = {
            let order = order.lock().unwrap();
            (
                order.get_order_id(),
                order.get_type(),
                order.get_side(), 
                order.get_price()
            )
        };
        if self.orders_map.contains_key(&order_id) { return None  }; 
        if order_type == OrderType::FillAndKill && !self.can_match(order_side, order_price){ return None };
        self.orders_map.insert(order_id, order.lock().unwrap().clone());
        let level = match order_side {
            Side::Buy => self.bids_map.entry(order_price).or_default(),
            Side::Sell => self.asks_map.entry(order_price).or_default(),
        };
        level.push_back(order);
        Some(self.match_order())
    }
    
    pub fn modify_orders(&mut self,order_modify: OrderModify) ->Option<Trades> {
        let order_id = order_modify.get_order_id();
        if !self.orders_map.contains_key(&order_id){ return None};
        let order_type = {
            let existing = self.orders_map.get(&order_id).unwrap();
            existing.get_type()
        };
        let new_order = Arc::new(Mutex::new(Order::new(
            order_id,
            order_type,
            order_modify.get_side(),
            order_modify.get_status(),
            order_modify.get_price(),
            order_modify.get_quantity(),
            order_modify.get_user_id()
        )));
        let _ = self.cancel_order(&order_id);
        self.add_order(new_order)
    }
    
    pub fn size(&self) -> usize  {
        return self.orders_map.len()
    }

    pub fn has_order(&self, order_id: &OrderId) -> bool {
        self.orders_map.contains_key(order_id)
    }

    pub fn get_order_type(&self, order_id: &OrderId) -> Option<OrderType> {
        self.orders_map.get(order_id).map(|o| o.get_type())
    }
    
    pub fn get_order_info(&self) ->OrderBookLevelInfo {
        let mut bids_info = VecDeque::new(); 
        let mut ask_info = VecDeque::new();
        for (price,orders) in self.bids_map.iter() {
            let total_qty:Quantity = orders.iter().map(|arc| arc.lock().unwrap().get_remaining_quantity()).sum();
            bids_info.push_front(LevelInfo::new(*price, total_qty));
        }
        for (price,orders) in self.asks_map.iter() {
            let total_qty:Quantity = orders.iter().map(|arc| arc.lock().unwrap().get_remaining_quantity()).sum();
            ask_info.push_back(LevelInfo::new(*price,total_qty));
        }
        OrderBookLevelInfo::new(Arc::new(bids_info),Arc::new(ask_info))
    }
}