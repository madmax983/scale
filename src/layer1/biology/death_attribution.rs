//! # Death attribution (Item 84 — "Causes unknown" legibility)
//!
//! ## The problem
//!
//! [`DeathCause`] is the only channel through which a death explains
//! itself: [`handle_pop_death_system`](crate::layer1::entities::pop::handle_pop_death_system)
//! reads it and falls back to `"Causes unknown"`, which the chronicle
//! bridge then prints verbatim (`"X has died. Cause: Causes unknown"`).
//! Before this module, exactly two systems ever set it —
//! `natural_death_system` ("Old Age") and `pressure_damage_system`
//! ("Atmospheric breach", only on its own killing tick). Every other
//! damage path in the game — starvation, hypothermia/heatstroke,
//! radiation sickness, rust lung, toxic smog, fire, explosions,
//! hazardous terrain, combat, wildlife, food poisoning, vermin fever —
//! reduced [`Health`] to zero in silence, so the modal chronicle entry
//! in long runs (notably Layer 2 Ready wipes, with LifeSupport intact)
//! was "Causes unknown". That is a fairness bug: the player cannot
//! learn what killed the colony.
//!
//! ## The design
//!
//! Three layers, in priority order:
//!
//! 1. **Explicit `DeathCause`** — a system that *knows* it killed the
//!    pop (old age, the pressure system's killing tick, scripted
//!    kills) inserts it directly. Always wins; never overwritten here.
//! 2. **[`LastDamageSource`]** — damage systems call
//!    [`record_damage`] / [`note_damage_world`] whenever they hurt a
//!    pop, naming the source. The *last* recorded source is the
//!    proximate cause: per-tick damage systems re-record every tick,
//!    so whichever system was actively killing the pop at the end is
//!    the one named.
//! 3. **State inference** — [`attribute_death_causes_system`] runs
//!    after `check_health_status_system` and before
//!    `handle_pop_death_system`. For any dead pop still lacking a
//!    `DeathCause`, it resolves one from the corpse's observable
//!    state (empty stomach, radiation sickness, rust lung, local
//!    pressure/temperature, [`SlainByFaction`], elder status) via the
//!    pure [`resolve_death_cause`] function. If nothing at all is
//!    diagnosable, the cause is `"Wounds"` — generic trauma for a pop
//!    whose health was zeroed by an unrecorded script — never the
//!    unattributed canary.
//!
//! `"Causes unknown"` ([`CAUSE_UNKNOWN`]) survives only as a canary:
//! it can still appear if a death bypasses this pipeline entirely
//! (e.g. a bare test world running `handle_pop_death_system` alone),
//! and [`DeathLedger`] counts it separately so any leak stays
//! measurable in the headless `deaths` report instead of silently
//! becoming the modal death again.
//!
//! [`DeathLedger`] is the colony's book of the dead: totals by cause
//! plus the most recent deaths, fed by a `PopDied` event reader. It
//! backs the headless `deaths` command and the `deaths=` /
//! `unknown_deaths=` STATS fields.
//!
//! All cause strings are original to Scale — no borrowed names.

use bevy_ecs::prelude::*;
use std::collections::{BTreeMap, VecDeque};

use crate::layer1::biology::health::{Dead, DeathCause, Health};
use crate::layer1::entities::pop::{Pop, PopDied};
use crate::layer1::lifecycle::{Age, LifeStage};
use crate::layer1::map::GridPosition;
use crate::layer1::needs::Needs;
use crate::layer1::physics::pressure::{PressureGrid, SUFFOCATION_PRESSURE_THRESHOLD};
use crate::layer1::radioactive::RadiationSickness;
use crate::layer1::social::martyrdom::{faction_name, SlainByFaction};
use crate::layer1::temperature::TemperatureGrid;
use crate::shared::time::SimulationTime;

