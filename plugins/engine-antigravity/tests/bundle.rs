//! The bundle says what the code does: `plugin.yaml` against what `AntigravityPlugin` registers
//! and what `AntigravityAdapter` answers.

use std::path::Path;

use tx_core::plugin::EngineMethod;
use tx_core::{Agent, Input, LlmEngine};
use tx_engine_antigravity::{AntigravityAdapter, AntigravityPlugin};

#[test]
fn the_manifest_matches_the_plugin() {
    tx_core::testing::check_bundle(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        vec![Box::new(AntigravityPlugin)],
    );
}

/// Core stores the engine's data under the manifest's `ext_key` and answers `interrupt` from
/// the manifest, without starting the program: both must be what the adapter says.
#[test]
fn the_manifest_says_what_the_adapter_answers() {
    let manifest = tx_core::testing::check_manifest(Path::new(env!("CARGO_MANIFEST_DIR")));
    let engine = manifest.engine.unwrap();
    let adapter = AntigravityAdapter::new("/nowhere");
    assert_eq!(engine.ext_key, adapter.ext_key());
    let session = Agent::from_json(
        r#"{"version":2,"id":"a1","name":"worker","engine":"antigravity","working_directory":"/w","roles":[],"state":"working","created_at":1.0,"last_modified_at":1.0,"tags":[]}"#,
    )
    .unwrap();
    assert_eq!(engine.notes, adapter.notes());
    // The new-agent form offers the manifest's models and efforts.
    assert_eq!(engine.models, adapter.models());
    assert_eq!(engine.efforts, adapter.efforts());
    assert_eq!(
        engine.methods,
        [
            EngineMethod::Archive,
            EngineMethod::Restore,
            EngineMethod::Check
        ]
    );
    let fixed = engine.interrupt.unwrap();
    assert_eq!(
        adapter.interrupt(&session),
        Some((Input::Key(fixed.key), fixed.state))
    );
}
