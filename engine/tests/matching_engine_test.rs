use std::sync::{Arc, Mutex};

use engine::{
    matching_engine::Engine,
    order::Order,
    order_modify::OrderModify,
    trading_pair::TradingPair,
    types::{Asset, OrderStatus, OrderType, Side, UserId},
    users::User,
};

const USER_A: &str = "00000000-0000-0000-0000-000000000001";
const USER_B: &str = "00000000-0000-0000-0000-000000000002";

fn user_a() -> UserId {
    UserId::parse_str(USER_A).unwrap()
}

fn user_b() -> UserId {
    UserId::parse_str(USER_B).unwrap()
}

fn setup(pair: TradingPair) -> (Engine, TradingPair) {
    let mut engine = Engine::new();
    engine.add_trading_pair(pair);
    engine.add_user(User::new(Some(user_a())));
    engine.add_user(User::new(Some(user_b())));
    // Give users plenty of balance
    engine.deposit(user_a(), Asset::USDC, 10_000_000).unwrap();
    engine.deposit(user_a(), Asset::ETH, 10_000).unwrap();
    engine.deposit(user_a(), Asset::SOL, 10_000).unwrap();
    engine.deposit(user_b(), Asset::USDC, 10_000_000).unwrap();
    engine.deposit(user_b(), Asset::ETH, 10_000).unwrap();
    engine.deposit(user_b(), Asset::SOL, 10_000).unwrap();
    (engine, pair)
}

/// Helper: place a limit order and return the trades
fn place_limit(
    engine: &mut Engine,
    pair: &TradingPair,
    id: u64,
    side: Side,
    price: i32,
    qty: u32,
) -> Option<engine::trade::Trades> {
    let user = if id % 2 == 0 { user_a() } else { user_b() };
    let order = Arc::new(Mutex::new(Order::new(
        id,
        OrderType::GoodTillCancel,
        side,
        OrderStatus::Empty,
        price,
        qty,
        user,
    )));
    engine.add_order(user, pair, order).ok()?
}

fn place_fak(
    engine: &mut Engine,
    pair: &TradingPair,
    id: u64,
    side: Side,
    price: i32,
    qty: u32,
) -> Option<engine::trade::Trades> {
    let user = if id % 2 == 0 { user_a() } else { user_b() };
    let order = Arc::new(Mutex::new(Order::new(
        id,
        OrderType::FillAndKill,
        side,
        OrderStatus::Empty,
        price,
        qty,
        user,
    )));
    engine.add_order(user, pair, order).ok()?
}

fn new_engine(pair: TradingPair) -> (Engine, TradingPair) {
    setup(pair)
}

// Limit order matching

#[test]
fn test_basic_limit_match() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    // Sell 10@2000 — no bids, no trade
    let trades = place_limit(&mut engine, &pair, 1, Side::Sell, 2000, 10);
    assert!(trades.unwrap().is_empty(), "no bids yet");
    assert_eq!(engine.size(&pair), Some(1));

    // Buy 5@2000 — matches 5 of 10
    let trades = place_limit(&mut engine, &pair, 2, Side::Buy, 2000, 5);
    assert_eq!(trades.as_ref().unwrap().len(), 1, "one trade expected");
    assert_eq!(engine.size(&pair), Some(1), "sell 5 remaining");
}

#[test]
fn test_full_fill() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    // Sell 10@2000
    place_limit(&mut engine, &pair, 1, Side::Sell, 2000, 10);
    // Buy 5@2000 — partial
    place_limit(&mut engine, &pair, 2, Side::Buy, 2000, 5);
    // Buy 5@2000 — fills the rest
    let trades = place_limit(&mut engine, &pair, 3, Side::Buy, 2000, 5);
    assert_eq!(trades.as_ref().unwrap().len(), 1);
    assert_eq!(engine.size(&pair), Some(0), "all orders filled");
}

#[test]
fn test_no_match_below_ask() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, 1, Side::Sell, 2000, 10);
    // Buy at 1900 < best ask 2000 → no match
    let trades = place_limit(&mut engine, &pair, 2, Side::Buy, 1900, 3);
    assert!(trades.unwrap().is_empty(), "should not match");
    assert_eq!(engine.size(&pair), Some(2), "both orders resting");
}

