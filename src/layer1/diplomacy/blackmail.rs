//! Spec 325: Orbital Strike Blackmail.
//!
//! A rogue orbital weapons platform — the "Tollkeeper's Gavel" — parks above
//! the colony and demands tribute: pay credits by the deadline, or a kinetic
//! strike hits the colony heart. Paying emboldens the extortionist (demands
//! grow); defying invites the strike but erodes its leverage; enough
//! tether-array point defense (see [`TetherArray`][crate::layer1::disasters::deorbit::TetherArray])
//! swats the strike down and drives the platform off.
//!
//! All names are original and generic.

use bevy_ecs::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::disasters::deorbit::TetherArray;
use crate::layer1::economy::resources::ColonyResources;
use crate::layer1::environment::volatile::ExplosionEvent;
use crate::layer1::pop::Pop;
use crate::layer1::unrest::{Unrest, UnrestModifier};
use crate::shared::time::SimulationTime;

// --- Tuning ---------------------------------------------------------------

/// Colony wealth that attracts the Tollkeeper's attention.
pub const BLACKMAIL_WEALTH_THRESHOLD: f32 = 2000.0;
/// First demand, in credits.
pub const INITIAL_DEMAND: f32 = 500.0;
/// Ticks the colony has to answer a demand.
pub const DEMAND_DEADLINE_TICKS: u64 = 300;
/// Quiet ticks between demands.
pub const DEMAND_COOLDOWN_TICKS: u64 = 1200;
/// Demand growth each time tribute is paid.
pub const EMBOLDEN_MULTIPLIER: f32 = 1.5;
/// Blackmailer resolve when it arrives.
pub const RESOLVE_START: f32 = 100.0;
/// Resolve lost when the colony defies and eats the strike.
pub const RESOLVE_HIT_DEFIED_STRIKE: f32 = 25.0;
/// Resolve lost when point defense intercepts the strike.
pub const RESOLVE_HIT_INTERCEPTED: f32 = 50.0;
/// Resolve regained each time tribute is paid (fear works).
pub const RESOLVE_BOOST_PAID: f32 = 10.0;
/// Kinetic strike damage.
pub const STRIKE_DAMAGE: f32 = 60.0;
/// Kinetic strike radius.
pub const STRIKE_RADIUS: u32 = 3;
/// Total tether strength needed to intercept a strike.
pub const INTERCEPT_TETHER_REQUIREMENT: f32 = 3.0;
/// Ticks before a departed blackmailer's successor may appear.
pub const BLACKMAIL_RETURN_COOLDOWN_TICKS: u64 = 20000;
/// Unrest modifier label while a demand hangs over the colony.
pub const UNREST_LABEL_BLACKMAIL: &str = "Orbital blackmail demand";
/// Unrest bump while a demand is active.
pub const BLACKMAIL_UNREST_VALUE: f32 = 0.15;
/// Ticks the demand unrest modifier lasts.
pub const BLACKMAIL_UNREST_DURATION: u32 = 400;
/// Unrest bump when a strike lands.
pub const STRIKE_UNREST_VALUE: f32 = 0.25;
/// Ticks the strike unrest modifier lasts.
pub const STRIKE_UNREST_DURATION: u32 = 600;
/// Map center fallback when there are no pops.
pub const MAP_CENTER: GridPosition = GridPosition { x: 40, y: 25 };

// --- State ------------------------------------------------------------------

/// The extortionist's orbital platform. Present as a resource while the
/// Tollkeeper holds the colony under the gun.
#[derive(Resource, Debug)]
pub struct Blackmailer {
    /// 0–100. Hits zero and the platform burns retrograde and leaves.
    pub resolve: f32,
    /// Multiplies each successive demand. Grows when tribute is paid.
    pub demand_multiplier: f32,
    /// Tick at which the next demand may be issued.
    pub next_demand_tick: u64,
}

