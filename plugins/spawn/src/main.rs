//! `tx-spawn`: the `spawn` plugin as its own program.

fn main() -> std::process::ExitCode {
    tx_core::sdk::main(vec![Box::new(tx_spawn::SpawnPlugin)])
}
