//! Meta layer: state that outlives a single colony (cross-run).
//!
//! Spec 414 "The Black Box" is the first tenant: an Archive seals a
//! fallen colony's tech + lore, and a future colony decrypts the ruin.

/// The Black Box: cross-run tech/lore preservation (Spec 414).
pub mod archive;

pub use archive::{
    Archive, BlackBoxData, BlackBoxDiskSync, BlackBoxState, DecryptedArchive, DecryptArchiveEvent,
    GameOverEvent, RuinedArchive, UploadProgress,
};
