use std::sync::{Arc, Mutex, Once};

use engine::{
    engine::{CoreEngine, ExchangeEngine},
    order::Order,
    order_modify::OrderModify,
    trading_pair::TradingPair,
    types::{Asset, OrderStatus, OrderType, Side, UserId},
    users::User,
};

static INIT: Once = Once::new();

fn init_test_logging() {
    INIT.call_once(|| {
        engine::logging::init_test();
    });
}

const USER_A: &str = "00000000-0000-0000-0000-000000000001";
const USER_B: &str = "00000000-0000-0000-0000-000000000002";

fn user_a() -> UserId {
    UserId::parse_str(USER_A).unwrap()
}

fn user_b() -> UserId {
    UserId::parse_str(USER_B).unwrap()
}

fn setup(pair: TradingPair) -> (CoreEngine, TradingPair) {
    init_test_logging();
    let mut engine = CoreEngine::new();
    engine.add_trading_pair(pair);
    engine.add_user(User::new(Some(user_a())));
    engine.add_user(User::new(Some(user_b())));
    engine.deposit(user_a(), Asset::USDC, 10_000_000).unwrap();
    engine.deposit(user_a(), Asset::ETH, 10_000).unwrap();
    engine.deposit(user_a(), Asset::SOL, 10_000).unwrap();
    engine.deposit(user_b(), Asset::USDC, 10_000_000).unwrap();
    engine.deposit(user_b(), Asset::ETH, 10_000).unwrap();
    engine.deposit(user_b(), Asset::SOL, 10_000).unwrap();
    (engine, pair)
}

/// Helper: place a limit order and return the result
fn place_limit(
    engine: &mut CoreEngine,
    pair: &TradingPair,
    user: UserId,
    side: Side,
    price: i32,
    qty: u32,
) -> Option<engine::engine::AddOrderResult> {
    let order_id = engine.next_id();
    let order = Arc::new(Mutex::new(Order::new(
        order_id,
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
    engine: &mut CoreEngine,
    pair: &TradingPair,
    user: UserId,
    side: Side,
    price: i32,
    qty: u32,
) -> Option<engine::engine::AddOrderResult> {
    let order_id = engine.next_id();
    let order = Arc::new(Mutex::new(Order::new(
        order_id,
        OrderType::FillAndKill,
        side,
        OrderStatus::Empty,
        price,
        qty,
        user,
    )));
    engine.add_order(user, pair, order).ok()?
}

fn new_engine(pair: TradingPair) -> (CoreEngine, TradingPair) {
    setup(pair)
}

// Limit order matching

#[test]
fn test_basic_limit_match() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    let sell = place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 10);
    assert!(sell.as_ref().unwrap().trades.as_ref().unwrap().is_empty(), "no bids yet");
    assert_eq!(engine.size(&pair), Some(1));

    let buy = place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 5);
    assert_eq!(buy.as_ref().unwrap().trades.as_ref().unwrap().len(), 1, "one trade expected");
    assert_eq!(engine.size(&pair), Some(1), "sell 5 remaining");
}

#[test]
fn test_full_fill() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 10);
    place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 5);
    let result = place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 5);
    assert_eq!(result.as_ref().unwrap().trades.as_ref().unwrap().len(), 1);
    assert_eq!(engine.size(&pair), Some(0), "all orders filled");
}

#[test]
fn test_no_match_below_ask() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 10);
    let buy = place_limit(&mut engine, &pair, user_a(), Side::Buy, 1900, 3);
    assert!(buy.as_ref().unwrap().trades.as_ref().unwrap().is_empty(), "should not match");
    assert_eq!(engine.size(&pair), Some(2), "both orders resting");
}

#[test]
fn test_multi_price_levels() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 5);
    place_limit(&mut engine, &pair, user_b(), Side::Sell, 1900, 3);
    place_limit(&mut engine, &pair, user_b(), Side::Sell, 1800, 2);

    let buy = place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 10);
    assert_eq!(buy.as_ref().unwrap().trades.as_ref().unwrap().len(), 3, "three matches expected");
    assert_eq!(engine.size(&pair), Some(0), "all filled");
}

// Cancel

