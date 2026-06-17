pub mod bist;
// NOT: exchange_connector (ölü ExchangeConnector trait) + binance_connector (executor'ı o
// trait ardına saran eski venue denemesi) + coinbase/kucoin/bybit stub'ları + exchanges.rs
// kaldırıldı (çoklu-piyasa Faz 0-C). Venue soyutlaması artık robot::venue (VenueAdapter).
