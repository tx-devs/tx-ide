//! `tx`: the CLI people type. Exit 0 on success, 1 on a core error, 2 on misuse.

fn main() -> std::process::ExitCode {
    tx_core::cli::tx()
}
