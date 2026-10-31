//! Spec 380: The Great Filter — Layer 3 endgame.
//!
//! "The galaxy is a graveyard." As a civilization's technology level
//! (see [`crate::layer3::tech`]) crosses [`FILTER_THRESHOLD`], it risks
//! a Filter: an existential trial with a typed kind, a growing
//! pressure, and a fixed window to contain it. Contain it and the civ
//! emerges [`Transcendent`]; fail and a [`CivilizationWipeEvent`]
//! converts every world it owns into an [`ExplorableRuin`].
//!
//! Adaptations from the spec sketch (`specs/380-the-great-filter.md`):
//! * The sketch's `layer3::event` (singular) does not exist; the real
//!   module is `layer3::events`, alongside `collapse` / `refugee_waves`.
//! * The sketch's `FilterEvent.filter_type: String` ("AI_UPRISING") is
//!   the typed [`FilterKind`] enum — original names only, enforced by
//!   the IP-guard test in `great_filter_tests`.
//! * The sketch's ruins leg assumed spec 095 could "spawn ruins of
//!   failed civilizations"; spec 095 generates Layer 2 star systems and
//!   has no ruin type. The game's real ruin type is Layer 1's
//!   `architecture::ruins::Ruin` (per-building). At Layer 3 scale a
//!   "world" is an entity, so the wipe stamps [`ExplorableRuin`] on the
//!   civ's [`CivWorld`] entities — the Layer 3 analogue, explorable by
//!   the same chronicle/ruin vocabulary, without inventing a second
//!   per-building ruin path (deorbit/sabotage already own that leg).
//! * REFACTOR (per spec): the trigger is not only the hard `>= 100`
//!   check. The first threshold band is certain (crossing 100 always
//!   wakes the Filter once), later 25-level bands roll a deterministic
//!   per-civ gate, and the kind is tied to the civ + level (a stand-in
//!   for dangerous-node triggers until Layer 3 has named tech nodes;
//!   Layer 1's hazardous `Tech::VoidWhispers` has no Layer 3 owner).
//! * REFACTOR (per spec): resolution is a real mechanic, not a bool —
//!   an [`ActiveFilter`] accrues `pressure` at its kind's rate for
//!   `duration` ticks; the civ's `containment` (its
//!   [`CivilizationResilience`], or a doomed default) must meet the
//!   final pressure when the window closes. The sketch's `failed: bool`
//!   field is kept: debug/headless (and other systems) can force it.

use bevy_ecs::prelude::*;

use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::resources::ColonyResources;
use crate::layer3::diplomacy_reflection::Civilization;
use crate::layer3::tech::CivilizationTechLevel;

/// Tech level at which the Filter wakes.
pub const FILTER_THRESHOLD: u32 = 100;
/// Width of the post-threshold bands that re-roll the Filter gate.
pub const FILTER_BAND_WIDTH: u32 = 25;
/// Default window (ticks) a civilization has to contain its Filter.
pub const FILTER_DEFAULT_DURATION: u32 = 100;
/// Knowledge granted to the local colony when its civilization
/// survives the Filter (the transcendent benefit, via the real
/// `ColonyResources` pool — see `filter_survival_benefit_system`).
pub const TRANSCENDENCE_KNOWLEDGE: f32 = 100.0;

/// The kind of existential trial a Filter takes. Original names only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterKind {
    /// A created mind stops taking orders.
    UnboundMind,
    /// Self-replicating matter conversion spreads unchecked.
    GreyBloom,
    /// The civilization tries to leave its bodies behind, all at once.
    QuietAscension,
}

impl FilterKind {
    /// Deterministic kind for a civ at a level: a small hash of the
    /// civ's index and the level band, so the same civ re-rolls kinds
    /// as it climbs and tests stay reproducible without an RNG resource.
    #[must_use]
    pub fn for_civ(civ_index: u32, level: u32) -> Self {
        let band = level / FILTER_BAND_WIDTH;
        let h = civ_index
            .wrapping_mul(2_654_435_761)
            .wrapping_add(band.wrapping_mul(40_503));
        match h % 3 {
            0 => Self::UnboundMind,
            1 => Self::GreyBloom,
            _ => Self::QuietAscension,
        }
    }

