//! Plexi voice agent — gpt-realtime-2 WebSocket session (#816).
pub mod schema;
pub mod session;

pub fn start_cli() -> i32 {
    session::run()
}

pub fn schema_cli() -> i32 {
    schema::print_schema()
}