#[test]
fn test_cancel_resting_order() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    let buy = place_limit(&mut engine, &pair, user_a(), Side::Buy, 1900, 5).unwrap();
    place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 10);

    assert!(engine.cancel_order(&pair, &buy.order_id), "cancel should succeed");
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

    let sell = place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 5).unwrap();
    let buy = place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 5).unwrap();

    assert!(!engine.cancel_order(&pair, &sell.order_id), "order 1 gone");
    assert!(!engine.cancel_order(&pair, &buy.order_id), "order 2 gone");
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

    place_limit(&mut engine, &pair, user_a(), Side::Buy, 1900, 3);
    let sell = place_limit(&mut engine, &pair, user_b(), Side::Sell, 2100, 10).unwrap();

    let modify = OrderModify::new(sell.order_id, 1900, Side::Sell, 10, OrderStatus::Empty, user_b());
    let result = engine.modify_order(&pair, modify);
    assert_eq!(result.as_ref().unwrap().trades.as_ref().unwrap().len(), 1, "one trade from modify");
    assert_eq!(engine.size(&pair), Some(1), "sell has 7 left");
}

// FillAndKill

#[test]
fn test_fak_rejected_when_no_match() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 5);
    let result = place_fak(&mut engine, &pair, user_a(), Side::Buy, 1800, 3);
    assert!(result.is_none(), "FaK should be rejected");
    assert_eq!(engine.size(&pair), Some(1), "only original sell remains");
}

#[test]
fn test_fak_matched_partial_then_killed() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 3);
    let result = place_fak(&mut engine, &pair, user_a(), Side::Buy, 2000, 5);
    assert_eq!(result.as_ref().unwrap().trades.as_ref().unwrap().len(), 1, "one match");
    assert_eq!(engine.size(&pair), Some(0), "all consumed or killed");
}

#[test]
fn test_fak_matched_fully() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 5);
    let result = place_fak(&mut engine, &pair, user_a(), Side::Buy, 2000, 5);
    assert_eq!(result.as_ref().unwrap().trades.as_ref().unwrap().len(), 1, "one match");
    assert_eq!(engine.size(&pair), Some(0), "both filled");
}

// Multiple pairs

#[test]
fn test_multiple_pairs_independent() {
    let mut engine = CoreEngine::new();
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

    place_limit(&mut engine, &eth, user_b(), Side::Sell, 2000, 5);
    place_limit(&mut engine, &eth, user_a(), Side::Buy, 2000, 3);

    place_limit(&mut engine, &sol, user_b(), Side::Sell, 100, 20);
    place_limit(&mut engine, &sol, user_a(), Side::Buy, 100, 10);

    assert_eq!(engine.size(&eth), Some(1), "ETH sell 2 left");
    assert_eq!(engine.size(&sol), Some(1), "sell 10 left, buy 10 filled");
}

// Order IDs are engine-generated, so duplicates are impossible through the engine API.
// This test verifies that each placed order gets a unique snowflake ID.
#[test]
fn test_each_order_gets_unique_id() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    let r1 = place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 5).unwrap();
    let r2 = place_limit(&mut engine, &pair, user_b(), Side::Sell, 2100, 5).unwrap();
    let r3 = place_limit(&mut engine, &pair, user_a(), Side::Buy, 1900, 3).unwrap();

    assert_ne!(r1.order_id, r2.order_id);
    assert_ne!(r2.order_id, r3.order_id);
    assert_ne!(r1.order_id, r3.order_id);
    assert_eq!(engine.size(&pair), Some(3));
}

// Remove trading pair

#[test]
fn test_remove_trading_pair() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 5);
    assert!(engine.size(&pair).is_some());

    let removed = engine.remove_trading_pair(&pair);
    assert!(removed.is_some(), "orderbook returned");
    assert!(engine.size(&pair).is_none(), "pair removed");
}

