use engine::snowflake::{
    SnowflakeGenerator, MACHINE_SHIFT, MAX_MACHINE_ID,
};

#[test]
fn test_snowflake_ids_are_unique() {
    let mut generator = SnowflakeGenerator::new(1, 1);
    let mut ids = std::collections::HashSet::new();
    for _ in 0..10_000 {
        let id = generator.next_id();
        assert!(ids.insert(id), "Duplicate ID: {}", id);
    }
}

#[test]
fn test_snowflake_ids_are_time_sortable() {
    let mut generator = SnowflakeGenerator::new(1, 1);
    let id1 = generator.next_id();
    let id2 = generator.next_id();
    let id3 = generator.next_id();
    assert!(id1 < id2);
    assert!(id2 < id3);
}

#[test]
fn test_snowflake_encodes_machine_and_datacenter() {
    let mut gen1 = SnowflakeGenerator::new(1, 1);
    let mut gen2 = SnowflakeGenerator::new(2, 1);
    let id1 = gen1.next_id();
    let id2 = gen2.next_id();
    assert_ne!(id1, id2);
    assert_eq!((id1 >> MACHINE_SHIFT) & MAX_MACHINE_ID, 1);
    assert_eq!((id2 >> MACHINE_SHIFT) & MAX_MACHINE_ID, 2);
}
