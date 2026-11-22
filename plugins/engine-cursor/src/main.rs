//! `tx-engine-cursor`: the `engine-cursor` plugin as its own program. It serves the Cursor engine
//! to core (`--tx-engine`) and is the hook command of the agents it launches
//! (`--tx-hook <Event>`).

fn main() -> std::process::ExitCode {
    tx_core::sdk::main(vec![Box::new(tx_engine_cursor::CursorPlugin)])
}
