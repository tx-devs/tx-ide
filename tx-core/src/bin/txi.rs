//! `txi`: tx's internal entry point, run by the hooks an engine wires; never listed in `tx help`.

fn main() -> std::process::ExitCode {
    tx_core::cli::txi()
}
