use std::collections::HashMap;

use crate::{
    level_info::OrderBookLevelInfo,
    order::{Order, OrderPointer},
    order_modify::OrderModify,
    orderbook::OrderBook,
    trade::Trades,
    trading_pair::TradingPair,
    types::{Asset, OrderId, Quantity, Side, UserId},
    users::User,
};

use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct Engine {
    orderbooks: HashMap<TradingPair, OrderBook>,
    users: HashMap<UserId, User>,
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            orderbooks: HashMap::new(),
            users: HashMap::new(),
        }
    }

    pub fn add_trading_pair(&mut self, pair: TradingPair) {
        self.orderbooks.entry(pair).or_insert(OrderBook::new());
    }

    pub fn remove_trading_pair(&mut self, pair: &TradingPair) -> Option<OrderBook> {
        self.orderbooks.remove(pair)
    }

    pub fn add_user(&mut self, user: User) {
        let id = user.get_user_id();
        self.users.insert(id, user);
    }

    pub fn remove_user(&mut self, user_id: &UserId) -> Option<User> {
        self.users.remove(user_id)
    }

    pub fn deposit(
        &mut self,
        user_id: UserId,
        asset: Asset,
        amount: Quantity,
    ) -> Result<(), String> {
        self.users
            .get_mut(&user_id)
            .ok_or("User not found")?
            .add_balance(asset, amount);
        Ok(())
    }

    pub fn get_user_balance(
        &self,
        user_id: &UserId,
    ) -> Option<HashMap<Asset, (Quantity, Quantity)>> {
        let user = self.users.get(user_id)?;
        let mut result = HashMap::new();
        for asset in [
            Asset::ETH,
            Asset::SOL,
            Asset::BTC,
            Asset::USDC,
            Asset::USDT,
        ] {
            let bal = user.get_balance(&asset);
            let locked = user.get_locked(&asset);
            if bal > 0 || locked > 0 {
                result.insert(asset, (bal, locked));
            }
        }
        Some(result)
    }

    pub fn add_order(
        &mut self,
        user_id: UserId,
        pair: &TradingPair,
        order: OrderPointer,
    ) -> Result<Option<Trades>, String> {
        let (order_id, side, price, quantity) = {
            let o = order.lock().unwrap();
            (
                o.get_order_id(),
                o.get_side(),
                o.get_price(),
                o.get_initial_quantity(),
            )
        };

        let (lock_asset, lock_amount) = match side {
            Side::Buy => (
                pair.quote,
                (price as u32)
                    .checked_mul(quantity)
                    .ok_or("Lock amount overflow")?,
            ),
            Side::Sell => (pair.base, quantity),
        };

        {
            let user = self.users.get_mut(&user_id).ok_or("User not found")?;
            user.lock(order_id, lock_asset, lock_amount)?;
        }

        let book = self
            .orderbooks
            .get_mut(pair)
            .ok_or("Trading pair not found")?;

        match book.add_order(order) {
            Some(trades) => {
                let incoming_id = order_id;
                let incoming_side = side;

                for trade in &trades {
                    let bid = trade.get_bid_trade_info();
                    let ask = trade.get_ask_trade_info();
                    let qty = bid.get_quantity();

                    let quote_amount = match incoming_side {
                        Side::Buy => (ask.get_price() as u32) * qty,
                        Side::Sell => (bid.get_price() as u32) * qty,
                    };

                    if let Some(buyer) = self.users.get_mut(&bid.get_user_id()) {
                        let _ = buyer.apply_fill(
                            bid.get_order_id(),
                            pair.quote,
                            quote_amount,
                            pair.base,
                            qty,
                        );
                    }
                    if let Some(seller) = self.users.get_mut(&ask.get_user_id()) {
                        let _ = seller.apply_fill(
                            ask.get_order_id(),
                            pair.base,
                            qty,
                            pair.quote,
                            quote_amount,
                        );
                    }
                }

                if !self
                    .orderbooks
                    .get(pair)
                    .map_or(false, |b| b.has_order(&incoming_id))
                {
                    if let Some(user) = self.users.get_mut(&user_id) {
                        let _ = user.unlock_order(&incoming_id);
                    }
                }

                Ok(Some(trades))
            }
            None => {
                if let Some(user) = self.users.get_mut(&user_id) {
                    let _ = user.unlock_order(&order_id);
                }
                Ok(None)
            }
        }
    }

    pub fn cancel_order(&mut self, pair: &TradingPair, order_id: &OrderId) -> bool {
        let order = match self
            .orderbooks
            .get_mut(pair)
            .and_then(|book| book.cancel_order(order_id))
        {
            Some(order) => order,
            None => return false,
        };

        let uid = order.get_user_id();
        if let Some(user) = self.users.get_mut(&uid) {
            user.unlock_order(order_id).is_ok()
        } else {
            true
        }
    }

    pub fn modify_order(
        &mut self,
        pair: &TradingPair,
        modify_order: OrderModify,
    ) -> Option<Trades> {
        let order_id = modify_order.get_order_id();
        let user_id = modify_order.get_user_id();

        let order_type = self
            .orderbooks
            .get(pair)?
            .get_order_type(&order_id)?;

        self.cancel_order(pair, &order_id);

        let new_order = Arc::new(Mutex::new(Order::new(
            order_id,
            order_type,
            modify_order.get_side(),
            modify_order.get_status(),
            modify_order.get_price(),
            modify_order.get_quantity(),
            user_id,
        )));

        self.add_order(user_id, pair, new_order).ok()?
    }

    pub fn get_order_info(&self, pair: &TradingPair) -> Option<OrderBookLevelInfo> {
        self.orderbooks
            .get(pair)
            .map(|book: &OrderBook| book.get_order_info())
    }

    pub fn size(&self, pair: &TradingPair) -> Option<usize> {
        self.orderbooks.get(pair).map(|book| book.size())
    }
}
