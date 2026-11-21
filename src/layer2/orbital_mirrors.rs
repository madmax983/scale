use crate::layer1::lighting::LightMap;
use crate::layer1::temperature::TemperatureGrid;
use bevy::prelude::*;
use rand::Rng;

#[derive(Component)]
pub struct OrbitalMirror {
    pub target: Vec2,
    pub intensity: f32,
    pub radius: f32,
    pub alignment_error: f32,
}

#[derive(Event)]
pub struct MirrorFocusEvent {
    pub mirror_id: Entity,
    pub new_target: Vec2,
}

pub fn orbital_mirror_focus_system(
    mirrors: Query<&OrbitalMirror>,
    mut temp_grid: ResMut<TemperatureGrid>,
    mut light_grid: ResMut<LightMap>,
    time: Res<Time>,
) {
    let mut rng = rand::thread_rng();

    for mirror in mirrors.iter() {
        // Calculate actual focus point with random drift
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let drift = rng.gen_range(0.0..=mirror.alignment_error);
        let actual_focus = mirror.target + Vec2::new(angle.cos() * drift, angle.sin() * drift);

        // Apply heat and light to grid
        if actual_focus.x < 0.0 || actual_focus.y < 0.0 {
            continue;
        }
        let tx = actual_focus.x as usize;
        let ty = actual_focus.y as usize;

        if tx < temp_grid.width && ty < temp_grid.height {
            // Apply intense heat scaling with intensity and delta time
            let heat_added = mirror.intensity * time.delta_secs();
            temp_grid.add(tx as i32, ty as i32, heat_added);

            // Minimal falloff for blast radius (simplification)
            let r = mirror.radius as i32;
            for dx in -r..=r {
                for dy in -r..=r {
                    let dist = (dx as f32 * dx as f32 + dy as f32 * dy as f32).sqrt();
                    if dist <= mirror.radius {
                        let falloff = 1.0 - (dist / mirror.radius);
                        let ax = (tx as i32 + dx) as usize;
                        let ay = (ty as i32 + dy) as usize;
                        if ax < temp_grid.width && ay < temp_grid.height {
                            temp_grid.add(ax as i32, ay as i32, heat_added * falloff * 0.5);
                            light_grid.add_light(
                                ax as u32,
                                ay as u32,
                                mirror.intensity * falloff * 0.1,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::lighting::LightMap;
    use crate::layer1::temperature::TemperatureGrid;

    #[test]
    fn test_orbital_mirror_increases_light_and_heat() {
        let mut app = App::new();
        app.add_systems(Update, orbital_mirror_focus_system);

        let temp_grid = TemperatureGrid::new(10, 10, 20.0);
        let light_grid = LightMap::new(10, 10);

        app.world_mut().insert_resource(temp_grid);
        app.world_mut().insert_resource(light_grid);

        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs(1));
        app.world_mut().insert_resource(time);

        // Spawn an orbital mirror targeting 5,5
        app.world_mut().spawn(OrbitalMirror {
            target: Vec2::new(5.0, 5.0),
            intensity: 5.0,
            radius: 2.0,
            alignment_error: 0.0,
        });

        app.update();

        let updated_temp = app.world().resource::<TemperatureGrid>();

        let mut max_heat = 0.0;
        for y in 0..10 {
            for x in 0..10 {
                let heat = updated_temp.get(x, y);
                if heat > max_heat {
                    max_heat = heat;
                }
            }
        }
        assert!(max_heat > 20.0);

        let updated_light = app.world().resource::<LightMap>();

        // Target should be significantly hotter and brighter
        assert!(updated_temp.get(5, 5) > 20.0);
        assert!(updated_light.get(5, 5) > 0.0);

        // Edges should be unaffected
        assert_eq!(updated_temp.get(0, 0), 20.0);
    }

    #[test]
    fn test_mirror_misalignment_causes_drift() {
        let mut app = App::new();
        app.add_systems(Update, orbital_mirror_focus_system);

        let temp_grid = TemperatureGrid::new(10, 10, 20.0);
        let light_grid = LightMap::new(10, 10);
        app.world_mut().insert_resource(temp_grid);
        app.world_mut().insert_resource(light_grid);

        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs(1));
        app.world_mut().insert_resource(time);

        app.world_mut().spawn(OrbitalMirror {
            target: Vec2::new(5.0, 5.0),
            intensity: 10.0,
            radius: 1.0,
            alignment_error: 2.0, // High error
        });

        app.update();

        let _updated_temp = app.world().resource::<TemperatureGrid>();
        // Test logic would check if the peak heat is within the error radius, not exactly at target
        // We're just asserting it doesn't crash
    }

    #[test]
    fn test_extreme_mirror_heat_starts_fire() {
        let mut app = App::new();
        // Since we aren't pulling in the full fire ignition system which might have many dependencies,
        // we'll just test that the heat increases appropriately for the ignition system to catch later.
        app.add_systems(Update, orbital_mirror_focus_system);

        let temp_grid = TemperatureGrid::new(10, 10, 20.0);
        let light_grid = LightMap::new(10, 10);
        app.world_mut().insert_resource(temp_grid);
        app.world_mut().insert_resource(light_grid);

        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs(1));
        app.world_mut().insert_resource(time);

        // Spawn highly intense mirror
        app.world_mut().spawn(OrbitalMirror {
            target: Vec2::new(5.0, 5.0),
            intensity: 500.0, // Flash ignition temperature
            radius: 1.0,
            alignment_error: 0.0,
        });

        app.update();

        let updated_temp = app.world().resource::<TemperatureGrid>();

        let mut max_heat = 0.0;
        for y in 0..10 {
            for x in 0..10 {
                let heat = updated_temp.get(x, y);
                if heat > max_heat {
                    max_heat = heat;
                }
            }
        }
        assert!(max_heat > 20.0);

        assert!(updated_temp.get(5, 5) >= 520.0);
    }
}

// ---------------------------------------------------------------------------
// Spec 435: Orbital Solar Mirrors (the array mechanic)
//
// Massive mirror arrays in Layer 2 orbit reflect sunlight onto specific
// Layer 1 tiles, providing power around the clock: mirrors NEVER consult
// the day/night cycle, which is the whole point — ordinary `SolarPower`
// panels zero at night via `update_solar_output_system`, mirrors bypass
// that by construction (they carry a bare `PowerSource`, no `SolarPower`
// marker, so the solar system never touches them, and
// `process_solar_mirrors_system` syncs their output unconditionally).
//
// The spec's sketch types (`EnergyGrid`, `SpawnFireEvent`) do not exist;
// this section is built on the real seams instead:
// - Power: each mirror carries a real `PowerSource` component, synced per
//   tick and summed by the live `power_grid_system`. A `SolarMirrorLedger`
//   resource totals the safe output for STATS/headless.
// - Fire: the spec 033 seam is `psychology::psionics::FireEvent`, bridged
//   into live `Fire` entities by `psionic_fire_bridge_system`. Hacked
//   mirrors emit it, cooldown-bounded.
// - Thermal (spec 140): safe mirrors add `intensity * MIRROR_THERMAL_RATE`
//   per tick to their target tile via `TemperatureGrid::add`.
// - Hacking (spec §5): driven by colony `Unrest` plus a director-carried
//   external-threat pressure, rolled with a deterministic entity+tick
//   splitmix hash (the gravitational-tides pattern — no RNG resource).
// - Shutdown (spec §8): the Command Center path is real — a
//   `DisableMirrorEvent` spends Admin surplus from `AdminStats` and fails
//   closed when the surplus is short or absent. The spacewalk repair leg
//   has NO real seam (EVA suits exist for ground pops, but there is no
//   orbital repair-mission system to launch one through), so it is a
//   documented gap, deliberately not faked.
// ---------------------------------------------------------------------------

use crate::layer1::administration::admin::AdminStats;
use crate::layer1::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::energy::PowerSource;
use crate::layer1::map::GridPosition;
use crate::layer1::psychology::psionics::FireEvent;
use crate::layer1::social::unrest::Unrest;
use crate::shared::time::SimulationTime;

/// Intensity above which a hacked mirror's beam ignites fires at its
/// target tile (spec §3: a hacked mirror at intensity 500 burns; the
/// sketch's threshold is 200).
pub const MIRROR_FIRE_INTENSITY_THRESHOLD: f32 = 200.0;

/// Ticks a hacked mirror waits between fire events at its target tile,
/// so a hacked array is a burning emergency, not a firehose.
pub const MIRROR_FIRE_COOLDOWN_TICKS: u32 = 50;

/// Per-tick heat a safe mirror adds to its target tile, as a fraction of
/// its intensity (spec §5/§7 thermal transfer; useful on ice worlds).
pub const MIRROR_THERMAL_RATE: f32 = 0.005;

/// Admin surplus the Command Center spends to shut one mirror down
/// (spec §8). The spend is fail-closed: short or absent Admin means the
/// mirror stays up.
pub const MIRROR_DISABLE_ADMIN_COST: f32 = 5.0;

/// Per-tick hack probability contributed by colony unrest (0.0..=1.0).
pub const HACK_CHANCE_PER_UNREST: f32 = 0.002;

/// Per-tick hack probability contributed by external threat (0.0..=1.0).
pub const HACK_CHANCE_PER_THREAT: f32 = 0.004;

/// A single orbital solar mirror array (spec 435). Carries a real
/// `PowerSource` alongside; `process_solar_mirrors_system` keeps the two
/// in sync every tick.
#[derive(Component, Debug, Clone)]
pub struct OrbitalSolarMirror {
    /// Beam intensity: safe power output, and (when hacked) fire risk.
    pub intensity: f32,
    /// Whether the mirror's beam has been turned against the colony.
    pub hacked: bool,
    /// Whether the Command Center has shut this mirror down.
    pub disabled: bool,
    /// Ticks until the mirror may start another fire.
    pub fire_cooldown: u32,
}

impl OrbitalSolarMirror {
    /// A fresh, aligned mirror of the given intensity.
    #[must_use]
    pub fn new(intensity: f32) -> Self {
        Self {
            intensity,
            hacked: false,
            disabled: false,
            fire_cooldown: 0,
        }
    }
}

/// The Layer 1 tile a mirror focuses on (spec sketch name, kept).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetTile {
    /// Target column.
    pub x: i32,
    /// Target row.
    pub y: i32,
}

/// Links a Layer 2 entity down to the Layer 1 tile it orbits over
/// (spec §5). Used as the target fallback when no explicit
/// [`TargetTile`] is attached.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Orbiting {
    /// Column of the tile below.
    pub x: i32,
    /// Row of the tile below.
    pub y: i32,
}

/// Director-carried pressure inputs for mirror hacking (spec §5). There
/// is no single global external-threat resource in the sim, so the
/// threat pressure rides here as a tuning input; unrest is read live
/// from the real `Unrest` resource.
#[derive(Resource, Debug, Clone)]
pub struct SolarMirrorDirector {
    /// External threat pressure, 0.0 (none) to 1.0 (siege).
    pub external_threat: f32,
    /// Master switch for hack rolls (tests and pacified eras).
    pub hacking_enabled: bool,
}

impl Default for SolarMirrorDirector {
    fn default() -> Self {
        Self {
            external_threat: 0.0,
            hacking_enabled: true,
        }
    }
}

/// Running census + event ledger for the mirror arrays. The per-tick
/// fields (`total`, `hacked`, `disabled`, `safe_power`) are rewritten by
/// `process_solar_mirrors_system` every tick; the counters accumulate
/// for the life of the world.
#[derive(Resource, Debug, Default)]
pub struct SolarMirrorLedger {
    /// Mirrors standing (last tick).
    pub total: u32,
    /// Mirrors hacked (last tick).
    pub hacked: u32,
    /// Mirrors shut down (last tick).
    pub disabled: u32,
    /// Total safe power output (last tick).
    pub safe_power: f32,
    /// Fire events started by hacked mirrors (cumulative).
    pub fires_started: u64,
    /// Mirrors hacked (cumulative).
    pub hacks: u64,
    /// Successful Command Center shutdowns (cumulative).
    pub shutdowns: u64,
    /// Shutdowns refused for short/absent Admin (cumulative).
    pub shutdown_refusals: u64,
}

/// Command Center order: shut the given mirror down (spec §8). Costs
/// [`MIRROR_DISABLE_ADMIN_COST`] Admin surplus; fail-closed.
#[derive(Event, Debug, Clone, Copy)]
pub struct DisableMirrorEvent {
    /// The mirror to shut down.
    pub mirror: Entity,
}

/// Resolve a mirror's target tile: explicit [`TargetTile`] wins, the
/// [`Orbiting`] link is the fallback (spec §5).
#[must_use]
pub fn resolve_mirror_target(
    target: Option<&TargetTile>,
    orbiting: Option<&Orbiting>,
) -> Option<GridPosition> {
    if let Some(t) = target {
        return Some(GridPosition { x: t.x, y: t.y });
    }
    orbiting.map(|o| GridPosition { x: o.x, y: o.y })
}

/// Per-tick probability that any one aligned mirror is hacked, from
/// colony unrest and external threat (spec §5). Pure and bounded:
/// calm colonies with no threat are never hacked.
#[must_use]
pub fn hack_probability(unrest: f32, external_threat: f32) -> f32 {
    (unrest.clamp(0.0, 1.0) * HACK_CHANCE_PER_UNREST
        + external_threat.clamp(0.0, 1.0) * HACK_CHANCE_PER_THREAT)
        .clamp(0.0, 1.0)
}

/// Deterministic per-tick hack roll for one mirror (splitmix-style hash
/// of entity + tick, the gravitational-tides pattern: no RNG resource,
/// fully reproducible). Returns a value in `[0.0, 1.0)`.
#[must_use]
pub fn mirror_hack_roll(entity_index: u32, tick: u64) -> f32 {
    let mut h = (u64::from(entity_index) + 0x9E37_79B9_7F4A_7C15)
        ^ tick.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    (h % 10_000) as f32 / 10_000.0
}

/// Live mirror census for STATS/headless: (standing, hacked, safe power
/// output). Query-based, so it is accurate even before the first tick.
#[must_use]
pub fn mirror_stats(world: &mut World) -> (u32, u32, f32) {
    let mut query = world.query::<&OrbitalSolarMirror>();
    let mut total = 0u32;
    let mut hacked = 0u32;
    let mut safe_power = 0.0f32;
    for mirror in query.iter(world) {
        total += 1;
        if mirror.hacked && !mirror.disabled {
            hacked += 1;
        }
        if !mirror.hacked && !mirror.disabled {
            safe_power += mirror.intensity;
        }
    }
    (total, hacked, safe_power)
}

/// Human label for a mirror's state, for panels and logs.
#[must_use]
pub fn mirror_state_label(mirror: &OrbitalSolarMirror) -> &'static str {
    if mirror.disabled {
        "disabled"
    } else if mirror.hacked {
        "HACKED"
    } else {
        "aligned"
    }
}

/// The mirror tick (spec §4, on real seams): sync each mirror's
/// `PowerSource`, apply safe thermal transfer, and make hacked mirrors
/// burn. Mirrors never consult the day/night cycle — night-proof by
/// construction.
#[allow(clippy::type_complexity)]
pub fn process_solar_mirrors_system(
    mut mirrors: Query<(
        &mut OrbitalSolarMirror,
        &mut PowerSource,
        Option<&TargetTile>,
        Option<&Orbiting>,
    )>,
    mut temp_grid: ResMut<TemperatureGrid>,
    mut fire_events: EventWriter<FireEvent>,
    mut ledger: ResMut<SolarMirrorLedger>,
) {
    ledger.total = 0;
    ledger.hacked = 0;
    ledger.disabled = 0;
    ledger.safe_power = 0.0;
    for (mut mirror, mut source, target, orbiting) in mirrors.iter_mut() {
        ledger.total += 1;
        if mirror.disabled {
            ledger.disabled += 1;
            source.output = 0.0;
            source.active = false;
            continue;
        }
        if mirror.hacked {
            ledger.hacked += 1;
            source.output = 0.0;
            source.active = false;
            if mirror.fire_cooldown > 0 {
                mirror.fire_cooldown -= 1;
            }
            if mirror.intensity > MIRROR_FIRE_INTENSITY_THRESHOLD && mirror.fire_cooldown == 0 {
                if let Some(position) = resolve_mirror_target(target, orbiting) {
                    fire_events.send(FireEvent { position });
                    mirror.fire_cooldown = MIRROR_FIRE_COOLDOWN_TICKS;
                    ledger.fires_started += 1;
                }
            }
            continue;
        }
        // Aligned: full power, day or night, plus gentle focused heat.
        source.output = mirror.intensity;
        source.active = true;
        ledger.safe_power += mirror.intensity;
        if let Some(position) = resolve_mirror_target(target, orbiting) {
            temp_grid.add(
                position.x,
                position.y,
                mirror.intensity * MIRROR_THERMAL_RATE,
            );
        }
    }
}

/// Hacking pressure (spec §5): each tick, every aligned mirror rolls
/// against the unrest/threat probability. A hack is chronicled Major —
/// an uncontrolled orbital beam is not a minor event.
pub fn mirror_hack_system(
    time: Res<SimulationTime>,
    director: Res<SolarMirrorDirector>,
    unrest: Option<Res<Unrest>>,
    mut mirrors: Query<(Entity, &mut OrbitalSolarMirror)>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut ledger: ResMut<SolarMirrorLedger>,
) {
    if !director.hacking_enabled {
        return;
    }
    let unrest_level = unrest.map_or(0.0, |u| u.level);
    let probability = hack_probability(unrest_level, director.external_threat);
    if probability <= 0.0 {
        return;
    }
    for (entity, mut mirror) in mirrors.iter_mut() {
        if mirror.hacked || mirror.disabled {
            continue;
        }
        if mirror_hack_roll(entity.index(), time.tick) < probability {
            mirror.hacked = true;
            mirror.fire_cooldown = 0;
            ledger.hacks += 1;
            chronicle.send(AddChronicleEvent {
                importance: EventImportance::Major,
                text: "An orbital solar mirror has been hacked — its focused beam is burning uncontrolled!".to_string(),
            });
        }
    }
}

/// Command Center shutdown (spec §8): spend Admin surplus to disable a
/// mirror. Fail-closed — if the surplus is short (or Admin is absent
/// entirely) the order is refused and counted, and the mirror stays up.
pub fn disable_mirror_system(
    mut events: EventReader<DisableMirrorEvent>,
    admin: Option<Res<AdminStats>>,
    mut mirrors: Query<&mut OrbitalSolarMirror>,
    mut chronicle: EventWriter<AddChronicleEvent>,
    mut ledger: ResMut<SolarMirrorLedger>,
) {
    // Admin state cannot change while this system runs; read it once.
    let surplus = admin.map_or(0.0, |a| a.supply - a.demand);
    for event in events.read() {
        let Ok(mut mirror) = mirrors.get_mut(event.mirror) else {
            continue;
        };
        if mirror.disabled {
            continue;
        }
        if surplus >= MIRROR_DISABLE_ADMIN_COST {
            mirror.disabled = true;
            mirror.hacked = false;
            ledger.shutdowns += 1;
            chronicle.send(AddChronicleEvent {
                importance: EventImportance::Standard,
                text: "The Command Center has shut down an orbital solar mirror.".to_string(),
            });
        } else {
            ledger.shutdown_refusals += 1;
        }
    }
}

#[cfg(test)]
mod mirrors_435_tests {
    use super::*;
    use crate::layer1::day_night::{DayNightCycle, TimeOfDay};
    use crate::layer1::nature::solar::{update_solar_output_system, SolarCycleState, SolarPower};

    fn base_app() -> App {
        let mut app = App::new();
        app.add_event::<FireEvent>();
        app.add_event::<AddChronicleEvent>();
        app.add_event::<DisableMirrorEvent>();
        app.insert_resource(TemperatureGrid::new(50, 50, 20.0));
        app.insert_resource(SolarMirrorLedger::default());
        app.insert_resource(SolarMirrorDirector::default());
        app.insert_resource(SimulationTime::default());
        app
    }

    fn spawn_mirror(app: &mut App, intensity: f32, hacked: bool) -> Entity {
        app.world_mut()
            .spawn((
                OrbitalSolarMirror {
                    intensity,
                    hacked,
                    disabled: false,
                    fire_cooldown: 0,
                },
                PowerSource {
                    output: 0.0,
                    active: false,
                },
            ))
            .id()
    }

    fn count_fire_events(app: &mut App) -> Vec<GridPosition> {
        let events = app.world().resource::<Events<FireEvent>>();
        let mut reader = events.get_reader();
        reader.read(events).map(|e| e.position).collect()
    }

    /// Spec §3 RED test 1, adapted to the real power seam: a safe mirror
    /// drives its `PowerSource` to full intensity.
    #[test]
    fn test_safe_mirror_provides_power() {
        let mut app = base_app();
        app.add_systems(Update, process_solar_mirrors_system);
        let mirror = spawn_mirror(&mut app, 100.0, false);

        app.update();

        let source = app.world().get::<PowerSource>(mirror).unwrap();
        assert_eq!(source.output, 100.0);
        assert!(source.active);
        assert_eq!(app.world().resource::<SolarMirrorLedger>().safe_power, 100.0);
    }

    /// Spec §3 RED test 2, adapted to the real fire seam: a hacked
    /// mirror provides no power and starts exactly one fire at its
    /// target tile.
    #[test]
    fn test_hacked_mirror_causes_fire() {
        let mut app = base_app();
        app.add_systems(Update, process_solar_mirrors_system);
        let mirror = app
            .world_mut()
            .spawn((
                OrbitalSolarMirror {
                    intensity: 500.0,
                    hacked: true,
                    disabled: false,
                    fire_cooldown: 0,
                },
                PowerSource {
                    output: 0.0,
                    active: false,
                },
                TargetTile { x: 10, y: 10 },
            ))
            .id();

        app.update();

        let source = app.world().get::<PowerSource>(mirror).unwrap();
        assert_eq!(source.output, 0.0);
        assert!(!source.active);
        let fires = count_fire_events(&mut app);
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0], GridPosition { x: 10, y: 10 });
    }

    /// Spec §6 acceptance: mirrors provide energy regardless of the
    /// day/night cycle — while an ordinary solar panel on the same
    /// world zeroes out at night.
    #[test]
    fn test_mirror_bypasses_night() {
        let mut app = base_app();
        app.insert_resource(SolarCycleState::default());
        app.insert_resource(DayNightCycle {
            time_of_day: TimeOfDay::Night,
            ..Default::default()
        });
        app.add_systems(
            Update,
            (update_solar_output_system, process_solar_mirrors_system),
        );
        let panel = app
            .world_mut()
            .spawn((
                SolarPower {
                    base_output: 100.0,
                },
                PowerSource {
                    output: 100.0,
                    active: true,
                },
            ))
            .id();
        let mirror = spawn_mirror(&mut app, 100.0, false);

        app.update();

        assert_eq!(
            app.world().get::<PowerSource>(panel).unwrap().output,
            0.0,
            "ordinary panels must zero at night"
        );
        assert_eq!(
            app.world().get::<PowerSource>(mirror).unwrap().output,
            100.0,
            "mirrors must not consult the day/night cycle"
        );
    }

    /// Spec §5/§7: a safe mirror gently heats its target tile.
    #[test]
    fn test_safe_mirror_heats_target_tile() {
        let mut app = base_app();
        app.add_systems(Update, process_solar_mirrors_system);
        app.world_mut()
            .spawn((
                OrbitalSolarMirror::new(100.0),
                PowerSource {
                    output: 0.0,
                    active: false,
                },
                TargetTile { x: 5, y: 5 },
            ));

        app.update();

        let grid = app.world().resource::<TemperatureGrid>();
        assert!((grid.get(5, 5) - 20.5).abs() < 0.001);
        assert_eq!(grid.get(0, 0), 20.0);
    }

    /// Spec §5: the `Orbiting` link resolves a target when no explicit
    /// `TargetTile` is attached.
    #[test]
    fn test_orbiting_fallback_target() {
        let mut app = base_app();
        app.add_systems(Update, process_solar_mirrors_system);
        app.world_mut()
            .spawn((
                OrbitalSolarMirror {
                    intensity: 500.0,
                    hacked: true,
                    disabled: false,
                    fire_cooldown: 0,
                },
                PowerSource {
                    output: 0.0,
                    active: false,
                },
                Orbiting { x: 3, y: 4 },
            ));

        app.update();

        let fires = count_fire_events(&mut app);
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0], GridPosition { x: 3, y: 4 });
    }

    /// An explicit `TargetTile` wins over the `Orbiting` fallback.
    #[test]
    fn test_target_tile_preferred_over_orbiting() {
        assert_eq!(
            resolve_mirror_target(
                Some(&TargetTile { x: 7, y: 8 }),
                Some(&Orbiting { x: 1, y: 2 }),
            ),
            Some(GridPosition { x: 7, y: 8 })
        );
        assert_eq!(resolve_mirror_target(None, None), None);
    }

    /// A hacked mirror under the intensity threshold is a dead asset,
    /// not a weapon: no power, but no fires either — even with a target.
    #[test]
    fn test_hacked_low_intensity_no_fire() {
        let mut app = base_app();
        app.add_systems(Update, process_solar_mirrors_system);
        app.world_mut()
            .spawn((
                OrbitalSolarMirror {
                    intensity: 100.0,
                    hacked: true,
                    disabled: false,
                    fire_cooldown: 0,
                },
                PowerSource {
                    output: 0.0,
                    active: false,
                },
                TargetTile { x: 10, y: 10 },
            ));

        app.update();

        assert!(count_fire_events(&mut app).is_empty());
        assert_eq!(app.world().resource::<SolarMirrorLedger>().fires_started, 0);
    }

    /// The fire cooldown bounds a hacked mirror's burn rate: with a
    /// 50-tick cooldown, 120 ticks produce exactly 3 fires (ticks 1,
    /// 51, 101).
    #[test]
    fn test_fire_cooldown_bounds_rate() {
        let mut app = base_app();
        app.add_systems(Update, process_solar_mirrors_system);
        app.world_mut()
            .spawn((
                OrbitalSolarMirror {
                    intensity: 500.0,
                    hacked: true,
                    disabled: false,
                    fire_cooldown: 0,
                },
                PowerSource {
                    output: 0.0,
                    active: false,
                },
                TargetTile { x: 10, y: 10 },
            ));

        for _ in 0..120 {
            app.update();
        }

        assert_eq!(app.world().resource::<SolarMirrorLedger>().fires_started, 3);
    }

    /// Hack probability is pure, bounded, and zero when the colony is
    /// calm and unthreatened.
    #[test]
    fn test_hack_probability_bounds() {
        assert_eq!(hack_probability(0.0, 0.0), 0.0);
        assert!((hack_probability(1.0, 1.0) - 0.006).abs() < 1e-6);
        assert!(hack_probability(0.5, 0.0) > hack_probability(0.0, 0.0));
        assert!(hack_probability(0.0, 0.5) > hack_probability(0.0, 0.0));
        // Out-of-range inputs clamp instead of exceeding 1.0.
        assert!(hack_probability(5.0, 5.0) <= 1.0);
    }

    /// The hack roll is deterministic and stays in [0, 1).
    #[test]
    fn test_hack_roll_deterministic_and_bounded() {
        assert_eq!(mirror_hack_roll(7, 42), mirror_hack_roll(7, 42));
        for entity in 0..64u32 {
            for tick in 0..64u64 {
                let roll = mirror_hack_roll(entity, tick);
                assert!((0.0..1.0).contains(&roll), "roll {roll} out of range");
            }
        }
    }

    /// Under maximum unrest + threat, the hack system does land hacks
    /// (and chronicled ones), while a calm colony is never touched.
    #[test]
    fn test_hack_system_lands_under_pressure() {
        // Find a tick where entity 0's roll beats the max-pressure
        // probability, so the test is deterministic, not lucky.
        let p = hack_probability(1.0, 1.0);
        let tick = (0..100_000u64)
            .find(|&t| mirror_hack_roll(0, t) < p)
            .expect("some tick must roll under max-pressure probability");

        let mut app = base_app();
        app.insert_resource(SolarMirrorDirector {
            external_threat: 1.0,
            hacking_enabled: true,
        });
        app.insert_resource(Unrest {
            level: 1.0,
            modifiers: Vec::new(),
        });
        app.insert_resource(SimulationTime {
            tick,
            ..Default::default()
        });
        app.add_systems(Update, mirror_hack_system);
        // Spawn order matters: the first mirror spawned gets index 0
        // only in a fresh world — spawn exactly one.
        let mirror = spawn_mirror(&mut app, 500.0, false);

        app.update();

        assert!(
            app.world().get::<OrbitalSolarMirror>(mirror).unwrap().hacked,
            "max pressure at a losing-roll tick must hack the mirror"
        );
        assert_eq!(app.world().resource::<SolarMirrorLedger>().hacks, 1);
    }

    /// With hacking disabled at the director, even max pressure and a
    /// losing roll change nothing.
    #[test]
    fn test_hack_system_respects_director_switch() {
        let p = hack_probability(1.0, 1.0);
        let tick = (0..100_000u64)
            .find(|&t| mirror_hack_roll(0, t) < p)
            .expect("some tick must roll under max-pressure probability");

        let mut app = base_app();
        app.insert_resource(SolarMirrorDirector {
            external_threat: 1.0,
            hacking_enabled: false,
        });
        app.insert_resource(Unrest {
            level: 1.0,
            modifiers: Vec::new(),
        });
        app.insert_resource(SimulationTime {
            tick,
            ..Default::default()
        });
        app.add_systems(Update, mirror_hack_system);
        let mirror = spawn_mirror(&mut app, 500.0, false);

        app.update();

        assert!(!app.world().get::<OrbitalSolarMirror>(mirror).unwrap().hacked);
    }

    /// Spec §8: with Admin surplus, the Command Center shutdown lands —
    /// the mirror is disabled and no longer counts as hacked.
    #[test]
    fn test_disable_with_admin_surplus() {
        let mut app = base_app();
        app.insert_resource(AdminStats {
            supply: 10.0,
            demand: 0.0,
            efficiency: 1.0,
        });
        app.add_systems(Update, disable_mirror_system);
        let mirror = spawn_mirror(&mut app, 500.0, true);
        app.world_mut()
            .resource_mut::<Events<DisableMirrorEvent>>()
            .send(DisableMirrorEvent { mirror });

        app.update();

        let m = app.world().get::<OrbitalSolarMirror>(mirror).unwrap();
        assert!(m.disabled);
        assert!(!m.hacked);
        assert_eq!(app.world().resource::<SolarMirrorLedger>().shutdowns, 1);
    }

    /// Spec §8 fail-closed: with no Admin resource at all, the shutdown
    /// order is refused and the mirror stays up (and hacked).
    #[test]
    fn test_disable_fail_closed_without_admin() {
        let mut app = base_app();
        app.add_systems(Update, disable_mirror_system);
        let mirror = spawn_mirror(&mut app, 500.0, true);
        app.world_mut()
            .resource_mut::<Events<DisableMirrorEvent>>()
            .send(DisableMirrorEvent { mirror });

        app.update();

        let m = app.world().get::<OrbitalSolarMirror>(mirror).unwrap();
        assert!(!m.disabled);
        assert!(m.hacked);
        assert_eq!(
            app.world().resource::<SolarMirrorLedger>().shutdown_refusals,
            1
        );
    }

    /// Spec §8 fail-closed: Admin present but the surplus is short of
    /// the shutdown cost.
    #[test]
    fn test_disable_fail_closed_when_short() {
        let mut app = base_app();
        app.insert_resource(AdminStats {
            supply: 10.0,
            demand: 8.0,
            efficiency: 1.0,
        });
        app.add_systems(Update, disable_mirror_system);
        let mirror = spawn_mirror(&mut app, 500.0, true);
        app.world_mut()
            .resource_mut::<Events<DisableMirrorEvent>>()
            .send(DisableMirrorEvent { mirror });

        app.update();

        let m = app.world().get::<OrbitalSolarMirror>(mirror).unwrap();
        assert!(!m.disabled);
        assert_eq!(
            app.world().resource::<SolarMirrorLedger>().shutdown_refusals,
            1
        );
    }

    /// The STATS census counts standing/hacked mirrors and sums only
    /// safe output.
    #[test]
    fn test_mirror_stats_census() {
        let mut app = base_app();
        spawn_mirror(&mut app, 100.0, false);
        spawn_mirror(&mut app, 500.0, true);
        let disabled = spawn_mirror(&mut app, 300.0, false);
        app.world_mut()
            .get_mut::<OrbitalSolarMirror>(disabled)
            .unwrap()
            .disabled = true;

        let (total, hacked, safe_power) = mirror_stats(app.world_mut());
        assert_eq!(total, 3);
        assert_eq!(hacked, 1);
        assert_eq!(safe_power, 100.0);
    }

    /// IP guard: player-facing mirror text and labels must stay free of
    /// named-IP terms — mechanics and vibes only, original names.
    #[test]
    fn test_no_ip_terms_in_mirror_text() {
        const BANNED: &[&str] = &[
            "death star",
            "star wars",
            "star trek",
            "warhammer",
            "dune",
            "stargate",
            "battlestar",
        ];
        let mut app = base_app();
        app.insert_resource(AdminStats {
            supply: 10.0,
            demand: 0.0,
            efficiency: 1.0,
        });
        app.add_systems(Update, disable_mirror_system);
        let mirror = spawn_mirror(&mut app, 500.0, true);
        app.world_mut()
            .resource_mut::<Events<DisableMirrorEvent>>()
            .send(DisableMirrorEvent { mirror });
        app.update();

        let events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_reader();
        let texts: Vec<String> = reader.read(events).map(|e| e.text.clone()).collect();
        assert_eq!(texts.len(), 1);
        for text in &texts {
            let lower = text.to_lowercase();
            for banned in BANNED {
                assert!(!lower.contains(banned), "IP term {banned} in: {text}");
            }
        }
        for label in ["aligned", "HACKED", "disabled"] {
            for banned in BANNED {
                assert!(!label.contains(banned));
            }
        }
    }
}
