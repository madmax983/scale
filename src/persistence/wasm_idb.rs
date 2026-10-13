//! WASM IndexedDB backend (spec 1382 §4) — compiled only for wasm32.
//!
//! IndexedDB is async while the ECS is sync, so this bridge splits the
//! work exactly as the spec prescribes:
//!
//! * The game fires a *request* ([`WasmSaveBridge::request_save`],
//!   `request_load`, `request_list`, `request_delete`) — or uses the
//!   sync [`SaveStore`] impl, which fires the same requests.
//! * The bridge performs the IDB operation against database
//!   `scale-saves`, object store `saves` (key = slot name).
//! * When IDB answers, a [`SaveLoadCompleted`] is queued; the game
//!   polls [`WasmSaveBridge::poll_completions`] each frame.
//!
//! Sync reads (`SaveStore::get`/`list`) are served from an in-memory
//! cache that requests keep warm: `put` updates it optimistically,
//! load/list completions replace it with what IDB actually holds.
//! Stored values are JSON envelopes `{"slot": ..., "data": "<blob>"}`
//! so a `get_all` refresh can recover slot names from values alone
//! (metadata is derived from the blob header on list — see
//! `metadata_from_blob` in the parent module).
//!
//! Runtime verification note: there is no browser on the build sprite,
//! so this bridge is verified by `cargo check --target
//! wasm32-unknown-unknown` only; the driver verifies live IndexedDB
//! behavior in a real browser after deploy.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{IdbDatabase, IdbTransactionMode};

use super::{
    metadata_from_blob, sanitize_slot, SaveError, SaveLoadCompleted, SaveLoadKind, SaveMetadata,
    SaveStore,
};

/// IndexedDB database name.
pub const IDB_DATABASE: &str = "scale-saves";
/// IndexedDB object store name.
pub const IDB_STORE: &str = "saves";

#[derive(Default)]
struct BridgeInner {
    cache: RefCell<HashMap<String, Vec<u8>>>,
    completions: RefCell<Vec<SaveLoadCompleted>>,
    pending_apply: RefCell<Option<String>>,
}

thread_local! {
    /// The one shared bridge for this page: the WASM shell, the menu
    /// input handler, and the autosave path must all see the same cache
    /// and completion queue, and `Rc` state cannot live in a `static`
    /// or a Bevy resource — a thread-local is the honest home on the
    /// single-threaded web runtime.
    static SHARED_BRIDGE: RefCell<WasmSaveBridge> =
        RefCell::new(WasmSaveBridge::new());
}

/// Handle to the page-wide shared bridge.
#[must_use]
pub fn shared_bridge() -> WasmSaveBridge {
    SHARED_BRIDGE.with(|bridge| bridge.borrow().clone())
}

/// The async IndexedDB save bridge. Cloneable handle (shared `Rc` state).
#[derive(Clone, Default)]
pub struct WasmSaveBridge {
    inner: Rc<BridgeInner>,
}

impl WasmSaveBridge {
    /// Create a bridge with an empty cache. Call
    /// [`request_persistent_storage`] once at startup and
    /// [`WasmSaveBridge::request_list`] to warm the cache from IDB.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a slot whose next successful load completion should be
    /// applied to the world by the shell (menu "Load Autosave" path).
    pub fn set_pending_apply(&self, slot: &str) {
        *self.inner.pending_apply.borrow_mut() = Some(slot.to_string());
    }

    /// Peek at the pending-apply slot, if any.
    #[must_use]
    pub fn peek_pending_apply(&self) -> Option<String> {
        self.inner.pending_apply.borrow().clone()
    }

    /// Take the pending-apply slot, if any.
    #[must_use]
    pub fn take_pending_apply(&self) -> Option<String> {
        self.inner.pending_apply.borrow_mut().take()
    }

    /// Drain queued completion events (call once per frame).
    #[must_use]
    pub fn poll_completions(&self) -> Vec<SaveLoadCompleted> {
        std::mem::take(&mut *self.inner.completions.borrow_mut())
    }

