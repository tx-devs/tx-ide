//! `tx` (the CLI people type) and `txi` (the internal entry point machines run): argument
//! parsing, the built-in verbs, help and the dispatch to plugin verbs. The two binaries are
//! one call each into here; every command beyond the built-in ones comes from a plugin.

mod args;
mod check;
mod gc;
mod harness;
mod plugins;
mod tools;
mod tx;
mod txi;
mod update;

pub use tx::tx;
pub use txi::txi;
