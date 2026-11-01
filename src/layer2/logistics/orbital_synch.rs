//! Spec 384: Orbital Synch — the heavens gate Layer 1 industry.
//!
//! Certain advanced buildings only function while the colony planet is
//! inside a specific orbital arc: solar panels drink in bursts when the
//! planet swings through the solar arc, and the observatory's telescope
//! only resolves its target body inside a narrow synch window. Between
//! windows they produce nothing — the steady fuel `Generator` is the
//! deliberate contrast, and is never synch-gated.
//!
//! Adaptations from the spec sketch:
//! * The spec's `PlanetaryOrbit { angle }` resource does not exist and
//!   no parallel resource is invented here. The angle is the real
//!   [`Orbit`] angle (radians) of the colony planet — the entity
//!   carrying [`ColonyLocation`], spawned by `layer2::generation` and
//!   drifted every tick by spec 379's `advance_orbits_system`. This
//!   module converts that one angle to degrees and gates on it.
//! * The spec's sketch buildings map to real types: "Solar Array" is
//!   `BuildingType::SolarPanel` (a real `PowerSource`), "Telescope" is
//!   `BuildingType::Observatory`. There is no tidal generator building
//!   in the codebase; no new building type is added for this spec.
//! * The energy seam is `PowerSource.active`: the grid BFS in
//!   `layer1::energy` only counts sources with `active == true`, so an
//!   out-of-synch source contributes exactly zero, whatever its
//!   nameplate output (the solar cycle system rewrites `output` every
//!   tick but never touches `active`).
//! * State changes are observable, per the spec REFACTOR note about
//!   flip-flopping: [`SynchChangedEvent`] fires on rising/falling edges
//!   only, each edge writes one chronicle entry, and [`SynchLedger`]
//!   counts transitions for the headless panel / STATS.

use crate::layer1::building::{Building, BuildingType};
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::energy::PowerSource;
use crate::layer2::generation::ColonyLocation;
use crate::layer2::system::Orbit;
use bevy_ecs::prelude::*;
use std::f32::consts::TAU;

/// Which orbital arc a building is bound to.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SynchKind {
    /// Solar panels: a wide arc straddling angle 0 (wraps).
    Solar,
    /// Observatory telescopes: a narrow arc around 90 degrees.
    Telescope,
}

impl SynchKind {
    /// Parse a headless / CLI kind name (case-insensitive, aliases ok).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "solar" | "panel" | "array" | "solarpanel" => Some(Self::Solar),
            "telescope" | "observatory" | "scope" => Some(Self::Telescope),
            _ => None,
        }
    }

    /// Short display label (original name, IP-guard tested).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Solar => "Solar",
            Self::Telescope => "Telescope",
        }
    }

    /// The synch window in degrees: `(start, end)`, inclusive on both
    /// ends. `start > end` means the window wraps across 0 degrees.
    #[must_use]
    pub const fn window(&self) -> (f32, f32) {
        match self {
            // 120-degree arc straddling angle 0 — the wrap case is the
            // production default, not an edge case.
            Self::Solar => (300.0, 60.0),
            // The spec's example window, kept verbatim.
            Self::Telescope => (80.0, 100.0),
        }
    }

    /// Centre of the window in degrees (the `synchwin` debug seam
    /// parks the colony orbit here).
    #[must_use]
    pub fn window_center(&self) -> f32 {
        let (start, end) = self.window();
        if start <= end {
            (start + end) / 2.0
        } else {
            ((start + end + 360.0) / 2.0).rem_euclid(360.0)
        }
    }

    /// Human-readable window, e.g. `300->60 deg`.
    #[must_use]
    pub fn window_label(&self) -> String {
        let (start, end) = self.window();
        format!("{start:.0}->{end:.0} deg")
    }
}