/// Died of old age (matches the string `natural_death_system` has
/// always written, and martyrdom's `NATURAL_DEATH_REASON`).
pub const CAUSE_OLD_AGE: &str = "Old Age";
/// Hunger reached zero and starvation damage finished the pop.
pub const CAUSE_STARVATION: &str = "Starvation";
/// Suffocated in vacuum / a depressurized tile. Matches the string
/// `pressure_damage_system` writes on its killing tick.
pub const CAUSE_ATMOSPHERIC_BREACH: &str = "Atmospheric breach";
/// Froze: sustained exposure below the cold tolerance.
pub const CAUSE_HYPOTHERMIA: &str = "Hypothermia";
/// Overheated: sustained exposure above the heat tolerance.
pub const CAUSE_HEATSTROKE: &str = "Heatstroke";
/// Radiation sickness severity passed the lethal band.
pub const CAUSE_RADIATION_SICKNESS: &str = "Radiation sickness";
/// Rust lung degradation (also the toxic-exposure end state).
pub const CAUSE_RUST_LUNG: &str = "Rust lung";
/// Stood in dense smog without a mask.
pub const CAUSE_TOXIC_SMOG: &str = "Toxic smog";
/// Direct toxic-gas damage (the non-rust-lung path of
/// `apply_toxic_gas_damage`).
pub const CAUSE_TOXIC_GAS: &str = "Toxic gas exposure";
/// Burned by a fire tile.
pub const CAUSE_FIRE: &str = "Fire";
/// Killed by an explosion (volatile stores, sabotage, deorbit debris).
pub const CAUSE_EXPLOSION: &str = "Explosion";
/// Stood on a magma geome tile.
pub const CAUSE_MAGMA: &str = "Magma exposure";
/// Stood on a spore-bloom geome tile.
pub const CAUSE_SPORE_BLOOM: &str = "Spore bloom exposure";
/// Other hazardous-terrain (geome) damage.
pub const CAUSE_HAZARDOUS_TERRAIN: &str = "Hazardous terrain";
/// Killed through the combat path (`execute_attack`).
pub const CAUSE_COMBAT: &str = "Combat wounds";
/// Killed by a fauna attack.
pub const CAUSE_WILDLIFE: &str = "Wildlife attack";
/// Ate a poisoned meal.
pub const CAUSE_FOOD_POISONING: &str = "Food poisoning";
/// Vermin fever (symbiotic pest disease) damage.
pub const CAUSE_VERMIN_FEVER: &str = "Vermin fever";
/// Generic trauma fallback: health was zeroed by an unrecorded source
/// and no state on the corpse identifies it. Deliberately distinct
/// from [`CAUSE_UNKNOWN`] so the canary stays meaningful.
pub const CAUSE_WOUNDS: &str = "Wounds";
/// The canary: a death reached `handle_pop_death_system` with no
/// attribution at all. Should not occur in the full simulation
/// schedule; [`DeathLedger::unknown_count`] keeps it visible.
pub const CAUSE_UNKNOWN: &str = "Causes unknown";

/// Every canonical cause this module can produce (checked by the
/// IP-guard test; `resolve_death_cause` may additionally format
/// `"Slain by …"` from `faction_name`).
pub const ALL_CAUSES: &[&str] = &[
    CAUSE_OLD_AGE,
    CAUSE_STARVATION,
    CAUSE_ATMOSPHERIC_BREACH,
    CAUSE_HYPOTHERMIA,
    CAUSE_HEATSTROKE,
    CAUSE_RADIATION_SICKNESS,
    CAUSE_RUST_LUNG,
    CAUSE_TOXIC_SMOG,
    CAUSE_TOXIC_GAS,
    CAUSE_FIRE,
    CAUSE_EXPLOSION,
    CAUSE_MAGMA,
    CAUSE_SPORE_BLOOM,
    CAUSE_HAZARDOUS_TERRAIN,
    CAUSE_COMBAT,
    CAUSE_WILDLIFE,
    CAUSE_FOOD_POISONING,
    CAUSE_VERMIN_FEVER,
    CAUSE_WOUNDS,
    CAUSE_UNKNOWN,
];

/// The most recent damage a pop took, recorded by the damage system
/// itself at the moment it applied the damage. The killing system's
/// record is the one still standing at death time because per-tick
/// damage systems re-record on every tick they hurt the pop.
#[derive(Component, Debug, Clone)]
pub struct LastDamageSource {
    /// Canonical cause label (see the `CAUSE_*` constants).
    pub cause: String,
    /// Tick on which the damage was recorded.
    pub tick: u64,
}

/// Record `cause` as `entity`'s latest damage source (query/command
/// style systems). Uses `try_insert`: the entity may already be
/// despawned by an earlier system in the same tick batch.
pub fn record_damage(commands: &mut Commands, entity: Entity, cause: &str, tick: u64) {
    commands.entity(entity).try_insert(LastDamageSource {
        cause: cause.to_string(),
        tick,
    });
}

