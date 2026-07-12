use std::sync::{Arc, Mutex};

use engine::{
    matching_engine::{
        AddOrderResult, 
        Engine
    }, 
    order::Order, 
    order_modify::OrderModify, 
    trading_pair::TradingPair, 
    types::{
        Asset, 
        OrderStatus, 
        OrderType, 
        Price, 
        Quantity, 
        Side, 
        UserId
    }, 
    users::User,
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

fn print_balances(engine: &Engine, user_id: &UserId, label: &str) {
    if let Some(balances) = engine.get_user_balance(user_id) {
        println!("  {} balances: {:?}", label, balances);
    }
}

fn place(engine: &mut Engine, user_id: UserId, pair: &TradingPair, side: Side, price: Price, qty: Quantity) -> AddOrderResult {
    let order = Arc::new(Mutex::new(Order::new(
        0, OrderType::GoodTillCancel, side, OrderStatus::Empty, price, qty, user_id,
    )));
    match engine.add_order(user_id, pair, order) {
        Ok(result) => {
            let result = result.unwrap();
            println!("  Place {} {}@{} → trades: {}, order_id: {}",
                if side == Side::Buy { "BUY" } else { "SELL" }, qty, price,
                result.trades.as_ref().map(|t| t.len()).unwrap_or(0),
                result.order_id);
            result
        }
        Err(e) => panic!("Place order failed: {}", e),
    }
}

fn place_fak(engine: &mut Engine, user_id: UserId, pair: &TradingPair, side: Side, price: Price, qty: Quantity) -> Option<engine::matching_engine::AddOrderResult> {
    let order = Arc::new(Mutex::new(Order::new(
        0, OrderType::FillAndKill, side, OrderStatus::Empty, price, qty, user_id,
    )));
    match engine.add_order(user_id, pair, order) {
        Ok(result) => {
            if let Some(ref r) = result {
                println!("  Place FAK {} {}@{} → trades: {}, order_id: {}",
                    if side == Side::Buy { "BUY" } else { "SELL" }, qty, price,
                    r.trades.as_ref().map(|t| t.len()).unwrap_or(0),
                    r.order_id);
            } else {
                println!("  Place FAK {} {}@{} → rejected",
                    if side == Side::Buy { "BUY" } else { "SELL" }, qty, price);
            }
            result
        }
        Err(e) => panic!("Place FAK failed: {}", e),
    }
}

fn main() {
    let mut engine = Engine::new();
    let eth_usdc = TradingPair::new(Asset::ETH, Asset::USDC);
    let sol_usdc = TradingPair::new(Asset::SOL, Asset::USDC);
    engine.add_trading_pair(eth_usdc);
    engine.add_trading_pair(sol_usdc);

    // Create users with initial balances
    let alice = UserId::new_v4();
    let bob = UserId::new_v4();
    engine.add_user(User::new(Some(alice)));
    engine.add_user(User::new(Some(bob)));
    engine.deposit(alice, Asset::USDC, 100_000).unwrap();
    engine.deposit(bob, Asset::ETH, 50).unwrap();
    engine.deposit(bob, Asset::SOL, 200).unwrap();

    // ============================================================================
    print_separator("Initial balances");
    // ============================================================================
    print_balances(&engine, &alice, "Alice");
    print_balances(&engine, &bob, "Bob");

    // ============================================================================
    print_separator("1. Basic match — Bob sells 10 ETH, Alice buys 5 (partial fill)");
    // ============================================================================
    place(&mut engine, bob, &eth_usdc, Side::Sell, 2000, 10);
    place(&mut engine, alice, &eth_usdc, Side::Buy, 2000, 5);
    print_book(&engine, &eth_usdc, "After test 1");
    println!("  Orders remaining: {:?}", engine.size(&eth_usdc));
    print_balances(&engine, &alice, "Alice");
    print_balances(&engine, &bob, "Bob");

    // ============================================================================
    print_separator("2. No match — Alice buys below best ask");
    // ============================================================================
    place(&mut engine, alice, &eth_usdc, Side::Buy, 1900, 3);
    print_book(&engine, &eth_usdc, "After test 2");
    print_balances(&engine, &alice, "Alice");

    // ============================================================================
    print_separator("3. Full fill remaining — Alice buys remaining 5 ETH");
    // ============================================================================
    place(&mut engine, alice, &eth_usdc, Side::Buy, 2000, 5);
    print_book(&engine, &eth_usdc, "After test 3");
    println!("  Orders remaining: {:?}", engine.size(&eth_usdc));
    print_balances(&engine, &alice, "Alice");
    print_balances(&engine, &bob, "Bob");

    // ============================================================================
    print_separator("4. Cancel order");
    // ============================================================================
    let sell2 = place(&mut engine, bob, &eth_usdc, Side::Sell, 2100, 8);
    print_book(&engine, &eth_usdc, "Before cancel");
    println!("  Orders before cancel: {:?}", engine.size(&eth_usdc));
    let cancelled = engine.cancel_order(&eth_usdc, &sell2.order_id);
    println!("  Cancel returned: {}", cancelled);
    print_book(&engine, &eth_usdc, "After cancel");
    println!("  Orders after cancel: {:?}", engine.size(&eth_usdc));
    print_balances(&engine, &bob, "Bob (after cancel)");

    // ============================================================================
    print_separator("5. Modify order — change price to trigger match");
    // ============================================================================
    let sell3 = place(&mut engine, bob, &eth_usdc, Side::Sell, 2100, 10);
    print_book(&engine, &eth_usdc, "Before modify");
    println!("  Orders before modify: {:?}", engine.size(&eth_usdc));
    let modify = OrderModify::new(sell3.order_id, 1900, Side::Sell, 10, OrderStatus::Empty, bob);
    let mod_result = engine.modify_order(&eth_usdc, modify);
    println!("  Modify result: {:?}", mod_result.as_ref().map(|r| (r.order_id, r.trades.as_ref().map(|t| t.len()))));
    print_book(&engine, &eth_usdc, "After modify");
    println!("  Orders after modify: {:?}", engine.size(&eth_usdc));
    print_balances(&engine, &alice, "Alice (after modify)");
    print_balances(&engine, &bob, "Bob (after modify)");

    // ============================================================================
    print_separator("6. FillAndKill — can't match (buy below best ask)");
    // ============================================================================
    place_fak(&mut engine, alice, &eth_usdc, Side::Buy, 1800, 2);
    print_book(&engine, &eth_usdc, "After test 6");
    println!("  Orders: {:?}", engine.size(&eth_usdc));
    print_balances(&engine, &alice, "Alice (FAK rejected)");

    // ============================================================================
    print_separator("7. FillAndKill — matches fully");
    // ============================================================================
    place_fak(&mut engine, alice, &eth_usdc, Side::Buy, 1900, 2);
    print_book(&engine, &eth_usdc, "After test 7");
    println!("  Orders: {:?}", engine.size(&eth_usdc));
    print_balances(&engine, &alice, "Alice (FAK filled)");
    print_balances(&engine, &bob, "Bob (FAK filled)");

    // ============================================================================
    print_separator("8. Multiple trading pairs (SOL/USDC)");
    // ============================================================================
    place(&mut engine, bob, &sol_usdc, Side::Sell, 100, 20);
    place(&mut engine, alice, &sol_usdc, Side::Buy, 100, 10);
    place(&mut engine, alice, &sol_usdc, Side::Buy, 105, 5);
    print_book(&engine, &sol_usdc, "SOL/USDC book");
    print_book(&engine, &eth_usdc, "ETH/USDC book");
    println!("  SOL pair orders: {:?}", engine.size(&sol_usdc));
    println!("  ETH pair orders (should be unchanged): {:?}", engine.size(&eth_usdc));
    print_balances(&engine, &alice, "Alice (multi-pair)");
    print_balances(&engine, &bob, "Bob (multi-pair)");

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