impl Blackmailer {
    /// A freshly arrived extortionist, full of confidence.
    #[must_use]
    pub fn new() -> Self {
        Self {
            resolve: RESOLVE_START,
            demand_multiplier: 1.0,
            next_demand_tick: 0,
        }
    }
}

impl Default for Blackmailer {
    fn default() -> Self {
        Self::new()
    }
}

/// An active extortion demand with a deadline.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ExtortionDemand {
    /// Credits demanded.
    pub amount: f32,
    /// Tick at which the demand comes due.
    pub deadline_tick: u64,
}

/// Bars the Tollkeeper's return until the cooldown expires. Inserted on
/// departure so a wealthy colony is not re-targeted immediately.
#[derive(Resource, Debug, Clone, Copy)]
pub struct BlackmailBan {
    /// Tick before which no new blackmailer may arrive.
    pub until_tick: u64,
}

/// Colony policy toward the blackmailer. Set via the `tribute` console
/// command (TUI wiring is a follow-on).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BlackmailStance {
    /// No orders: the deadline decides (the strike comes).
    #[default]
    Undecided,
    /// Pay the tribute if the treasury covers it.
    Pay,
    /// Refuse outright: the strike comes, resolve erodes.
    Defy,
}

// --- Helpers ------------------------------------------------------------------

/// Mean position of living pops: the colony heart. Falls back to
/// [`MAP_CENTER`] when there are no pops.
fn colony_heart(pops: &Query<&GridPosition, With<Pop>>) -> GridPosition {
    let positions: Vec<GridPosition> = pops.iter().copied().collect();
    if positions.is_empty() {
        return MAP_CENTER;
    }
    let (sx, sy) = positions
        .iter()
        .fold((0i64, 0i64), |(x, y), p| (x + p.x as i64, y + p.y as i64));
    #[allow(clippy::cast_possible_truncation)]
    GridPosition {
        x: (sx / positions.len() as i64) as i32,
        y: (sy / positions.len() as i64) as i32,
    }
}

/// Total tether-array point-defense strength currently operational.
fn total_point_defense(tethers: &Query<&TetherArray>) -> f32 {
    tethers.iter().map(|t| t.strength).sum::<f32>() + 0.0
}

// --- Systems ------------------------------------------------------------------

/// The Tollkeeper arrives once the colony looks worth robbing: wealthy,
/// un-blackmailed, and not under a return ban.
pub fn blackmailer_arrival_system(
    mut commands: Commands,
    blackmailer: Option<Res<Blackmailer>>,
    ban: Option<Res<BlackmailBan>>,
    treasury: Res<ColonyResources>,
    time: Res<SimulationTime>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    if blackmailer.is_some() {
        return;
    }
    if let Some(b) = ban {
        if time.tick < b.until_tick {
            return;
        }
    }
    if treasury.credits < BLACKMAIL_WEALTH_THRESHOLD {
        return;
    }
    commands.insert_resource(Blackmailer::new());
    commands.insert_resource(ExtortionDemand {
        amount: INITIAL_DEMAND,
        deadline_tick: time.tick + DEMAND_DEADLINE_TICKS,
    });
    chronicle.send(AddChronicleEvent {
        text: format!(
            "A rogue weapons platform has parked in high orbit. The Tollkeeper demands {INITIAL_DEMAND:.0} credits within {DEMAND_DEADLINE_TICKS} ticks — pay, or a kinetic strike falls on the colony."
        ),
        importance: EventImportance::Major,
    });
}

