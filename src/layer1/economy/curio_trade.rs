//! Spec 334: The Curio Trade.
//!
//! Your colony is a curiosity to the bored elites of the Core Worlds.
//! Unique local items can be sold for massive credits as "art", but
//! removing them hollows out the colony's culture.
//!
//! Mechanic: exporting a curio-class resource ([`ResourceType::MemoryCore`] —
//! a memory core extracted from a dead pop) via [`execute_trade`] grants a
//! huge credit premium but fires a [`CulturalLossEvent`], which applies a
//! timed negative [`MoodModifier`] to every pop's [`Morale`]. The tension is
//! cultural heritage vs. economic survival: strip-mine the sacred to pay for
//! the shield generator, and watch the colony sink into spiritual depression.
//!
//! Mapping notes (spec 334 was written against mock types):
//! - Spec's `ColonyResources { credits }` is the real
//!   [`ColonyResources`], which already tracks `credits: f32`.
//! - Spec's `Pop { id }` / `Morale { current: 100.0 }` map to the real
//!   [`Morale`] component (`value` + timed `modifiers`). The spec's RED test
//!   asserted `morale.current < 100.0`; the equivalent here is the presence
//!   of a negative "Cultural Loss" [`MoodModifier`].
//! - Spec's `ResourceType::SingingCrystal` maps to the real curio-class
//!   resource [`ResourceType::MemoryCore`]: the colony's own dead, sold as
//!   curios to Core World elites. [`is_curio_resource`] is the classifier.
//! - The spec's REFACTOR phase suggested applying a *timed* stress/mood
//!   modifier instead of a flat morale deduction; the real morale
//!   architecture already works that way, so it is implemented directly.

use crate::layer1::chronicle::{Chronicle, EventImportance};
use crate::layer1::resources::{ColonyResources, ResourceType};
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;

/// Marker component tagging a physical curio entity.
///
/// Per the spec's technical guidance: tagged on specific resource entities
/// (e.g. harvested memory cores) before they are packed into cargo, so the
/// colony's curios are identifiable while they remain local.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Curio;

/// Credits granted per unit of curio exported (spec: "massive premium";
/// a normal trade good moves ~10 credits per unit).
pub const CURIO_CREDIT_PREMIUM: f32 = 1000.0;

/// Morale modifier value applied per unit of curio exported.
pub const CULTURAL_LOSS_VALUE: f32 = -0.25;

/// How long (ticks) the cultural-loss mood modifier lingers.
pub const CULTURAL_LOSS_DURATION: u32 = 600;

/// Event emitted when any trade deal completes.
///
/// The real trade path is [`crate::layer1::economy::trade::execute_trade`],
/// which sends this on every successful deal so curio exports can be
/// detected without changing the shape of [`crate::layer1::economy::trade::TradeDeal`].
#[derive(Event, Debug, Clone)]
pub struct TradeCompletedEvent {
    /// The resource the colony gave up in the deal.
    pub resource: ResourceType,
    /// Whether the exported resource is curio-class.
    pub is_curio: bool,
    /// Units exported.
    pub amount: f32,
}

/// Event emitted when curios leave the colony.
///
/// Carries the total morale-magnitude of the loss; consumed by
/// [`apply_cultural_loss_system`].
#[derive(Event, Debug, Clone)]
pub struct CulturalLossEvent {
    /// Negative morale value to apply (summed across exported units).
    pub magnitude: f32,
}

/// Ledger of the colony's curio trade (headless `curio` command reads this).
#[derive(Resource, Debug, Default)]
pub struct CurioTradeLedger {
    /// Total curio units exported, all time.
    pub total_curios_exported: f32,
    /// Total credits earned from curio exports, all time.
    pub total_credits_earned: f32,
    /// Total cultural-loss magnitude inflicted, all time.
    pub total_cultural_loss: f32,
}

/// Classifies curio-class resources.
///
/// Memory cores — extracted from dead pops — are the colony's curios:
/// unique, irreplaceable, and morbidly fascinating to Core World elites.
#[must_use]
pub fn is_curio_resource(resource: ResourceType) -> bool {
    matches!(resource, ResourceType::MemoryCore)
}

