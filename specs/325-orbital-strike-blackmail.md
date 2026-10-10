# 325: Orbital Strike Blackmail

## 1. Overview

A rogue orbital weapons platform — the "Tollkeeper's Gavel" — parks above the colony and demands tribute: pay credits by the deadline, or a kinetic strike hits the colony heart. Paying emboldens the extortionist (demands grow larger and come sooner); defying invites the strike but erodes its leverage; a colony with enough tether-array point defense can swat the strike down and drive the platform off entirely. This creates a tense, escalating extortion arc with a real counter-play path, feeding the colony's unrest and chronicle.

## 2. Dependencies

- `277` Orbital Megastructure Deorbiting (for `TetherArray` point-defense interception and the `ExplosionEvent` damage pipeline)
- `323` The Informant's Dilemma (for `Unrest` / `UnrestModifier` conventions)
- Economy `Wallet` (colony credits are the tribute currency)

## 3. RED Phase: Tests First

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_event::<AddChronicleEvent>();
        app.add_event::<ExplosionEvent>();
        app.init_resource::<BlackmailStance>();
        app.init_resource::<SimulationTime>();
        app.init_resource::<Wallet>();
        app.init_resource::<Unrest>();
        app
    }

    #[test]
    fn test_blackmailer_arrives_when_colony_is_wealthy() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, blackmailer_arrival_system);
        app.world_mut().resource_mut::<Wallet>().credits = BLACKMAIL_WEALTH_THRESHOLD + 100.0;

        // Act
        app.update();

        // Assert
        assert!(app.world().get_resource::<Blackmailer>().is_some(), "Blackmailer should arrive once colony wealth crosses the threshold");
        assert!(app.world().get_resource::<ExtortionDemand>().is_some(), "Arrival should issue the first demand");
    }

    #[test]
    fn test_paying_tribute_deducts_credits_and_emboldens() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, extortion_deadline_system);
        app.world_mut().insert_resource(Blackmailer::new());
        app.world_mut().insert_resource(ExtortionDemand { amount: 500.0, deadline_tick: 10 });
        app.world_mut().insert_resource(BlackmailStance::Pay);
        app.world_mut().resource_mut::<Wallet>().credits = 5000.0;
        app.world_mut().resource_mut::<SimulationTime>().tick = 10;

        // Act
        app.update();

        // Assert
        let wallet = app.world().resource::<Wallet>();
        assert!((wallet.credits - 4500.0).abs() < 0.01, "Tribute should be deducted from the wallet");
        let blackmailer = app.world().resource::<Blackmailer>();
        assert!(blackmailer.demand_multiplier > 1.0, "Paying should embolden the blackmailer");
        assert!(app.world().get_resource::<ExtortionDemand>().is_none(), "Resolved demand should be cleared");
    }

    #[test]
    fn test_defying_triggers_strike_on_colony_heart() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, extortion_deadline_system);
        app.world_mut().insert_resource(Blackmailer::new());
        app.world_mut().insert_resource(ExtortionDemand { amount: 500.0, deadline_tick: 10 });
        app.world_mut().insert_resource(BlackmailStance::Defy);
        app.world_mut().resource_mut::<SimulationTime>().tick = 10;
        // A pop marks the colony heart at (7, 7).
        app.world_mut().spawn((Pop::default(), GridPosition { x: 7, y: 7 }));

        // Act
        app.update();

        // Assert
        let events = app.world().resource::<Events<ExplosionEvent>>();
        let mut reader = events.get_cursor();
        let mut struck = false;
        for ev in reader.read(events) {
            if ev.center.x == 7 && ev.center.y == 7 {
                struck = true;
                assert!(ev.damage > 0.0, "Strike should deal damage");
            }
        }
        assert!(struck, "Defying the demand should trigger a strike on the colony heart");
        let blackmailer = app.world().resource::<Blackmailer>();
        assert!(blackmailer.resolve < RESOLVE_START, "A defied strike should erode blackmailer resolve");
    }

    #[test]
    fn test_tether_arrays_intercept_the_strike() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, extortion_deadline_system);
        app.world_mut().insert_resource(Blackmailer::new());
        app.world_mut().insert_resource(ExtortionDemand { amount: 500.0, deadline_tick: 10 });
        app.world_mut().insert_resource(BlackmailStance::Defy);
        app.world_mut().resource_mut::<SimulationTime>().tick = 10;
        app.world_mut().spawn((Pop::default(), GridPosition { x: 7, y: 7 }));
        // Enough point defense to swat the strike down.
        for _ in 0..4 {
            app.world_mut().spawn(TetherArray::new());
        }

        // Act
        app.update();

        // Assert
        let events = app.world().resource::<Events<ExplosionEvent>>();
        let mut reader = events.get_cursor();
        assert_eq!(reader.read(events).count(), 0, "Intercepted strike should deal no damage");
        let blackmailer = app.world().resource::<Blackmailer>();
        assert!(blackmailer.resolve <= RESOLVE_START - RESOLVE_HIT_INTERCEPTED, "Interception should crater blackmailer resolve");
    }

    #[test]
    fn test_blackmailer_departs_when_resolve_breaks() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, (extortion_deadline_system, blackmailer_departure_system).chain());
        let mut blackmailer = Blackmailer::new();
        blackmailer.resolve = 10.0;
        app.world_mut().insert_resource(blackmailer);
        app.world_mut().insert_resource(ExtortionDemand { amount: 500.0, deadline_tick: 10 });
        app.world_mut().insert_resource(BlackmailStance::Defy);
        app.world_mut().resource_mut::<SimulationTime>().tick = 10;
        app.world_mut().spawn((Pop::default(), GridPosition { x: 7, y: 7 }));

        // Act
        app.update();

        // Assert
        assert!(app.world().get_resource::<Blackmailer>().is_none(), "Blackmailer should depart when resolve breaks");
        assert!(app.world().get_resource::<ExtortionDemand>().is_none(), "Demand should be cleared on departure");
    }

    #[test]
    fn test_active_demand_raises_unrest() {
        // Arrange
        let mut app = test_app();
        app.add_systems(Update, blackmail_unrest_system);
        app.world_mut().insert_resource(Blackmailer::new());
        app.world_mut().insert_resource(ExtortionDemand { amount: 500.0, deadline_tick: 100 });

        // Act
        app.update();

        // Assert
        let unrest = app.world().resource::<Unrest>();
        assert!(unrest.modifiers.iter().any(|m| m.label == UNREST_LABEL_BLACKMAIL), "Active demand should add an unrest modifier");
    }
}
```

## 4. GREEN Phase: Minimal Implementation

```rust
use bevy::prelude::*;