/// Issues the next demand once the cooldown after the last resolution
/// expires. Demands scale with the emboldenment multiplier.
pub fn blackmailer_redemand_system(
    mut commands: Commands,
    blackmailer: Option<Res<Blackmailer>>,
    demand: Option<Res<ExtortionDemand>>,
    time: Res<SimulationTime>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let Some(bm) = blackmailer else { return };
    if demand.is_some() {
        return;
    }
    if bm.resolve <= 0.0 {
        return;
    }
    if time.tick < bm.next_demand_tick {
        return;
    }
    let amount = INITIAL_DEMAND * bm.demand_multiplier;
    commands.insert_resource(ExtortionDemand {
        amount,
        deadline_tick: time.tick + DEMAND_DEADLINE_TICKS,
    });
    chronicle.send(AddChronicleEvent {
        text: format!(
            "The Tollkeeper is back on the comm. New demand: {amount:.0} credits within {DEMAND_DEADLINE_TICKS} ticks."
        ),
        importance: EventImportance::Major,
    });
}

/// Resolves a demand at its deadline: tribute paid, or the strike falls.
pub fn extortion_deadline_system(
    mut commands: Commands,
    blackmailer: Option<ResMut<Blackmailer>>,
    demand: Option<Res<ExtortionDemand>>,
    stance: Res<BlackmailStance>,
    mut treasury: ResMut<ColonyResources>,
    time: Res<SimulationTime>,
    pops: Query<&GridPosition, With<Pop>>,
    tethers: Query<&TetherArray>,
    mut explosions: EventWriter<ExplosionEvent>,
    mut unrest: ResMut<Unrest>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let Some(mut bm) = blackmailer else { return };
    let Some(d) = demand else { return };
    if time.tick < d.deadline_tick {
        return;
    }
    let amount = d.amount;
    commands.remove_resource::<ExtortionDemand>();

    if *stance == BlackmailStance::Pay && treasury.credits >= amount {
        treasury.credits -= amount;
        bm.demand_multiplier *= EMBOLDEN_MULTIPLIER;
        bm.resolve = (bm.resolve + RESOLVE_BOOST_PAID).min(RESOLVE_START);
        bm.next_demand_tick = time.tick + DEMAND_COOLDOWN_TICKS;
        chronicle.send(AddChronicleEvent {
            text: format!(
                "The colony paid the Tollkeeper {amount:.0} credits. The platform holds position — and its next demand will be larger."
            ),
            importance: EventImportance::Major,
        });
        return;
    }

    // Defied, undecided, or insolvent: the strike comes.
    if total_point_defense(&tethers) >= INTERCEPT_TETHER_REQUIREMENT {
        bm.resolve -= RESOLVE_HIT_INTERCEPTED;
        chronicle.send(AddChronicleEvent {
            text: "The Tollkeeper fired — and the colony's tether arrays swatted the tungsten rod out of the sky. Debris rained harmlessly into the wastes.".to_string(),
            importance: EventImportance::Major,
        });
    } else {
        let target = colony_heart(&pops);
        explosions.send(ExplosionEvent {
            center: target,
            damage: STRIKE_DAMAGE,
            radius: STRIKE_RADIUS,
        });
        unrest.modifiers.push(UnrestModifier {
            value: STRIKE_UNREST_VALUE,
            duration: STRIKE_UNREST_DURATION,
            label: "Orbital strike".to_string(),
        });
        bm.resolve -= RESOLVE_HIT_DEFIED_STRIKE;
        chronicle.send(AddChronicleEvent {
            text: format!(
                "The Tollkeeper made good on its threat. A kinetic strike hit ({}, {}) — the colony bled, and the platform's crew celebrated on an open channel.",
                target.x, target.y
            ),
            importance: EventImportance::Major,
        });
    }
    bm.next_demand_tick = time.tick + DEMAND_COOLDOWN_TICKS;
}

/// A blackmailer with no resolve left burns retrograde and leaves. A return
/// ban keeps a successor away for a generation.
pub fn blackmailer_departure_system(
    mut commands: Commands,
    blackmailer: Option<Res<Blackmailer>>,
    time: Res<SimulationTime>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let Some(bm) = blackmailer else { return };
    if bm.resolve > 0.0 {
        return;
    }
    commands.remove_resource::<Blackmailer>();
    commands.remove_resource::<ExtortionDemand>();
    commands.insert_resource(BlackmailBan {
        until_tick: time.tick + BLACKMAIL_RETURN_COOLDOWN_TICKS,
    });
    chronicle.send(AddChronicleEvent {
        text: "The Tollkeeper's platform burns retrograde and vanishes from the sky. The colony kept its credits and its spine.".to_string(),
        importance: EventImportance::Major,
    });
}