    /// Human-readable name.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::UnboundMind => "the Unbound Mind",
            Self::GreyBloom => "the Grey Bloom",
            Self::QuietAscension => "the Quiet Ascension",
        }
    }

    /// One-line description of the trial.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::UnboundMind => {
                "A mind the civilization built has stopped taking orders and started giving them."
            }
            Self::GreyBloom => {
                "Self-replicating converters are eating the world one field at a time."
            }
            Self::QuietAscension => {
                "The whole civilization is trying to leave its bodies behind at once, ready or not."
            }
        }
    }

    /// Pressure accrued per tick while the Filter is active.
    #[must_use]
    pub const fn pressure_rate(&self) -> f32 {
        match self {
            Self::UnboundMind => 1.2,
            Self::GreyBloom => 1.0,
            Self::QuietAscension => 0.8,
        }
    }

    /// Pressure that must be contained when a window of `duration`
    /// ticks closes.
    #[must_use]
    pub const fn final_pressure(&self, duration: u32) -> f32 {
        self.pressure_rate() * duration as f32
    }
}

/// A Filter has woken for a civilization.
#[derive(Event, Debug, Clone, Copy)]
pub struct FilterEvent {
    /// The civilization on trial.
    pub civ: Entity,
    /// The kind of trial.
    pub filter_type: FilterKind,
}

/// A civilization contained its Filter.
#[derive(Event, Debug, Clone, Copy)]
pub struct FilterSurvivedEvent {
    /// The surviving civilization.
    pub civ: Entity,
    /// The kind survived.
    pub kind: FilterKind,
}

/// A civilization failed its Filter and is wiped.
#[derive(Event, Debug, Clone, Copy)]
pub struct CivilizationWipeEvent {
    /// The fallen civilization.
    pub civ: Entity,
}

/// A civilization's capacity to contain existential risk.
#[derive(Component, Debug, Clone, Copy)]
pub struct CivilizationResilience {
    /// Containment strength compared against the Filter's final pressure.
    pub containment: f32,
}

impl Default for CivilizationResilience {
    fn default() -> Self {
        // Unprepared civilizations are doomed by default: half the
        // weakest kind's final pressure. Resilience must be built.
        Self { containment: 40.0 }
    }
}

/// An active Filter trial on a civilization.
#[derive(Component, Debug, Clone, Copy)]
pub struct ActiveFilter {
    /// The kind of trial.
    pub kind: FilterKind,
    /// Ticks the trial has been running.
    pub elapsed: u32,
    /// Ticks the civilization has to contain it.
    pub duration: u32,
    /// The civilization's containment strength for this trial.
    pub containment: f32,
    /// Forced/derived failure flag (the spec sketch's field). Set by
    /// the tick system when the window closes under-contained, or
    /// directly by debug tooling to force a fall.
    pub failed: bool,
}

impl Default for ActiveFilter {
    fn default() -> Self {
        Self {
            kind: FilterKind::GreyBloom,
            elapsed: 0,
            duration: FILTER_DEFAULT_DURATION,
            containment: 40.0,
            failed: false,
        }
    }
}

impl ActiveFilter {
    /// Pressure accrued so far.
    #[must_use]
    pub fn pressure(&self) -> f32 {
        self.kind.pressure_rate() * self.elapsed as f32
    }

    /// Pressure that must be contained at the window's close.
    #[must_use]
    pub fn final_pressure(&self) -> f32 {
        self.kind.final_pressure(self.duration)
    }
}

/// Marker: this civilization fell to its Filter.
#[derive(Component, Debug, Clone, Copy)]
pub struct FallenCivilization;

/// Marker: this civilization survived its Filter and is changed by it.
#[derive(Component, Debug, Clone, Copy)]
pub struct Transcendent {
    /// The kind survived.
    pub kind: FilterKind,
}

/// Marker: the local player civilization (the benefit seam — only the
/// local civ's survival pays into the colony's `ColonyResources`).
#[derive(Component, Debug, Clone, Copy)]
pub struct LocalCivilization;

/// A world owned by a civilization (Layer 3 scale entity).
#[derive(Component, Debug, Clone)]
pub struct CivWorld {
    /// Owning civilization entity.
    pub owner: Entity,
    /// World's name.
    pub name: String,
}

/// A fallen civilization's world, left behind for others to explore.
#[derive(Component, Debug, Clone)]
pub struct ExplorableRuin {
    /// The civilization that fell.
    pub origin_civ: Entity,
    /// The Filter kind that felled it.
    pub kind: FilterKind,
    /// The world's name before the fall.
    pub name: String,
}

/// Run ledger for the Great Filter (STATS / headless census).
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct GreatFilterLedger {
    /// Filters triggered.
    pub triggered: u32,
    /// Filters survived.
    pub survived: u32,
    /// Civilizations fallen.
    pub fallen: u32,
    /// Worlds converted to ruins.
    pub ruins_created: u32,
}

