//! No-op text-styling shims for wasm32, where `crossterm` is unavailable
//! and `comfy-table` is built without its `tty` feature (which needs crossterm).
//!
//! A handful of shared pretty-printing paths (`Display` impls, one warning
//! banner) use `crossterm::style::{Color, Stylize}` for terminal colors and
//! `comfy_table::{Color as TableColor, Attribute}` + `Cell::fg` /
//! `Cell::add_attribute` for table styling. The WASM build renders through
//! the ratzilla DOM backend instead, so on wasm32 all styling collapses to
//! the identity: text passes through unchanged and the call sites compile
//! without modification.

/// Color placeholder mirroring the `crossterm::style::Color` variants used
/// in this codebase. Accepted and ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Color {
    Blue,
    Cyan,
    DarkGrey,
    Green,
    Magenta,
    Red,
    White,
    Yellow,
}

/// No-op styling: mirrors the `crossterm::style::Stylize` methods used in
/// this codebase so styled call sites compile unchanged on wasm32.
pub trait Stylize: Sized {
    /// Ignore the requested color, return the value unchanged.
    fn with(self, _color: Color) -> Self {
        self
    }
    /// Ignore the requested color, return the value unchanged.
    fn yellow(self) -> Self {
        self
    }
}

impl<T> Stylize for T {}

/// Stand-in for `comfy_table::Color` (only exists with comfy-table's `tty`
/// feature). Accepted and ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum TableColor {
    Cyan,
    DarkGrey,
    Green,
    Magenta,
    Red,
    White,
    Yellow,
}

/// Stand-in for `comfy_table::Attribute` (only exists with comfy-table's
/// `tty` feature). Accepted and ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum TableAttribute {
    Bold,
    Italic,
}

/// No-op cell styling: mirrors the `comfy_table::Cell` styling methods used
/// in this codebase (`Cell::fg` / `Cell::add_attribute` are `tty`-gated and
/// absent on wasm32), so pretty-printing call sites compile unchanged.
pub trait CellStyle {
    /// Ignore the requested color, return the cell unchanged.
    fn fg(self, _color: TableColor) -> Self;
    /// Ignore the requested attribute, return the cell unchanged.
    fn add_attribute(self, _attr: TableAttribute) -> Self;
}

impl CellStyle for comfy_table::Cell {
    fn fg(self, _color: TableColor) -> Self {
        self
    }
    fn add_attribute(self, _attr: TableAttribute) -> Self {
        self
    }
}