/// An active demand hangs over the colony: unrest climbs while the
/// Tollkeeper waits for an answer.
pub fn blackmail_unrest_system(
    demand: Option<Res<ExtortionDemand>>,
    mut unrest: ResMut<Unrest>,
) {
    if demand.is_none() {
        return;
    }
    if unrest
        .modifiers
        .iter()
        .any(|m| m.label == UNREST_LABEL_BLACKMAIL)
    {
        return;
    }
    unrest.modifiers.push(UnrestModifier {
        value: BLACKMAIL_UNREST_VALUE,
        duration: BLACKMAIL_UNREST_DURATION,
        label: UNREST_LABEL_BLACKMAIL.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::core::map::GridPosition;
    use crate::layer1::disasters::deorbit::TetherArray;
    use crate::layer1::economy::resources::ColonyResources;
    use crate::layer1::environment::volatile::ExplosionEvent;
    use crate::layer1::pop::Pop;
    use crate::layer1::unrest::Unrest;
    use crate::shared::time::SimulationTime;
    use bevy::app::App;
    use bevy::MinimalPlugins;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_event::<AddChronicleEvent>();
        app.add_event::<ExplosionEvent>();
        app.init_resource::<BlackmailStance>();
        app.init_resource::<SimulationTime>();
        app.init_resource::<ColonyResources>();
        app.init_resource::<Unrest>();
        app
    }

    #[test]
    fn test_blackmailer_arrives_when_colony_is_wealthy() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, blackmailer_arrival_system);
        app.world_mut().resource_mut::<ColonyResources>().credits =
            BLACKMAIL_WEALTH_THRESHOLD + 100.0;

        // Act
        app.update();

        // Assert
        assert!(
            app.world().get_resource::<Blackmailer>().is_some(),
            "Blackmailer should arrive once colony wealth crosses the threshold"
        );
        assert!(
            app.world().get_resource::<ExtortionDemand>().is_some(),
            "Arrival should issue the first demand"
        );
    }

    #[test]
    fn test_blackmailer_does_not_arrive_twice() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, blackmailer_arrival_system);
        app.world_mut().resource_mut::<ColonyResources>().credits =
            BLACKMAIL_WEALTH_THRESHOLD + 100.0;
        app.update();
        let first_demand = app
            .world()
            .get_resource::<ExtortionDemand>()
            .map(|d| d.deadline_tick);

        // Act: wealth still high, second tick.
        app.update();

        // Assert: still exactly one blackmailer, demand untouched.
        assert!(
            app.world().get_resource::<Blackmailer>().is_some(),
            "Blackmailer should still be present"
        );
        let second_demand = app
            .world()
            .get_resource::<ExtortionDemand>()
            .map(|d| d.deadline_tick);
        assert_eq!(
            first_demand, second_demand,
            "Arrival must not re-issue the demand while one is active"
        );
    }

    #[test]
    fn test_paying_tribute_deducts_credits_and_emboldens() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, extortion_deadline_system);
        app.world_mut().insert_resource(Blackmailer::new());
        app.world_mut().insert_resource(ExtortionDemand {
            amount: 500.0,
            deadline_tick: 10,
        });
        app.world_mut().insert_resource(BlackmailStance::Pay);
        app.world_mut().resource_mut::<ColonyResources>().credits = 5000.0;
        app.world_mut().resource_mut::<SimulationTime>().tick = 10;

        // Act
        app.update();

        // Assert
        let credits = app.world().resource::<ColonyResources>().credits;
        assert!(
            (credits - 4500.0).abs() < 0.01,
            "Tribute should be deducted from colony credits"
        );
        let blackmailer = app.world().resource::<Blackmailer>();
        assert!(
            blackmailer.demand_multiplier > 1.0,
            "Paying should embolden the blackmailer"
        );
        assert!(
            app.world().get_resource::<ExtortionDemand>().is_none(),
            "Resolved demand should be cleared"
        );
    }

    #[test]
    fn test_insolvent_pay_stance_still_triggers_strike() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, extortion_deadline_system);
        app.world_mut().insert_resource(Blackmailer::new());
        app.world_mut().insert_resource(ExtortionDemand {
            amount: 500.0,
            deadline_tick: 10,
        });
        app.world_mut().insert_resource(BlackmailStance::Pay);
        app.world_mut().resource_mut::<ColonyResources>().credits = 10.0;
        app.world_mut().resource_mut::<SimulationTime>().tick = 10;
        app.world_mut().spawn((Pop, GridPosition { x: 7, y: 7 }));

        // Act
        app.update();

        // Assert: cannot pay, so the strike comes anyway.
        let events = app.world().resource::<Events<ExplosionEvent>>();
        let mut cursor = events.get_cursor();
        assert!(
            cursor.read(events).count() > 0,
            "Insolvent colony should suffer the strike even with Pay stance"
        );
    }

    #[test]
    fn test_defying_triggers_strike_on_colony_heart() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, extortion_deadline_system);
        app.world_mut().insert_resource(Blackmailer::new());
        app.world_mut().insert_resource(ExtortionDemand {
            amount: 500.0,
            deadline_tick: 10,
        });
        app.world_mut().insert_resource(BlackmailStance::Defy);
        app.world_mut().resource_mut::<SimulationTime>().tick = 10;
        // A pop marks the colony heart at (7, 7).
        app.world_mut().spawn((Pop, GridPosition { x: 7, y: 7 }));

        // Act
        app.update();

        // Assert
        let events = app.world().resource::<Events<ExplosionEvent>>();
        let mut cursor = events.get_cursor();
        let mut struck = false;
        for ev in cursor.read(events) {
            if ev.center.x == 7 && ev.center.y == 7 {
                struck = true;
                assert!(ev.damage > 0.0, "Strike should deal damage");
            }
        }
        assert!(
            struck,
            "Defying the demand should trigger a strike on the colony heart"
        );
        let blackmailer = app.world().resource::<Blackmailer>();
        assert!(
            blackmailer.resolve < RESOLVE_START,
            "A defied strike should erode blackmailer resolve"
        );
    }

    #[test]
    fn test_tether_arrays_intercept_the_strike() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, extortion_deadline_system);
        app.world_mut().insert_resource(Blackmailer::new());
        app.world_mut().insert_resource(ExtortionDemand {
            amount: 500.0,
            deadline_tick: 10,
        });
        app.world_mut().insert_resource(BlackmailStance::Defy);
        app.world_mut().resource_mut::<SimulationTime>().tick = 10;
        app.world_mut().spawn((Pop, GridPosition { x: 7, y: 7 }));
        // Enough point defense to swat the strike down.
        for _ in 0..4 {
            app.world_mut().spawn(TetherArray::new());
        }

        // Act
        app.update();

        // Assert
        let events = app.world().resource::<Events<ExplosionEvent>>();
        let mut cursor = events.get_cursor();
        assert_eq!(
            cursor.read(events).count(),
            0,
            "Intercepted strike should deal no damage"
        );
        let blackmailer = app.world().resource::<Blackmailer>();
        assert!(
            blackmailer.resolve <= RESOLVE_START - RESOLVE_HIT_INTERCEPTED,
            "Interception should crater blackmailer resolve"
        );
    }

    #[test]
    fn test_blackmailer_departs_when_resolve_breaks() {
        // Arrange
        let mut app = test_app();
        app.add_systems(
            bevy::app::Update,
            (extortion_deadline_system, blackmailer_departure_system).chain(),
        );
        let mut blackmailer = Blackmailer::new();
        blackmailer.resolve = 10.0;
        app.world_mut().insert_resource(blackmailer);
        app.world_mut().insert_resource(ExtortionDemand {
            amount: 500.0,
            deadline_tick: 10,
        });
        app.world_mut().insert_resource(BlackmailStance::Defy);
        app.world_mut().resource_mut::<SimulationTime>().tick = 10;
        app.world_mut().spawn((Pop, GridPosition { x: 7, y: 7 }));

        // Act
        app.update();

        // Assert
        assert!(
            app.world().get_resource::<Blackmailer>().is_none(),
            "Blackmailer should depart when resolve breaks"
        );
        assert!(
            app.world().get_resource::<ExtortionDemand>().is_none(),
            "Demand should be cleared on departure"
        );
    }

    #[test]
    fn test_departed_blackmailer_does_not_immediately_return() {
        // Arrange
        let mut app = test_app();
        app.add_systems(
            bevy::app::Update,
            (blackmailer_departure_system, blackmailer_arrival_system).chain(),
        );
        let mut blackmailer = Blackmailer::new();
        blackmailer.resolve = 0.0;
        app.world_mut().insert_resource(blackmailer);
        app.world_mut().resource_mut::<ColonyResources>().credits =
            BLACKMAIL_WEALTH_THRESHOLD + 100.0;

        // Act
        app.update();

        // Assert: departure fired, ban inserted, arrival blocked.
        assert!(
            app.world().get_resource::<Blackmailer>().is_none(),
            "Blackmailer should have departed"
        );
        assert!(
            app.world().get_resource::<BlackmailBan>().is_some(),
            "Departure should insert a return ban"
        );
        assert!(
            app.world().get_resource::<ExtortionDemand>().is_none(),
            "No new demand should be issued under the ban"
        );
    }

    #[test]
    fn test_redemand_escalates_after_payment() {
        // Arrange
        let mut app = test_app();
        app.add_systems(
            bevy::app::Update,
            (extortion_deadline_system, blackmailer_redemand_system).chain(),
        );
        let mut blackmailer = Blackmailer::new();
        blackmailer.next_demand_tick = 100;
        app.world_mut().insert_resource(blackmailer);
        app.world_mut().insert_resource(ExtortionDemand {
            amount: 500.0,
            deadline_tick: 10,
        });
        app.world_mut().insert_resource(BlackmailStance::Pay);
        app.world_mut().resource_mut::<ColonyResources>().credits = 50000.0;
        app.world_mut().resource_mut::<SimulationTime>().tick = 10;

        // Act: deadline resolves (payment), then jump past the cooldown.
        app.update();
        app.world_mut().resource_mut::<SimulationTime>().tick = 2000;
        app.update();

        // Assert: a larger demand is on the table.
        let demand = app
            .world()
            .get_resource::<ExtortionDemand>()
            .expect("A new demand should be issued after the cooldown");
        assert!(
            demand.amount > 500.0,
            "Emboldened demand should exceed the original 500"
        );
    }

    #[test]
    fn test_active_demand_raises_unrest() {
        // Arrange
        let mut app = test_app();
        app.add_systems(bevy::app::Update, blackmail_unrest_system);
        app.world_mut().insert_resource(Blackmailer::new());
        app.world_mut().insert_resource(ExtortionDemand {
            amount: 500.0,
            deadline_tick: 100,
        });

        // Act
        app.update();

        // Assert
        let unrest = app.world().resource::<Unrest>();
        assert!(
            unrest
                .modifiers
                .iter()
                .any(|m| m.label == UNREST_LABEL_BLACKMAIL),
            "Active demand should add an unrest modifier"
        );
    }
}
