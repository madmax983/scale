//! Spec 378: Comms Relay — "Signal Noise" fog of war on the System Map.
//!
//! The system map (Layer 2) starts drowned in static: a
//! [`SignalNoiseMap`] covers every cell with noise. A powered
//! [`CommsRelay`] burns a clear disc out of that static; an unpowered
//! relay clears nothing. Entities tagged [`SignalContact`] are hidden
//! while they sit in noise and revealed — with an
//! [`EntityRevealedEvent`] and a chronicle entry — the first time a
//! clear zone reaches them.
//!
//! # Adaptation notes (spec sketch vs. the real tree)
//!
//! * The spec sketched `crate::layer2::map::SystemCoordinate` and
//!   `PowerConsumer { consumed }`. Neither exists: Layer 2 entities
//!   position themselves with `Transform` (see `layer2::sensors`), and
//!   the real `PowerConsumer` is `{ active, demand }`
//!   (`layer1::energy`). This module therefore defines its own small
//!   [`SystemCoordinate`] component for map-space entities and gates
//!   relay coverage on the real `PowerConsumer::active` wherever one
//!   is attached.
//! * `BuildingType::CommsRelay` already exists in Layer 1 (a 30-metal
//!   building). Rather than invent a second relay, powered colony
//!   Comms Relay *buildings* also clear noise around their
//!   `GridPosition` — see [`process_building_comms_coverage_system`],
//!   which mirrors the powered-CommandCenter pattern in
//!   `layer2::visibility`.
//! * This module is deliberately separate from
//!   `layer2::communications` (signal decay / latency for orders):
//!   that module moves messages, this one decides what the colony can
//!   *see*.

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::energy::PowerConsumer;
use crate::layer1::map::GridPosition;
use crate::layer2::sensors::VisibilityStatus;
use bevy::prelude::Transform;
use bevy_ecs::prelude::*;
use std::collections::HashSet;

/// Side length of the (square) system map the noise grid covers.
pub const SIGNAL_MAP_SIDE: usize = 100;
/// Default detection range of a standalone comms relay.
pub const DEFAULT_RELAY_RANGE: f32 = 10.0;
/// Range at which a powered colony Comms Relay building clears noise.
pub const BUILDING_RELAY_RANGE: f32 = 15.0;
/// Power demand of a colony Comms Relay building.
pub const BUILDING_RELAY_DEMAND: f32 = 5.0;

/// Position of an entity on the system map, in map cells.
///
/// Layer 2's real entities mostly use `Transform`; this component is
/// the lightweight map-space alternative the coverage and reveal
/// systems accept alongside it (a relay/contact may carry either).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct SystemCoordinate {
    /// Horizontal map coordinate.
    pub x: f32,
    /// Vertical map coordinate.
    pub y: f32,
}

/// A comms relay: while active (and powered, when it carries a
/// `PowerConsumer`) it clears signal noise within `range`.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct CommsRelay {
    /// Radius, in map cells, of the clear disc this relay maintains.
    pub range: f32,
    /// Whether the relay itself is switched on. A relay with a
    /// `PowerConsumer` must *also* be powered to clear noise.
    pub active: bool,
}

impl Default for CommsRelay {
    fn default() -> Self {
        Self {
            range: DEFAULT_RELAY_RANGE,
            active: true,
        }
    }
}

/// Marker for an entity that signal noise can hide and a clear zone
/// can reveal (an approaching ship, a comet, a distress beacon...).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct SignalContact;

/// Fired once each time a [`SignalContact`] enters a clear zone.
#[derive(Event, Debug, Clone, Copy, PartialEq)]
pub struct EntityRevealedEvent {
    /// The revealed entity.
    pub entity: Entity,
    /// Map x of the contact when revealed.
    pub x: f32,
    /// Map y of the contact when revealed.
    pub y: f32,
}

/// The "Signal Noise" fog covering the system map.
///
/// `noise` is a row-major grid of `radius * radius` cells; `true`
/// means the cell is drowned in static. (The spec names the side
/// length `radius`; the field keeps that name for compatibility.)
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct SignalNoiseMap {
    /// Side length of the square grid.
    pub radius: usize,
    /// Row-major noise flags, `radius * radius` entries.
    pub noise: Vec<bool>,
}

