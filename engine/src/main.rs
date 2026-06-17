use std::sync::{Arc, Mutex};

use engine::{
    matching_engine::Engine,
    order::Order,
    order_modify::OrderModify,
    trading_pair::TradingPair,
    types::{Asset, OrderId, OrderStatus, OrderType, Price, Quantity, Side},
};

fn print_separator(label: &str) {
    println!("\n{} {}", "=".repeat(60), label);
}

fn print_book(engine: &Engine, pair: &TradingPair, label: &str) {
    let info = engine.get_order_info(pair);
    println!("  {} → Bids: {:?}, Asks: {:?}", label,
        info.as_ref().map(|i| i.get_bids()),
        info.as_ref().map(|i| i.get_asks()));
}

fn place(engine: &mut Engine, pair: &TradingPair, id: OrderId, side: Side, price: Price, qty: Quantity) {
    let order = Arc::new(Mutex::new(Order::new(
        id, OrderType::GoodTillCancel, side, OrderStatus::Empty, price, qty,
    )));
    let trades = engine.add_order(pair, order);
    println!("  Place {} {}@{} → trades: {:?}", if matches!(side, Side::Buy) { "BUY" } else { "SELL" }, qty, price, trades.as_ref().map(|t| t.len()));
}

fn place_fak(engine: &mut Engine, pair: &TradingPair, id: OrderId, side: Side, price: Price, qty: Quantity) {
    let order = Arc::new(Mutex::new(Order::new(
        id, OrderType::FillAndKill, side, OrderStatus::Empty, price, qty,
    )));
    let trades = engine.add_order(pair, order);
    println!("  Place FAK {} {}@{} → trades: {:?}", if matches!(side, Side::Buy) { "BUY" } else { "SELL" }, qty, price, trades.as_ref().map(|t| t.len()));
}

fn main() {
    let mut engine = Engine::new();
    let eth_usdc = TradingPair::new(Asset::ETH, Asset::USDC);
    let sol_usdc = TradingPair::new(Asset::SOL, Asset::USDC);
    engine.add_trading_pair(eth_usdc);
    engine.add_trading_pair(sol_usdc);

    // ============================================================================
    print_separator("1. Basic match — sell 10, buy 5 at same price (partial fill)");
    // ============================================================================
    place(&mut engine, &eth_usdc, 1, Side::Sell, 2000, 10);
    place(&mut engine, &eth_usdc, 2, Side::Buy, 2000, 5);
    print_book(&engine, &eth_usdc, "After test 1");
    println!("  Orders remaining: {:?}", engine.size(&eth_usdc));

    // ============================================================================
    print_separator("2. No match — buy below best ask");
    // ============================================================================
    place(&mut engine, &eth_usdc, 3, Side::Buy, 1900, 3);
    print_book(&engine, &eth_usdc, "After test 2");
    println!("  Orders remaining: {:?}", engine.size(&eth_usdc));

    // ============================================================================
    print_separator("3. Full fill remaining — buy to match the remaining 5 sell");
    // ============================================================================
    place(&mut engine, &eth_usdc, 4, Side::Buy, 2000, 5);
    print_book(&engine, &eth_usdc, "After test 3");
    println!("  Orders remaining: {:?}", engine.size(&eth_usdc));

    // ============================================================================
    print_separator("4. Cancel order");
    // ============================================================================
    place(&mut engine, &eth_usdc, 5, Side::Sell, 2100, 8);
    print_book(&engine, &eth_usdc, "Before cancel");
    println!("  Orders before cancel: {:?}", engine.size(&eth_usdc));
    let cancelled = engine.cancel_order(&eth_usdc, &5);
    println!("  Cancel returned: {}", cancelled);
    print_book(&engine, &eth_usdc, "After cancel");
    println!("  Orders after cancel: {:?}", engine.size(&eth_usdc));

    // ============================================================================
    print_separator("5. Modify order — change price to trigger match");
    // ============================================================================
    place(&mut engine, &eth_usdc, 6, Side::Sell, 2100, 10);
    print_book(&engine, &eth_usdc, "Before modify");
    println!("  Orders before modify: {:?}", engine.size(&eth_usdc));
    let modify = OrderModify::new(6, 1900, Side::Sell, 10, OrderStatus::Empty);
    let mod_trades = engine.modify_order(&eth_usdc, modify);
    println!("  Modify trades: {:?}", mod_trades.as_ref().map(|t| t.len()));
    print_book(&engine, &eth_usdc, "After modify");
    println!("  Orders after modify: {:?}", engine.size(&eth_usdc));

    // ============================================================================
    print_separator("6. FillAndKill — can't match (buy below best ask)");
    // ============================================================================
    place_fak(&mut engine, &eth_usdc, 7, Side::Buy, 1800, 2);
    print_book(&engine, &eth_usdc, "After test 6");
    println!("  Orders: {:?}", engine.size(&eth_usdc));

    // ============================================================================
    print_separator("7. FillAndKill — matches fully");
    // ============================================================================
    place_fak(&mut engine, &eth_usdc, 8, Side::Buy, 1900, 2);
    print_book(&engine, &eth_usdc, "After test 7");
    println!("  Orders: {:?}", engine.size(&eth_usdc));

    // ============================================================================
    print_separator("8. Multiple trading pairs (SOL/USDC)");
    // ============================================================================
    place(&mut engine, &sol_usdc, 101, Side::Sell, 100, 20);
    place(&mut engine, &sol_usdc, 102, Side::Buy, 100, 10);
    place(&mut engine, &sol_usdc, 103, Side::Buy, 105, 5);
    print_book(&engine, &sol_usdc, "SOL/USDC book");
    print_book(&engine, &eth_usdc, "ETH/USDC book");
    println!("  SOL pair orders: {:?}", engine.size(&sol_usdc));
    println!("  ETH pair orders (should be unchanged): {:?}", engine.size(&eth_usdc));

    // ============================================================================
    print_separator("9. Remove trading pair");
    // ============================================================================
    let removed = engine.remove_trading_pair(&sol_usdc);
    println!("  SOL/USDC removed: {}", removed.is_some());
    println!("  SOL pair exists: {:?}", engine.size(&sol_usdc));
    println!("  ETH pair still exists: {:?}", engine.size(&eth_usdc));

    // ============================================================================
    print_separator("DONE — all tests complete");
    // ============================================================================
}