#[test]
fn test_remove_nonexistent_pair() {
    let mut engine = CoreEngine::new();
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
    let mut engine = CoreEngine::new();
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
    place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 5);
    let result = place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 5).unwrap();
    let trades = result.trades.unwrap();
    let trade = &trades[0];

    assert!(trade.get_trade_id() != 0, "trade_id should be non-zero");
    assert!(trade.get_timestamp() != 0, "timestamp should be non-zero");

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
    place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 5);
    place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 3);
    let result = place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 10).unwrap();
    assert_eq!(result.trades.as_ref().unwrap().len(), 2);
    assert_eq!(engine.size(&pair), Some(1));
}

// ---------------------------------------------------------------------------
// Snowflake ID tests
// ---------------------------------------------------------------------------

#[test]
fn test_order_ids_are_unique() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    let r1 = place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 5);
    let r2 = place_limit(&mut engine, &pair, user_b(), Side::Sell, 2100, 5);
    let r3 = place_limit(&mut engine, &pair, user_b(), Side::Sell, 2200, 5);

    let id1 = r1.unwrap().order_id;
    let id2 = r2.unwrap().order_id;
    let id3 = r3.unwrap().order_id;

    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);
}

#[test]
fn test_trade_ids_are_unique() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 10);
    let r1 = place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 5);
    let r2 = place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 5);

    let tid1 = r1.unwrap().trades.unwrap()[0].get_trade_id();
    let tid2 = r2.unwrap().trades.unwrap()[0].get_trade_id();

    assert_ne!(tid1, tid2);
}

// ---------------------------------------------------------------------------
// Self-trade prevention
// ---------------------------------------------------------------------------

#[test]
fn test_self_trade_prevention_buy_skips_own_sell() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_a(), Side::Sell, 2000, 10);
    let result = place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 5);

    assert!(result.as_ref().unwrap().trades.as_ref().unwrap().is_empty(), "no trades from self-trade");
    assert_eq!(engine.size(&pair), Some(2), "both orders still in book");
}

#[test]
fn test_self_trade_prevention_sell_skips_own_buy() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 10);
    let result = place_limit(&mut engine, &pair, user_a(), Side::Sell, 2000, 5);

    assert!(result.as_ref().unwrap().trades.as_ref().unwrap().is_empty(), "no trades from self-trade");
    assert_eq!(engine.size(&pair), Some(2), "both orders still in book");
}

#[test]
fn test_cross_user_trade_unaffected() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_a(), Side::Sell, 2000, 10);
    let result = place_limit(&mut engine, &pair, user_b(), Side::Buy, 2000, 5);

    assert_eq!(result.as_ref().unwrap().trades.as_ref().unwrap().len(), 1, "normal cross-user trade");
    assert_eq!(engine.size(&pair), Some(1), "sell has 5 remaining");
}

#[test]
fn test_self_trade_skip_then_match_different_user() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_a(), Side::Sell, 2000, 10);
    place_limit(&mut engine, &pair, user_b(), Side::Sell, 2000, 10);

    let result = place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 5);

    assert_eq!(result.as_ref().unwrap().trades.as_ref().unwrap().len(), 1, "matched only user B");
    assert_eq!(engine.size(&pair), Some(2), "A sell (10) + B sell (5) remaining");
}

#[test]
fn test_self_trade_skips_all_own_orders() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_a(), Side::Sell, 2000, 5);
    place_limit(&mut engine, &pair, user_a(), Side::Sell, 2000, 5);

    let result = place_limit(&mut engine, &pair, user_a(), Side::Buy, 2000, 10);

    assert!(result.as_ref().unwrap().trades.as_ref().unwrap().is_empty(), "all skipped");
    assert_eq!(engine.size(&pair), Some(3), "2 sells + 1 buy all resting");
}

#[test]
fn test_self_trade_fak_no_match() {
    let (mut engine, pair) = new_engine(TradingPair::new(Asset::ETH, Asset::USDC));

    place_limit(&mut engine, &pair, user_a(), Side::Sell, 2000, 10);
    let result = place_fak(&mut engine, &pair, user_a(), Side::Buy, 2000, 5);

    assert!(result.is_some(), "FAK placed but not filled");
    assert!(result.as_ref().unwrap().trades.as_ref().unwrap().is_empty(), "no trades from self-trade");
    assert_eq!(engine.size(&pair), Some(1), "only A's sell remains — FAK buy was cancelled");
}
