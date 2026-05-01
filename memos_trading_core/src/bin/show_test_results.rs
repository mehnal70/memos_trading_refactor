use std::fs;
use std::path::Path;
use serde_json::Value;
use memos_trading_core::robot::dashboard::Dashboard;

fn main() {
    let test_results_path = "../test_results.json";
    if Path::new(test_results_path).exists() {
        let data = fs::read_to_string(test_results_path).expect("test_results.json okunamadı");
        let json: Value = serde_json::from_str(&data).expect("test_results.json parse edilemedi");
        Dashboard::show_test_results(&json);
    } else {
        println!("test_results.json bulunamadı, test runner ile oluşturun.");
    }
}