    fn push_completion(&self, kind: SaveLoadKind, slot: &str, ok: bool, message: String) {
        self.inner.completions.borrow_mut().push(SaveLoadCompleted {
            kind,
            slot: slot.to_string(),
            ok,
            message,
        });
    }

    /// Fire an async IDB put for `slot`. The cache is updated
    /// optimistically; the completion reports the IDB outcome.
    pub fn request_save(&self, slot: &str, data: &[u8]) {
        let Ok(slot) = sanitize_slot(slot) else {
            self.push_completion(
                SaveLoadKind::Save,
                slot,
                false,
                format!("invalid slot name '{slot}'"),
            );
            return;
        };
        self.inner
            .cache
            .borrow_mut()
            .insert(slot.clone(), data.to_vec());
        let Ok(text) = String::from_utf8(data.to_vec()) else {
            self.push_completion(
                SaveLoadKind::Save,
                &slot,
                false,
                "save blob is not UTF-8 JSON".to_string(),
            );
            return;
        };
        let envelope = serde_json::json!({ "slot": slot, "data": text }).to_string();
        let bridge = self.clone();
        with_db(move |db| {
            let outcome = put_envelope(&db, &slot, &envelope);
            match outcome {
                Ok(request) => {
                    watch_request(
                        request,
                        bridge.clone(),
                        SaveLoadKind::Save,
                        slot.clone(),
                        "saved",
                    );
                }
                Err(msg) => bridge.push_completion(SaveLoadKind::Save, &slot, false, msg),
            }
        });
    }

    /// Fire an async IDB get for `slot`; on success the blob lands in
    /// the cache and a completion is queued.
    pub fn request_load(&self, slot: &str) {
        let Ok(slot) = sanitize_slot(slot) else {
            self.push_completion(
                SaveLoadKind::Load,
                slot,
                false,
                format!("invalid slot name '{slot}'"),
            );
            return;
        };
        let bridge = self.clone();
        let slot_for_db = slot.clone();
        with_db(move |db| {
            let outcome = (|| -> Result<web_sys::IdbRequest, String> {
                let tx = db
                    .transaction_with_str_and_mode(&IDB_STORE, IdbTransactionMode::Readonly)
                    .map_err(|_| "IDB transaction failed".to_string())?;
                let store = tx
                    .object_store(IDB_STORE)
                    .map_err(|_| "IDB object store missing".to_string())?;
                store
                    .get(&JsValue::from_str(&slot_for_db))
                    .map_err(|_| "IDB get failed".to_string())
            })();
            match outcome {
                Ok(request) => {
                    let bridge2 = bridge.clone();
                    let slot2 = slot.clone();
                    let req = request.clone();
                    let on_ok = Closure::once_into_js(move |_: web_sys::Event| {
                        match req.result() {
                            Ok(value) => {
                                if value.is_undefined() || value.is_null() {
                                    bridge2.push_completion(
                                        SaveLoadKind::Load,
                                        &slot2,
                                        false,
                                        format!("no save in slot '{slot2}'"),
                                    );
                                } else if let Some(blob) = decode_envelope(&value) {
                                    bridge2
                                        .inner
                                        .cache
                                        .borrow_mut()
                                        .insert(slot2.clone(), blob);
                                    bridge2.push_completion(
                                        SaveLoadKind::Load,
                                        &slot2,
                                        true,
                                        "loaded into cache".to_string(),
                                    );
                                } else {
                                    bridge2.push_completion(
                                        SaveLoadKind::Load,
                                        &slot2,
                                        false,
                                        "stored save envelope unreadable".to_string(),
                                    );
                                }
                            }
                            Err(_) => bridge2.push_completion(
                                SaveLoadKind::Load,
                                &slot2,
                                false,
                                "IDB get failed".to_string(),
                            ),
                        }
                    });
                    request.set_onsuccess(Some(on_ok.unchecked_ref()));
                    let bridge3 = bridge.clone();
                    let slot3 = slot.clone();
                    let on_err = Closure::once_into_js(move |_: web_sys::Event| {
                        bridge3.push_completion(
                            SaveLoadKind::Load,
                            &slot3,
                            false,
                            "IDB get error".to_string(),
                        );
                    });
                    request.set_onerror(Some(on_err.unchecked_ref()));
                }
                Err(msg) => bridge.push_completion(SaveLoadKind::Load, &slot, false, msg),
            }
        });
    }

