use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{Asset, OrderId, Quantity, UserId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    pub asset: Asset,
    pub amount: Quantity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    id: UserId,
    locked_orders: HashMap<OrderId, LockEntry>,
    balances: HashMap<Asset, Quantity>,
}

impl User {
    pub fn new(user_id: Option<UserId>) -> Self {
        User {
            id: user_id.unwrap_or(Uuid::new_v4()),
            locked_orders: HashMap::new(),
            balances: HashMap::new(),
        }
    }

    pub fn add_balance(&mut self, asset: Asset, amount: Quantity) {
        *self.balances.entry(asset).or_insert(0) += amount;
    }

    pub fn get_available_balance(&self, asset: &Asset) -> Quantity {
        let total = self.balances.get(asset).copied().unwrap_or(0);
        let locked: Quantity = self
            .locked_orders
            .values()
            .filter(|e| e.asset == *asset)
            .map(|e| e.amount)
            .sum();
        total.saturating_sub(locked)
    }

    pub fn lock(
        &mut self,
        order_id: OrderId,
        asset: Asset,
        amount: Quantity,
    ) -> Result<(), String> {
        if self.get_available_balance(&asset) < amount {
            return Err("Insufficient balance".into());
        }
        *self.balances.entry(asset).or_insert(0) -= amount;
        self.locked_orders.insert(order_id, LockEntry { asset, amount });
        Ok(())
    }

    pub fn unlock_order(&mut self, order_id: &OrderId) -> Result<(), String> {
        let entry = self
            .locked_orders
            .remove(order_id)
            .ok_or("Order not locked")?;
        *self.balances.entry(entry.asset).or_insert(0) += entry.amount;
        Ok(())
    }

    pub fn apply_fill(
        &mut self,
        order_id: OrderId,
        debit_asset: Asset,
        debit_amount: Quantity,
        credit_asset: Asset,
        credit_amount: Quantity,
    ) -> Result<(), String> {
        let entry = self
            .locked_orders
            .get_mut(&order_id)
            .ok_or("Order not found in locked orders")?;
        if entry.asset != debit_asset {
            return Err("Locked asset mismatch".into());
        }
        if entry.amount < debit_amount {
            return Err("Locked amount insufficient for fill".into());
        }
        entry.amount -= debit_amount;
        if entry.amount == 0 {
            self.locked_orders.remove(&order_id);
        }
        *self.balances.entry(credit_asset).or_insert(0) += credit_amount;
        Ok(())
    }

    pub fn get_user_id(&self) -> UserId {
        self.id
    }

    pub fn get_balance(&self, asset: &Asset) -> Quantity {
        self.balances.get(asset).copied().unwrap_or(0)
    }

    pub fn get_locked(&self, asset: &Asset) -> Quantity {
        self.locked_orders
            .values()
            .filter(|e| e.asset == *asset)
            .map(|e| e.amount)
            .sum()
    }
}
