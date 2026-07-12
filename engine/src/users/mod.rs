//! User account and balance management.
//!
//! Each [`User`](crate::users::User) tracks balances across all assets and manages locked funds
//! for resting orders. The locking protocol ensures users cannot spend funds
//! that are committed to open orders.
//!
//! # Balance Semantics
//!
//! ```text
//! available = total_balance - sum(locked amounts across all orders)
//! ```
//!
//! When an order is placed, funds are moved from `available` to `locked`.
//! When an order is cancelled, locked funds return to `available`.
//! When a fill occurs, locked funds are converted to the received asset.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::{debug, trace, warn};
use uuid::Uuid;

use crate::types::{Asset, OrderId, Quantity, UserId};

/// A record of funds locked by a single resting order.
///
/// Created when an order is placed ([`User::lock`]) and removed when the
/// order is cancelled ([`User::unlock_order`]) or fully filled ([`User::apply_fill`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    /// The asset being locked (e.g. USDC for a buy order, ETH for a sell order)
    pub asset: Asset,
    /// Number of units locked by this order
    pub amount: Quantity,
}

/// A user account with balance and order-lock tracking.
///
/// Users are identified by UUID and managed by the
/// [`CoreEngine`](crate::engine::CoreEngine). Each user has:
///
/// - **Balances** — total amount of each asset held
/// - **Locked orders** — funds committed to resting orders, keyed by order ID
///
/// The `available` balance for an asset is `total - locked`. This prevents
/// users from placing orders they cannot cover.
///
/// # Examples
///
/// ```ignore
/// let mut user = User::new(None); // generates random UUID
/// user.add_balance(Asset::USDC, 10000);
/// assert_eq!(user.get_available_balance(&Asset::USDC), 10000);
///
/// user.lock(order_id, Asset::USDC, 5000).unwrap();
/// assert_eq!(user.get_available_balance(&Asset::USDC), 5000);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// UUID v4 identifier
    id: UserId,
    /// Map of order ID → locked funds for that order
    locked_orders: HashMap<OrderId, LockEntry>,
    /// Total balance per asset (includes both available and locked)
    balances: HashMap<Asset, Quantity>,
}

impl User {
    /// Creates a new user.
    ///
    /// # Arguments
    ///
    /// * `user_id` — Optional UUID. If `None`, a random UUID v4 is generated.
    pub fn new(user_id: Option<UserId>) -> Self {
        let id = user_id.unwrap_or(Uuid::new_v4());
        trace!(user_id = %id, "User created");
        User {
            id,
            locked_orders: HashMap::new(),
            balances: HashMap::new(),
        }
    }

    /// Credits the user's balance for the given asset.
    ///
    /// Called by [`ExchangeEngine::deposit`](crate::engine::ExchangeEngine::deposit)
    /// when a user deposits funds.
    ///
    /// # Arguments
    ///
    /// * `asset` — The asset to credit
    /// * `amount` — Number of units to add
    pub fn add_balance(&mut self, asset: Asset, amount: Quantity) {
        let prev = self.balances.get(&asset).copied().unwrap_or(0);
        *self.balances.entry(asset).or_insert(0) += amount;
        debug!(
            user = %self.id,
            asset = ?asset,
            amount,
            previous = prev,
            new_total = self.balances.get(&asset),
            "Balance credited"
        );
    }

    /// Returns the available (unlocked) balance for the given asset.
    ///
    /// This is `total_balance - sum(locked amounts)`. Use this to check
    /// whether the user can afford to place a new order.
    ///
    /// # Arguments
    ///
    /// * `asset` — The asset to query
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