/// Record `cause` as `entity`'s latest damage source (world-style
/// exclusive systems, which hold `&mut World` instead of `Commands`).
pub fn note_damage_world(world: &mut World, entity: Entity, cause: &str) {
    let tick = world.get_resource::<SimulationTime>().map_or(0, |t| t.tick);
    if world.get_entity(entity).is_ok() {
        world.entity_mut(entity).insert(LastDamageSource {
            cause: cause.to_string(),
            tick,
        });
    }
}

/// Observable state of a corpse at attribution time. Pure data so
/// [`resolve_death_cause`] can be unit-tested without a world.
#[derive(Debug, Clone, Default)]
pub struct DeathSnapshot {
    /// Killer faction, if a [`SlainByFaction`] marker is present.
    pub slain_by_faction: Option<u32>,
    /// Last recorded damage source, if any system recorded one.
    pub last_damage_cause: Option<String>,
    /// Hunger at death (`Needs.hunger`); `0.0` means starving.
    pub hunger: Option<f32>,
    /// Radiation sickness severity, if the component is present.
    pub radiation_severity: Option<f32>,
    /// Whether the pop had rust lung.
    pub has_rust_lung: bool,
    /// Air pressure at the death tile, if a pressure grid exists.
    pub pressure: Option<f32>,
    /// Temperature at the death tile, if a temperature grid exists.
    pub temperature: Option<f32>,
    /// Whether the pop was an Elder.
    pub is_elder: bool,
}

/// Resolve a death cause from a corpse snapshot. Priority:
/// faction kill > recorded damage source > starvation > radiation >
/// rust lung > suffocation > hypothermia/heatstroke > old age >
/// generic wounds. Never returns [`CAUSE_UNKNOWN`]: a real damage
/// source or diagnosable state always names *something* specific,
/// and the bare fallback is trauma, not ignorance.
#[must_use]
pub fn resolve_death_cause(snap: &DeathSnapshot) -> String {
    if let Some(faction_id) = snap.slain_by_faction {
        return format!("Slain by {}", faction_name(faction_id));
    }
    if let Some(cause) = &snap.last_damage_cause {
        return cause.clone();
    }
    if snap.hunger.is_some_and(|h| h <= 0.0) {
        return CAUSE_STARVATION.to_string();
    }
    if snap.radiation_severity.is_some_and(|s| s > 50.0) {
        return CAUSE_RADIATION_SICKNESS.to_string();
    }
    if snap.has_rust_lung {
        return CAUSE_RUST_LUNG.to_string();
    }
    if snap
        .pressure
        .is_some_and(|p| p < SUFFOCATION_PRESSURE_THRESHOLD)
    {
        return CAUSE_ATMOSPHERIC_BREACH.to_string();
    }
    if let Some(temp) = snap.temperature {
        // Mirror thermal_damage_system's base tolerances (before
        // clothing insulation): below 10C is hypothermia country,
        // above 35C is heatstroke country.
        if temp < 10.0 {
            return CAUSE_HYPOTHERMIA.to_string();
        }
        if temp > 35.0 {
            return CAUSE_HEATSTROKE.to_string();
        }
    }
    if snap.is_elder {
        return CAUSE_OLD_AGE.to_string();
    }
    CAUSE_WOUNDS.to_string()
}

/// System: give every freshly-dead pop a [`DeathCause`] before
/// `handle_pop_death_system` reads it. Runs after
/// `check_health_status_system` (which applies the `Dead` marker)
/// and before the death handler in the Consumption set. Pops that
/// already carry an explicit `DeathCause` are never overwritten.
#[allow(clippy::type_complexity)]
pub fn attribute_death_causes_system(
    mut commands: Commands,
    query: Query<
        (
            Entity,
            Option<&LastDamageSource>,
            Option<&Needs>,
            Option<&Health>,
            Option<&Age>,
            Option<&GridPosition>,
            Option<&RadiationSickness>,
            Option<&SlainByFaction>,
        ),
        (With<Pop>, With<Dead>, Without<DeathCause>),
    >,
    temperature_grid: Option<Res<TemperatureGrid>>,
    pressure_grid: Option<Res<PressureGrid>>,
) {
    for (entity, last, needs, health, age, pos, sickness, slain) in &query {
        let snap = DeathSnapshot {
            slain_by_faction: slain.map(|s| s.faction_id),
            last_damage_cause: last.map(|l| l.cause.clone()),
            hunger: needs.map(|n| n.hunger),
            radiation_severity: sickness.map(|s| s.severity),
            has_rust_lung: health.is_some_and(|h| h.has_rust_lung),
            pressure: match (pos, &pressure_grid) {
                (Some(p), Some(grid)) => Some(grid.get(p.x, p.y)),
                _ => None,
            },
            temperature: match (pos, &temperature_grid) {
                (Some(p), Some(grid)) => Some(grid.get_safe(p.x, p.y)),
                _ => None,
            },
            is_elder: age.is_some_and(|a| a.stage == LifeStage::Elder),
        };
        let cause = resolve_death_cause(&snap);
        commands.entity(entity).insert(DeathCause(cause));
    }
}