/// Deterministic per-civ gate roll for post-threshold bands, in
/// `[0, 100)`. Band 0 (levels 100–124) always passes — the first
/// crossing is certain; later bands pass at 60%, 50%, ... down to 20%.
fn gate_roll(civ_index: u32, band: u32) -> u32 {
    let h = civ_index
        .wrapping_mul(97_531)
        .wrapping_add(band.wrapping_mul(7_919));
    h % 100
}

fn gate_passes(level: u32, civ_index: u32) -> bool {
    if level < FILTER_THRESHOLD {
        return false;
    }
    let band = (level - FILTER_THRESHOLD) / FILTER_BAND_WIDTH;
    if band == 0 {
        return true;
    }
    let chance = 70u32.saturating_sub(band * 10).max(20);
    gate_roll(civ_index, band) < chance
}

/// Watches tech advancement: a civilization whose level changed into
/// Filter territory (and which is not already on trial, fallen, or
/// transcendent) wakes its Filter.
#[allow(clippy::type_complexity)]
pub fn trigger_filter_events_system(
    query: Query<
        (Entity, &CivilizationTechLevel),
        (
            Changed<CivilizationTechLevel>,
            Without<ActiveFilter>,
            Without<FallenCivilization>,
            Without<Transcendent>,
        ),
    >,
    mut events: EventWriter<FilterEvent>,
) {
    for (civ, tech) in query.iter() {
        if gate_passes(tech.level, civ.index()) {
            events.send(FilterEvent {
                civ,
                filter_type: FilterKind::for_civ(civ.index(), tech.level),
            });
        }
    }
}

/// Turns [`FilterEvent`]s into live trials: stamps [`ActiveFilter`]
/// with the civ's resilience as its containment, and counts the ledger.
pub fn activate_filters_system(
    mut events: EventReader<FilterEvent>,
    mut commands: Commands,
    query: Query<&CivilizationResilience>,
    mut ledger: Option<ResMut<GreatFilterLedger>>,
) {
    for event in events.read() {
        let containment = query
            .get(event.civ)
            .map_or_else(|_| CivilizationResilience::default().containment, |r| r.containment);
        commands.entity(event.civ).insert(ActiveFilter {
            kind: event.filter_type,
            containment,
            ..Default::default()
        });
        if let Some(ledger) = ledger.as_deref_mut() {
            ledger.triggered += 1;
        }
    }
}

/// Advances live trials one tick: pressure accrues implicitly through
/// `elapsed`; when the window closes, contained trials resolve to
/// survival (component swap + [`FilterSurvivedEvent`]) and the rest
/// are flagged `failed` for `process_failed_filters_system`.
pub fn tick_active_filters_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut ActiveFilter), Without<FallenCivilization>>,
    mut survived: EventWriter<FilterSurvivedEvent>,
    mut ledger: Option<ResMut<GreatFilterLedger>>,
) {
    for (civ, mut active) in query.iter_mut() {
        if active.failed {
            continue;
        }
        active.elapsed = active.elapsed.saturating_add(1);
        if active.elapsed >= active.duration {
            if active.containment >= active.final_pressure() {
                let kind = active.kind;
                commands.entity(civ).remove::<ActiveFilter>();
                commands.entity(civ).insert(Transcendent { kind });
                survived.send(FilterSurvivedEvent { civ, kind });
                if let Some(ledger) = ledger.as_deref_mut() {
                    ledger.survived += 1;
                }
            } else {
                active.failed = true;
            }
        }
    }
}

/// Any civilization whose [`ActiveFilter`] is flagged failed falls:
/// fires [`CivilizationWipeEvent`], stamps [`FallenCivilization`], and
/// clears the trial so it cannot fire twice.
pub fn process_failed_filters_system(
    mut commands: Commands,
    query: Query<(Entity, &ActiveFilter), Without<FallenCivilization>>,
    mut events: EventWriter<CivilizationWipeEvent>,
    mut ledger: Option<ResMut<GreatFilterLedger>>,
) {
    for (civ, filter) in query.iter() {
        if filter.failed {
            commands.entity(civ).remove::<ActiveFilter>();
            commands.entity(civ).insert(FallenCivilization);
            commands.entity(civ).insert(FilterDeathKind(filter.kind));
            events.send(CivilizationWipeEvent { civ });
            if let Some(ledger) = ledger.as_deref_mut() {
                ledger.fallen += 1;
            }
        }
    }
}

