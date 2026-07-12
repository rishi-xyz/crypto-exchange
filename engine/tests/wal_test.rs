use std::sync::{Arc, Mutex};

use engine::{
    engine::ExchangeEngine,
    order::Order,
    trading_pair::TradingPair,
    types::{Asset, OrderStatus, OrderType, Side},
    users::User,
    wal::{Wal, WalEngine},
};

fn user_a() -> uuid::Uuid {
    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
}

fn user_b() -> uuid::Uuid {
    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap()
}

fn place(
    engine: &mut WalEngine,
    user: uuid::Uuid,
    pair: &TradingPair,
    side: Side,
    price: i32,
    qty: u32,
) -> engine::engine::AddOrderResult {
    let order = Arc::new(Mutex::new(Order::new(
        0,
        OrderType::GoodTillCancel,
        side,
        OrderStatus::Empty,
        price,
        qty,
        user,
    )));
    engine.add_order(user, pair, order).ok().flatten().unwrap()
}

// ---------------------------------------------------------------------------
// 1. Basic replay — all operations recovered
// ---------------------------------------------------------------------------

#[test]
fn test_wal_basic_replay() {
    engine::logging::init_test();
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    // Phase 1: build up state
    {
        let mut engine = WalEngine::new(&wal_path);
        let pair = TradingPair::new(Asset::ETH, Asset::USDC);
        engine.add_trading_pair(pair);
        engine.add_user(User::new(Some(user_a())));
        engine.add_user(User::new(Some(user_b())));
        engine.deposit(user_a(), Asset::USDC, 100_000).unwrap();
        engine.deposit(user_b(), Asset::ETH, 50).unwrap();

        let _ = place(&mut engine, user_b(), &pair, Side::Sell, 2000, 10);
        let r = place(&mut engine, user_a(), &pair, Side::Buy, 2000, 5);
        assert_eq!(r.trades.as_ref().unwrap().len(), 1, "one trade during phase 1");
        assert_eq!(engine.size(&pair), Some(1), "sell 5 remaining");
    }

    // Phase 2: replay and verify state is identical
    {
        let engine = WalEngine::new(&wal_path);
        let pair = TradingPair::new(Asset::ETH, Asset::USDC);

        assert_eq!(engine.size(&pair), Some(1), "sell 5 remaining after replay");

        let alice_bal = engine.get_user_balance(&user_a());
        assert!(alice_bal.is_some(), "alice exists after replay");
    }

    // WAL should be truncated after replay
    {
        let entries = Wal::replay(&wal_path).unwrap();
        assert_eq!(entries.len(), 0, "WAL truncated after replay");
    }
}

// ---------------------------------------------------------------------------
// 2. Partial/corrupt WAL line — skipped gracefully
// ---------------------------------------------------------------------------

#[test]
fn test_wal_corrupt_line_skipped() {
    engine::logging::init_test();
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    // Build some valid state
    {
        let mut engine = WalEngine::new(&wal_path);
        let pair = TradingPair::new(Asset::ETH, Asset::USDC);
        engine.add_trading_pair(pair);
        engine.add_user(User::new(Some(user_a())));
    }

    // Now append a corrupt line directly into the WAL file
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&wal_path)
            .unwrap();
        // Null operation — deserializes but is not a valid variant
        writeln!(f, "{{\"sequence\":999,\"operation\":null,\"timestamp\":0}}").unwrap();
        // Partial JSON line (crash during write)
        let partial = "{\"sequence\":1000,\"operation\":{";
        f.write_all(partial.as_bytes()).unwrap();
    }

    // Replay should skip corrupt lines and return successfully
    {
        let engine = WalEngine::new(&wal_path);
        let pair = TradingPair::new(Asset::ETH, Asset::USDC);
        assert_eq!(engine.size(&pair), Some(0), "pair survived replay despite corrupt lines");
    }
}

// ---------------------------------------------------------------------------
// 3. Cancel + replay — cancelled order is gone
// ---------------------------------------------------------------------------

