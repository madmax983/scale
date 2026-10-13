//! Settings crumbs (spec 1382 §4): tiny preferences only.
//!
//! On WASM these persist in `localStorage` under `scale.settings` —
//! last scenario + last origin selection, used to pre-select menu
//! entries. Nothing larger ever goes to localStorage (~5 MB cap,
//! string-only, synchronous). On native, settings are a no-op stub:
//! the values live in the save snapshot header instead.

use super::SaveError;
use serde::{Deserialize, Serialize};

/// localStorage key for the settings crumbs (WASM only).
pub const SETTINGS_KEY: &str = "scale.settings";

/// Tiny persisted preferences.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveSettings {
    /// Last scenario id/name chosen on the menu.
    pub last_scenario: Option<String>,
    /// Last adventurer origin id chosen on the menu.
    pub last_origin: Option<String>,
}

/// Load settings crumbs. WASM: from localStorage. Native: defaults.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn load_settings() -> SaveSettings {
    let text = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(SETTINGS_KEY).ok().flatten());
    text.and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Load settings crumbs (native stub: no localStorage on desktop).
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn load_settings() -> SaveSettings {
    SaveSettings::default()
}

/// Store settings crumbs. WASM: to localStorage. Native: no-op.
///
/// # Errors
/// [`SaveError::Backend`] if localStorage refuses the write.
#[cfg(target_arch = "wasm32")]
pub fn store_settings(settings: &SaveSettings) -> Result<(), SaveError> {
    let text = serde_json::to_string(settings)
        .map_err(|e| SaveError::Serialize(e.to_string()))?;
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or_else(|| SaveError::Backend("localStorage unavailable".to_string()))?;
    storage
        .set_item(SETTINGS_KEY, &text)
        .map_err(|_| SaveError::Backend("localStorage write failed".to_string()))
}

/// Store settings crumbs (native stub).
#[cfg(not(target_arch = "wasm32"))]
pub fn store_settings(_settings: &SaveSettings) -> Result<(), SaveError> {
    Ok(())
}
