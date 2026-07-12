//! # Matching Engine Demo
//!
//! End-to-end demonstration of the matching engine covering:
//!
//! 1. Basic partial fill (seller rests, buyer matches partially)
//! 2. No match (buyer below best ask)
//! 3. Full fill of remaining quantity
//! 4. Order cancellation with balance unlock
//! 5. Modify (cancel-replace) to trigger a match
//! 6. FillAndKill — rejected when no match available
//! 7. FillAndKill — matches fully
//! 8. Multiple trading pairs (ETH-USDC + SOL-USDC)
//! 9. Trading pair removal
//!
//! Run with `cargo run` from the `engine/` directory.

use std::sync::{Arc, Mutex};

use engine::{
    engine::{AddOrderResult, ExchangeEngine, engine_from_env},
    order::Order,
    order_modify::OrderModify,
    trading_pair::TradingPair,
    types::{Asset, OrderStatus, OrderType, Price, Quantity, Side, UserId},
    users::User,
};

/// Helper: creates a GoodTillCancel limit order and places it. Panics on failure.
fn place(
    engine: &mut dyn ExchangeEngine,
    user_id: UserId,
    pair: &TradingPair,
    side: Side,
    price: Price,
    qty: Quantity,
) -> AddOrderResult {
    let order = Arc::new(Mutex::new(Order::new(
        0,
        OrderType::GoodTillCancel,
        side,
        OrderStatus::Empty,
        price,
        qty,
        user_id,
    )));
    match engine.add_order(user_id, pair, order) {
        Ok(result) => {
            let result = result.unwrap();
            tracing::info!(
                side = ?side,
                price,
                qty,
                trades = result.trades.as_ref().map_or(0, |t| t.len()),
                order_id = result.order_id,
                "Order placed"
            );
            result
        }
        Err(e) => {
            tracing::error!(error = %e, side = ?side, price, qty, "Order placement failed");
            panic!("Place order failed: {}", e);
        }
    }
}

/// Helper: creates a FillAndKill limit order and places it. Returns `None` if rejected (no match).
fn place_fak(
    engine: &mut dyn ExchangeEngine,
    user_id: UserId,
    pair: &TradingPair,
    side: Side,
    price: Price,
    qty: Quantity,
) -> Option<AddOrderResult> {
    let order = Arc::new(Mutex::new(Order::new(
        0,
        OrderType::FillAndKill,
        side,
        OrderStatus::Empty,
        price,
        qty,
        user_id,
    )));
    match engine.add_order(user_id, pair, order) {
        Ok(result) => {
            if let Some(ref r) = result {
                tracing::info!(
                    side = ?side,
                    price,
                    qty,
                    trades = r.trades.as_ref().map_or(0, |t| t.len()),
                    order_id = r.order_id,
                    "FAK order placed"
                );
            } else {
                tracing::info!(
                    side = ?side,
                    price,
                    qty,
                    "FAK order rejected (no match)"
                );
            }
            result
        }
        Err(e) => {
            tracing::error!(error = %e, side = ?side, price, qty, "FAK order failed");
            panic!("Place FAK failed: {}", e);
        }
    }
}

