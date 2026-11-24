//! `tx-engine-antigravity`: the `engine-antigravity` plugin as its own program. It serves the
//! Antigravity engine to core (`--tx-engine`) and is the hook command of the agents it launches
//! (`--tx-hook <Event>`).

fn main() -> std::process::ExitCode {
    tx_core::sdk::main(vec![Box::new(tx_engine_antigravity::AntigravityPlugin)])
}