impl Default for SignalNoiseMap {
    fn default() -> Self {
        Self::fully_noisy(SIGNAL_MAP_SIDE)
    }
}

impl SignalNoiseMap {
    /// A map of the given side length, noisy everywhere.
    #[must_use]
    pub fn fully_noisy(radius: usize) -> Self {
        Self {
            radius,
            noise: vec![true; radius * radius],
        }
    }

    /// Is the cell at `(x, y)` hidden by noise? Out-of-bounds and
    /// negative coordinates are always noisy — the void does not
    /// resolve into a picture just because you left the chart.
    #[must_use]
    pub fn is_noisy(&self, x: f32, y: f32) -> bool {
        if x < 0.0 || y < 0.0 {
            return true;
        }
        let xi = x as usize;
        let yi = y as usize;
        if xi >= self.radius || yi >= self.radius {
            return true;
        }
        self.noise.get(yi * self.radius + xi).copied().unwrap_or(true)
    }

    /// Clear every cell within `range` of `(center_x, center_y)`.
    pub fn clear_noise(&mut self, center_x: f32, center_y: f32, range: f32) {
        if range <= 0.0 || self.radius == 0 {
            return;
        }
        // Only scan the disc's bounding box, clamped to the grid.
        let x0 = (center_x - range).floor().max(0.0) as usize;
        let y0 = (center_y - range).floor().max(0.0) as usize;
        let x1 = ((center_x + range).ceil() as usize).min(self.radius - 1);
        let y1 = ((center_y + range).ceil() as usize).min(self.radius - 1);
        let range_sq = range * range;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                if dx * dx + dy * dy <= range_sq {
                    let index = y * self.radius + x;
                    if let Some(cell) = self.noise.get_mut(index) {
                        *cell = false;
                    }
                }
            }
        }
    }

    /// Drown the whole map in noise again (coverage is recomputed
    /// from the live relays every tick).
    pub fn reset_to_noise(&mut self) {
        self.noise.fill(true);
    }

    /// Number of cells currently clear of noise.
    #[must_use]
    pub fn clear_count(&self) -> usize {
        self.noise.iter().filter(|cell| !**cell).count()
    }

    /// Number of cells still hidden by noise.
    #[must_use]
    pub fn noisy_count(&self) -> usize {
        self.noise.iter().filter(|cell| **cell).count()
    }

    /// Fraction of the map that is clear, `0.0..=1.0`.
    #[must_use]
    pub fn clear_fraction(&self) -> f32 {
        if self.noise.is_empty() {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let total = self.noise.len() as f32;
        #[allow(clippy::cast_precision_loss)]
        let clear = self.clear_count() as f32;
        clear / total
    }
}

/// Bookkeeping for contact reveals: which contacts are currently
/// revealed, and how many reveals have ever fired.
#[derive(Resource, Default, Debug, Clone)]
pub struct CommsRevealState {
    /// Contacts currently standing in a clear zone.
    pub revealed: HashSet<Entity>,
    /// Total reveal events fired since the world began.
    pub total_revealed: u32,
}

/// Recompute the noise map from the standalone relays.
///
/// The map resets to full noise, then every relay that is switched
/// on — and powered, if it carries a `PowerConsumer` — clears its
/// disc. An inactive or unpowered relay clears nothing.
#[allow(clippy::type_complexity)]
pub fn process_comms_coverage_system(
    mut map: ResMut<SignalNoiseMap>,
    relays: Query<(
        &CommsRelay,
        Option<&SystemCoordinate>,
        Option<&Transform>,
        Option<&PowerConsumer>,
    )>,
) {
    map.reset_to_noise();
    for (relay, coord, transform, power) in &relays {
        let powered = power.is_none_or(|consumer| consumer.active);
        if !relay.active || !powered {
            continue;
        }
        let (x, y) = position_of(coord, transform);
        map.clear_noise(x, y, relay.range);
    }
}