/// One entry in the colony's book of the dead.
#[derive(Debug, Clone)]
pub struct DeathRecord {
    /// Tick of death.
    pub tick: u64,
    /// Name of the deceased.
    pub name: String,
    /// Attributed cause of death.
    pub cause: String,
}

/// Colony-wide death ledger: totals by cause plus the most recent
/// deaths. Fed by [`death_ledger_system`] from `PopDied` events;
/// read by the headless `deaths` command and STATS.
#[derive(Resource, Default, Debug, Clone)]
pub struct DeathLedger {
    /// Total deaths recorded.
    pub total: u64,
    /// Deaths per cause label.
    pub by_cause: BTreeMap<String, u64>,
    /// Most recent deaths, oldest first, capped at [`Self::RECENT_CAP`].
    pub recent: VecDeque<DeathRecord>,
}

impl DeathLedger {
    /// Maximum number of recent deaths retained.
    pub const RECENT_CAP: usize = 32;

    /// Record one death.
    pub fn record(&mut self, tick: u64, name: &str, cause: &str) {
        self.total += 1;
        *self.by_cause.entry(cause.to_string()).or_insert(0) += 1;
        self.recent.push_back(DeathRecord {
            tick,
            name: name.to_string(),
            cause: cause.to_string(),
        });
        while self.recent.len() > Self::RECENT_CAP {
            self.recent.pop_front();
        }
    }

    /// Deaths that reached the ledger with no attribution (canary).
    #[must_use]
    pub fn unknown_count(&self) -> u64 {
        self.by_cause.get(CAUSE_UNKNOWN).copied().unwrap_or(0)
    }

