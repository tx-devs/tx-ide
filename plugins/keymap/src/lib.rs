//! tx-core plugin `keymap`: prefix+? draws a keyboard of the prefix table. Each key is a tx
//! shortcut (a plugin's, from `<data>/keys/.installed`), a tmux one (built in, or the user's own
//! tmux.conf) or free; pressing a key shows its keystroke, its description (the bind's `-N` note)
//! and its owner. Tab switches the mode (base, shift, control, option) and Shift-Tab the
//! keyboard (mac, windows). Publishes `keys/keymap.tmux`.

pub mod bindings;
pub mod keyboard;
pub mod view;

use std::sync::Arc;

use tx_client::tmux::Ui;
use tx_core::{Plugin, PluginError, Registry, TxCore, Verb, VerbError, VerbIo};

pub const PLUGIN_ID: &str = "keymap";

#[derive(Clone, Copy, Debug, Default)]
pub struct KeymapPlugin;

impl Plugin for KeymapPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn register(&self, r: &mut Registry) -> Result<(), PluginError> {
        r.verb(Arc::new(Keymap));
        Ok(())
    }
}

pub struct Keymap;

impl Verb for Keymap {
    fn name(&self) -> &'static str {
        "keymap"
    }

    fn usage(&self) -> &'static str {
        "keymap   a keyboard of the prefix shortcuts: tx, tmux or free; press a key to see what it does (prefix+?)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        if !argv.is_empty() {
            let _ = writeln!(io.stderr, "usage: tx keymap");
            return Ok(2);
        }
        let failed = |error: &dyn std::fmt::Display| VerbError::Failed(error.to_string());
        let bindings = bindings::Bindings::load(&Ui::from_core(core), &core.config().directories.data)
            .map_err(|error| failed(&error))?;
        view::run(bindings).map_err(|error| failed(&error))?;
        Ok(0)
    }
}