/// Grants the curio credit premium and fires cultural loss.
///
/// Reads [`TradeCompletedEvent`]; non-curio deals pass through untouched.
pub fn process_curio_export_system(
    mut events: EventReader<TradeCompletedEvent>,
    mut resources: ResMut<ColonyResources>,
    mut loss_events: EventWriter<CulturalLossEvent>,
    mut ledger: ResMut<CurioTradeLedger>,
    mut chronicle: Option<ResMut<Chronicle>>,
    time: Option<Res<SimulationTime>>,
) {
    let tick = time.map(|t| t.tick).unwrap_or(0);
    for event in events.read() {
        if !event.is_curio {
            continue;
        }
        let payout = CURIO_CREDIT_PREMIUM * event.amount;
        resources.credits += payout;
        // Magnitude is positive; the loss system negates it into the modifier.
        let magnitude = -CULTURAL_LOSS_VALUE * event.amount;
        loss_events.send(CulturalLossEvent { magnitude });
        ledger.total_curios_exported += event.amount;
        ledger.total_credits_earned += payout;
        ledger.total_cultural_loss += magnitude;
        if let Some(chronicle) = chronicle.as_mut() {
            chronicle.add_event(
                tick,
                format!(
                    "The colony sold {:.0} memory cores to Core World elites for {:.0} credits. The old songs feel thinner. [CULTURAL_LOSS]",
                    event.amount, payout
                ),
                EventImportance::Major,
            );
        }
    }
}