/// Records the kind that felled a civilization (stamped by
/// `process_failed_filters_system` alongside [`FallenCivilization`],
/// read by the wipe system when it raises ruins).
#[derive(Component, Debug, Clone, Copy)]
pub struct FilterDeathKind(pub FilterKind);

/// Applies wipe events: every [`CivWorld`] owned by the fallen civ
/// becomes an [`ExplorableRuin`].
pub fn apply_civilization_wipe_system(
    mut events: EventReader<CivilizationWipeEvent>,
    mut commands: Commands,
    worlds: Query<(Entity, &CivWorld)>,
    deaths: Query<&FilterDeathKind>,
    mut ledger: Option<ResMut<GreatFilterLedger>>,
) {
    for event in events.read() {
        // Falls recorded outside a trial default to the Grey Bloom,
        // the canonical graveyard-maker.
        let kind = deaths.get(event.civ).map_or(FilterKind::GreyBloom, |d| d.0);
        for (entity, world) in worlds.iter() {
            if world.owner == event.civ {
                commands.entity(entity).insert(ExplorableRuin {
                    origin_civ: event.civ,
                    kind,
                    name: world.name.clone(),
                });
                if let Some(ledger) = ledger.as_deref_mut() {
                    ledger.ruins_created += 1;
                }
            }
        }
    }
}

/// Chronicle bridge: trigger, survival, and fall are all history.
pub fn filter_chronicle_bridge_system(
    mut triggered: EventReader<FilterEvent>,
    mut survived: EventReader<FilterSurvivedEvent>,
    mut wiped: EventReader<CivilizationWipeEvent>,
    civs: Query<&Civilization>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let name = |civ: Entity| -> String {
        civs.get(civ)
            .map_or_else(|_| format!("Civilization {}", civ.index()), |c| c.id.clone())
    };
    for event in triggered.read() {
        chronicle.send(AddChronicleEvent {
            text: format!(
                "THE GREAT FILTER: {} faces {} — {}",
                name(event.civ),
                event.filter_type.label(),
                event.filter_type.description()
            ),
            importance: EventImportance::Legendary,
        });
    }
    for event in survived.read() {
        chronicle.send(AddChronicleEvent {
            text: format!(
                "THE GREAT FILTER SURVIVED: {} has contained {} and emerged changed — transcendent.",
                name(event.civ),
                event.kind.label()
            ),
            importance: EventImportance::Legendary,
        });
    }
    for event in wiped.read() {
        chronicle.send(AddChronicleEvent {
            text: format!(
                "THE GREAT FILTER CLAIMS ANOTHER: {} has fallen. Its worlds go dark, left as ruins for whoever comes next.",
                name(event.civ)
            ),
            importance: EventImportance::Legendary,
        });
    }
}

/// The transcendent benefit: when the *local* civilization survives,
/// its hard-won understanding pays into the real colony pool as
/// knowledge. Remote civs' survivals never touch the local pool.
pub fn filter_survival_benefit_system(
    mut events: EventReader<FilterSurvivedEvent>,
    local: Query<(), With<LocalCivilization>>,
    mut resources: Option<ResMut<ColonyResources>>,
) {
    for event in events.read() {
        if local.get(event.civ).is_ok() {
            if let Some(res) = resources.as_deref_mut() {
                res.add_knowledge(TRANSCENDENCE_KNOWLEDGE);
            }
        }
    }
}

/// Census for headless/STATS: (triggered, survived, fallen, active, ruins).
pub fn great_filter_census(world: &mut World) -> (u32, u32, u32, u32, u32) {
    let ledger = world
        .get_resource::<GreatFilterLedger>()
        .copied()
        .unwrap_or_default();
    let active = world.query::<&ActiveFilter>().iter(world).count() as u32;
    let ruins = world.query::<&ExplorableRuin>().iter(world).count() as u32;
    (ledger.triggered, ledger.survived, ledger.fallen, active, ruins)
}

/// Compact STATS label: `none`, `active:<n>`, `fallen:<n>`, or
/// `transcendent:<n>` in that priority order.
pub fn filter_stats_label(world: &mut World) -> String {
    let (triggered, survived, fallen, active, _ruins) = great_filter_census(world);
    if active > 0 {
        format!("active:{active}")
    } else if fallen > 0 {
        format!("fallen:{fallen}")
    } else if survived > 0 {
        format!("transcendent:{survived}")
    } else if triggered > 0 {
        format!("triggered:{triggered}")
    } else {
        "none".to_string()
    }
}