    /// Locks funds for a resting order.
    ///
    /// Moves `amount` units from available balance to locked. Called by the engine
    /// before an order enters the book.
    ///
    /// # Arguments
    ///
    /// * `order_id` — ID of the order these funds are locked for
    /// * `asset` — The asset to lock
    /// * `amount` — Number of units to lock
    ///
    /// # Errors
    ///
    /// Returns `Err("Insufficient balance")` if `amount > available_balance`.
    pub fn lock(
        &mut self,
        order_id: OrderId,
        asset: Asset,
        amount: Quantity,
    ) -> Result<(), String> {
        let available = self.get_available_balance(&asset);
        if available < amount {
            warn!(
                user = %self.id,
                order_id,
                asset = ?asset,
                requested = amount,
                available,
                "Insufficient balance"
            );
            return Err("Insufficient balance".into());
        }
        *self.balances.entry(asset).or_insert(0) -= amount;
        self.locked_orders
            .insert(order_id, LockEntry { asset, amount });
        debug!(
            user = %self.id,
            order_id,
            asset = ?asset,
            amount,
            "Funds locked"
        );
        Ok(())
    }

    /// Returns all locked funds to available balance for the given order.
    ///
    /// Called when an order is cancelled or when a FAK order fails to match.
    /// The order's locked funds are fully released.
    ///
    /// # Arguments
    ///
    /// * `order_id` — ID of the order to unlock
    ///
    /// # Errors
    ///
    /// Returns `Err("Order not locked")` if the order ID is not found.
    pub fn unlock_order(&mut self, order_id: &OrderId) -> Result<(), String> {
        let entry = self
            .locked_orders
            .remove(order_id)
            .ok_or("Order not locked")?;
        debug!(
            user = %self.id,
            order_id = %order_id,
            asset = ?entry.asset,
            amount = entry.amount,
            "Funds unlocked"
        );
        *self.balances.entry(entry.asset).or_insert(0) += entry.amount;
        Ok(())
    }

    /// Applies a fill to a locked order, settling the trade.
    ///
    /// Decrements the locked amount for `debit_asset` and credits the received
    /// `credit_asset`. If the locked amount reaches zero, the lock entry is removed.
    ///
    /// # Arguments
    ///
    /// * `order_id` — ID of the filled order
    /// * `debit_asset` — Asset being spent (must match the locked asset)
    /// * `debit_amount` — Units to deduct from the lock
    /// * `credit_asset` — Asset being received
    /// * `credit_amount` — Units to credit to the balance
    ///
    /// # Errors
    ///
    /// - `"Order not found in locked orders"` — order_id not in locked_orders
    /// - `"Locked asset mismatch"` — debit_asset doesn't match the locked asset
    /// - `"Locked amount insufficient for fill"` — debit_amount > locked amount
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
            warn!(
                user = %self.id,
                order_id,
                expected = ?entry.asset,
                got = ?debit_asset,
                "Locked asset mismatch"
            );
            return Err("Locked asset mismatch".into());
        }
        if entry.amount < debit_amount {
            warn!(
                user = %self.id,
                order_id,
                locked = entry.amount,
                requested = debit_amount,
                "Locked amount insufficient for fill"
            );
            return Err("Locked amount insufficient for fill".into());
        }
        entry.amount -= debit_amount;
        if entry.amount == 0 {
            self.locked_orders.remove(&order_id);
        }
        *self.balances.entry(credit_asset).or_insert(0) += credit_amount;
        debug!(
            user = %self.id,
            order_id,
            debit = ?debit_asset,
            debit_amount,
            credit = ?credit_asset,
            credit_amount,
            "Fill settled"
        );
        Ok(())
    }

    /// Returns the user's UUID.
    pub fn get_user_id(&self) -> UserId {
        self.id
    }

    /// Returns the total balance for the given asset (including locked amounts).
    ///
    /// To get only the spendable balance, use [`get_available_balance`](User::get_available_balance).
    pub fn get_balance(&self, asset: &Asset) -> Quantity {
        self.balances.get(asset).copied().unwrap_or(0)
    }

    /// Returns the total locked amount for the given asset across all orders.
    pub fn get_locked(&self, asset: &Asset) -> Quantity {
        self.locked_orders
            .values()
            .filter(|e| e.asset == *asset)
            .map(|e| e.amount)
            .sum()
    }
}
