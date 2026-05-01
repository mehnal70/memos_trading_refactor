# Memos Trading - AI Coding Agent Instructions

## Architecture Overview

**Multi-layer Rust/TypeScript trading platform** with three distinct packages:

1. **`memos_trading_core/`** (Rust): Core trading logic library
   - Strategy engine (`strategies.rs`): Trait-based strategy system with `MaCrossoverStrategy`, `RsiStrategy`
   - Type definitions (`types.rs`): `Candle`, `Signal`, `Trade`, `StrategyParams`, `Exchange`, `Market`
   - Database abstraction (`database.rs`): Async trait for candle/trade persistence (not yet implemented)
   - Risk management (`risk.rs`): Position sizing, stop-loss/take-profit calculations
   - Modules `api.rs`, `engine.rs`, `portfolio.rs`, `indicators.rs` are placeholders for future implementation

2. **`memos_trading_desktop/`** (Tauri + React + TypeScript): Desktop GUI application
   - Frontend: React 18 + TypeScript in `src/` (Vite build)
   - Backend: `src-tauri/` Rust layer that bridges frontend to `memos_trading_core`
   - Tauri commands expose strategies: `ma_signal`, `rsi_signal`, `get_version` (see `src-tauri/src/main.rs`)
   - Frontend invokes backend via `@tauri-apps/api/core` `invoke()` calls

3. **`memos_trading_wasm/`** (Rust → WebAssembly): Browser-compatible bindings
   - Exposes `TradingWasm` class with methods like `get_ma_signal`, `get_rsi_signal`
   - Uses `wasm-bindgen` to compile core strategies for web use

**Data Flow**: User input (React) → Tauri invoke → Rust command → `memos_trading_core` strategy → Signal response → React UI

## Development Workflows

### Building & Running

**Desktop app (primary use case)**:
```bash
cd memos_trading_desktop
npm install  # First time only
npm run tauri:dev  # Development mode with hot reload
npm run tauri:build  # Production build (output in src-tauri/target/release/)
```

**WASM module**:
```bash
cd memos_trading_wasm
cargo build --target wasm32-unknown-unknown
wasm-bindgen target/wasm32-unknown-unknown/debug/memos_trading_wasm.wasm --out-dir pkg
```

**Core library tests**:
```bash
cd memos_trading_core
cargo test
```

**Important**: The desktop dev script (`tauri:dev`) includes `LD_LIBRARY_PATH` override for Linux GTK compatibility. Don't remove this.

## Code Conventions & Patterns

### Rust (Core & Backend)

- **Turkish comments**: All inline comments in Rust use Turkish (`// Hareketli ortalamaları hesapla`). Maintain this convention.
- **Error handling**: Use `Result<T>` with custom error enum `MemosTradingError` from `lib.rs`. Propagate errors with `?` operator.
- **Strategy pattern**: All trading strategies implement the `Strategy` trait with `generate_signal()` and `name()` methods. Always return `Result<Signal>` where `Signal` is `Buy | Sell | Hold`.
- **Async traits**: Use `#[async_trait]` for async trait methods (see `Database` trait in `database.rs`).
- **Parameter passing**: Strategies accept `&[Candle]` slices and `&StrategyParams` struct. Use `Option<T>` for optional params (e.g., `fast: Option<usize>`).
- **Workspace dependencies**: Shared deps like `tokio`, `serde`, `chrono` are defined in root `Cargo.toml` `[workspace.dependencies]`. Reference them without version in member crates.

### TypeScript/React (Frontend)

- **Tauri communication**: Always use `invoke<ResponseType>('command_name', { param })` from `@tauri-apps/api/core`. Commands match Rust `#[tauri::command]` functions.
- **State management**: Use React hooks (`useState`, `useEffect`). No external state library currently.
- **Type safety**: Define interfaces for all Tauri responses (e.g., `SignalResponse`, `VersionResponse`).

### Cross-Language Data Structures

When adding new Tauri commands:
1. Define Rust structs with `#[derive(Serialize, Deserialize)]` in `src-tauri/src/lib.rs` or `main.rs`
2. Create matching TypeScript interfaces in React components
3. Convert frontend data to `Candle` structs in Rust layer (see `ma_signal` command for pattern)
4. Map `Signal` enum to string literals: `"BUY"`, `"SELL"`, `"HOLD"`

## Critical Implementation Details

### Strategy Calculation Pattern

All strategies follow this template (from `strategies.rs`):
1. Validate input length: `if candles.len() < period { return Ok(Signal::Hold); }`
2. Extract parameters from `StrategyParams` with `.unwrap_or(default)`
3. Calculate indicators using helper functions (`calculate_sma`, `calculate_rsi`)
4. Return `Signal` based on thresholds

### Candle Construction

When receiving price data (e.g., from frontend as `Vec<f64>`), construct candles with:
- `timestamp: Utc::now()` (from `chrono`)
- Set `open`, `high`, `low` equal to `close` for simplified data
- Use placeholder `symbol` like `"CHART"` and `interval` like `"1m"`
- Set `volume: 0.0` if not available

### Risk Management

`RiskManager` (in `risk.rs`) calculates:
- Stop loss: `entry_price * (1.0 - stop_loss_pct / 100.0)`
- Take profit: `entry_price * (1.0 + take_profit_pct / 100.0)`
- Position size: `capital * max_position_size_pct / 100.0 / entry_price`

## Current Limitations & TODOs

- **Database**: `Database` trait exists but has no implementation. SQLite integration planned.
- **API integration**: `api.rs` is a placeholder. No live exchange connections yet.
- **Backtesting**: `engine.rs` not implemented. Strategies tested manually via desktop app.
- **Portfolio**: `portfolio.rs` placeholder. No multi-position tracking.
- **Indicators**: `indicators.rs` placeholder. Current indicators (`calculate_sma`, `calculate_rsi`) live in `strategies.rs`.

When implementing these, follow the established async/trait patterns and maintain Turkish comments in Rust code.

## Testing Strategy

- Unit tests for core calculation functions (SMA, RSI) in `strategies.rs`
- Integration tests for Tauri commands should mock candle data
- Manual testing via desktop app's UI is primary validation method currently
