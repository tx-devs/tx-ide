//! What every program that runs as tx shares, so that a plugin program resolves the directories
//! and reads `config.yaml` exactly as `tx` does: the global flags, the config file, opening the
//! core, serving an engine and its hooks, and laying out the shipped plugins.

pub mod args;
pub mod bundle;
pub mod engine_program;
pub mod setup;
pub mod user_config;
