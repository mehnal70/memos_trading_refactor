use memos_market_ibkr::{IbkrBridge, IBKR_DEFAULT_ADDR};

#[test]
fn default_bridge_has_stable_local_endpoint_and_monotonic_ids() {
    let bridge = IbkrBridge::with_defaults(None);
    assert_eq!(bridge.addr(), IBKR_DEFAULT_ADDR);
    assert_eq!(bridge.next_id(), 1);
    assert_eq!(bridge.next_id(), 2);
}