use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::map::GridPosition;
use crate::layer1::disasters::deorbit::TetherArray;
use crate::layer1::economy::Wallet;
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
/// Unrest modifier label while a demand hangs over the colony.
pub const UNREST_LABEL_BLACKMAIL: &str = "Orbital blackmail demand";
/// Unrest bump while a demand is active.
pub const BLACKMAIL_UNREST_VALUE: f32 = 0.15;
/// Unrest bump when a strike lands.
pub const STRIKE_UNREST_VALUE: f32 = 0.25;

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

/// An active extortion demand with a deadline.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ExtortionDemand {
    pub amount: f32,
    pub deadline_tick: u64,
}

/// Colony policy toward the blackmailer. Set via the `tribute` console
/// command (TUI wiring is a follow-on).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BlackmailStance {
    #[default]
    Undecided,
    Pay,
    Defy,
}

// --- Systems ------------------------------------------------------------------

pub fn blackmailer_arrival_system(/* ... */) { /* ... */ }
pub fn extortion_deadline_system(/* ... */) { /* ... */ }
pub fn blackmailer_departure_system(/* ... */) { /* ... */ }
pub fn blackmail_unrest_system(/* ... */) { /* ... */ }
```

## 5. REFACTOR Phase: Quality & Design

- **Strike targeting:** aim at the highest-value district (most buildings) rather than the raw pop centroid when that data is cheap to compute.
- **Negotiation:** allow a partial-payment haggle that buys more time at a resolve cost to the blackmailer.
- **TUI:** surface the active demand + deadline in the HUD and bind a key to flip `BlackmailStance`.
- **Cross-faction play:** rival colonies can hire the Tollkeeper to lean on the player (layer-3 hook).

## 6. Acceptance Criteria (Testable!)

- [ ] All tests in RED phase pass.
- [ ] `cargo test` returns 0 failures.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] Blackmailer arrives when colony wealth crosses the threshold (once).
- [ ] Paying deducts credits, clears the demand, emboldens the next demand.
- [ ] Defying (or undecided/insolvent at deadline) fires an `ExplosionEvent` at the colony heart.
- [ ] Enough tether arrays intercept the strike: no explosion, big resolve hit.
- [ ] Resolve at/below zero removes the blackmailer and clears the demand.
- [ ] Active demand adds an `UnrestModifier`; strikes spike unrest.

## 7. Technical Guidance

- Implement inside `src/layer1/diplomacy/blackmail.rs`; register `pub mod blackmail;` in `src/layer1/diplomacy/mod.rs`; chain the four systems in `src/simulation.rs` next to the other diplomacy registrations.
- Reuse `total_tether_strength`-style logic via a direct `Query<&TetherArray>` sum (that helper takes `&mut World` and cannot run inside a system).
- Colony heart = mean position of living pops, falling back to map center (mirror `deorbit.rs`).
- All names original and generic (IP-guard: no named characters, places, or distinctive IP).

## 8. Questions

*Builder: add questions here if spec is unclear.*