    /// Fire an async IDB refresh of the whole cache (`get_all`).
    pub fn request_list(&self) {
        let bridge = self.clone();
        with_db(move |db| {
            let outcome = (|| -> Result<web_sys::IdbRequest, String> {
                let tx = db
                    .transaction_with_str_and_mode(&IDB_STORE, IdbTransactionMode::Readonly)
                    .map_err(|_| "IDB transaction failed".to_string())?;
                let store = tx
                    .object_store(IDB_STORE)
                    .map_err(|_| "IDB object store missing".to_string())?;
                store
                    .get_all()
                    .map_err(|_| "IDB get_all failed".to_string())
            })();
            match outcome {
                Ok(request) => {
                    let bridge2 = bridge.clone();
                    let req = request.clone();
                    let on_ok = Closure::once_into_js(move |_: web_sys::Event| {
                        let mut fresh = HashMap::new();
                        if let Ok(value) = req.result() {
                            if let Some(array) = value.dyn_ref::<js_sys::Array>() {
                                for item in array.iter() {
                                    if let Some((slot, blob)) = decode_envelope_full(&item) {
                                        fresh.insert(slot, blob);
                                    }
                                }
                            }
                        }
                        let count = fresh.len();
                        *bridge2.inner.cache.borrow_mut() = fresh;
                        bridge2.push_completion(
                            SaveLoadKind::List,
                            "",
                            true,
                            format!("{count} save(s) in IndexedDB"),
                        );
                    });
                    request.set_onsuccess(Some(on_ok.unchecked_ref()));
                    let bridge3 = bridge.clone();
                    let on_err = Closure::once_into_js(move |_: web_sys::Event| {
                        bridge3.push_completion(
                            SaveLoadKind::List,
                            "",
                            false,
                            "IDB get_all error".to_string(),
                        );
                    });
                    request.set_onerror(Some(on_err.unchecked_ref()));
                }
                Err(msg) => bridge.push_completion(SaveLoadKind::List, "", false, msg),
            }
        });
    }

    /// Fire an async IDB delete for `slot` (cache updated immediately).
    pub fn request_delete(&self, slot: &str) {
        let Ok(slot) = sanitize_slot(slot) else {
            self.push_completion(
                SaveLoadKind::Delete,
                slot,
                false,
                format!("invalid slot name '{slot}'"),
            );
            return;
        };
        self.inner.cache.borrow_mut().remove(&slot);
        let bridge = self.clone();
        let slot_for_db = slot.clone();
        with_db(move |db| {
            let outcome = (|| -> Result<web_sys::IdbRequest, String> {
                let tx = db
                    .transaction_with_str_and_mode(&IDB_STORE, IdbTransactionMode::Readwrite)
                    .map_err(|_| "IDB transaction failed".to_string())?;
                let store = tx
                    .object_store(IDB_STORE)
                    .map_err(|_| "IDB object store missing".to_string())?;
                store
                    .delete(&JsValue::from_str(&slot_for_db))
                    .map_err(|_| "IDB delete failed".to_string())
            })();
            match outcome {
                Ok(request) => watch_request(request, bridge, SaveLoadKind::Delete, slot, "deleted"),
                Err(msg) => bridge.push_completion(SaveLoadKind::Delete, &slot, false, msg),
            }
        });
    }
}

impl SaveStore for WasmSaveBridge {
    fn put(&mut self, slot: &str, data: &[u8]) -> Result<(), SaveError> {
        let slot = sanitize_slot(slot)?;
        self.request_save(&slot, data);
        Ok(())
    }

    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, SaveError> {
        let slot = sanitize_slot(slot)?;
        Ok(self.inner.cache.borrow().get(&slot).cloned())
    }

    fn list(&self) -> Result<Vec<SaveMetadata>, SaveError> {
        let mut out = Vec::new();
        for (slot, data) in self.inner.cache.borrow().iter() {
            out.push(metadata_from_blob(slot, data)?);
        }
        out.sort_by(|a, b| a.slot.cmp(&b.slot));
        Ok(out)
    }