    /// One-line breakdown, causes sorted by count (desc) then name:
    /// `"Starvation: 3, Explosion: 1"`. Empty string when no deaths.
    #[must_use]
    pub fn breakdown(&self) -> String {
        let mut pairs: Vec<(&String, &u64)> = self.by_cause.iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        pairs
            .iter()
            .map(|(cause, count)| format!("{cause}: {count}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// System: fold every `PopDied` event into the [`DeathLedger`].
pub fn death_ledger_system(mut events: EventReader<PopDied>, ledger: Option<ResMut<DeathLedger>>) {
    let Some(mut ledger) = ledger else { return };
    for event in events.read() {
        ledger.record(event.tick, &event.name, &event.reason);
    }
}

/// Multi-line death report for the headless `deaths` command.
#[must_use]
pub fn format_deaths_report(world: &World) -> String {
    let Some(ledger) = world.get_resource::<DeathLedger>() else {
        return "DEATHS total=0 (no death ledger in this world)".to_string();
    };
    let mut out = format!(
        "DEATHS total={} unknown={}",
        ledger.total,
        ledger.unknown_count()
    );
    let breakdown = ledger.breakdown();
    if !breakdown.is_empty() {
        out.push_str(&format!("\nBy cause: {breakdown}"));
    }
    for record in &ledger.recent {
        out.push_str(&format!(
            "\n  tick {} — {} — {}",
            record.tick, record.name, record.cause
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::biology::health::check_health_status_system;
    use crate::layer1::entities::pop::handle_pop_death_system;
    use bevy_ecs::event::Events;

    // ---- pure resolver ----

    #[test]
    fn resolver_prefers_slain_by_over_recorded_damage() {
        let snap = DeathSnapshot {
            slain_by_faction: Some(13),
            last_damage_cause: Some(CAUSE_EXPLOSION.to_string()),
            hunger: Some(0.0),
            ..Default::default()
        };
        assert_eq!(resolve_death_cause(&snap), "Slain by the pirate raiders");
    }

    #[test]
    fn resolver_prefers_recorded_damage_over_state() {
        // Starving AND finished off by an explosion: the explosion is
        // the proximate cause, and it was recorded last.
        let snap = DeathSnapshot {
            last_damage_cause: Some(CAUSE_EXPLOSION.to_string()),
            hunger: Some(0.0),
            ..Default::default()
        };
        assert_eq!(resolve_death_cause(&snap), CAUSE_EXPLOSION);
    }

    #[test]
    fn resolver_starvation_from_empty_stomach() {
        let snap = DeathSnapshot {
            hunger: Some(0.0),
            ..Default::default()
        };
        assert_eq!(resolve_death_cause(&snap), CAUSE_STARVATION);
    }

    #[test]
    fn resolver_radiation_from_severe_sickness() {
        let snap = DeathSnapshot {
            hunger: Some(0.8),
            radiation_severity: Some(95.0),
            ..Default::default()
        };
        assert_eq!(resolve_death_cause(&snap), CAUSE_RADIATION_SICKNESS);
    }

    #[test]
    fn resolver_rust_lung_flag() {
        let snap = DeathSnapshot {
            hunger: Some(0.8),
            has_rust_lung: true,
            ..Default::default()
        };
        assert_eq!(resolve_death_cause(&snap), CAUSE_RUST_LUNG);
    }

    #[test]
    fn resolver_suffocation_from_vacuum_tile() {
        let snap = DeathSnapshot {
            hunger: Some(0.8),
            pressure: Some(0.0),
            temperature: Some(20.0),
            ..Default::default()
        };
        assert_eq!(resolve_death_cause(&snap), CAUSE_ATMOSPHERIC_BREACH);
    }

    #[test]
    fn resolver_hypothermia_and_heatstroke_from_tile_temperature() {
        let cold = DeathSnapshot {
            hunger: Some(0.8),
            pressure: Some(1.0),
            temperature: Some(-15.0),
            ..Default::default()
        };
        assert_eq!(resolve_death_cause(&cold), CAUSE_HYPOTHERMIA);
        let hot = DeathSnapshot {
            hunger: Some(0.8),
            pressure: Some(1.0),
            temperature: Some(44.0),
            ..Default::default()
        };
        assert_eq!(resolve_death_cause(&hot), CAUSE_HEATSTROKE);
    }

    #[test]
    fn resolver_elder_falls_back_to_old_age() {
        let snap = DeathSnapshot {
            hunger: Some(0.8),
            pressure: Some(1.0),
            temperature: Some(20.0),
            is_elder: true,
            ..Default::default()
        };
        assert_eq!(resolve_death_cause(&snap), CAUSE_OLD_AGE);
    }

    #[test]
    fn resolver_undiagnosable_death_is_wounds_never_unknown() {
        let snap = DeathSnapshot {
            hunger: Some(0.8),
            pressure: Some(1.0),
            temperature: Some(20.0),
            ..Default::default()
        };
        let cause = resolve_death_cause(&snap);
        assert_eq!(cause, CAUSE_WOUNDS);
        assert_ne!(cause, CAUSE_UNKNOWN);
    }

    #[test]
    fn ip_guard_causes_are_original_names() {
        // Concept-only rule: cause labels must not lift named IP.
        const BANNED: &[&str] = &[
            "billy",
            "windsor",
            "sad king",
            "skywalker",
            "vader",
            "winterfell",
            "gondor",
            "hogwarts",
            "area 51",
            "shimmer",
            "southern reach",
        ];
        for cause in ALL_CAUSES {
            let lower = cause.to_lowercase();
            for banned in BANNED {
                assert!(
                    !lower.contains(banned),
                    "cause label {cause:?} contains banned term {banned:?}"
                );
            }
        }
        // The slain-by format uses martyrdom's original faction names.
        let slain = resolve_death_cause(&DeathSnapshot {
            slain_by_faction: Some(13),
            ..Default::default()
        });
        for banned in BANNED {
            assert!(!slain.to_lowercase().contains(banned));
        }
    }

    // ---- full death pipeline (damage -> check -> attribute -> handle) ----

    fn pipeline_world() -> World {
        let mut world = World::new();
        world.init_resource::<Events<PopDied>>();
        world.insert_resource(SimulationTime::default());
        world
    }

    fn death_pipeline() -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                check_health_status_system,
                attribute_death_causes_system,
                handle_pop_death_system,
            )
                .chain(),
        );
        schedule
    }

    fn pop_died_reasons(world: &mut World) -> Vec<String> {
        let events = world.resource::<Events<PopDied>>();
        let mut cursor = events.get_cursor();
        cursor.read(events).map(|e| e.reason.clone()).collect()
    }

    fn low_health_pop(world: &mut World, current: f32) -> Entity {
        world
            .spawn((
                Pop,
                crate::layer1::entities::pop::PopName("Testling".to_string()),
                GridPosition { x: 5, y: 5 },
                Health {
                    current,
                    max: 100.0,
                    has_rust_lung: false,
                },
                Needs::default(),
            ))
            .id()
    }

    #[test]
    fn starvation_death_is_attributed_end_to_end() {
        let mut world = pipeline_world();
        let pop = world
            .spawn((
                Pop,
                crate::layer1::entities::pop::PopName("Hungry".to_string()),
                GridPosition { x: 5, y: 5 },
                Health {
                    current: 0.1,
                    max: 100.0,
                    has_rust_lung: false,
                },
                Needs {
                    hunger: 0.0,
                    ..Default::default()
                },
            ))
            .id();

        // Exclusive (world-style) system: call it directly, exactly as
        // the real schedule does each tick, then run the death chain.
        crate::layer1::needs::starvation_damage_system(&mut world);
        let mut schedule = death_pipeline();
        schedule.run(&mut world);

        assert_eq!(
            world.get::<DeathCause>(pop).map(|c| c.0.as_str()),
            Some(CAUSE_STARVATION)
        );
        assert_eq!(pop_died_reasons(&mut world), vec![CAUSE_STARVATION]);
    }

    #[test]
    fn hypothermia_death_is_attributed_end_to_end() {
        let mut world = pipeline_world();
        world.init_resource::<Events<crate::layer1::temperature::ThermalDamageEvent>>();
        world.insert_resource(crate::layer1::temperature::TemperatureGrid::new(
            10, 10, -20.0,
        ));
        let pop = world
            .spawn((
                Pop,
                crate::layer1::entities::pop::PopName("Frosty".to_string()),
                GridPosition { x: 5, y: 5 },
                Health {
                    current: 0.4,
                    max: 100.0,
                    has_rust_lung: false,
                },
                Needs::default(),
                crate::layer1::items::Equipment::default(),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                crate::layer1::temperature::thermal_damage_system,
                check_health_status_system,
                attribute_death_causes_system,
                handle_pop_death_system,
            )
                .chain(),
        );
        schedule.run(&mut world);

        assert_eq!(
            world.get::<DeathCause>(pop).map(|c| c.0.as_str()),
            Some(CAUSE_HYPOTHERMIA)
        );
        assert_eq!(pop_died_reasons(&mut world), vec![CAUSE_HYPOTHERMIA]);
    }

    #[test]
    fn explosion_death_is_attributed_end_to_end() {
        let mut world = pipeline_world();
        world.init_resource::<Events<crate::layer1::environment::volatile::ExplosionEvent>>();
        let pop = low_health_pop(&mut world, 30.0);
        world
            .resource_mut::<Events<crate::layer1::environment::volatile::ExplosionEvent>>()
            .send(crate::layer1::environment::volatile::ExplosionEvent {
                center: GridPosition { x: 5, y: 5 },
                damage: 50.0,
                radius: 2,
            });

        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                crate::layer1::environment::volatile::handle_explosion_system,
                check_health_status_system,
                attribute_death_causes_system,
                handle_pop_death_system,
            )
                .chain(),
        );
        schedule.run(&mut world);

        let _ = pop;
        assert_eq!(pop_died_reasons(&mut world), vec![CAUSE_EXPLOSION]);
    }

    #[test]
    fn vacuum_death_is_inferred_without_recorded_damage() {
        // Health zeroed by an unrecorded script while standing in
        // vacuum: inference from the pressure grid must still name
        // the breach — this is the Layer 2 Ready wipe signature.
        let mut world = pipeline_world();
        let mut grid = PressureGrid::new(10, 10);
        grid.set(5, 5, 0.0);
        world.insert_resource(grid);
        let pop = low_health_pop(&mut world, 100.0);
        world.get_mut::<Health>(pop).unwrap().current = 0.0;

        let mut schedule = death_pipeline();
        schedule.run(&mut world);

        assert_eq!(
            world.get::<DeathCause>(pop).map(|c| c.0.as_str()),
            Some(CAUSE_ATMOSPHERIC_BREACH)
        );
        assert_eq!(pop_died_reasons(&mut world), vec![CAUSE_ATMOSPHERIC_BREACH]);
    }

    #[test]
    fn pressure_system_kill_keeps_atmospheric_breach() {
        let mut world = pipeline_world();
        world.init_resource::<Events<crate::layer1::chronicle::AddChronicleEvent>>();
        let mut grid = PressureGrid::new(10, 10);
        grid.set(5, 5, 0.0);
        world.insert_resource(grid);
        let _pop = low_health_pop(&mut world, 0.5);

        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                crate::layer1::physics::pressure::pressure_damage_system,
                check_health_status_system,
                attribute_death_causes_system,
                handle_pop_death_system,
            )
                .chain(),
        );
        schedule.run(&mut world);

        assert_eq!(pop_died_reasons(&mut world), vec![CAUSE_ATMOSPHERIC_BREACH]);
    }