/// Powered colony Comms Relay *buildings* clear noise around their
/// tile (range [`BUILDING_RELAY_RANGE`]).
///
/// Runs after [`process_comms_coverage_system`] in the schedule, so
/// it must not reset the map — it only adds its discs on top. A
/// building that also carries a [`CommsRelay`] component is skipped
/// here; the standalone system already applied its own range.
pub fn process_building_comms_coverage_system(
    mut map: ResMut<SignalNoiseMap>,
    buildings: Query<(&Building, &PowerConsumer, &GridPosition), Without<CommsRelay>>,
) {
    for (building, power, position) in &buildings {
        if building.building_type != BuildingType::CommsRelay || !power.active {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let (x, y) = (position.x as f32, position.y as f32);
        map.clear_noise(x, y, BUILDING_RELAY_RANGE);
    }
}

/// Reveal contacts standing in clear zones; hide them again in noise.
///
/// A contact that crosses into a clear zone fires exactly one
/// [`EntityRevealedEvent`] (and gets `VisibilityStatus.is_visible`
/// set, the seam `layer2::sensors` renders from) until it slips back
/// into the static, which re-arms the reveal.
#[allow(clippy::type_complexity)]
pub fn reveal_signal_contacts_system(
    map: Res<SignalNoiseMap>,
    mut state: ResMut<CommsRevealState>,
    mut events: EventWriter<EntityRevealedEvent>,
    mut contacts: Query<
        (
            Entity,
            Option<&SystemCoordinate>,
            Option<&Transform>,
            Option<&mut VisibilityStatus>,
        ),
        With<SignalContact>,
    >,
) {
    for (entity, coord, transform, visibility) in &mut contacts {
        let (x, y) = position_of(coord, transform);
        let mut visibility = visibility;
        if map.is_noisy(x, y) {
            state.revealed.remove(&entity);
            if let Some(visibility) = visibility.as_deref_mut() {
                visibility.is_visible = false;
            }
            continue;
        }
        if let Some(visibility) = visibility.as_deref_mut() {
            visibility.is_visible = true;
        }
        if state.revealed.insert(entity) {
            state.total_revealed += 1;
            events.send(EntityRevealedEvent { entity, x, y });
        }
    }
}

/// Bridge reveals into the colony chronicle: the first clear picture
/// of something out in the dark is worth writing down.
pub fn comms_reveal_chronicle_bridge_system(
    mut revealed: EventReader<EntityRevealedEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for event in revealed.read() {
        chronicle.send(AddChronicleEvent {
            text: format!(
                "Signal resolved through the static: contact revealed at ({:.0}, {:.0}).",
                event.x, event.y
            ),
            importance: EventImportance::Standard,
        });
    }
}

fn position_of(coord: Option<&SystemCoordinate>, transform: Option<&Transform>) -> (f32, f32) {
    if let Some(coord) = coord {
        (coord.x, coord.y)
    } else if let Some(transform) = transform {
        (transform.translation.x, transform.translation.y)
    } else {
        (0.0, 0.0)
    }
}

/// Headless/STATS helper: `(clear cells, total cells, active relays,
/// total reveals)` read straight from the world.
#[must_use]
pub fn comms_census(world: &mut World) -> (usize, usize, usize, u32) {
    let (clear, total) = world
        .get_resource::<SignalNoiseMap>()
        .map_or((0, 0), |map| (map.clear_count(), map.noise.len()));
    let relays = {
        let mut query = world.query::<(&Building, &PowerConsumer)>();
        query
            .iter(world)
            .filter(|(building, power)| {
                building.building_type == BuildingType::CommsRelay && power.active
            })
            .count()
    };
    let standalone = {
        let mut query = world.query::<&CommsRelay>();
        query.iter(world).filter(|relay| relay.active).count()
    };
    let revealed = world
        .get_resource::<CommsRevealState>()
        .map_or(0, |state| state.total_revealed);
    (clear, total, relays + standalone, revealed)
}

/// One-word STATS label, e.g. `742/10000c,2r,1v` — clear/total cells,
/// active relays, total reveals. `off` when the map was never init.
#[must_use]
pub fn comms_stats_label(world: &mut World) -> String {
    if world.get_resource::<SignalNoiseMap>().is_none() {
        return "off".to_string();
    }
    let (clear, total, relays, revealed) = comms_census(world);
    format!("{clear}/{total}c,{relays}r,{revealed}v")
}