    fn delete(&mut self, slot: &str) -> Result<(), SaveError> {
        let slot = sanitize_slot(slot)?;
        self.request_delete(&slot);
        Ok(())
    }
}

/// Ask the browser to persist our storage (best-effort, once at
/// startup; failure is not fatal — spec §4).
pub fn request_persistent_storage() {
    if let Some(window) = web_sys::window() {
        let navigator = window.navigator();
        let storage = navigator.storage();
        let _ = storage.persist();
    }
}

/// Open (creating if needed) the `scale-saves` database and hand it to
/// `f`. Open failures are dropped silently here — callers observe them
/// as missing completions; the sync cache keeps the game playable.
fn with_db<F>(f: F)
where
    F: FnOnce(IdbDatabase) + 'static,
{
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(Some(factory)) = window.indexed_db() else {
        return;
    };
    let Ok(open_request) = factory.open_with_u32(IDB_DATABASE, 1) else {
        return;
    };
    let req_for_upgrade = open_request.clone();
    let on_upgrade = Closure::once_into_js(move |_: web_sys::Event| {
        if let Ok(value) = req_for_upgrade.result() {
            if let Ok(db) = value.dyn_into::<IdbDatabase>() {
                // Creating an existing store errors — ignored, the
                // store either already exists or was just created.
                let _ = db.create_object_store(IDB_STORE);
            }
        }
    });
    open_request.set_onupgradeneeded(Some(on_upgrade.unchecked_ref()));
    let f = Rc::new(RefCell::new(Some(f)));
    let req_for_success = open_request.clone();
    let on_success = Closure::once_into_js(move |_: web_sys::Event| {
        if let Ok(value) = req_for_success.result() {
            if let Ok(db) = value.dyn_into::<IdbDatabase>() {
                if let Some(f) = f.borrow_mut().take() {
                    f(db);
                }
            }
        }
    });
    open_request.set_onsuccess(Some(on_success.unchecked_ref()));
}

fn put_envelope(
    db: &IdbDatabase,
    slot: &str,
    envelope: &str,
) -> Result<web_sys::IdbRequest, String> {
    let tx = db
        .transaction_with_str_and_mode(&IDB_STORE, IdbTransactionMode::Readwrite)
        .map_err(|_| "IDB transaction failed".to_string())?;
    let store = tx
        .object_store(IDB_STORE)
        .map_err(|_| "IDB object store missing".to_string())?;
    store
        .put_with_key(&JsValue::from_str(envelope), &JsValue::from_str(slot))
        .map_err(|_| "IDB put failed".to_string())
}

fn watch_request(
    request: web_sys::IdbRequest,
    bridge: WasmSaveBridge,
    kind: SaveLoadKind,
    slot: String,
    verb: &'static str,
) {
    let bridge_ok = bridge.clone();
    let slot_ok = slot.clone();
    let on_ok = Closure::once_into_js(move |_: web_sys::Event| {
        bridge_ok.push_completion(kind, &slot_ok, true, format!("{verb} '{slot_ok}'"));
    });
    request.set_onsuccess(Some(on_ok.unchecked_ref()));
    let on_err = Closure::once_into_js(move |_: web_sys::Event| {
        bridge.push_completion(kind, &slot, false, format!("IDB {verb} failed for '{slot}'"));
    });
    request.set_onerror(Some(on_err.unchecked_ref()));
}

/// Decode a stored envelope value to the blob bytes (load path).
fn decode_envelope(value: &JsValue) -> Option<Vec<u8>> {
    decode_envelope_full(value).map(|(_, blob)| blob)
}

/// Decode a stored envelope value to (slot, blob bytes).
fn decode_envelope_full(value: &JsValue) -> Option<(String, Vec<u8>)> {
    let text = value.as_string()?;
    let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
    let slot = parsed.get("slot")?.as_str()?.to_string();
    let data = parsed.get("data")?.as_str()?.to_string();
    Some((slot, data.into_bytes()))
}