/// Applies cultural loss as a timed mood modifier on every pop.
///
/// Per the spec's refactor guidance, loss lands as a *timed*
/// [`MoodModifier`] ("Cultural Loss") rather than a flat deduction, so the
/// existing morale-decay machinery carries it away.
pub fn apply_cultural_loss_system(
    mut events: EventReader<CulturalLossEvent>,
    mut pops: Query<&mut Morale>,
) {
    for event in events.read() {
        for mut morale in pops.iter_mut() {
            morale.add_modifier(MoodModifier {
                label: "Cultural Loss".to_string(),
                value: -event.magnitude,
                duration: CULTURAL_LOSS_DURATION,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::pop::Pop;
    use bevy_app::{App, Update};
    fn test_resources() -> ColonyResources {
        ColonyResources {
            credits: 0.0,
            ..Default::default()
        }
    }

    #[test]
    fn test_curio_export_grants_high_credits() {
        let mut app = App::new();
        app.add_event::<TradeCompletedEvent>();
        app.add_event::<CulturalLossEvent>();
        app.add_systems(Update, process_curio_export_system);

        app.world_mut().insert_resource(test_resources());
        app.world_mut().insert_resource(CurioTradeLedger::default());
        app.world_mut()
            .send_event(TradeCompletedEvent {
                resource: ResourceType::MemoryCore,
                is_curio: true,
                amount: 1.0,
            });

        app.update();

        let resources = app.world().get_resource::<ColonyResources>().unwrap();
        assert!(
            resources.credits >= 1000.0,
            "Curios should sell for a massive premium, got {}",
            resources.credits
        );
    }

    #[test]
    fn test_non_curio_export_grants_no_premium() {
        let mut app = App::new();
        app.add_event::<TradeCompletedEvent>();
        app.add_event::<CulturalLossEvent>();
        app.add_systems(Update, process_curio_export_system);

        app.world_mut().insert_resource(test_resources());
        app.world_mut().insert_resource(CurioTradeLedger::default());
        app.world_mut()
            .send_event(TradeCompletedEvent {
                resource: ResourceType::Wood,
                is_curio: false,
                amount: 10.0,
            });

        app.update();

        let resources = app.world().get_resource::<ColonyResources>().unwrap();
        assert_eq!(
            resources.credits, 0.0,
            "Non-curio exports must not grant the curio premium"
        );
        let losses: usize = app
            .world()
            .get_resource::<Events<CulturalLossEvent>>()
            .map(|e| e.iter_current_update_events().count())
            .unwrap_or(0);
        assert_eq!(losses, 0, "Non-curio exports must not cause cultural loss");
    }

    #[test]
    fn test_curio_export_causes_cultural_loss() {
        let mut app = App::new();
        app.add_event::<CulturalLossEvent>();
        app.add_systems(Update, apply_cultural_loss_system);

        let entity = app
            .world_mut()
            .spawn((Pop, Morale::default()))
            .id();
        app.world_mut()
            .send_event(CulturalLossEvent { magnitude: 0.25 });

        app.update();

        let morale = app.world().get::<Morale>(entity).unwrap();
        let loss = morale
            .modifiers
            .iter()
            .find(|m| m.label == "Cultural Loss");
        assert!(
            loss.is_some_and(|m| m.value < 0.0),
            "Pops should carry a negative Cultural Loss modifier when curios are exported"
        );
    }

    #[test]
    fn test_cultural_loss_modifier_is_timed() {
        let mut app = App::new();
        app.add_event::<CulturalLossEvent>();
        app.add_systems(Update, apply_cultural_loss_system);

        let entity = app
            .world_mut()
            .spawn((Pop, Morale::default()))
            .id();
        app.world_mut()
            .send_event(CulturalLossEvent { magnitude: 0.5 });

        app.update();

        let morale = app.world().get::<Morale>(entity).unwrap();
        let loss = morale
            .modifiers
            .iter()
            .find(|m| m.label == "Cultural Loss")
            .unwrap();
        assert_eq!(
            loss.duration, CULTURAL_LOSS_DURATION,
            "Cultural loss should be a timed modifier, not permanent"
        );
    }

    #[test]
    fn test_is_curio_resource_classification() {
        assert!(is_curio_resource(ResourceType::MemoryCore));
        assert!(!is_curio_resource(ResourceType::Wood));
        assert!(!is_curio_resource(ResourceType::Metal));
        assert!(!is_curio_resource(ResourceType::Food));
    }

    #[test]
    fn test_ledger_tracks_curio_exports() {
        let mut app = App::new();
        app.add_event::<TradeCompletedEvent>();
        app.add_event::<CulturalLossEvent>();
        app.add_systems(Update, process_curio_export_system);

        app.world_mut().insert_resource(test_resources());
        app.world_mut().insert_resource(CurioTradeLedger::default());
        app.world_mut()
            .send_event(TradeCompletedEvent {
                resource: ResourceType::MemoryCore,
                is_curio: true,
                amount: 2.0,
            });

        app.update();

        let ledger = app.world().get_resource::<CurioTradeLedger>().unwrap();
        assert_eq!(ledger.total_curios_exported, 2.0);
        assert!(ledger.total_credits_earned >= 2000.0);
        assert!(ledger.total_cultural_loss > 0.0);
    }

    #[test]
    fn test_curio_marker_component_exists() {
        let mut world = World::new();
        let entity = world.spawn(Curio).id();
        assert!(world.get::<Curio>(entity).is_some());
    }

    /// Live-schedule end-to-end: a real curio trade through `execute_trade`
    /// flows through the actual game schedule — credits premium, ledger,
    /// timed grief on pops, chronicle entry.
    #[test]
    fn test_curio_trade_in_live_schedule() {
        use crate::layer1::economy::trade::{execute_trade, TradeDeal};
        use crate::setup::{setup_world_with_config, SetupConfig};
        use crate::simulation::run_simulation_tick;

        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        });

        // Stock the colony with curios (harvested memory cores).
        world.resource_mut::<ColonyResources>().memory_cores = 5.0;
        let credits_before = world.resource::<ColonyResources>().credits;

        // The player (or AI) accepts a curio deal: 2 memory cores for goods.
        let deal = TradeDeal {
            cost_resource: ResourceType::MemoryCore,
            cost_amount: 2.0,
            give_resource: ResourceType::Metal,
            give_amount: 3.0,
        };
        assert!(execute_trade(&mut world, &deal));

        // Run the real schedule: process_curio_export_system then
        // apply_cultural_loss_system are chained in Layer1SystemSet::Economy.
        run_simulation_tick(&mut world);

        // Massive credit premium landed.
        let resources = world.resource::<ColonyResources>();
        assert!(
            resources.credits >= credits_before + 2000.0,
            "curio export should pay the massive premium, credits went {} -> {}",
            credits_before,
            resources.credits
        );

        // Ledger tracked the export.
        let ledger = world.resource::<CurioTradeLedger>();
        assert_eq!(ledger.total_curios_exported, 2.0);
        assert!(ledger.total_credits_earned >= 2000.0);

        // Every pop grieves with a timed modifier.
        let mut grieving = 0;
        let mut pop_count = 0;
        let mut query = world.query::<&Morale>();
        for morale in query.iter(&world) {
            pop_count += 1;
            if morale
                .modifiers
                .iter()
                .any(|m| m.label == "Cultural Loss" && m.value < 0.0)
            {
                grieving += 1;
            }
        }
        assert!(pop_count > 0, "setup world should have pops");
        assert_eq!(
            grieving, pop_count,
            "all pops should carry the Cultural Loss modifier"
        );
    }
}