    #[test]
    fn unrecorded_benign_death_is_wounds_never_unknown() {
        let mut world = pipeline_world();
        let pop = low_health_pop(&mut world, 100.0);
        world.get_mut::<Health>(pop).unwrap().current = 0.0;

        let mut schedule = death_pipeline();
        schedule.run(&mut world);

        let reasons = pop_died_reasons(&mut world);
        assert_eq!(reasons, vec![CAUSE_WOUNDS]);
        assert!(!reasons.iter().any(|r| r == CAUSE_UNKNOWN));
    }

    #[test]
    fn explicit_death_cause_is_never_overwritten() {
        let mut world = pipeline_world();
        let pop = world
            .spawn((
                Pop,
                crate::layer1::entities::pop::PopName("Elder".to_string()),
                GridPosition { x: 5, y: 5 },
                Health {
                    current: 0.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                Needs {
                    hunger: 0.0,
                    ..Default::default()
                },
                DeathCause(CAUSE_OLD_AGE.to_string()),
            ))
            .id();
        let _ = pop;

        let mut schedule = death_pipeline();
        schedule.run(&mut world);

        assert_eq!(pop_died_reasons(&mut world), vec![CAUSE_OLD_AGE]);
    }

    #[test]
    fn slain_by_faction_is_attributed_end_to_end() {
        let mut world = pipeline_world();
        let pop = low_health_pop(&mut world, 100.0);
        world
            .entity_mut(pop)
            .insert(SlainByFaction { faction_id: 13 });
        world.get_mut::<Health>(pop).unwrap().current = 0.0;

        let mut schedule = death_pipeline();
        schedule.run(&mut world);

        assert_eq!(
            pop_died_reasons(&mut world),
            vec!["Slain by the pirate raiders"]
        );
    }

    #[test]
    fn ledger_counts_deaths_by_cause() {
        let mut world = World::new();
        world.init_resource::<Events<PopDied>>();
        world.init_resource::<DeathLedger>();
        let mut schedule = Schedule::default();
        schedule.add_systems(death_ledger_system);

        {
            let mut events = world.resource_mut::<Events<PopDied>>();
            for (name, reason) in [
                ("Ada", CAUSE_STARVATION),
                ("Bryn", CAUSE_STARVATION),
                ("Cole", CAUSE_EXPLOSION),
                ("Dara", CAUSE_UNKNOWN),
            ] {
                events.send(PopDied {
                    entity: Entity::PLACEHOLDER,
                    name: name.to_string(),
                    tick: 42,
                    reason: reason.to_string(),
                });
            }
        }
        schedule.run(&mut world);

        let ledger = world.resource::<DeathLedger>();
        assert_eq!(ledger.total, 4);
        assert_eq!(ledger.by_cause.get(CAUSE_STARVATION), Some(&2));
        assert_eq!(ledger.by_cause.get(CAUSE_EXPLOSION), Some(&1));
        assert_eq!(ledger.unknown_count(), 1);
        assert_eq!(ledger.recent.len(), 4);
        assert!(ledger.breakdown().contains("Starvation: 2"));
    }
}