/// A building gated by the colony planet's orbital position.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct OrbitalSynch {
    /// Which arc this building is bound to.
    pub kind: SynchKind,
    /// Whether the building is inside its window right now.
    pub active: bool,
    /// Window start, degrees (inclusive).
    pub required_angle_start: f32,
    /// Window end, degrees (inclusive); below `start` means wrap.
    pub required_angle_end: f32,
}

impl OrbitalSynch {
    /// A synch gate with an explicit window (spec sketch constructor).
    #[must_use]
    pub const fn new(kind: SynchKind, required_angle_start: f32, required_angle_end: f32) -> Self {
        Self {
            kind,
            active: false,
            required_angle_start,
            required_angle_end,
        }
    }

    /// A synch gate with the kind's default window.
    #[must_use]
    pub const fn for_kind(kind: SynchKind) -> Self {
        let (start, end) = kind.window();
        Self::new(kind, start, end)
    }

    /// Whether `angle_degrees` is inside this building's window.
    #[must_use]
    pub fn contains(&self, angle_degrees: f32) -> bool {
        angle_in_window(
            angle_degrees,
            self.required_angle_start,
            self.required_angle_end,
        )
    }
}

/// Whether `angle` (degrees, any range) falls inside the inclusive
/// window `[start, end]` (degrees). Windows with `start > end` wrap
/// across 0: 350 -> 10 contains 355 and 5, but not 180.
#[must_use]
pub fn angle_in_window(angle: f32, start: f32, end: f32) -> bool {
    let angle = angle.rem_euclid(360.0);
    let start = start.rem_euclid(360.0);
    let end = end.rem_euclid(360.0);
    if start <= end {
        angle >= start && angle <= end
    } else {
        angle >= start || angle <= end
    }
}

/// Edge event: a synch-gated building entered or left its window.
/// Fired on transitions only — never per-tick while steady.
#[derive(Event, Debug, Clone, Copy, PartialEq)]
pub struct SynchChangedEvent {
    /// The building whose synch state flipped.
    pub entity: Entity,
    /// Which arc it is bound to.
    pub kind: SynchKind,
    /// The new state (`true` = entered the window).
    pub active: bool,
    /// Colony orbital angle in degrees when the edge fired.
    pub angle_degrees: f32,
}

/// Running ledger of synch activity, for the headless panel and the
/// STATS `synch=` field.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct SynchLedger {
    /// Gated buildings currently inside their windows.
    pub in_synch: u32,
    /// Gated buildings currently outside their windows.
    pub gated: u32,
    /// Total rising + falling edges since world start.
    pub transitions: u32,
}

/// The colony planet's current orbital angle in degrees, read from the
/// real `Orbit` on the `ColonyLocation` entity. `None` when the world
/// has no colony body (pre-generation, or a bare test world).
#[must_use]
pub fn colony_angle_degrees(world: &mut World) -> Option<f32> {
    let mut query = world.query_filtered::<&Orbit, With<ColonyLocation>>();
    query
        .iter(world)
        .next()
        .map(|orbit| orbit.angle.rem_euclid(TAU).to_degrees())
}

/// Debug seam (headless `synchwin`): park the colony planet's real
/// `Orbit` at `degrees`. Headless worlds have no `DayNightCycle`-driven
/// orbit driver visible to the console, so playtests move the orbit
/// explicitly — the same pattern spec 379's panel used. Returns `false`
/// when the world has no colony body to move.
pub fn set_colony_angle_degrees(world: &mut World, degrees: f32) -> bool {
    let mut query = world.query_filtered::<&mut Orbit, With<ColonyLocation>>();
    let mut moved = false;
    for mut orbit in query.iter_mut(world) {
        orbit.angle = degrees.to_radians().rem_euclid(TAU);
        moved = true;
    }
    moved
}