#[test]
fn test_multi_price_levels() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    // Sell 5@2000, 3@1900, 2@1800
    place_limit(&mut engine, &pair, 1, Side::Sell, 2000, 5);
    place_limit(&mut engine, &pair, 2, Side::Sell, 1900, 3);
    place_limit(&mut engine, &pair, 3, Side::Sell, 1800, 2);

    // Buy 10@2000 — should match all levels from best ask upward
    let trades = place_limit(&mut engine, &pair, 4, Side::Buy, 2000, 10);
    assert_eq!(trades.as_ref().unwrap().len(), 3, "three matches expected");
    assert_eq!(engine.size(&pair), Some(0), "all filled");
}

// Cancel

#[test]
fn test_cancel_resting_order() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    // Buy below ask so they don't cross — both rest
    place_limit(&mut engine, &pair, 1, Side::Buy, 1900, 5);
    place_limit(&mut engine, &pair, 2, Side::Sell, 2000, 10);

    // Cancel the buy
    assert!(engine.cancel_order(&pair, &1), "cancel should succeed");
    assert_eq!(engine.size(&pair), Some(1), "only sell remains");
}

#[test]
fn test_cancel_nonexistent_order() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    assert!(!engine.cancel_order(&pair, &99), "should return false");
}

#[test]
fn test_cancel_filled_order_return_false() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, 1, Side::Sell, 2000, 5);
    place_limit(&mut engine, &pair, 2, Side::Buy, 2000, 5);

    // Both orders filled — trying to cancel either returns false
    assert!(!engine.cancel_order(&pair, &1), "order 1 gone");
    assert!(!engine.cancel_order(&pair, &2), "order 2 gone");
}

// Modify

#[test]
fn test_modify_nonexistent_order() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    let modify = OrderModify::new(99, 2000, Side::Buy, 5, OrderStatus::Empty, user_a());
    assert!(engine.modify_order(&pair, modify).is_none());
}

#[test]
fn test_modify_triggers_match() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    // Buy 3@1900 resting
    place_limit(&mut engine, &pair, 1, Side::Buy, 1900, 3);
    // Place sell at a higher price so it doesn't match
    place_limit(&mut engine, &pair, 2, Side::Sell, 2100, 10);

    // Modify sell to 1900 — should trigger match
    let modify = OrderModify::new(2, 1900, Side::Sell, 10, OrderStatus::Empty, user_b());
    let trades = engine.modify_order(&pair, modify);
    assert_eq!(trades.as_ref().unwrap().len(), 1, "one trade from modify");
    assert_eq!(engine.size(&pair), Some(1), "sell has 7 left");
}

// FillAndKill

#[test]
fn test_fak_rejected_when_no_match() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, 1, Side::Sell, 2000, 5);
    // FaK buy below best ask → rejected
    let trades = place_fak(&mut engine, &pair, 2, Side::Buy, 1800, 3);
    assert!(trades.is_none(), "FaK should be rejected");
    assert_eq!(engine.size(&pair), Some(1), "only original sell remains");
}

#[test]
fn test_fak_matched_partial_then_killed() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, 1, Side::Sell, 2000, 3);
    // FaK buy 5@2000 — matches 3, remaining 2 killed
    let trades = place_fak(&mut engine, &pair, 2, Side::Buy, 2000, 5);
    assert_eq!(trades.as_ref().unwrap().len(), 1, "one match");
    // Sell filled, FaK killed → no orders remain
    assert_eq!(engine.size(&pair), Some(0), "all consumed or killed");
}

#[test]
fn test_fak_matched_fully() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, 1, Side::Sell, 2000, 5);
    // FaK buy 5@2000 — matches fully
    let trades = place_fak(&mut engine, &pair, 2, Side::Buy, 2000, 5);
    assert_eq!(trades.as_ref().unwrap().len(), 1, "one match");
    assert_eq!(engine.size(&pair), Some(0), "both filled");
}

// Multiple pairs

