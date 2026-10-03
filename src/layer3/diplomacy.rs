//! Layer 3 Diplomacy Modules
//!
//! This module handles diplomatic interactions and events, including proxy wars, cultural ransom, succession crises, and brain drain.
use crate::layer1::spore_diplomat::SporeInfection;
use bevy::prelude::*;

pub mod brain_drain;
pub mod cargo_cult_diplomat;
pub mod cultural_pressure;
pub mod cultural_ransom;
pub mod dead_internet;
pub mod endless_draft;
pub mod fading_homeworld;
pub mod proxy_wars;
pub mod succession;
pub mod wormhole_dumping;

#[derive(Component)]
pub struct Envoy;

#[derive(Component)]
pub struct Treaty {
    pub has_spore_propagation: bool,
}

#[derive(Resource)]
pub struct ActiveNegotiation {
    pub envoy: Entity,
    pub treaty: Entity,
}

pub fn diplomatic_negotiation_system(
    negotiation_opt: Option<Res<ActiveNegotiation>>,
    envoys: Query<&SporeInfection, With<Envoy>>,
    mut treaties: Query<&mut Treaty>,
) {
    if let Some(negotiation) = negotiation_opt {
        if let Ok(infection) = envoys.get(negotiation.envoy) {
            if infection.severity > 0.0 {
                if let Ok(mut treaty) = treaties.get_mut(negotiation.treaty) {
                    treaty.has_spore_propagation = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::pop::Pop;

    #[test]
    fn test_spore_diplomat_inserts_hidden_clause() {
        let mut app = App::new();
        let pop = app
            .world_mut()
            .spawn((Pop, Envoy, SporeInfection { severity: 1.0 }))
            .id();
        let treaty = app
            .world_mut()
            .spawn(Treaty {
                has_spore_propagation: false,
            })
            .id();
        app.world_mut()
            .insert_resource(ActiveNegotiation { envoy: pop, treaty });

        app.add_systems(Update, diplomatic_negotiation_system);
        app.update();

        let treaty_data = app.world().get::<Treaty>(treaty).unwrap();
        assert!(treaty_data.has_spore_propagation);
    }
}

#[derive(Event, Debug, Clone)]
pub struct RepossessionInvasionEvent {
    pub target_system: Entity,
    pub faction_id: Entity,
}

#[derive(Event, Debug, Clone)]
pub struct WarningDiplomaticMessageEvent {
    pub target_system: Entity,
    pub faction_id: Entity,
}
pub mod flesh_tax;
pub mod galactic_games;
pub mod god_mind;
pub mod red_tape_defense;
pub mod retro_contracts;
pub mod system_sovereignty;

pub mod diplomatic_fashion;
pub mod open_source_science;
pub mod trade_embargoes;

pub mod bureaucratic_blackout;
pub mod xenolinguistics;
