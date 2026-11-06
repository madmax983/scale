//! Physical Inventory Management.
//!
//! Handles the storage of physical items, tools, and curios within a Pop's personal inventory
//! or a container's storage, enforcing capacity limits.

use crate::layer1::items::ItemType;
use bevy_ecs::prelude::*;

/// A specific item stored within an [`Inventory`].
///
/// ## Examples
///
/// ```rust
/// use scale::layer1::economy::inventory::InventoryItem;
/// use scale::layer1::items::ItemType;
///
/// let item = InventoryItem { item_type: ItemType::Potato, entity: None };
/// assert_eq!(item.item_type, ItemType::Potato);
/// ```
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct InventoryItem {
    /// The specific type of the item (e.g., Potato, Curio).
    pub item_type: ItemType,
    /// The optional entity associated with this item (if preserved).
    pub entity: Option<Entity>,
}

/// Component attached to entities that can carry physical items (e.g., Pops, Crates).
///
/// ## Examples
///
/// ```rust
/// use scale::layer1::economy::inventory::Inventory;
///
/// let inv = Inventory::default();
/// assert_eq!(inv.capacity, 20);
/// assert!(inv.items.is_empty());
/// ```
#[derive(Component, Debug, Clone)]
pub struct Inventory {
    /// The list of items currently held in the inventory.
    pub items: Vec<InventoryItem>,
    /// The maximum number of items this inventory can hold.
    pub capacity: usize,
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            capacity: 20,
        }
    }
}

impl Inventory {
    /// Adds an item to the inventory.
    ///
    /// # Deprecated
    /// Use [`Inventory::try_add`] to handle capacity limits safely.
    /// This method will panic or silently fail in future versions if full.
    /// Currently, it just calls `try_add` and ignores the result (simulating old behavior but safe).
    pub fn add(&mut self, item: InventoryItem) {
        let _ = self.try_add(item);
    }

    /// Checks if the inventory contains at least one item of the given type.
    ///
    /// ## Examples
    ///
    /// ```rust
    /// use scale::layer1::economy::inventory::{Inventory, InventoryItem};
    /// use scale::layer1::economy::items::ItemType;
    ///
    /// let mut inv = Inventory::default();
    /// inv.try_add(InventoryItem { item_type: ItemType::Potato, entity: None });
    ///
    /// assert!(inv.has_item(ItemType::Potato));
    /// assert!(!inv.has_item(ItemType::Tool));
    /// ```
    #[must_use]
    pub fn has_item(&self, item_type: ItemType) -> bool {
        self.items.iter().any(|i| i.item_type == item_type)
    }

    /// Counts how many items of the given type this inventory holds.
    #[must_use]
    pub fn count(&self, item_type: ItemType) -> usize {
        self.items.iter().filter(|i| i.item_type == item_type).count()
    }

    /// Tries to add an item to the inventory.
    ///
    /// Returns `true` if successfully added, `false` if the inventory is at capacity.
    ///
    /// ## Examples
    ///
    /// ```rust
    /// use scale::layer1::economy::inventory::{Inventory, InventoryItem};
    /// use scale::layer1::economy::items::ItemType;
    ///
    /// let mut inv = Inventory { capacity: 1, ..Default::default() };
    /// let item = InventoryItem { item_type: ItemType::Potato, entity: None };
    ///
    /// assert!(inv.try_add(item.clone())); // First fits
    /// assert!(!inv.try_add(item));       // Second fails due to capacity
    /// ```
    pub fn try_add(&mut self, item: InventoryItem) -> bool {
        if self.items.len() >= self.capacity {
            return false;
        }
        self.items.push(item);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::items::ItemType;

    #[test]
    fn test_inventory_add() {
        let mut inventory = Inventory::default();
        let item = InventoryItem {
            item_type: ItemType::Potato,
            entity: None,
        };
        inventory.add(item.clone());
        assert_eq!(inventory.items.len(), 1);
        assert_eq!(inventory.items[0], item);
    }

    #[test]
    fn test_inventory_capacity() {
        let mut inventory = Inventory {
            capacity: 2,
            ..Default::default()
        };
        let item = InventoryItem {
            item_type: ItemType::Potato,
            entity: None,
        };

        assert!(inventory.try_add(item.clone()));
        assert!(inventory.try_add(item.clone()));
        assert!(!inventory.try_add(item.clone())); // Should fail
        assert_eq!(inventory.items.len(), 2);
    }
}
#[cfg(test)]
mod inventory_extra_tests {
    use super::*;
    use crate::layer1::items::ItemType;

    #[test]
    fn test_inventory_has_item() {
        let mut inventory = Inventory::default();
        let item = InventoryItem {
            item_type: ItemType::Potato,
            entity: None,
        };
        inventory.try_add(item);

        assert!(inventory.has_item(ItemType::Potato));
        assert!(!inventory.has_item(ItemType::Meat));
    }
}

#[cfg(test)]
mod inventory_count_tests {
    use super::*;
    use crate::layer1::items::ItemType;

    #[test]
    fn test_inventory_count() {
        let mut inventory = Inventory::default();
        assert_eq!(inventory.count(ItemType::Potato), 0);
        inventory.add(InventoryItem {
            item_type: ItemType::Potato,
            entity: None,
        });
        inventory.add(InventoryItem {
            item_type: ItemType::Potato,
            entity: None,
        });
        inventory.add(InventoryItem {
            item_type: ItemType::Meat,
            entity: None,
        });
        assert_eq!(inventory.count(ItemType::Potato), 2);
        assert_eq!(inventory.count(ItemType::Meat), 1);
        assert_eq!(inventory.count(ItemType::Tool), 0);
    }
}