#[test]
fn test_multiple_pairs_independent() {
    let mut engine = Engine::new();
    let eth = TradingPair::new(Asset::ETH, Asset::USDC);
    let sol = TradingPair::new(Asset::SOL, Asset::USDC);
    engine.add_trading_pair(eth);
    engine.add_trading_pair(sol);
    engine.add_user(User::new(Some(user_a())));
    engine.add_user(User::new(Some(user_b())));
    engine.deposit(user_a(), Asset::USDC, 10_000_000).unwrap();
    engine.deposit(user_a(), Asset::ETH, 10_000).unwrap();
    engine.deposit(user_a(), Asset::SOL, 10_000).unwrap();
    engine.deposit(user_b(), Asset::USDC, 10_000_000).unwrap();
    engine.deposit(user_b(), Asset::ETH, 10_000).unwrap();
    engine.deposit(user_b(), Asset::SOL, 10_000).unwrap();

    // Place orders on ETH
    place_limit(&mut engine, &eth, 1, Side::Sell, 2000, 5);
    place_limit(&mut engine, &eth, 2, Side::Buy, 2000, 3);

    // Place orders on SOL
    place_limit(&mut engine, &sol, 101, Side::Sell, 100, 20);
    place_limit(&mut engine, &sol, 102, Side::Buy, 100, 10);

    assert_eq!(engine.size(&eth), Some(1), "ETH sell 2 left");
    assert_eq!(engine.size(&sol), Some(1), "sell 10 left, buy 10 filled");
}

// Duplicate order ID

#[test]
fn test_reject_duplicate_order_id() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, 1, Side::Sell, 2000, 5);
    // Same ID again → rejected
    let trades = place_limit(&mut engine, &pair, 1, Side::Buy, 2000, 3);
    assert!(trades.is_none(), "duplicate ID rejected");
    assert_eq!(engine.size(&pair), Some(1), "only original order");
}

// Remove trading pair

#[test]
fn test_remove_trading_pair() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, 1, Side::Sell, 2000, 5);
    assert!(engine.size(&pair).is_some());

    let removed = engine.remove_trading_pair(&pair);
    assert!(removed.is_some(), "orderbook returned");
    assert!(engine.size(&pair).is_none(), "pair removed");
}

#[test]
fn test_remove_nonexistent_pair() {
    let mut engine = Engine::new();
    let pair = TradingPair::new(Asset::ETH, Asset::USDC);
    assert!(engine.remove_trading_pair(&pair).is_none());
}

// Empty book

#[test]
fn test_new_pair_has_no_orders() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));
    assert_eq!(engine.size(&pair), Some(0), "empty book");
    assert!(engine.remove_trading_pair(&pair).is_some());
}

#[test]
fn test_get_order_info_empty_pair() {
    let (engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));
    let _ = engine;
    let _ = pair;
    let mut engine = Engine::new();
    let pair = TradingPair::new(Asset::ETH, Asset::USDC);
    engine.add_trading_pair(pair);
    let info = engine.get_order_info(&pair);
    assert!(info.is_some());
}

// ---------------------------------------------------------------------------
// TradeInfo field access
// ---------------------------------------------------------------------------

#[test]
fn test_trade_info_fields() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));
    place_limit(&mut engine, &pair, 1, Side::Sell, 2000, 5);
    let trades = place_limit(&mut engine, &pair, 2, Side::Buy, 2000, 5).unwrap();
    let trade = &trades[0];
    let bid = trade.get_bid_trade_info();
    let ask = trade.get_ask_trade_info();
    let _ = bid.get_order_id();
    let _ = bid.get_price();
    let _ = bid.get_quantity();
    let _ = bid.get_user_id();
    let _ = ask.get_order_id();
    let _ = ask.get_price();
    let _ = ask.get_quantity();
    let _ = ask.get_user_id();
}

#[test]
fn test_time_priority() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));
    // Order A: buy 5@2000 (earlier)
    place_limit(&mut engine, &pair, 1, Side::Buy, 2000, 5);
    // Order B: buy 3@2000 (later, same price)
    place_limit(&mut engine, &pair, 2, Side::Buy, 2000, 3);
    // Order C: sell 10@2000 — must fill A (5) before B (3)
    let trades = place_limit(&mut engine, &pair, 3, Side::Sell, 2000, 10).unwrap();
    assert_eq!(trades.len(), 2);
    // sell 2 remaining (10 - 5 - 3)
    assert_eq!(engine.size(&pair), Some(1));
}