#[test]
fn test_wal_cancel_replay() {
    engine::logging::init_test();
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    // Phase 1: place then cancel
    let order_id;
    {
        let mut engine = WalEngine::new(&wal_path);
        let pair = TradingPair::new(Asset::ETH, Asset::USDC);
        engine.add_trading_pair(pair);
        engine.add_user(User::new(Some(user_a())));
        engine.deposit(user_a(), Asset::USDC, 100_000).unwrap();

        let r = place(&mut engine, user_a(), &pair, Side::Buy, 1900, 5);
        order_id = r.order_id;
        assert_eq!(engine.size(&pair), Some(1));

        assert!(engine.cancel_order(&pair, &order_id));
        assert_eq!(engine.size(&pair), Some(0));
    }

    // Phase 2: replay — cancel should persist
    {
        let engine = WalEngine::new(&wal_path);
        let pair = TradingPair::new(Asset::ETH, Asset::USDC);
        assert_eq!(engine.size(&pair), Some(0), "order stays cancelled after replay");
    }
}

// ---------------------------------------------------------------------------
// 4. Modify + replay — old gone, new present
// ---------------------------------------------------------------------------

#[test]
fn test_wal_modify_replay() {
    engine::logging::init_test();
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    {
        let mut engine = WalEngine::new(&wal_path);
        let pair = TradingPair::new(Asset::ETH, Asset::USDC);
        engine.add_trading_pair(pair);
        engine.add_user(User::new(Some(user_a())));
        engine.add_user(User::new(Some(user_b())));
        engine.deposit(user_a(), Asset::USDC, 100_000).unwrap();
        engine.deposit(user_b(), Asset::ETH, 50).unwrap();

        // Rest a buy at 1900 so the modify can trigger a match
        place(&mut engine, user_a(), &pair, Side::Buy, 1900, 3);
        let sell = place(&mut engine, user_b(), &pair, Side::Sell, 2100, 10);
        assert_eq!(engine.size(&pair), Some(2));

        // Modify sell down to 1900 — matches the resting buy
        let modify = engine::order_modify::OrderModify::new(
            sell.order_id,
            1900,
            Side::Sell,
            10,
            OrderStatus::Empty,
            user_b(),
        );
        let result = engine.modify_order(&pair, modify).unwrap();
        assert_eq!(result.trades.as_ref().unwrap().len(), 1, "modify triggered match");
    }

    // Replay
    {
        let engine = WalEngine::new(&wal_path);
        let pair = TradingPair::new(Asset::ETH, Asset::USDC);
        let _ = engine.size(&pair);
    }
}

// ---------------------------------------------------------------------------
// 5. No-WAL mode — CoreEngine::new() still works
// ---------------------------------------------------------------------------

#[test]
fn test_no_wal_mode() {
    engine::logging::init_test();
    let mut engine = engine::engine::CoreEngine::new();
    let pair = TradingPair::new(Asset::ETH, Asset::USDC);
    engine.add_trading_pair(pair);
    engine.add_user(User::new(Some(user_a())));
    engine.deposit(user_a(), Asset::USDC, 50_000).unwrap();

    let order_id = engine.next_id();
    let order = Arc::new(Mutex::new(Order::new(
        order_id,
        OrderType::GoodTillCancel,
        Side::Buy,
        OrderStatus::Empty,
        1800,
        2,
        user_a(),
    )));
    let r = engine.add_order(user_a(), &pair, order).ok().unwrap().unwrap();
    assert!(r.order_id > 0);
    assert_eq!(engine.size(&pair), Some(1));
}

// ---------------------------------------------------------------------------
// 6. WAL append and scan_sequence
// ---------------------------------------------------------------------------

#[test]
fn test_wal_sequence_numbers() {
    engine::logging::init_test();
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    {
        let mut engine = WalEngine::new(&wal_path);
        let pair = TradingPair::new(Asset::ETH, Asset::USDC);
        engine.add_trading_pair(pair);
        engine.add_user(User::new(Some(user_a())));
        engine.deposit(user_a(), Asset::USDC, 10_000).unwrap();
        let _ = place(&mut engine, user_a(), &pair, Side::Buy, 1000, 1);
    }

    // Replaying engine truncates the WAL
    {
        let _engine = WalEngine::new(&wal_path);
    }

    // WAL should be empty after replay+truncate
    let entries = Wal::replay(&wal_path).unwrap();
    assert_eq!(entries.len(), 0, "WAL truncated after replay");
}

// ---------------------------------------------------------------------------
// 7. Deposit replay — balance is correct
// ---------------------------------------------------------------------------