/// One-shot spawn tagging: buildings that come into existence as a
/// SolarPanel or Observatory gain their [`OrbitalSynch`] gate. Runs on
/// `Added<Building>` so both `spawn_building` placements and direct
/// spawns are covered; buildings that already carry a gate (hand-tuned
/// windows) are left alone. The fuel Generator is deliberately absent —
/// steady power is the contrast the spec asks for.
pub fn apply_orbital_synch_spawn_system(
    mut commands: Commands,
    added: Query<(Entity, &Building, Option<&OrbitalSynch>), Added<Building>>,
) {
    for (entity, building, existing) in &added {
        if existing.is_some() {
            continue;
        }
        let kind = match building.building_type {
            BuildingType::SolarPanel => Some(SynchKind::Solar),
            BuildingType::Observatory => Some(SynchKind::Telescope),
            _ => None,
        };
        if let Some(kind) = kind {
            commands.entity(entity).insert(OrbitalSynch::for_kind(kind));
        }
    }
}

/// The gating system: recompute every gated building's `active` from
/// the colony planet's real `Orbit` angle, force its `PowerSource`
/// on/off through the real energy seam, and emit edge events +
/// chronicle entries on transitions only.
///
/// When the world has no colony body yet, nothing is gated (buildings
/// keep whatever state they spawned with) — gating against a missing
/// angle would dark the whole colony on tick one.
#[allow(clippy::type_complexity)]
pub fn update_orbital_synch_system(
    colony: Query<&Orbit, With<ColonyLocation>>,
    mut buildings: Query<(Entity, &mut OrbitalSynch, Option<&mut PowerSource>)>,
    mut ledger: ResMut<SynchLedger>,
    mut changed: EventWriter<SynchChangedEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let Some(orbit) = colony.iter().next() else {
        return;
    };
    let angle = orbit.angle.rem_euclid(TAU).to_degrees();

    let mut in_synch = 0u32;
    let mut gated = 0u32;
    for (entity, mut synch, power) in &mut buildings {
        let now_active = synch.contains(angle);
        if now_active {
            in_synch += 1;
        } else {
            gated += 1;
        }
        // Energy seam: an out-of-synch source is switched off, so the
        // grid counts exactly zero from it; an in-synch source is
        // switched back on (its output stays whatever the solar cycle
        // or fuel systems set — this system only owns the switch).
        if let Some(mut source) = power {
            source.active = now_active;
        }
        if now_active != synch.active {
            synch.active = now_active;
            ledger.transitions += 1;
            changed.send(SynchChangedEvent {
                entity,
                kind: synch.kind,
                active: now_active,
                angle_degrees: angle,
            });
            let text = if now_active {
                format!(
                    "Orbital synch: the {} swings into its arc at {angle:.0} degrees — burst production begins.",
                    synch.kind.label()
                )
            } else {
                format!(
                    "Orbital synch: the {} drifts out of its arc at {angle:.0} degrees — it falls silent until the heavens return.",
                    synch.kind.label()
                )
            };
            chronicle.send(AddChronicleEvent {
                text,
                importance: EventImportance::Standard,
            });
        }
    }
    ledger.in_synch = in_synch;
    ledger.gated = gated;
}

/// Census of gated buildings: `(in_synch, total)`.
#[must_use]
pub fn synch_census(world: &mut World) -> (usize, usize) {
    let mut query = world.query::<&OrbitalSynch>();
    let mut in_synch = 0usize;
    let mut total = 0usize;
    for synch in query.iter(world) {
        total += 1;
        if synch.active {
            in_synch += 1;
        }
    }
    (in_synch, total)
}

/// One-line STATS label for the `synch=` field: `off` when no building
/// is gated, else `<in>/<total>@<angle>deg` (or `<in>/<total>` when the
/// world has no colony body to read an angle from).
#[must_use]
pub fn synch_stats_label(world: &mut World) -> String {
    let (in_synch, total) = synch_census(world);
    if total == 0 {
        return "off".to_string();
    }
    match colony_angle_degrees(world) {
        Some(angle) => format!("{in_synch}/{total}@{angle:.0}deg"),
        None => format!("{in_synch}/{total}"),
    }
}