fn main() {
    let tracing_enabled = std::env::var("TRACING_ENABLED")
        .unwrap_or_else(|_| "true".into())
        != "false";

    if tracing_enabled {
        engine::logging::init();
    }

    let _guard = tracing::info_span!(
        "engine_demo",
        service = "matching-engine",
        version = env!("CARGO_PKG_VERSION")
    )
    .entered();

    tracing::info!("Matching engine demo starting");

    let mut engine: Box<dyn ExchangeEngine> = engine_from_env();
    let eth_usdc = TradingPair::new(Asset::ETH, Asset::USDC);
    let sol_usdc = TradingPair::new(Asset::SOL, Asset::USDC);
    engine.add_trading_pair(eth_usdc);
    engine.add_trading_pair(sol_usdc);

    let alice = UserId::new_v4();
    let bob = UserId::new_v4();
    engine.add_user(User::new(Some(alice)));
    engine.add_user(User::new(Some(bob)));
    engine.deposit(alice, Asset::USDC, 100_000).unwrap();
    engine.deposit(bob, Asset::ETH, 50).unwrap();
    engine.deposit(bob, Asset::SOL, 200).unwrap();

    tracing::info!(alice = %alice, bob = %bob, "Users created and funded");

    // ============================================================================
    tracing::info!("=== Initial balances ===");
    // ============================================================================
    if let Some(balances) = engine.get_user_balance(&alice) {
        tracing::info!(user = "Alice", balances = ?balances, "Balances");
    }
    if let Some(balances) = engine.get_user_balance(&bob) {
        tracing::info!(user = "Bob", balances = ?balances, "Balances");
    }

    // ============================================================================
    tracing::info!("=== 1. Basic match — Bob sells 10 ETH, Alice buys 5 (partial fill) ===");
    // ============================================================================
    place(&mut *engine, bob, &eth_usdc, Side::Sell, 2000, 10);
    place(&mut *engine, alice, &eth_usdc, Side::Buy, 2000, 5);
    if let Some(info) = engine.get_order_info(&eth_usdc) {
        tracing::debug!(pair = "ETH-USDC", bids = ?info.get_bids(), asks = ?info.get_asks(), "Orderbook");
    }
    tracing::info!(pair = "ETH-USDC", remaining = ?engine.size(&eth_usdc), "Orders remaining");
    if let Some(balances) = engine.get_user_balance(&alice) {
        tracing::info!(user = "Alice", balances = ?balances, "Balances");
    }
    if let Some(balances) = engine.get_user_balance(&bob) {
        tracing::info!(user = "Bob", balances = ?balances, "Balances");
    }

    // ============================================================================
    tracing::info!("=== 2. No match — Alice buys below best ask ===");
    // ============================================================================
    place(&mut *engine, alice, &eth_usdc, Side::Buy, 1900, 3);
    if let Some(info) = engine.get_order_info(&eth_usdc) {
        tracing::debug!(pair = "ETH-USDC", bids = ?info.get_bids(), asks = ?info.get_asks(), "Orderbook");
    }
    if let Some(balances) = engine.get_user_balance(&alice) {
        tracing::info!(user = "Alice", balances = ?balances, "Balances");
    }

    // ============================================================================
    tracing::info!("=== 3. Full fill remaining — Alice buys remaining 5 ETH ===");
    // ============================================================================
    place(&mut *engine, alice, &eth_usdc, Side::Buy, 2000, 5);
    if let Some(info) = engine.get_order_info(&eth_usdc) {
        tracing::debug!(pair = "ETH-USDC", bids = ?info.get_bids(), asks = ?info.get_asks(), "Orderbook");
    }
    tracing::info!(pair = "ETH-USDC", remaining = ?engine.size(&eth_usdc), "Orders remaining");
    if let Some(balances) = engine.get_user_balance(&alice) {
        tracing::info!(user = "Alice", balances = ?balances, "Balances");
    }
    if let Some(balances) = engine.get_user_balance(&bob) {
        tracing::info!(user = "Bob", balances = ?balances, "Balances");
    }

    // ============================================================================
    tracing::info!("=== 4. Cancel order ===");
    // ============================================================================
    let sell2 = place(&mut *engine, bob, &eth_usdc, Side::Sell, 2100, 8);
    if let Some(info) = engine.get_order_info(&eth_usdc) {
        tracing::debug!(pair = "ETH-USDC", bids = ?info.get_bids(), asks = ?info.get_asks(), "Orderbook before cancel");
    }
    tracing::info!(pair = "ETH-USDC", count = ?engine.size(&eth_usdc), "Orders before cancel");
    let cancelled = engine.cancel_order(&eth_usdc, &sell2.order_id);
    tracing::info!(order_id = sell2.order_id, cancelled, "Cancel result");
    if let Some(info) = engine.get_order_info(&eth_usdc) {
        tracing::debug!(pair = "ETH-USDC", bids = ?info.get_bids(), asks = ?info.get_asks(), "Orderbook after cancel");
    }
    tracing::info!(pair = "ETH-USDC", count = ?engine.size(&eth_usdc), "Orders after cancel");
    if let Some(balances) = engine.get_user_balance(&bob) {
        tracing::info!(user = "Bob", balances = ?balances, "Balances after cancel");
    }

    // ============================================================================
    tracing::info!("=== 5. Modify order — change price to trigger match ===");
    // ============================================================================
    let sell3 = place(&mut *engine, bob, &eth_usdc, Side::Sell, 2100, 10);
    if let Some(info) = engine.get_order_info(&eth_usdc) {
        tracing::debug!(pair = "ETH-USDC", bids = ?info.get_bids(), asks = ?info.get_asks(), "Orderbook before modify");
    }
    tracing::info!(pair = "ETH-USDC", count = ?engine.size(&eth_usdc), "Orders before modify");
    let modify = OrderModify::new(sell3.order_id, 1900, Side::Sell, 10, OrderStatus::Empty, bob);
    let mod_result = engine.modify_order(&eth_usdc, modify);
    tracing::info!(
        new_order_id = mod_result.as_ref().map(|r| r.order_id),
        trades = mod_result.as_ref().and_then(|r| r.trades.as_ref()).map_or(0, |t| t.len()),
        "Modify result"
    );
    if let Some(info) = engine.get_order_info(&eth_usdc) {
        tracing::debug!(pair = "ETH-USDC", bids = ?info.get_bids(), asks = ?info.get_asks(), "Orderbook after modify");
    }
    tracing::info!(pair = "ETH-USDC", count = ?engine.size(&eth_usdc), "Orders after modify");
    if let Some(balances) = engine.get_user_balance(&alice) {
        tracing::info!(user = "Alice", balances = ?balances, "Balances after modify");
    }
    if let Some(balances) = engine.get_user_balance(&bob) {
        tracing::info!(user = "Bob", balances = ?balances, "Balances after modify");
    }

    // ============================================================================
    tracing::info!("=== 6. FillAndKill — can't match (buy below best ask) ===");
    // ============================================================================
    place_fak(&mut *engine, alice, &eth_usdc, Side::Buy, 1800, 2);
    if let Some(info) = engine.get_order_info(&eth_usdc) {
        tracing::debug!(pair = "ETH-USDC", bids = ?info.get_bids(), asks = ?info.get_asks(), "Orderbook");
    }
    tracing::info!(pair = "ETH-USDC", count = ?engine.size(&eth_usdc), "Orders");
    if let Some(balances) = engine.get_user_balance(&alice) {
        tracing::info!(user = "Alice", balances = ?balances, "Balances (FAK rejected)");
    }

    // ============================================================================
    tracing::info!("=== 7. FillAndKill — matches fully ===");
    // ============================================================================
    place_fak(&mut *engine, alice, &eth_usdc, Side::Buy, 1900, 2);
    if let Some(info) = engine.get_order_info(&eth_usdc) {
        tracing::debug!(pair = "ETH-USDC", bids = ?info.get_bids(), asks = ?info.get_asks(), "Orderbook");
    }
    tracing::info!(pair = "ETH-USDC", count = ?engine.size(&eth_usdc), "Orders");
    if let Some(balances) = engine.get_user_balance(&alice) {
        tracing::info!(user = "Alice", balances = ?balances, "Balances (FAK filled)");
    }
    if let Some(balances) = engine.get_user_balance(&bob) {
        tracing::info!(user = "Bob", balances = ?balances, "Balances (FAK filled)");
    }

    // ============================================================================
    tracing::info!("=== 8. Multiple trading pairs (SOL/USDC) ===");
    // ============================================================================
    place(&mut *engine, bob, &sol_usdc, Side::Sell, 100, 20);
    place(&mut *engine, alice, &sol_usdc, Side::Buy, 100, 10);
    place(&mut *engine, alice, &sol_usdc, Side::Buy, 105, 5);
    if let Some(info) = engine.get_order_info(&sol_usdc) {
        tracing::debug!(pair = "SOL-USDC", bids = ?info.get_bids(), asks = ?info.get_asks(), "Orderbook");
    }
    if let Some(info) = engine.get_order_info(&eth_usdc) {
        tracing::debug!(pair = "ETH-USDC", bids = ?info.get_bids(), asks = ?info.get_asks(), "Orderbook");
    }
    tracing::info!(pair = "SOL-USDC", count = ?engine.size(&sol_usdc), "SOL pair orders");
    tracing::info!(pair = "ETH-USDC", count = ?engine.size(&eth_usdc), "ETH pair orders (unchanged)");
    if let Some(balances) = engine.get_user_balance(&alice) {
        tracing::info!(user = "Alice", balances = ?balances, "Balances (multi-pair)");
    }
    if let Some(balances) = engine.get_user_balance(&bob) {
        tracing::info!(user = "Bob", balances = ?balances, "Balances (multi-pair)");
    }

    // ============================================================================
    tracing::info!("=== 9. Remove trading pair ===");
    // ============================================================================
    let removed = engine.remove_trading_pair(&sol_usdc);
    tracing::info!(pair = "SOL-USDC", removed = removed.is_some(), "Trading pair removed");
    tracing::info!(pair = "SOL-USDC", exists = engine.size(&sol_usdc).is_some(), "SOL pair exists");
    tracing::info!(pair = "ETH-USDC", exists = engine.size(&eth_usdc).is_some(), "ETH pair exists");

    // ============================================================================
    tracing::info!("=== DONE — all tests complete ===");
    // ============================================================================
}