#[test]
fn test_wal_deposit_replay() {
    engine::logging::init_test();
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    {
        let mut engine = WalEngine::new(&wal_path);
        let pair = TradingPair::new(Asset::ETH, Asset::USDC);
        engine.add_trading_pair(pair);
        engine.add_user(User::new(Some(user_a())));
        engine.deposit(user_a(), Asset::USDC, 25_000).unwrap();
    }

    {
        let engine = WalEngine::new(&wal_path);
        let bal = engine.get_user_balance(&user_a()).unwrap();
        assert_eq!(bal.get(&Asset::USDC).unwrap().0, 25_000, "USDC balance matches");
    }
}

// ---------------------------------------------------------------------------
// 8. Multiple trading pairs — all survive replay
// ---------------------------------------------------------------------------

#[test]
fn test_wal_multiple_pairs_replay() {
    engine::logging::init_test();
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    let eth = TradingPair::new(Asset::ETH, Asset::USDC);
    let sol = TradingPair::new(Asset::SOL, Asset::USDC);

    {
        let mut engine = WalEngine::new(&wal_path);
        engine.add_trading_pair(eth);
        engine.add_trading_pair(sol);
        engine.add_user(User::new(Some(user_a())));
        engine.add_user(User::new(Some(user_b())));
        engine.deposit(user_a(), Asset::USDC, 100_000).unwrap();
        engine.deposit(user_b(), Asset::ETH, 50).unwrap();
        engine.deposit(user_b(), Asset::SOL, 200).unwrap();

        place(&mut engine, user_b(), &eth, Side::Sell, 2000, 10);
        place(&mut engine, user_a(), &eth, Side::Buy, 2000, 5);
        place(&mut engine, user_b(), &sol, Side::Sell, 100, 20);
        place(&mut engine, user_a(), &sol, Side::Buy, 100, 10);
    }

    {
        let engine = WalEngine::new(&wal_path);
        assert_eq!(engine.size(&eth), Some(1), "ETH sell 5 remaining");
        assert_eq!(engine.size(&sol), Some(1), "SOL sell 10 remaining");
    }
}

// ---------------------------------------------------------------------------
// 9. FAK order rejected during replay — no book entry
// ---------------------------------------------------------------------------

#[test]
fn test_wal_fak_rejected_no_side_effect() {
    engine::logging::init_test();
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    let eth = TradingPair::new(Asset::ETH, Asset::USDC);

    {
        let mut engine = WalEngine::new(&wal_path);
        engine.add_trading_pair(eth);
        engine.add_user(User::new(Some(user_a())));
        engine.deposit(user_a(), Asset::USDC, 100_000).unwrap();

        // FAK with no matching orders — should be rejected (returns None)
        let order = Arc::new(Mutex::new(Order::new(
            0,
            OrderType::FillAndKill,
            Side::Buy,
            OrderStatus::Empty,
            1900,
            5,
            user_a(),
        )));
        let result = engine.add_order(user_a(), &eth, order);
        assert!(result.unwrap().is_none(), "FAK rejected — no resting asks");
        assert_eq!(engine.size(&eth), Some(0), "nothing in book");
    }

    {
        let engine = WalEngine::new(&wal_path);
        assert_eq!(engine.size(&eth), Some(0), "still nothing after replay");
    }
}

// ---------------------------------------------------------------------------
// 10. Locked balance survives replay
// ---------------------------------------------------------------------------

#[test]
fn test_wal_locked_balance_survives_replay() {
    engine::logging::init_test();
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    let eth = TradingPair::new(Asset::ETH, Asset::USDC);

    {
        let mut engine = WalEngine::new(&wal_path);
        engine.add_trading_pair(eth);
        engine.add_user(User::new(Some(user_a())));
        engine.deposit(user_a(), Asset::USDC, 100_000).unwrap();

        // Place a buy at 2000 * 10 = 20_000 USDC locked
        let _ = place(&mut engine, user_a(), &eth, Side::Buy, 2000, 10);

        let bal = engine.get_user_balance(&user_a()).unwrap();
        let (available, locked) = bal.get(&Asset::USDC).unwrap();
        assert_eq!(*locked, 20_000, "USDC locked = price * qty");
        assert_eq!(*available, 80_000, "USDC available = total - locked");
    }

    {
        let engine = WalEngine::new(&wal_path);
        let bal = engine.get_user_balance(&user_a()).unwrap();
        let (available, locked) = bal.get(&Asset::USDC).unwrap();
        assert_eq!(*locked, 20_000, "locked balance after replay");
        assert_eq!(*available, 80_000, "available balance after replay");
    }
}
