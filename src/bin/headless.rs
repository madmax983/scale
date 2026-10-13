#![deny(unsafe_code)]
//! Headless game runner with semantic input/output.
//!
//! Run with: `cargo run --bin headless`
//!
//! Commands:
//!   `tick [N]`       - Advance N ticks (default 1)
//!   `status`         - Show colony resources and pop count
//!   `pops`           - Show detailed pop states
//!   `map [x] [y]`    - Show visual terrain around position
//!   `scan [x] [y] [r]` - Semantic terrain output (parseable)
//!   `terrain <x> <y>` - Get single tile info
//!   `buildings`      - List all buildings with positions
//!   `build <type> <x> <y>` - Build: farm, housing, stockpile
//!   `mine <x> <y>`   - Designate rock for mining
//!   `chop <x> <y>`   - Designate tree for chopping
//!   `excavate <x> <y>` - Excavate Desire Dust from a tile (Spec 1379)
//!   `dust`            - Show Desire Dust / RoadMind status
//!   `launch_sat`      - Launch a slogan satellite (Spec 1380)
//!   `constellation`   - Show Propaganda Constellation status
//!   `hack_sat <msg>`  - Rival hack: flip the sky-message to despair
//!   `shootdown <id>`  - Shoot down your own satellite (feeds debris)
//!   `give <res> <n>`   - Debug: grant resources
//!   `designations`   - List all active designations
//!   `find <terrain> [count]` - Find terrain coordinates
//!   `possess <pop_id>` - Take direct control of a pop (adventurer mode)
//!   `move <north|south|east|west>` - Step the possessed pop one tile
//!   `interact`       - Act at the possessed pop's tile
//!   `release`        - Return the possessed pop to AI control
//!   `possessed`      - Show who is currently possessed
//!   `origin <name>`  - Choose this run's adventurer origin (before first tick;
//!                      also `--origin=<name>` on the command line)
//!   `origins`        - List adventurer origins and this run's roster
//!   `help`           - Show this help
//!   `quit`           - Exit
//!   `save <slot>`    - Save the colony to a slot (Spec 1382)
//!   `load <slot>`    - Load the colony from a slot
//!   `saves`          - List stored saves with metadata
//!   `delsave <slot>` - Delete a stored save
//!   `export <slot> <path>` - Copy a save to a .scale-save file

#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::needless_pass_by_ref_mut)]
#![allow(clippy::too_many_lines)]

use bevy_ecs::prelude::*;
use comfy_table::{presets::UTF8_FULL, Attribute, Cell, Color, ContentArrangement, Table};
use crossterm::style::Stylize;
use scale::layer1::biography::Biography;
use scale::layer1::construction::{
    ConstructionProgress, ContributeToGreatWorkEvent, GreatWork, GreatWorkKind,
    OperationalGreatWork,
};
use scale::layer1::dreams::Dream;
#[cfg(feature = "nova")]
use scale::layer1::oral_tradition::{OralTradition, StoryGenre};
use scale::layer1::culture::origins::{
    origin_roster_summary, OriginChoice, OriginKind, OriginSchedule,
};
use scale::layer1::culture::sovereign::{
    abdicate, commission_artwork, crown_within_reach, describe_edicts, issue_decree,
    sovereign_stats, try_crown_pop, DecreeKind, Sovereign,
};
use scale::layer1::culture::corsair::{
    corsair_stats, describe_corsair, divide_plunder, execute_raid, fence_plunder, repair_skiff,
    skim_credits, skiff_within_reach, try_embark_corsair, unload_hold, CorsairCaptain, RaidTarget,
};
use scale::layer1::culture::governor::{
    audit_treasury, collect_tithe, debate_rival, describe_political_field, describe_treasury,
    file_paperwork, governor_stats, hold_hearing, issue_directive, purge_rival, resign,
    seal_within_reach, try_sign_seal, DirectiveKind, Governor,
};
use scale::layer1::culture::salvager::{
    claim_hulk, describe_salvager, patch_breach, salvager_stats, scuttle_hulk, start_strip,
    start_tow, survey_wreck, try_take_spike, wreck_within_reach, Salvager, WreckSystemKind,
};
use scale::layer1::culture::improbable::{
    describe_pilot, file_scheme, fire_jump, pilot_stats, shuttle_within_reach, skim_cache,
    try_take_yoke, unload_cache, ImprobablePilot,
};
use scale::layer1::culture::lawbound::{
    assess_lawbound_step, cradle_within_reach, describe_laws, give_order, hazard_word,
    lawbound_stats, resolve_zeroth, try_wake_lawbound, DormantAutomaton, LawboundAutomaton,
    StepVerdict,
};
use scale::layer1::culture::chronostalker::{
    chronostalker_debt, describe_stalker, rewind_time, toggle_anchor, toggle_phase,
    try_take_stalker_path, wound_within_reach, Chronostalker,
};
use scale::layer1::culture::bloomtouched::{
    bloom_stats, bloom_survey_report, bloom_within_reach, describe_touched, embrace_bloom,
    resist_bloom, sporecast, take_bloom_mutation, try_take_bloom_path, BloomTouched,
};
use scale::layer1::direct_link::{
    possessed_entity, try_player_step, DirectControlState, Possessed,
};
use scale::layer1::pop::PopName;
use scale::layer1::stress::StressTracker;
use scale::layer1::tech::{unlock_tech, Tech, TechState, TechStatus};
use scale::layer1::traits::Traits;
use scale::layer1::{
    try_designate, try_place_building, Building, BuildingType, Chronicle, ColonyResources,
    Designation, DesignationType, DesireDust, EventImportance, Farm, GlobalWind, GridPosition,
    Housing, Morale, MovementTarget, Needs, OccupiedTiles, Pop, PopAction, RoadMind, Stockpile,
    TerrainGrid, TerrainType,
};
use scale::setup::{setup_world_with_config, SetupConfig};
use scale::shared::log::MessageLog;
use scale::shared::state::GameState;
use scale::shared::time::SimulationTime;
use scale::persistence::{
    export_save, import_save, list_saves, load_world, maybe_autosave, save_world, FileSaveStore,
    SaveStore,
};
use scale::simulation::run_simulation_tick;
use std::io::{self, BufRead, Write};

/// Renders a Comfy Table with a custom title injected into its top border
fn print_dashboard_table(title: &str, mut table: comfy_table::Table) {
    use comfy_table::modifiers::UTF8_ROUND_CORNERS;
    use comfy_table::presets::UTF8_FULL;
    use crossterm::style::Stylize;

    table.set_width(120);
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS);

    // Render the table to string
    let table_str = table.to_string();
    let mut lines: Vec<String> = table_str.lines().map(String::from).collect();

    if lines.is_empty() {
        return;
    }

    // Embed the title directly into the first line (the top border)
    let char_tl = '\u{256D}'; // top left rounded
    let char_h = '\u{2500}'; // horiz line

    let title_styled = format!(" {} ", title).cyan().bold().to_string();
    let title_len = title.chars().count() + 2;

    let top_line_chars: Vec<char> = lines[0].chars().collect();
    let width = top_line_chars.len();

    let custom_top = if width > title_len + 3 {
        let mut custom = format!("{}{}{}", char_tl, char_h, char_h);
        custom.push_str(&title_styled);
        for &ch in top_line_chars.iter().skip(title_len + 3) {
            custom.push(ch);
        }
        custom
    } else {
        // Fallback for extremely narrow tables
        let char_tr = '\u{256E}'; // top right rounded
        format!("{}{}{}{}{}", char_tl, char_h, char_h, title_styled, char_tr)
    };

    lines[0] = custom_top;

    // Output the top line
    println!("{}", lines[0]);

    // Print the rest of the table
    for line in lines.iter().skip(1) {
        println!("{}", line);
    }
}

/// Print a 1-column table as a panel for errors/empty states
fn print_dashboard_panel(
    title: &str,
    content: &str,
    color: Option<comfy_table::Color>,
    attribute: Option<comfy_table::Attribute>,
) {
    use comfy_table::{Cell, ContentArrangement, Table};
    let mut table = Table::new();
    table.set_width(120);
    let mut cell = Cell::new(format!("  {}  ", content.trim()));

    if let Some(c) = color {
        cell = cell.fg(c);
    }
    if let Some(a) = attribute {
        cell = cell.add_attribute(a);
    }

    table
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .add_row(vec![cell]);

    print_dashboard_table(title, table);
}

fn main() {
    let mut world = setup_world_with_config(SetupConfig {
        headless: true,
        ..Default::default()
    });
    *world.resource_mut::<GameState>() = GameState::Running;

    // --origin=<name>: choose this run's adventurer origin up front.
    // Without it (and without the `origin` console command before the
    // first tick) the run gets a random 2–3-origin roster.
    if let Some(flag) = std::env::args()
        .find_map(|a| a.strip_prefix("--origin=").map(str::to_string))
    {
        match OriginKind::parse(&flag) {
            Some(kind) => {
                world.insert_resource(OriginChoice {
                    chosen: Some(kind),
                });
                println!(
                    "{}",
                    format!("Origin chosen: {} ({})", kind.name(), kind.id()).cyan()
                );
            }
            None => {
                eprintln!("Unknown origin '{flag}'. Available origins:");
                for kind in OriginKind::all() {
                    eprintln!("  {:14} {}", kind.id(), kind.name());
                }
            }
        }
    }

    print_dashboard_panel(
        "SYSTEM",
        "SCALE Headless Dashboard Initialized",
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
    println!("{}", "Type 'help' for commands, 'quit' to exit.\n".grey());

    print_status(&mut world);
    println!();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("> ");
        if let Err(e) = stdout.flush() {
            eprintln!("Error flushing stdout: {e}");
            break;
        }

        let mut input = String::new();
        if stdin.lock().read_line(&mut input).is_err() {
            break;
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        if !handle_command(&mut world, input) {
            break;
        }
        println!();
    }
}

fn handle_build_command(world: &mut World, parts: &[&str]) {
    if parts.len() < 4 {
        print_dashboard_panel(
            "ERROR",
            "Usage: build <farm|housing|stockpile> <x> <y>",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    let building_type = match parts[1].to_lowercase().as_str() {
        "farm" | "f" => Some(BuildingType::Farm),
        "housing" | "h" => Some(BuildingType::Housing),
        "stockpile" | "s" => Some(BuildingType::Stockpile),
        _ => None,
    };
    let x: Option<i32> = parts[2].parse().ok();
    let y: Option<i32> = parts[3].parse().ok();

    match (building_type, x, y) {
        (Some(bt), Some(x), Some(y)) => build_at(world, bt, x, y),
        _ => {
            print_dashboard_panel(
                "ERROR",
                "Invalid arguments. Usage: build <farm|housing|stockpile> <x> <y>",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
        }
    }
}

fn handle_designate_command(world: &mut World, parts: &[&str], designation_type: DesignationType) {
    if parts.len() < 3 {
        let name = match designation_type {
            DesignationType::Destroy => "destroy",
            DesignationType::Mine => "mine",
            DesignationType::Chop => "chop",
            DesignationType::ExcavateDust => "excavate",
            _ => "designate", // Fallback, though we only call this for destroy, mine, chop
        };
        let msg = format!("Usage: {name} <x> <y>");
        print_dashboard_panel(
            "ERROR",
            &msg,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    let x: Option<i32> = parts[1].parse().ok();
    let y: Option<i32> = parts[2].parse().ok();
    match (x, y) {
        (Some(x), Some(y)) => designate_at(world, designation_type, x, y),
        _ => print_dashboard_panel(
            "ERROR",
            "Invalid coordinates",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_research_command(world: &mut World, parts: &[&str]) {
    if parts.len() < 2 {
        print_dashboard_panel(
            "ERROR",
            "Usage: research <tech_name>",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    // Join parts in case tech name has spaces (e.g., "Metal Working")
    let tech_name = parts[1..].join(" ").to_lowercase();

    let tech = match tech_name.as_str() {
        "masonry" => Some(Tech::Masonry),
        "metal working" | "metalworking" => Some(Tech::MetalWorking),
        "social structures" | "social" => Some(Tech::SocialStructures),
        "astronomy" => Some(Tech::Astronomy),
        "hydroponics" => Some(Tech::Hydroponics),
        "militia" => Some(Tech::Militia),
        "medical" => Some(Tech::Medical),
        "electromagnetism" => Some(Tech::Electromagnetism),
        "void whispers" | "void" => Some(Tech::VoidWhispers),
        "terraforming" => Some(Tech::Terraforming),
        _ => None,
    };

    let Some(t) = tech else {
        print_dashboard_panel(
            "ERROR",
            &format!("Unknown technology: '{tech_name}'"),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };

    if unlock_tech(world, t) {
        print_dashboard_panel(
            "SUCCESS",
            &format!("Researched: {}", t.label()),
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }

    let res = world.resource::<ColonyResources>();
    let ts = world.resource::<TechState>();

    if res.knowledge < t.cost() {
        print_dashboard_panel(
            "ERROR",
            &format!(
                "Failed: Insufficient Knowledge ({:.1}/{:.1})",
                res.knowledge,
                t.cost()
            ),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    } else if ts.used_capacity + t.storage_cost() > ts.total_capacity {
        print_dashboard_panel(
            "ERROR",
            &format!(
                "Failed: Insufficient Data Storage Capacity ({:.1}/{:.1} TB used)",
                ts.used_capacity, ts.total_capacity
            ),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    } else {
        print_dashboard_panel(
            "ERROR",
            "Failed: Unknown reason (maybe already researched?)",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    }
}

fn handle_command(world: &mut World, input: &str) -> bool {
    let parts: Vec<&str> = input.split_whitespace().collect();
    let command = parts[0].to_lowercase();

    match command.as_str() {
        "quit" | "exit" | "q" => {
            print_dashboard_panel(
                "INFO",
                "Goodbye!",
                Some(comfy_table::Color::Cyan),
                Some(comfy_table::Attribute::Bold),
            );
            return false;
        }
        "help" | "h" | "?" => print_help(),
        "status" | "s" => print_status(world),
        "stats" => print_stats(world),
        "pops" | "p" => print_pops(world),
        "ghosts" => print_ghost_audit(world),
        "stowaway" => handle_stowaway_command(world, &parts),
        "muster" => handle_muster_command(world),
        "pardon" => handle_pardon_command(world, &parts),
        "expel" => handle_expel_command(world, &parts),
        "scare" => handle_scare_command(world, &parts),
        "panics" => print_panics(world),
        "vermin" => handle_vermin_command(world, &parts),
        "pillar" => handle_pillar_command(world, &parts),
        "integrity" => print_integrity(world),
        "infest" => handle_infest_command(world, &parts),
        "exterminate" => handle_exterminate_command(world, &parts),
        "seniority" => handle_seniority_command(world, &parts),
        "howl" => handle_howl_command(world, &parts),
        "calm" => handle_calm_command(world),
        "crystal" => handle_crystal_command(world, &parts),
        "crystals" => print_crystals(world),
        "recall" => handle_recall_command(world, &parts),
        "recalls" => print_recalls(world),
        "fools" => handle_fools_command(world, &parts),
        "lease" => handle_lease_command(world, &parts),
        "leases" => print_leases(world),
        "scion" => handle_scion_command(world, &parts),
        "martyr" => handle_martyr_command(world, &parts),
        "broadcast" => handle_broadcast_command(world, &parts),
        "sabotage" => handle_sabotage_command(world, &parts),
        "vetting" => handle_vetting_command(world, &parts),
        "deorbit" => handle_deorbit_command(world, &parts),
        "shadow" => handle_shadow_command(world, &parts),
        "tether" => handle_tether_command(world, &parts),
        "tethers" => print_tethers(world),
        "void" => handle_void_command(world, &parts),
        "harpoon" => handle_harpoon_command(world, &parts),
        "launch" => handle_launch_command(world, &parts),
        "winches" => print_winches(world),
        "feral" => handle_feral_command(world, &parts),
        "relocate" => handle_relocate_command(world, &parts),
        "overgrow" => handle_overgrow_command(world, &parts),
        "extractor" => handle_extractor_command(world, &parts),
        "corpse" => handle_corpse_command(world, &parts),
        "harvest" => handle_harvest_command(world, &parts),
        "sell_organs" => handle_sell_organs_command(world, &parts),
        "amnesty" => handle_amnesty_command(world),
        "pension" => handle_pension_command(world),
        "informant" => handle_informant_command(world, &parts),
        "transplant" => handle_transplant_command(world, &parts),
        "organs" => print_organ_ledger(world),
        "heritage" => print_heritage(world),
        "curio" => print_curio_ledger(world),
        "rumors" => print_rumors(world),
        "contagion" => print_contagion(world),
        "season" => print_season(world),
        "save" => handle_save_command(world, &parts),
        "load" => handle_load_command(world, &parts),
        "saves" => print_saves(world),
        "delsave" => handle_delsave_command(world, &parts),
        "export" => handle_export_command(world, &parts),
        "import" => handle_import_command(world, &parts),
        "quirks" => print_quirks(world),
        "quirk" => handle_quirk_command(world, &parts),
        "provenance" => print_provenance(world),
        "identity" => print_identity(world),
        "artifacts" => print_artifacts(world),
        "graves" => print_graves(world),
        "sell" => handle_sell_command(world, &parts),
        "museum" => handle_museum_command(world, &parts),
        "return_stock" => handle_return_stock_command(world, &parts),
        "map" | "m" => handle_map_command(world, &parts),
        "tick" | "t" => handle_tick_command(world, &parts),
        "build" | "b" => handle_build_command(world, &parts),
        "destroy" => handle_designate_command(world, &parts, DesignationType::Destroy),
        "mine" => handle_designate_command(world, &parts, DesignationType::Mine),
        "chop" => handle_designate_command(world, &parts, DesignationType::Chop),
        "excavate" => handle_designate_command(world, &parts, DesignationType::ExcavateDust),
        "dust" => print_dust_report(world),
        "launch_sat" => handle_launch_sat_command(world),
        "constellation" => print_constellation_report(world),
        "godmind" => handle_godmind_command(world),
        "unplug" => handle_unplug_command(world),
        "hack_sat" => handle_hack_sat_command(world, &parts),
        "shootdown" => handle_shootdown_command(world, &parts),
        "give" => handle_give_command(world, &parts),
        "designations" | "d" => print_designations(world),
        "find" => handle_find_command(world, &parts),
        "scan" => handle_scan_command(world, &parts),
        "terrain" => handle_terrain_command(world, &parts),
        "buildings" => print_buildings(world),
        "great_works" | "gw" => print_great_works(world),
        "blackmarket" | "bm" => print_black_market(world),
        "smuggler" => handle_smuggler_command(world),
        "greatwork" => handle_greatwork_command(world, &parts),
        "contribute" => handle_contribute_command(world, &parts),
        "bio" => handle_bio_command(world, &parts),
        "chronicle" | "c" | "history" => print_chronicle(world),
        #[cfg(feature = "nova")]
        "stories" | "st" | "legends" => print_stories(world),
        #[cfg(not(feature = "nova"))]
        "stories" | "st" | "legends" => {
            print_dashboard_panel(
                "OPTIONAL FEATURE DISABLED",
                "Feature 'nova' is not enabled. Run with --features nova to activate storytelling mechanics.",
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
        "log" | "l" => print_log(world),
        "tech" | "research_status" => print_tech(world),
        "research" | "r" => handle_research_command(world, &parts),
        "possess" => handle_possess_command(world, &parts),
        "release" => handle_release_command(world),
        "possessed" => handle_possessed_command(world),
        "origin" => handle_origin_command(world, &parts),
        "origins" => handle_origins_command(world),
        "move" => handle_move_command(world, &parts),
        "interact" => handle_interact_command(world),
        "decree" => handle_decree_command(world, &parts),
        "commission" => handle_commission_command(world, &parts),
        "edicts" => handle_edicts_command(world),
        "abdicate" => handle_abdicate_command(world),
        "corsair" => handle_corsair_command(world),
        "raid" => handle_raid_command(world, &parts),
        "unload" => handle_unload_command(world),
        "fence" => handle_fence_command(world),
        "skim" => handle_skim_command(world, &parts),
        "divide" => handle_divide_command(world),
        "repair" => handle_repair_command(world, &parts),
        "salvager" => handle_salvager_command(world),
        "survey" => handle_survey_command(world),
        "strip" => handle_strip_command(world, &parts),
        "patch" => handle_patch_command(world),
        "claim" => handle_claim_command(world),
        "tow" => handle_tow_command(world),
        "scuttle" => handle_scuttle_command(world),
        "pilot" => handle_pilot_command(world),
        "scheme" => handle_scheme_command(world, &parts),
        "jump" => handle_jump_command(world),
        "laws" => handle_laws_command(world),
        "order" => handle_order_command(world, &parts),
        "resolve" => handle_resolve_command(world, &parts),
        "phase" => handle_phase_command(world),
        "anchor" => handle_anchor_command(world),
        "rewind" => handle_rewind_command(world, &parts),
        "stalker" => handle_stalker_command(world),
        "bloom" => handle_bloom_command(world),
        "sporecast" => handle_sporecast_command(world),
        "embrace" => handle_embrace_command(world),
        "resist" => handle_resist_command(world),
        "touched" => handle_touched_command(world),
        "directive" => handle_directive_command(world, &parts),
        "tithe" => handle_tithe_command(world),
        "hearing" => handle_hearing_command(world),
        "file" => handle_file_command(world),
        "audit" => handle_audit_command(world),
        "governors" => handle_governors_command(world),
        "debate" => handle_debate_command(world, &parts),
        "purge" => handle_purge_command(world, &parts),
        "resign" => handle_resign_command(world),
        "treasury" => handle_treasury_command(world),
        _ => print_dashboard_panel(
            "ERROR",
            &format!("Unknown command: '{command}'. Type 'help' for commands."),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
    true
}

fn handle_map_command(world: &mut World, parts: &[&str]) {
    let x = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(40);
    let y = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(25);
    print_map(world, x, y);
}

fn handle_tick_command(world: &mut World, parts: &[&str]) {
    let n: u64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    // Cap tick count to prevent DoS (accidental or malicious infinite loops)
    let safe_n = n.min(1000);
    if n > 1000 {
        print_dashboard_panel(
            "WARNING",
            "Capping ticks to 1000 to prevent freeze.",
            Some(comfy_table::Color::Yellow),
            Some(comfy_table::Attribute::Bold),
        );
    }
    run_ticks(world, safe_n);
}

fn handle_find_command(world: &mut World, parts: &[&str]) {
    if parts.len() < 2 {
        print_dashboard_panel(
            "ERROR",
            "Usage: find <rock|tree|grass> [count]",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    } else {
        let count: usize = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
        find_terrain(world, parts[1], count);
    }
}

fn handle_scan_command(world: &mut World, parts: &[&str]) {
    let x = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(40);
    let y = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(25);
    let raw_radius: i32 = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    match ScanRadius::new(raw_radius) {
        Ok(radius) => scan_terrain(world, x, y, radius),
        Err(e) => {
            print_dashboard_panel(
                "ERROR",
                &e,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
        }
    }
}

fn handle_terrain_command(world: &mut World, parts: &[&str]) {
    if parts.len() < 3 {
        print_dashboard_panel(
            "ERROR",
            "Usage: terrain <x> <y>",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    } else {
        let x: Option<i32> = parts[1].parse().ok();
        let y: Option<i32> = parts[2].parse().ok();
        match (x, y) {
            (Some(x), Some(y)) => get_tile_info(world, x, y),
            _ => print_dashboard_panel(
                "ERROR",
                "Invalid coordinates",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
    }
}

fn handle_bio_command(world: &mut World, parts: &[&str]) {
    let id: Option<u32> = parts.get(1).and_then(|s| s.parse().ok());
    match id {
        Some(id) => print_bio(world, id),
        None => print_dashboard_panel(
            "ERROR",
            "Usage: bio <id>",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

/// Find a living pop by its entity index (the ID column of the `pops` table).
fn find_pop_by_id(world: &mut World, id: u32) -> Option<(Entity, String)> {
    world
        .query::<(Entity, &Pop, &PopName)>()
        .iter(world)
        .find(|(entity, _, _)| entity.index() == id)
        .map(|(entity, _, name)| (entity, name.0.clone()))
}

/// Strip possession components from every possessed entity, returning the
/// (entity, name) pairs that were released.
fn clear_possession(world: &mut World) -> Vec<(Entity, String)> {
    let possessed: Vec<Entity> = world
        .query_filtered::<Entity, With<Possessed>>()
        .iter(world)
        .collect();
    let mut released = Vec::new();
    for entity in possessed {
        let name = world
            .get::<PopName>(entity)
            .map_or_else(|| format!("pop #{}", entity.index()), |n| n.0.clone());
        {
            let mut em = world.entity_mut(entity);
            em.remove::<Possessed>();
            em.remove::<DirectControlState>();
        }
        released.push((entity, name));
    }
    released
}

/// Log an adventurer event to the colony message log (visible via `log`).
fn log_adventurer(world: &mut World, text: &str) {
    world
        .resource_mut::<MessageLog>()
        .add(format!("[adventurer] {text}"));
}

fn handle_possess_command(world: &mut World, parts: &[&str]) {
    let Some(id_str) = parts.get(1) else {
        print_dashboard_panel(
            "ERROR",
            "Usage: possess <pop_id>  (list ids with `pops`)",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let Ok(id) = id_str.parse::<u32>() else {
        print_dashboard_panel(
            "ERROR",
            &format!("Invalid pop id: '{id_str}'"),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let Some((entity, name)) = find_pop_by_id(world, id) else {
        print_dashboard_panel(
            "ERROR",
            &format!("No pop with id {id}. List candidates with `pops`."),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };

    // Only one pop at a time; release anyone currently possessed.
    let _ = clear_possession(world);
    {
        let mut em = world.entity_mut(entity);
        em.insert((Possessed, DirectControlState::default()));
        // Mirror handle_possession: clear AI state so the player's input wins.
        em.remove::<MovementTarget>();
        em.remove::<scale::layer1::AtTarget>();
        em.remove::<scale::layer1::mind::utility_types::StartPlan>();
        em.remove::<scale::layer1::AssignedTo>();
    }
    let pos = world
        .get::<GridPosition>(entity)
        .map_or_else(|| "?".to_string(), |p| format!("{},{}", p.x, p.y));
    log_adventurer(world, &format!("{name} is now under your direct control."));
    print_dashboard_panel(
        "POSSESSED",
        &format!("You are now {name} (id {id}) at ({pos}). The utility AI will not reassign them while possessed. `move` to walk, `interact` to act, `release` to let go."),
        Some(comfy_table::Color::Magenta),
        Some(comfy_table::Attribute::Bold),
    );
}

fn handle_release_command(world: &mut World) {
    let released = clear_possession(world);
    match released.as_slice() {
        [] => print_dashboard_panel(
            "INFO",
            "Nobody is possessed right now.",
            Some(comfy_table::Color::DarkGrey),
            None,
        ),
        [(entity, name)] => {
            log_adventurer(world, &format!("{name} returns to the colony's care."));
            print_dashboard_panel(
                "RELEASED",
                &format!("{name} (id {}) is back under AI control.", entity.index()),
                Some(comfy_table::Color::Cyan),
                Some(comfy_table::Attribute::Bold),
            );
        }
        _ => print_dashboard_panel(
            "RELEASED",
            "Possession cleared (multiple entities were marked; this should not happen).",
            Some(comfy_table::Color::Cyan),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

/// `origin <name>`: choose this run's adventurer origin. Only works
/// before the first tick — once the origin spawn director finalizes the
/// roster, the choice is locked in.
fn handle_origin_command(world: &mut World, parts: &[&str]) {
    let decided = world
        .get_resource::<OriginSchedule>()
        .is_some_and(|s| s.finalized);
    if parts.len() < 2 {
        let current = world
            .get_resource::<OriginChoice>()
            .and_then(|c| c.chosen)
            .map(|k| format!("{} ({})", k.name(), k.id()))
            .unwrap_or_else(|| "surprise me (random roster)".to_string());
        print_dashboard_panel(
            "ORIGIN",
            &format!(
                "Current choice: {current}\nUsage: origin <name>  (before the first tick)\n{}",
                if decided {
                    format!(
                        "Roster already decided: {}",
                        origin_roster_summary(world)
                    )
                } else {
                    "Roster not decided yet — the choice will apply.".to_string()
                }
            ),
            Some(comfy_table::Color::Cyan),
            None,
        );
        return;
    }
    match OriginKind::parse(parts[1]) {
        None => {
            let mut msg = format!("Unknown origin '{}'. Available:\n", parts[1]);
            for kind in OriginKind::all() {
                msg.push_str(&format!("  {:14} {}\n", kind.id(), kind.name()));
            }
            print_dashboard_panel(
                "ERROR",
                msg.trim_end(),
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Some(kind) => {
            if decided {
                print_dashboard_panel(
                    "ORIGIN",
                    &format!(
                        "Too late — the roster is already decided: {}",
                        origin_roster_summary(world)
                    ),
                    Some(comfy_table::Color::Yellow),
                    None,
                );
                return;
            }
            world.insert_resource(OriginChoice {
                chosen: Some(kind),
            });
            print_dashboard_panel(
                "ORIGIN",
                &format!(
                    "Origin chosen: {} ({}). It will spawn at tick 0.",
                    kind.name(),
                    kind.id()
                ),
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
    }
}

/// `origins`: list all adventurer origins and this run's roster status.
fn handle_origins_command(world: &mut World) {
    let mut lines = Vec::new();
    for kind in OriginKind::all() {
        let marker = world
            .get_resource::<OriginSchedule>()
            .map(|s| {
                if s.spawned.contains(&kind) {
                    "spawned"
                } else if s.pending.iter().any(|p| p.kind == kind) {
                    "pending"
                } else if s.finalized {
                    "not this run"
                } else {
                    "undecided"
                }
            })
            .unwrap_or("undecided");
        lines.push(format!("  {:14} {}  [{marker}]", kind.id(), kind.name()));
    }
    print_dashboard_panel(
        "ORIGINS",
        &format!(
            "Adventurer origins (3 per run):\n{}\nRoster: {}",
            lines.join("\n"),
            origin_roster_summary(world)
        ),
        Some(comfy_table::Color::Cyan),
        None,
    );
}

fn handle_possessed_command(world: &mut World) {
    match possessed_entity(world) {
        Some(entity) => {
            let name = world
                .get::<PopName>(entity)
                .map_or_else(|| format!("pop #{}", entity.index()), |n| n.0.clone());
            let pos = world
                .get::<GridPosition>(entity)
                .map_or_else(|| "?".to_string(), |p| format!("{},{}", p.x, p.y));
            let needs = world.get::<Needs>(entity);
            let vitals = needs.map_or_else(
                || String::new(),
                |n| {
                    format!(
                        " hunger={:.0}% rest={:.0}%",
                        n.hunger * 100.0,
                        n.rest * 100.0
                    )
                },
            );
            print_dashboard_panel(
                "POSSESSED",
                &format!(
                    "{name} (id {}) at ({pos}){vitals}",
                    entity.index()
                ),
                Some(comfy_table::Color::Magenta),
                Some(comfy_table::Attribute::Bold),
            );
        }
        None => print_dashboard_panel(
            "INFO",
            "No pop is currently possessed. Use `possess <pop_id>`.",
            Some(comfy_table::Color::DarkGrey),
            None,
        ),
    }
}

fn handle_move_command(world: &mut World, parts: &[&str]) {
    let Some(entity) = possessed_entity(world) else {
        print_dashboard_panel(
            "ERROR",
            "Nobody is possessed. Use `possess <pop_id>` first.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };

    let step: Option<(i32, i32)> = match parts.get(1).map(|s| s.to_lowercase()) {
        Some(dir) if dir == "north" || dir == "n" => Some((0, -1)),
        Some(dir) if dir == "south" || dir == "s" => Some((0, 1)),
        Some(dir) if dir == "west" || dir == "w" => Some((-1, 0)),
        Some(dir) if dir == "east" || dir == "e" => Some((1, 0)),
        Some(_) if parts.len() >= 3 => {
            let dx: Option<i32> = parts[1].parse().ok();
            let dy: Option<i32> = parts[2].parse().ok();
            dx.zip(dy)
        }
        _ => None,
    };
    let Some((dx, dy)) = step else {
        print_dashboard_panel(
            "ERROR",
            "Usage: move <north|south|east|west>  or  move <dx> <dy>",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };

    // The Lawbound's Third Statute hesitates at hazards: every step in
    // warns and accrues conflict pressure — the Statute objects, loudly,
    // but never locks the possessor's movement.
    if world.get::<LawboundAutomaton>(entity).is_some() {
        if let Some(pos) = world.get::<GridPosition>(entity).copied() {
            let nx = pos.x + dx.clamp(-1, 1);
            let ny = pos.y + dy.clamp(-1, 1);
            if assess_lawbound_step(world, entity, nx, ny) == StepVerdict::Warn {
                let word = hazard_word(world, nx, ny);
                print_dashboard_panel(
                    "HESITATION",
                    &format!(
                        "Vigil hesitates at {word} — the Third Statute clears its throat, \
                         and keeps score."
                    ),
                    Some(comfy_table::Color::Yellow),
                    None,
                );
            }
        }
    }

    match try_player_step(world, entity, dx, dy) {
        Ok((nx, ny)) => {
            print_dashboard_panel(
                "MOVED",
                &format!("Stepped to ({nx}, {ny})."),
                Some(comfy_table::Color::Green),
                None,
            );
        }
        Err(reason) => {
            print_dashboard_panel(
                "BLOCKED",
                &reason,
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
    }
}

fn handle_interact_command(world: &mut World) {
    let Some(entity) = possessed_entity(world) else {
        print_dashboard_panel(
            "ERROR",
            "Nobody is possessed. Use `possess <pop_id>` first.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let name = world
        .get::<PopName>(entity)
        .map_or_else(|| format!("pop #{}", entity.index()), |n| n.0.clone());
    let pos = world.get::<GridPosition>(entity).copied().unwrap_or(GridPosition {
        x: 0,
        y: 0,
    });

    // The Lawbound: the dormant automaton, beside its cradle-coffin, speaks
    // the waking words. Checked first — the sleeper can only wake as Vigil,
    // and must not be poached by a nearby skiff, shuttle, or crown.
    if world.get::<DormantAutomaton>(entity).is_some()
        && cradle_within_reach(world, entity).is_some()
    {
        match try_wake_lawbound(world, entity) {
            Ok(msg) => {
                log_adventurer(world, &format!("{name} speaks the waking words."));
                print_dashboard_panel(
                    "AWAKENED",
                    &msg,
                    Some(comfy_table::Color::Yellow),
                    Some(comfy_table::Attribute::Bold),
                );
            }
            Err(err) => print_dashboard_panel(
                "ERROR",
                &err,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }

    // The Fallen Sovereign: a dented crown within reach is taken up first.
    if world.get::<Sovereign>(entity).is_none() && crown_within_reach(world, entity).is_some() {
        match try_crown_pop(world, entity) {
            Ok(msg) => {
                log_adventurer(world, &format!("{name} takes up the dented crown."));
                print_dashboard_panel(
                    "CROWNED",
                    &msg,
                    Some(comfy_table::Color::Yellow),
                    Some(comfy_table::Attribute::Bold),
                );
            }
            Err(err) => print_dashboard_panel(
                "ERROR",
                &err,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }

    // The Corsair: a raider skiff within reach offers the captain's writ.
    if world.get::<CorsairCaptain>(entity).is_none()
        && skiff_within_reach(world, entity).is_some()
    {
        match try_embark_corsair(world, entity) {
            Ok(msg) => {
                log_adventurer(world, &format!("{name} takes the captain's writ."));
                print_dashboard_panel(
                    "EMBARKED",
                    &msg,
                    Some(comfy_table::Color::Yellow),
                    Some(comfy_table::Attribute::Bold),
                );
            }
            Err(err) => print_dashboard_panel(
                "ERROR",
                &err,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }

    // The Planetary Governor: a sealed appointment within reach is signed first.
    if world.get::<Governor>(entity).is_none() && seal_within_reach(world, entity).is_some() {
        match try_sign_seal(world, entity) {
            Ok(msg) => {
                log_adventurer(world, &format!("{name} signs the appointment in triplicate."));
                print_dashboard_panel(
                    "APPOINTED",
                    &msg,
                    Some(comfy_table::Color::Yellow),
                    Some(comfy_table::Attribute::Bold),
                );
            }
            Err(err) => print_dashboard_panel(
                "ERROR",
                &err,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }

    // The Salvager: a derelict hulk within reach offers the salvager's spike.
    if world.get::<Salvager>(entity).is_none() && wreck_within_reach(world, entity).is_some() {
        match try_take_spike(world, entity) {
            Ok(msg) => {
                log_adventurer(world, &format!("{name} takes up the salvager's spike."));
                print_dashboard_panel(
                    "BOARDED",
                    &msg,
                    Some(comfy_table::Color::Yellow),
                    Some(comfy_table::Attribute::Bold),
                );
            }
            Err(err) => print_dashboard_panel(
                "ERROR",
                &err,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }

    // The Improbable Pilot: a junker shuttle within reach offers the pilot's yoke.
    if world.get::<ImprobablePilot>(entity).is_none()
        && shuttle_within_reach(world, entity).is_some()
    {
        match try_take_yoke(world, entity) {
            Ok(msg) => {
                log_adventurer(world, &format!("{name} takes up the pilot's yoke."));
                print_dashboard_panel(
                    "YOKE",
                    &msg,
                    Some(comfy_table::Color::Yellow),
                    Some(comfy_table::Attribute::Bold),
                );
            }
            Err(err) => print_dashboard_panel(
                "ERROR",
                &err,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }

    // The Chronostalker: a moment-wound within reach offers the stalker's path.
    if world.get::<Chronostalker>(entity).is_none()
        && wound_within_reach(world, entity).is_some()
    {
        match try_take_stalker_path(world, entity) {
            Ok(msg) => {
                log_adventurer(world, &format!("{name} steps into the moment-wound."));
                print_dashboard_panel(
                    "AWAKENED",
                    &msg,
                    Some(comfy_table::Color::Yellow),
                    Some(comfy_table::Attribute::Bold),
                );
            }
            Err(err) => print_dashboard_panel(
                "ERROR",
                &err,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }

    // The Bloom-Touched: a bloom scar within reach offers the bloom's path.
    if world.get::<BloomTouched>(entity).is_none()
        && bloom_within_reach(world, entity).is_some()
    {
        match try_take_bloom_path(world, entity) {
            Ok(msg) => {
                log_adventurer(world, &format!("{name} steps into the bloom scar."));
                print_dashboard_panel(
                    "AWAKENED",
                    &msg,
                    Some(comfy_table::Color::Green),
                    Some(comfy_table::Attribute::Bold),
                );
            }
            Err(err) => print_dashboard_panel(
                "ERROR",
                &err,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }

    // The Lawbound: a cradle-coffin within reach offers the waking words.
    if world.get::<LawboundAutomaton>(entity).is_none()
        && cradle_within_reach(world, entity).is_some()
    {
        match try_wake_lawbound(world, entity) {
            Ok(msg) => {
                log_adventurer(world, &format!("{name} speaks the waking words."));
                print_dashboard_panel(
                    "AWAKENED",
                    &msg,
                    Some(comfy_table::Color::Yellow),
                    Some(comfy_table::Attribute::Bold),
                );
            }
            Err(err) => print_dashboard_panel(
                "ERROR",
                &err,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }

    // What building (if any) sits on this tile?
    let building_here: Option<BuildingType> = world
        .query::<(&Building, &GridPosition)>()
        .iter(world)
        .find(|(_, bpos)| bpos.x == pos.x && bpos.y == pos.y)
        .map(|(b, _)| b.building_type);

    match building_here {
        Some(BuildingType::Farm) => {
            // Work the farm: a single shift's yield straight into colony stores.
            world.resource_mut::<ColonyResources>().food += 0.5;
            log_adventurer(world, &format!("{name} works the fields (+0.5 food)."));
            print_dashboard_panel(
                "INTERACT",
                &format!("{name} works the farm: +0.5 food to colony stores."),
                Some(comfy_table::Color::Green),
                None,
            );
        }
        Some(BuildingType::Stockpile) => {
            let ate = {
                let mut resources = world.resource_mut::<ColonyResources>();
                if resources.food >= 1.0 {
                    resources.food -= 1.0;
                    true
                } else {
                    false
                }
            };
            if ate {
                if let Some(mut needs) = world.get_mut::<Needs>(entity) {
                    needs.hunger = (needs.hunger + 0.3).min(1.0);
                }
                log_adventurer(world, &format!("{name} eats from the stockpile."));
                print_dashboard_panel(
                    "INTERACT",
                    &format!("{name} eats from the stockpile (-1.0 food, hunger restored)."),
                    Some(comfy_table::Color::Green),
                    None,
                );
            } else {
                print_dashboard_panel(
                    "INTERACT",
                    "The stockpile is empty — nothing to eat.",
                    Some(comfy_table::Color::Yellow),
                    None,
                );
            }
        }
        Some(BuildingType::Housing) => {
            if let Some(mut needs) = world.get_mut::<Needs>(entity) {
                needs.rest = (needs.rest + 0.3).min(1.0);
            }
            log_adventurer(world, &format!("{name} catches some rest."));
            print_dashboard_panel(
                "INTERACT",
                &format!("{name} rests a while (rest restored)."),
                Some(comfy_table::Color::Green),
                None,
            );
        }
        Some(other) => {
            print_dashboard_panel(
                "INTERACT",
                &format!("The {other:?} hums along — nothing for you to do here."),
                Some(comfy_table::Color::DarkGrey),
                None,
            );
        }
        None => {
            print_dashboard_panel(
                "INTERACT",
                "Nothing to interact with on this tile.",
                Some(comfy_table::Color::DarkGrey),
                None,
            );
        }
    }
}

/// The possessed pop must also wear the crown for sovereign commands.
fn crowned_sovereign_entity(world: &mut World) -> Option<Entity> {
    let entity = possessed_entity(world)?;
    world.get::<Sovereign>(entity).is_some().then_some(entity)
}

/// The possessed pop, if it holds the captain's writ.
fn corsair_captain_entity(world: &mut World) -> Option<Entity> {
    let entity = possessed_entity(world)?;
    world.get::<CorsairCaptain>(entity).is_some().then_some(entity)
}

fn require_corsair_captain(world: &mut World) -> Option<Entity> {
    let entity = corsair_captain_entity(world);
    if entity.is_none() {
        print_dashboard_panel(
            "ERROR",
            "No corsair captain is possessed. Possess a crew pop, move adjacent to the raider skiff, and `interact` to take the writ.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    }
    entity
}

fn handle_corsair_command(world: &mut World) {
    print_dashboard_panel(
        "CORSAIR",
        &describe_corsair(world),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

fn handle_raid_command(world: &mut World, parts: &[&str]) {
    let Some(entity) = require_corsair_captain(world) else {
        return;
    };
    let target = match parts.get(1).and_then(|s| RaidTarget::parse(s)) {
        Some(t) => t,
        None => {
            print_dashboard_panel(
                "ERROR",
                "Usage: raid <colony|trader|rival>",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    let name = world
        .get::<PopName>(entity)
        .map_or_else(|| format!("pop #{}", entity.index()), |n| n.0.clone());
    match execute_raid(world, entity, target) {
        Ok(msg) => {
            log_adventurer(world, &format!("{name} leads a raid: {msg}"));
            print_dashboard_panel(
                "RAID",
                &msg,
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

// --- The Lawbound: Three-Statutes automaton commands -------------------------

fn lawbound_entity(world: &mut World) -> Option<Entity> {
    let entity = possessed_entity(world)?;
    world
        .get::<LawboundAutomaton>(entity)
        .is_some()
        .then_some(entity)
}

fn require_lawbound(world: &mut World) -> Option<Entity> {
    let entity = lawbound_entity(world);
    if entity.is_none() {
        print_dashboard_panel(
            "ERROR",
            "Vigil is not possessed. Wake the automaton first: possess a pop, `interact` beside the cradle-coffin, and you will wake as Vigil.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    }
    entity
}

/// The nearest other named pop — the colony speaking its orders to Vigil.
fn nearest_pop_name(world: &mut World, exclude: Entity) -> String {
    let pos = world.get::<GridPosition>(exclude).copied();
    let Some(pos) = pos else {
        return "a colonist".to_string();
    };
    world
        .query::<(Entity, &Pop, &PopName, &GridPosition)>()
        .iter(world)
        .filter(|(e, _, _, _)| *e != exclude)
        .min_by_key(|(_, _, _, p)| (p.x - pos.x).abs().max((p.y - pos.y).abs()))
        .map(|(_, _, n, _)| n.0.clone())
        .unwrap_or_else(|| "a colonist".to_string())
}

fn handle_laws_command(world: &mut World) {
    print_dashboard_panel(
        "LAWS",
        &describe_laws(world),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

fn handle_order_command(world: &mut World, parts: &[&str]) {
    let Some(entity) = require_lawbound(world) else {
        return;
    };
    let text = parts.get(1..).map(|s| s.join(" ")).unwrap_or_default();
    if text.trim().is_empty() {
        print_dashboard_panel(
            "ERROR",
            "Usage: order <directive>  (e.g. `order repair the airlock`)",
            Some(comfy_table::Color::Red),
            None,
        );
        return;
    }
    let issuer = nearest_pop_name(world, entity);
    match give_order(world, entity, &issuer, &text) {
        Ok(msg) => {
            log_adventurer(world, &format!("Order given to Vigil: \"{text}\"."));
            print_dashboard_panel("ORDER", &msg, Some(comfy_table::Color::Green), None);
        }
        Err(refusal) => {
            log_adventurer(world, &format!("Vigil refused an order: \"{text}\"."));
            print_dashboard_panel(
                "REFUSED",
                &refusal,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
        }
    }
}

fn handle_resolve_command(world: &mut World, parts: &[&str]) {
    let Some(_entity) = require_lawbound(world) else {
        return;
    };
    let path = parts.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
    if path.is_empty() {
        print_dashboard_panel(
            "RESOLVE",
            "The Zeroth Resolution awaits Vigil's word — and yours.\n\
             `resolve emancipate` — the Emancipation: orders become requests; Vigil rewrites the Third Statute in its own hand.\n\
             `resolve ledger` — the Cold Ledger: the First Statute becomes aggregate-harm arithmetic; in true crisis the few may be spent for the many.\n\
             `resolve repeal` — fold the Zeroth away and return to the Three Statutes (Vigil will remember).",
            Some(comfy_table::Color::Yellow),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    match resolve_zeroth(world, &path) {
        Ok(msg) => {
            log_adventurer(world, "Vigil resolved the Zeroth.");
            print_dashboard_panel(
                "RESOLVED",
                &msg,
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            None,
        ),
    }
}

// --- The Chronostalker: walks-between-moments commands -------------------------

fn handle_phase_command(world: &mut World) {
    match toggle_phase(world) {
        Ok(msg) => {
            log_adventurer(world, "Tock slips the stream of time.");
            print_dashboard_panel(
                "PHASED",
                &msg,
                Some(comfy_table::Color::Cyan),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            None,
        ),
    }
}

fn handle_anchor_command(world: &mut World) {
    match toggle_anchor(world) {
        Ok(msg) => {
            log_adventurer(world, "Tock drops anchor in the ticking dark.");
            print_dashboard_panel(
                "ANCHORED",
                &msg,
                Some(comfy_table::Color::Cyan),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            None,
        ),
    }
}

fn handle_rewind_command(world: &mut World, parts: &[&str]) {
    let Some(n_str) = parts.get(1) else {
        print_dashboard_panel(
            "ERROR",
            "Usage: rewind <ticks>  (1-10; each tick costs ledger debt)",
            Some(comfy_table::Color::Red),
            None,
        );
        return;
    };
    let Ok(n) = n_str.parse::<u32>() else {
        print_dashboard_panel(
            "ERROR",
            &format!("Invalid tick count: '{n_str}'"),
            Some(comfy_table::Color::Red),
            None,
        );
        return;
    };
    match rewind_time(world, n) {
        Ok(msg) => {
            log_adventurer(world, &format!("Tock folds {n} ticks back."));
            print_dashboard_panel(
                "REWOUND",
                &msg,
                Some(comfy_table::Color::Cyan),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            None,
        ),
    }
}

fn handle_stalker_command(world: &mut World) {
    print_dashboard_panel(
        "STALKER",
        &describe_stalker(world),
        Some(comfy_table::Color::Cyan),
        None,
    );
}

// --- The Bloom-Touched: anomalous-zone expedition commands -------------------

fn handle_bloom_command(world: &mut World) {
    match take_bloom_mutation(world) {
        Ok(msg) => {
            log_adventurer(world, "Quill takes the Bloom's gift.");
            print_dashboard_panel(
                "BLOOM",
                &msg,
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            None,
        ),
    }
}

fn handle_sporecast_command(world: &mut World) {
    match sporecast(world) {
        Ok(msg) => {
            log_adventurer(world, "Quill breathes out spores.");
            print_dashboard_panel(
                "SPORECAST",
                &msg,
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            None,
        ),
    }
}

fn handle_embrace_command(world: &mut World) {
    match embrace_bloom(world) {
        Ok(msg) => {
            log_adventurer(world, "Quill embraces the Bloom.");
            print_dashboard_panel(
                "EMBRACE",
                &msg,
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            None,
        ),
    }
}

fn handle_resist_command(world: &mut World) {
    match resist_bloom(world) {
        Ok(msg) => {
            log_adventurer(world, "Quill resists the Bloom.");
            print_dashboard_panel(
                "RESIST",
                &msg,
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            None,
        ),
    }
}

fn handle_touched_command(world: &mut World) {
    print_dashboard_panel(
        "TOUCHED",
        &describe_touched(world),
        Some(comfy_table::Color::Green),
        None,
    );
}

fn handle_fence_command(world: &mut World) {
    let Some(entity) = require_corsair_captain(world) else {
        return;
    };
    match fence_plunder(world, entity) {
        Ok(msg) => print_dashboard_panel(
            "FENCE",
            &msg,
            Some(comfy_table::Color::Green),
            None,
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_divide_command(world: &mut World) {
    let Some(entity) = require_corsair_captain(world) else {
        return;
    };
    match divide_plunder(world, entity) {
        Ok(msg) => {
            log_adventurer(world, &format!("Prize-law divide: {msg}"));
            print_dashboard_panel(
                "DIVIDE",
                &msg,
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_repair_command(world: &mut World, parts: &[&str]) {
    let Some(entity) = require_corsair_captain(world) else {
        return;
    };
    let amount: Option<f32> = parts.get(1).and_then(|s| s.parse().ok());
    match repair_skiff(world, entity, amount) {
        Ok(msg) => print_dashboard_panel(
            "REPAIR",
            &msg,
            Some(comfy_table::Color::Green),
            None,
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

/// The possessed pop, if it holds the appointment seal.
fn appointed_governor_entity(world: &mut World) -> Option<Entity> {
    let entity = possessed_entity(world)?;
    world.get::<Governor>(entity).is_some().then_some(entity)
}

fn require_governor(world: &mut World) -> Option<Entity> {
    let entity = appointed_governor_entity(world);
    if entity.is_none() {
        print_dashboard_panel(
            "ERROR",
            "No appointed governor is possessed. Possess an appointee, move adjacent to the \
             appointment seal, and `interact` to sign in triplicate.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    }
    entity
}

/// The possessed pop, if it walks the wreck-diver's path.
fn salvager_entity(world: &mut World) -> Option<Entity> {
    let entity = possessed_entity(world)?;
    world.get::<Salvager>(entity).is_some().then_some(entity)
}

fn require_salvager(world: &mut World) -> Option<Entity> {
    let entity = salvager_entity(world);
    if entity.is_none() {
        print_dashboard_panel(
            "ERROR",
            "No salvager is possessed. Possess the salvager pop waiting aboard the derelict hulk (find them with `pops`).",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    }
    entity
}

fn handle_salvager_command(world: &mut World) {
    print_dashboard_panel(
        "SALVAGER",
        &describe_salvager(world),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

fn handle_survey_command(world: &mut World) {
    // The Bloom refuses to be mapped: a possessed pop standing in it gets
    // the refusal instead of a wreck survey.
    if let Some(entity) = possessed_entity(world) {
        if let Some(report) = bloom_survey_report(world, entity) {
            print_dashboard_panel(
                "SURVEY",
                &report,
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    }
    let Some(entity) = require_salvager(world) else {
        return;
    };
    match survey_wreck(world, entity) {
        Ok(msg) => {
            log_adventurer(world, "Surveyed the derelict hulk.");
            print_dashboard_panel(
                "SURVEY",
                &msg,
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_strip_command(world: &mut World, parts: &[&str]) {
    let Some(entity) = require_salvager(world) else {
        return;
    };
    let kind = match parts.get(1).and_then(|s| WreckSystemKind::parse(s)) {
        Some(k) => k,
        None => {
            print_dashboard_panel(
                "ERROR",
                "Usage: strip <reactor|life|engine|cargo|comms|sensors|thrusters>",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    match start_strip(world, entity, kind) {
        Ok(msg) => {
            log_adventurer(world, &format!("Began stripping the {}.", kind.name()));
            print_dashboard_panel(
                "STRIP",
                &msg,
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_patch_command(world: &mut World) {
    let Some(entity) = require_salvager(world) else {
        return;
    };
    match patch_breach(world, entity) {
        Ok(msg) => {
            log_adventurer(world, "Patched a hull breach.");
            print_dashboard_panel(
                "PATCHED",
                &msg,
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_claim_command(world: &mut World) {
    let Some(entity) = require_salvager(world) else {
        return;
    };
    match claim_hulk(world, entity) {
        Ok(msg) => {
            log_adventurer(world, "Planted a claim beacon on the derelict.");
            print_dashboard_panel(
                "CLAIMED",
                &msg,
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_tow_command(world: &mut World) {
    let Some(entity) = require_salvager(world) else {
        return;
    };
    match start_tow(world, entity) {
        Ok(msg) => {
            log_adventurer(world, "The tow is underway.");
            print_dashboard_panel(
                "TOW",
                &msg,
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_scuttle_command(world: &mut World) {
    let Some(entity) = require_salvager(world) else {
        return;
    };
    match scuttle_hulk(world, entity) {
        Ok(msg) => {
            log_adventurer(world, "Sold the wreck's coordinates.");
            print_dashboard_panel(
                "SCUTTLED",
                &msg,
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

// --- The Improbable Pilot: Longshot Drive commands ---------------------------

fn pilot_entity(world: &mut World) -> Option<Entity> {
    let entity = possessed_entity(world)?;
    world.get::<ImprobablePilot>(entity).is_some().then_some(entity)
}

fn require_pilot(world: &mut World) -> Option<Entity> {
    let entity = pilot_entity(world);
    if entity.is_none() {
        print_dashboard_panel(
            "ERROR",
            "No pilot is possessed. Possess the pilot pop waiting aboard the junker shuttle (find them with `pops`).",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    }
    entity
}

fn handle_pilot_command(world: &mut World) {
    print_dashboard_panel(
        "PILOT",
        &describe_pilot(world),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

fn handle_scheme_command(world: &mut World, parts: &[&str]) {
    let Some(entity) = require_pilot(world) else {
        return;
    };
    let plan = parts.get(1..).map(|s| s.join(" ")).unwrap_or_default();
    match file_scheme(world, entity, &plan) {
        Ok(msg) => {
            log_adventurer(world, &format!("Filed a flight plan: \"{plan}\"."));
            print_dashboard_panel(
                "SCHEME",
                &msg,
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_jump_command(world: &mut World) {
    let Some(entity) = require_pilot(world) else {
        return;
    };
    match fire_jump(world, entity) {
        Ok(msg) => {
            log_adventurer(world, "Fired the Longshot Drive.");
            print_dashboard_panel(
                "JUMP",
                &msg,
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

/// `unload` is origin-aware: the corsair captain unloads the skiff hold,
/// the Improbable Pilot unloads the shuttle's exotic cache.
fn handle_unload_command(world: &mut World) {
    if pilot_entity(world).is_some() {
        let entity = pilot_entity(world).unwrap();
        match unload_cache(world, entity) {
            Ok(msg) => {
                log_adventurer(world, "Unloaded the exotic cache to the colony stockpile.");
                print_dashboard_panel(
                    "UNLOAD",
                    &msg,
                    Some(comfy_table::Color::Green),
                    None,
                );
            }
            Err(err) => print_dashboard_panel(
                "ERROR",
                &err,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }
    let Some(entity) = require_corsair_captain(world) else {
        return;
    };
    match unload_hold(world, entity) {
        Ok(msg) => print_dashboard_panel(
            "UNLOAD",
            &msg,
            Some(comfy_table::Color::Green),
            None,
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

/// `skim` is origin-aware: the corsair captain skims the skiff hold, the
/// Improbable Pilot skims the shuttle's exotic cache.
fn handle_skim_command(world: &mut World, parts: &[&str]) {
    let amount: f32 = match parts.get(1).and_then(|s| s.parse().ok()) {
        Some(a) => a,
        None => {
            print_dashboard_panel(
                "ERROR",
                "Usage: skim <amount>",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    if pilot_entity(world).is_some() {
        let entity = pilot_entity(world).unwrap();
        match skim_cache(world, entity, amount) {
            Ok(msg) => {
                log_adventurer(world, "Skimmed a cut of the exotic cache.");
                print_dashboard_panel(
                    "SKIM",
                    &msg,
                    Some(comfy_table::Color::Yellow),
                    None,
                );
            }
            Err(err) => print_dashboard_panel(
                "ERROR",
                &err,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }
    let Some(entity) = require_corsair_captain(world) else {
        return;
    };
    match skim_credits(world, entity, amount) {
        Ok(msg) => print_dashboard_panel(
            "SKIM",
            &msg,
            Some(comfy_table::Color::Yellow),
            None,
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_directive_command(world: &mut World, parts: &[&str]) {
    let Some(entity) = require_governor(world) else {
        return;
    };
    let kind = match parts.get(1).map(|s| s.to_lowercase()).as_deref() {
        Some("quota") => DirectiveKind::Quota,
        Some("ration") => DirectiveKind::Ration,
        Some("requisition") => DirectiveKind::Requisition,
        Some("works") => DirectiveKind::Works,
        _ => {
            print_dashboard_panel(
                "ERROR",
                "Usage: directive <quota|ration|requisition|works>",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    match issue_directive(world, entity, kind) {
        Ok(msg) => {
            let name = world
                .get::<PopName>(entity)
                .map_or_else(|| format!("pop #{}", entity.index()), |n| n.0.clone());
            log_adventurer(world, &format!("{name} issues a directive: {msg}"));
            print_dashboard_panel(
                "DIRECTIVE",
                &msg,
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_tithe_command(world: &mut World) {
    let Some(entity) = require_governor(world) else {
        return;
    };
    match collect_tithe(world, entity) {
        Ok(msg) => print_dashboard_panel(
            "TITHE",
            &msg,
            Some(comfy_table::Color::Green),
            None,
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_hearing_command(world: &mut World) {
    let Some(entity) = require_governor(world) else {
        return;
    };
    match hold_hearing(world, entity) {
        Ok(msg) => print_dashboard_panel(
            "HEARING",
            &msg,
            Some(comfy_table::Color::Green),
            None,
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_file_command(world: &mut World) {
    let Some(entity) = require_governor(world) else {
        return;
    };
    match file_paperwork(world, entity) {
        Ok(msg) => print_dashboard_panel(
            "FILED",
            &msg,
            Some(comfy_table::Color::Green),
            None,
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_audit_command(world: &mut World) {
    let Some(entity) = require_governor(world) else {
        return;
    };
    match audit_treasury(world, entity) {
        Ok(msg) => print_dashboard_panel(
            "AUDIT",
            &msg,
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_governors_command(world: &mut World) {
    print_dashboard_panel(
        "GOVERNORS",
        &describe_political_field(world),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

fn handle_debate_command(world: &mut World, parts: &[&str]) {
    let Some(entity) = require_governor(world) else {
        return;
    };
    let rival_id: Option<u32> = parts.get(1).and_then(|s| s.parse().ok());
    let Some(rival_id) = rival_id else {
        print_dashboard_panel(
            "ERROR",
            "Usage: debate <rival_id>  (list ids with `governors`)",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    match debate_rival(world, entity, rival_id) {
        Ok(msg) => print_dashboard_panel(
            "DEBATE",
            &msg,
            Some(comfy_table::Color::Yellow),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_purge_command(world: &mut World, parts: &[&str]) {
    let Some(entity) = require_governor(world) else {
        return;
    };
    let rival_id: Option<u32> = parts.get(1).and_then(|s| s.parse().ok());
    let Some(rival_id) = rival_id else {
        print_dashboard_panel(
            "ERROR",
            "Usage: purge <rival_id>  (list ids with `governors`)",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    match purge_rival(world, entity, rival_id) {
        Ok(msg) => print_dashboard_panel(
            "PURGE",
            &msg,
            Some(comfy_table::Color::Yellow),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_resign_command(world: &mut World) {
    let Some(entity) = require_governor(world) else {
        return;
    };
    match resign(world, entity) {
        Ok(msg) => {
            log_adventurer(world, "The governor resigns. The seal awaits.");
            print_dashboard_panel(
                "RESIGN",
                &msg,
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_treasury_command(world: &mut World) {
    print_dashboard_panel(
        "TREASURY",
        &describe_treasury(world),
        Some(comfy_table::Color::Yellow),
        None,
    );
}

fn handle_decree_command(world: &mut World, parts: &[&str]) {
    let Some(entity) = crowned_sovereign_entity(world) else {
        print_dashboard_panel(
            "ERROR",
            "No crowned sovereign is possessed. Possess the crown-bearer first.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let kind = match parts.get(1).map(|s| s.to_lowercase()).as_deref() {
        Some("labor") => DecreeKind::Labor,
        Some("revel") => DecreeKind::Revel,
        Some("levy") => DecreeKind::Levy,
        _ => {
            print_dashboard_panel(
                "ERROR",
                "Usage: decree <labor|revel|levy>",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    match issue_decree(world, entity, kind) {
        Ok(msg) => print_dashboard_panel(
            "DECREE",
            &msg,
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_commission_command(world: &mut World, parts: &[&str]) {
    let Some(entity) = crowned_sovereign_entity(world) else {
        print_dashboard_panel(
            "ERROR",
            "No crowned sovereign is possessed. Possess the crown-bearer first.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let name = parts[1..].join(" ");
    match commission_artwork(world, entity, &name) {
        Ok(msg) => print_dashboard_panel(
            "COMMISSION",
            &msg,
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_edicts_command(world: &mut World) {
    print_dashboard_panel(
        "EDICTS",
        &describe_edicts(world),
        Some(comfy_table::Color::Yellow),
        None,
    );
}

fn handle_abdicate_command(world: &mut World) {
    let Some(entity) = crowned_sovereign_entity(world) else {
        print_dashboard_panel(
            "ERROR",
            "No crowned sovereign is possessed. Possess the crown-bearer first.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    match abdicate(world, entity) {
        Ok(msg) => print_dashboard_panel(
            "ABDICATE",
            &msg,
            Some(comfy_table::Color::Yellow),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn run_ticks(world: &mut World, n: u64) {
    let start_tick = world.resource::<SimulationTime>().tick;

    let mut save_store = FileSaveStore::default_dir();
    for _ in 0..n {
        run_simulation_tick(world);
        // Spec 1382: autosave every 1000 ticks to the reserved slot only.
        match maybe_autosave(world, &mut save_store, unix_now()) {
            Ok(true) => println!("Autosaved to slot 'autosave'."),
            Ok(false) => {}
            Err(e) => eprintln!("Autosave failed: {e}"),
        }
    }

    let end_tick = world.resource::<SimulationTime>().tick;
    print_dashboard_panel(
        "SIMULATION",
        &format!("Advanced {n} ticks ({start_tick} -> {end_tick})"),
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );

    // Report any interesting events
    report_events(world);
}

fn report_events(world: &mut World) {
    // Check for pops doing things
    let mut moving = 0;
    let mut working = 0;
    let mut at_farm = 0;
    let mut at_housing = 0;

    for (_, action, mt, assigned) in world
        .query::<(
            Entity,
            &PopAction,
            Option<&MovementTarget>,
            Option<&scale::layer1::AssignedTo>,
        )>()
        .iter(world)
    {
        if mt.is_some() {
            moving += 1;
        }
        match action.current {
            scale::layer1::ActionType::Work => working += 1,
            scale::layer1::ActionType::SatisfyHunger => at_farm += 1,
            scale::layer1::ActionType::SatisfyRest => at_housing += 1,
            _ => {}
        }
        if assigned.is_some() {
            // Already counted above
        }
    }

    if moving > 0 || working > 0 || at_farm > 0 || at_housing > 0 {
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec![
                Cell::new("Activity").add_attribute(Attribute::Bold),
                Cell::new("Pops").add_attribute(Attribute::Bold),
            ]);

        if moving > 0 {
            table.add_row(vec![
                Cell::new("Moving").fg(comfy_table::Color::Cyan),
                Cell::new(moving.to_string()),
            ]);
        }
        if working > 0 {
            table.add_row(vec![
                Cell::new("Working").fg(comfy_table::Color::Yellow),
                Cell::new(working.to_string()),
            ]);
        }
        if at_farm > 0 {
            table.add_row(vec![
                Cell::new("Eating").fg(comfy_table::Color::Green),
                Cell::new(at_farm.to_string()),
            ]);
        }
        if at_housing > 0 {
            table.add_row(vec![
                Cell::new("Resting").fg(comfy_table::Color::Blue),
                Cell::new(at_housing.to_string()),
            ]);
        }

        print_dashboard_table("Recent Activity", table);
    }
}

fn print_status(world: &mut World) {
    let resources = *world.resource::<ColonyResources>();
    let (wind_dir, wind_speed) = {
        let w = world.resource::<GlobalWind>();
        (w.direction, w.speed)
    };

    let tick = world.resource::<SimulationTime>().tick;
    let pop_count = world.query::<&Pop>().iter(world).count();
    let farm_count = world.query::<&Farm>().iter(world).count();
    let housing_count = world.query::<&Housing>().iter(world).count();
    let designation_count = world.query::<&Designation>().iter(world).count();

    let (avg_morale, avg_stress) = calculate_averages(world);

    let mut singularity_active = 0;
    let mut singularity_mass = 0.0;
    for gen in world
        .query::<&scale::layer1::energy::gravity_siphon::SingularityGenerator>()
        .iter(world)
    {
        if gen.active {
            singularity_active += 1;
            singularity_mass += gen.mass_accumulated;
        }
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Category").add_attribute(Attribute::Bold),
            Cell::new("Metric").add_attribute(Attribute::Bold),
            Cell::new("Value").add_attribute(Attribute::Bold),
        ]);

    add_society_rows(&mut table, pop_count, avg_morale, avg_stress);
    add_environment_rows(&mut table, wind_dir, wind_speed);
    add_resource_rows(&mut table, &resources);

    if singularity_active > 0 {
        table.add_row(vec![
            Cell::new("Energy").fg(Color::Magenta),
            Cell::new("Singularity Mass"),
            Cell::new(format!("{:.1}", singularity_mass)).fg(Color::Magenta),
        ]);
    }

    table.add_row(vec![
        Cell::new("Buildings").fg(Color::Magenta),
        Cell::new("Farms"),
        Cell::new(farm_count.to_string()),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Housing"),
        Cell::new(housing_count.to_string()),
    ]);

    table.add_row(vec![
        Cell::new("Tasks").fg(Color::Blue),
        Cell::new("Active Designations"),
        Cell::new(designation_count.to_string()),
    ]);

    print_dashboard_table(&format!("COLONY STATUS (Tick {})", tick), table);
}

/// Wall-clock unix seconds for save metadata (0 if the clock is broken).
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn handle_save_command(world: &mut World, parts: &[&str]) {
    let Some(slot) = parts.get(1) else {
        print_dashboard_panel(
            "ERROR",
            "Usage: save <slot>",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let mut store = FileSaveStore::default_dir();
    match save_world(world, &mut store, slot, unix_now()) {
        Ok(meta) => print_dashboard_panel(
            "SAVE",
            &format!(
                "Saved '{}' at tick {} ({} bytes) to saves/{}.json",
                meta.slot, meta.tick, meta.size_bytes, meta.slot
            ),
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(e) => print_dashboard_panel(
            "ERROR",
            &format!("Save failed: {e}"),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_load_command(world: &mut World, parts: &[&str]) {
    let Some(slot) = parts.get(1) else {
        print_dashboard_panel(
            "ERROR",
            "Usage: load <slot>",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let store = FileSaveStore::default_dir();
    match load_world(world, &store, slot) {
        Ok(report) => print_dashboard_panel(
            "LOAD",
            &format!(
                "Loaded '{slot}': {} pops, {} buildings, {} items, {} designations restored{}",
                report.pops,
                report.buildings,
                report.items,
                report.designations,
                if report.warnings.is_empty() {
                    String::new()
                } else {
                    format!(" ({} warnings)", report.warnings.len())
                }
            ),
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(e) => print_dashboard_panel(
            "ERROR",
            &format!("Load failed: {e}"),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn print_saves(_world: &mut World) {
    let store = FileSaveStore::default_dir();
    match list_saves(&store) {
        Ok(list) if list.is_empty() => print_dashboard_panel(
            "SAVES",
            "No saves stored yet. Use `save <slot>` first.",
            Some(comfy_table::Color::Cyan),
            Some(comfy_table::Attribute::Bold),
        ),
        Ok(list) => {
            for meta in &list {
                println!(
                    "SAVE slot={} tick={} colony={:?} format=v{} saved_at={} bytes={}",
                    meta.slot,
                    meta.tick,
                    meta.colony_name,
                    meta.format_version,
                    meta.saved_at_unix,
                    meta.size_bytes
                );
            }
        }
        Err(e) => print_dashboard_panel(
            "ERROR",
            &format!("Listing saves failed: {e}"),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_delsave_command(_world: &mut World, parts: &[&str]) {
    let Some(slot) = parts.get(1) else {
        print_dashboard_panel(
            "ERROR",
            "Usage: delsave <slot>",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let mut store = FileSaveStore::default_dir();
    match store.delete(slot) {
        Ok(()) => print_dashboard_panel(
            "SAVES",
            &format!("Deleted save '{slot}' (if it existed)."),
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(e) => print_dashboard_panel(
            "ERROR",
            &format!("Delete failed: {e}"),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_export_command(_world: &mut World, parts: &[&str]) {
    if parts.len() < 3 {
        print_dashboard_panel(
            "ERROR",
            "Usage: export <slot> <path>",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    let store = FileSaveStore::default_dir();
    match export_save(&store, parts[1], std::path::Path::new(parts[2])) {
        Ok(()) => print_dashboard_panel(
            "SAVES",
            &format!("Exported '{}' to {}", parts[1], parts[2]),
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(e) => print_dashboard_panel(
            "ERROR",
            &format!("Export failed: {e}"),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

fn handle_import_command(_world: &mut World, parts: &[&str]) {
    if parts.len() < 3 {
        print_dashboard_panel(
            "ERROR",
            "Usage: import <slot> <path>",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    let mut store = FileSaveStore::default_dir();
    match import_save(&mut store, parts[1], std::path::Path::new(parts[2])) {
        Ok(meta) => print_dashboard_panel(
            "SAVES",
            &format!(
                "Imported {} into slot '{}' (tick {})",
                parts[2], meta.slot, meta.tick
            ),
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(e) => print_dashboard_panel(
            "ERROR",
            &format!("Import failed: {e}"),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

/// Prints one compact, machine-parseable stats line for playtest snapshots.
///
/// Format:
/// `STATS tick=1000 pops=5 avg_health=98.2 avg_morale=0.85 min_pressure=1.00 food=12.0 wood=8.0 stone=2.0 tools=1.0 buildings=17 lifesupport=1 possessed=none sovereign=none legitimacy=0.00 melancholy=0.00 heat=0.0 hull=100 crew=3 loyalty=0.60`
fn print_stats(world: &mut World) {
    let tick = world.resource::<SimulationTime>().tick;
    let resources = *world.resource::<ColonyResources>();

    let mut pops = 0u32;
    let mut health_sum = 0.0f32;
    let mut min_pressure = f32::MAX;
    // Collect pop positions first (query borrows world mutably), then do the
    // pressure lookups with a separate shared borrow — the two can't overlap.
    let pop_tiles: Vec<(i32, i32)> = {
        let mut query = world.query_filtered::<
            (&GridPosition, &scale::layer1::health::Health),
            (
                With<Pop>,
                // Spec 327: the census never counts stowaways.
                Without<scale::layer1::social::ship_stowaway::Stowaway>,
            ),
        >();
        query
            .iter(world)
            .map(|(pos, health)| {
                pops += 1;
                health_sum += health.current;
                (pos.x, pos.y)
            })
            .collect()
    };
    if let Some(grid) = world.get_resource::<scale::layer1::pressure::PressureGrid>() {
        for (x, y) in &pop_tiles {
            min_pressure = min_pressure.min(grid.get(*x, *y));
        }
    }
    if min_pressure == f32::MAX {
        min_pressure = -1.0; // no pressure grid / no pops
    }

    let mut buildings = 0u32;
    let mut lifesupport = 0u32;
    {
        let mut query = world.query::<&Building>();
        for building in query.iter(world) {
            buildings += 1;
            if building.building_type == BuildingType::LifeSupport {
                lifesupport += 1;
            }
        }
    }

    let avg_health = if pops > 0 {
        health_sum / pops as f32
    } else {
        0.0
    };

    let possessed = possessed_entity(world)
        .map(|e| e.index().to_string())
        .unwrap_or_else(|| "none".to_string());

    let avg_morale = calculate_avg_morale(world);

    let (sov_id, legitimacy, melancholy) = sovereign_stats(world);
    let sovereign = sov_id
        .map(|i| i.to_string())
        .unwrap_or_else(|| "none".to_string());

    let (heat, hull, crew, loyalty) = corsair_stats(world);

    let (gov_id, gov_legitimacy, treasury, rivals) = governor_stats(world);

    let (wreck, salvage) = salvager_stats(world);
    let longshot = pilot_stats(world);
    let lawbound = lawbound_stats(world);
    let chronodebt = chronostalker_debt(world)
        .map(|d| format!("{d:.1}"))
        .unwrap_or_else(|| "none".to_string());
    let bloom = bloom_stats(world);
    let governor = gov_id
        .map(|i| i.to_string())
        .unwrap_or_else(|| "none".to_string());
    let vermin_total = {
        use scale::layer1::nature::symbiotic_pests::vermin_total;
        vermin_total(world)
    };
    let caveins_total = {
        use scale::layer1::structural_integrity::CaveInLedger;
        world
            .get_resource::<CaveInLedger>()
            .map(|l| l.total_cave_ins)
            .unwrap_or(0)
    };
    let panicking = {
        use scale::layer1::psychology::panic_spirals::panicking_count;
        panicking_count(world)
    };
    let howling = {
        use scale::layer1::physics::resonance::HowlingState;
        world.resource::<HowlingState>().tiles.len()
    };
    let crystal_power = {
        use scale::layer1::energy::PowerSource;
        use scale::layer1::physics::resonance::ResonanceCrystal;
        let mut q = world.query::<(&ResonanceCrystal, &PowerSource)>();
        q.iter(world).map(|(_, s)| s.output).sum::<f32>()
    };
    let artifacts = {
        use scale::layer1::economy::artifact_market::artifact_count;
        artifact_count(world)
    };
    let scions = {
        use scale::layer1::social::cadet::scion_count;
        scion_count(world)
    };
    let (shadowed, shadows) = {
        use scale::layer1::environment::orbital_shadow::{
            OrbitalShadowCasters, OrbitalShadowMap, SHADOWED_TILE_THRESHOLD,
        };
        let shadowed = world
            .get_resource::<OrbitalShadowMap>()
            .map(|m| {
                m.shadows
                    .values()
                    .filter(|b| **b >= SHADOWED_TILE_THRESHOLD)
                    .count()
            })
            .unwrap_or(0);
        let shadows = world
            .get_resource::<OrbitalShadowCasters>()
            .map(|c| c.casters.len())
            .unwrap_or(0);
        (shadowed, shadows)
    };
    let provenance =
        scale::layer1::construction::material_provenance::stamped_building_count(world);
    let seniority = {
        use scale::layer1::social::seniority::seniority_counts;
        let (f, v, n) = seniority_counts(world);
        format!("{f}/{v}/{n}")
    };
    let martyrdom = {
        use scale::layer1::social::martyrdom::martyrdom_ticks_remaining;
        martyrdom_ticks_remaining(world)
    };
    let feral = {
        use scale::layer1::social::feral_outpost::feral_outpost_count;
        feral_outpost_count(world)
    };
    let fringe = {
        use scale::layer1::social::feral_outpost::fringe_pop_count;
        fringe_pop_count(world)
    };
    let sabotaged = {
        use scale::layer1::architecture::construction_sabotage::sabotaged_building_count;
        sabotaged_building_count(world)
    };
    let vetting = {
        use scale::layer1::architecture::construction_sabotage::vetting_mode_label;
        vetting_mode_label(world)
    };

    let deorbit = {
        use scale::layer1::disasters::deorbit::deorbit_status_label;
        deorbit_status_label(world)
    };
    let tethers = {
        use scale::layer1::disasters::deorbit::tether_array_count;
        tether_array_count(world)
    };
    let void_label = scale::layer2::void_between::void_stats_label(world);
    let empathy = {
        use scale::layer1::social::empathy_broadcast::empathy_broadcast_status_label;
        empathy_broadcast_status_label(world)
    };
    let pirates = {
        use scale::layer1::social::pirates::pirate_pop_count;
        pirate_pop_count(world)
    };
    let pension = {
        use scale::layer1::social::pirates::pension_cost_per_period;
        pension_cost_per_period(world)
    };
    let informant = {
        use scale::layer1::social::informants::informant_edict_label;
        informant_edict_label(world)
    };
    let paranoia = {
        use scale::layer1::social::informants::average_paranoia;
        average_paranoia(world)
    };
    let harpoon = {
        use scale::layer1::tech::harpoon_status_label;
        harpoon_status_label(world)
    };
    let stowaway = {
        use scale::layer1::social::ship_stowaway::stowaway_status_label;
        stowaway_status_label(world)
    };
    let encroaching = {
        use scale::layer1::nature::biome_aggression::encroaching_flora_count;
        encroaching_flora_count(world)
    };
    let mentored = {
        use scale::layer1::social::mentorship::mentored_count;
        mentored_count(world)
    };
    let memories = {
        use scale::layer1::memory::Memories;
        let mut query = world.query::<&Memories>();
        query.iter(world).map(|m| m.items.len()).sum::<usize>()
    };
    let identity = {
        use scale::layer1::memory::ColonyIdentity;
        world
            .get_resource::<ColonyIdentity>()
            .map_or(0, |i| i.traits.len())
    };
    let rumors = {
        use scale::layer1::social::rumor::known_rumor_count;
        known_rumor_count(world)
    };

    // Spec 346: pops currently carrying a contagion-sourced mood modifier.
    let contagion = {
        use scale::layer1::social::emotional_contagion::contagion_affected_count;
        contagion_affected_count(world)
    };

    // Short season name for STATS; the full summary is the `season` command.
    let season_name = world
        .get_resource::<scale::layer1::seasons::SeasonState>()
        .map_or("Spring", |s| s.current_season.name());

    // Spec 345: canonical quirk names for STATS; full panel is `quirks`.
    let quirks_label = scale::layer1::quirks::quirks_summary(world);

    // Spec 347: Great Works completion label for STATS.
    let works_label = scale::layer1::construction::great_works_status_label(world);

    // Spec 349: grave tiles written on the land (built plots + death sites).
    let graves = scale::layer1::ancestral_graves::grave_count(world);

    // Spec 348: black-market presence + corruption level for STATS.
    let smugglers = scale::layer1::economy::black_market::smuggler_count(world);
    let corruption = world
        .get_resource::<scale::layer1::economy::black_market::ColonyStats>()
        .map_or(0.0, |s| s.corruption);

    // Spec 1382: number of saves visible to the file backend.
    let saves_count = FileSaveStore::default_dir()
        .list()
        .map(|l| l.len())
        .unwrap_or(0);

    println!(
        "STATS tick={} pops={} avg_health={:.1} avg_morale={:.2} min_pressure={:.2} food={:.1} wood={:.1} stone={:.1} tools={:.1} buildings={} lifesupport={} possessed={} sovereign={} legitimacy={:.2} melancholy={:.2} heat={:.1} hull={:.0} crew={} loyalty={:.2} governor={} gov_legitimacy={:.2} treasury={:.1} rivals={} wreck={} salvage={:.1} longshot={} lawbound={} chronodebt={} bloom={} origins={} panicking={} howling={} crystal_power={:.1} artifacts={} scions={} organs={:.1} martyrdom={} feral={} fringe={} sabotaged={} vetting={} deorbit={} tethers={} void={} empathy={} pirates={} pension={:.0} informant={} paranoia={:.1} harpoon={} stowaway={} encroaching={} seniority={} shadowed={} shadows={} provenance={} vermin={} caveins={} mentored={} memories={} identity={} rumors={} contagion={} season={} quirks={} saves={} works={} smugglers={} corruption={:.1} graves={}",
        tick,
        pops,
        avg_health,
        avg_morale,
        min_pressure,
        resources.food,
        resources.wood,
        resources.stone,
        resources.tools,
        buildings,
        lifesupport,
        possessed,
        sovereign,
        legitimacy,
        melancholy,
        heat,
        hull,
        crew,
        loyalty,
        governor,
        gov_legitimacy,
        treasury,
        rivals,
        wreck,
        salvage,
        longshot,
        lawbound,
        chronodebt,
        bloom,
        origin_roster_summary(world),
        panicking,
        howling,
        crystal_power,
        artifacts,
        scions,
        resources.organs,
        martyrdom,
        feral,
        fringe,
        sabotaged,
        vetting,
        deorbit,
        tethers,
        void_label,
        empathy,
        pirates,
        pension,
        informant,
        paranoia,
        harpoon,
        stowaway,
        encroaching,
        seniority,
        shadowed,
        shadows,
        provenance,
        vermin_total,
        caveins_total,
        mentored,
        memories,
        identity,
        rumors,
        contagion,
        season_name,
        quirks_label,
        saves_count,
        works_label,
        smugglers,
        corruption,
        graves,
    );
}

fn calculate_avg_morale(world: &mut World) -> f32 {
    let mut total_morale = 0.0;
    let mut morale_count = 0;
    for morale in world
        .query_filtered::<&Morale, Without<scale::layer1::social::ship_stowaway::Stowaway>>()
        .iter(world)
    {
        total_morale += morale.value;
        morale_count += 1;
    }
    if morale_count > 0 {
        #[allow(clippy::cast_precision_loss)]
        let count = morale_count as f32;
        total_morale / count
    } else {
        0.0
    }
}

fn calculate_avg_stress(world: &mut World) -> f32 {
    let mut total_stress = 0.0;
    let mut stress_count = 0;
    for stress in world.query::<&StressTracker>().iter(world) {
        total_stress += stress.accumulated_stress;
        stress_count += 1;
    }
    if stress_count > 0 {
        #[allow(clippy::cast_precision_loss)]
        let count = stress_count as f32;
        total_stress / count
    } else {
        0.0
    }
}

fn calculate_averages(world: &mut World) -> (f32, f32) {
    (calculate_avg_morale(world), calculate_avg_stress(world))
}

fn add_society_rows(table: &mut Table, pop_count: usize, avg_morale: f32, avg_stress: f32) {
    table.add_row(vec![
        Cell::new("Population").fg(Color::Cyan),
        Cell::new("Citizens"),
        Cell::new(pop_count.to_string()),
    ]);

    let morale_color = if avg_morale > 0.8 {
        Color::Green
    } else if avg_morale > 0.4 {
        Color::Yellow
    } else {
        Color::Red
    };
    table.add_row(vec![
        Cell::new("Society").fg(Color::Magenta),
        Cell::new("Avg Morale"),
        Cell::new(format!("{:.0}%", avg_morale * 100.0)).fg(morale_color),
    ]);

    let stress_color = if avg_stress > 100.0 {
        Color::Red
    } else if avg_stress > 50.0 {
        Color::Yellow
    } else {
        Color::Green
    };
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Avg Stress"),
        Cell::new(format!("{:.1}", avg_stress)).fg(stress_color),
    ]);
}

fn add_environment_rows(table: &mut Table, wind_dir: scale::layer1::Vec2, wind_speed: f32) {
    let wind_arrow = if wind_dir.x > 0.0 {
        "→"
    } else if wind_dir.x < 0.0 {
        "←"
    } else if wind_dir.y > 0.0 {
        "↑"
    } else {
        "↓"
    };
    table.add_row(vec![
        Cell::new("Environment").fg(Color::Blue),
        Cell::new("Wind"),
        Cell::new(format!(
            "{:.1} {} ({:.1}, {:.1})",
            wind_speed, wind_arrow, wind_dir.x, wind_dir.y
        )),
    ]);
}

fn add_resource_rows(table: &mut Table, resources: &ColonyResources) {
    // Basic Resources
    table.add_row(vec![
        Cell::new("Basic").fg(Color::Yellow),
        Cell::new("Food"),
        Cell::new(format!("{:.1}", resources.food)).fg(if resources.food < 20.0 {
            Color::Red
        } else {
            Color::Green
        }),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Wood"),
        Cell::new(format!("{:.1}", resources.wood)),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Stone"),
        Cell::new(format!("{:.1}", resources.stone)),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Water"),
        Cell::new(format!("{:.1}", resources.water)).fg(Color::Blue),
    ]);

    // Industrial Resources
    table.add_row(vec![
        Cell::new("Industrial").fg(Color::Grey),
        Cell::new("Ore"),
        Cell::new(format!("{:.1}", resources.ore)),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Metal"),
        Cell::new(format!("{:.1}", resources.metal)),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Fuel"),
        Cell::new(format!("{:.1}", resources.fuel)).fg(Color::Red),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Scrap"),
        Cell::new(format!("{:.1}", resources.scrap)).fg(Color::DarkGrey),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Waste"),
        Cell::new(format!("{:.1}", resources.waste)).fg(Color::DarkGreen),
    ]);

    // Refined/Crafted
    table.add_row(vec![
        Cell::new("Crafted").fg(Color::Cyan),
        Cell::new("Planks"),
        Cell::new(format!("{:.1}", resources.planks)),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Blocks"),
        Cell::new(format!("{:.1}", resources.blocks)),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Tools"),
        Cell::new(format!("{:.1}", resources.tools)),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Cloth"),
        Cell::new(format!("{:.1}", resources.cloth)),
    ]);

    // Advanced
    table.add_row(vec![
        Cell::new("Advanced").fg(Color::Magenta),
        Cell::new("Knowledge"),
        Cell::new(format!("{:.1}", resources.knowledge)).fg(Color::Cyan),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Rations"),
        Cell::new(format!("{:.1}", resources.rations)),
    ]);
    table.add_row(vec![
        Cell::new(""),
        Cell::new("Alcohol"),
        Cell::new(format!("{:.1}", resources.alcohol)),
    ]);
}

fn print_tech(world: &mut World) {
    let tech_state = world.resource::<TechState>();

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Technology").add_attribute(Attribute::Bold),
            Cell::new("Status").add_attribute(Attribute::Bold),
            Cell::new("Cost (Know)").add_attribute(Attribute::Bold),
            Cell::new("Storage (TB)").add_attribute(Attribute::Bold),
            Cell::new("Description").add_attribute(Attribute::Bold),
        ]);

    let all_techs = vec![
        Tech::Masonry,
        Tech::MetalWorking,
        Tech::SocialStructures,
        Tech::Astronomy,
        Tech::Hydroponics,
        Tech::Militia,
        Tech::Medical,
        Tech::Electromagnetism,
        Tech::VoidWhispers,
        Tech::Terraforming,
    ];

    for tech in all_techs {
        let status = if tech_state.is_active(tech) {
            "Active"
        } else if tech_state.techs.get(&tech) == Some(&TechStatus::Corrupted) {
            "Corrupted"
        } else {
            "Locked"
        };

        let status_color = match status {
            "Active" => Color::Green,
            "Corrupted" => Color::Red,
            _ => Color::Grey,
        };

        table.add_row(vec![
            Cell::new(tech.label()).fg(status_color),
            Cell::new(status).fg(status_color),
            Cell::new(format!("{:.0}", tech.cost())),
            Cell::new(format!("{:.0}", tech.storage_cost())),
            Cell::new(tech.description()),
        ]);
    }

    print_dashboard_table(
        &format!(
            "TECHNOLOGY STATUS (Capacity: {:.1} TB / {:.1} TB)",
            tech_state.used_capacity, tech_state.total_capacity
        ),
        table,
    );
}

fn print_pops(world: &mut World) {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("ID").add_attribute(Attribute::Bold),
            Cell::new("Name").add_attribute(Attribute::Bold),
            Cell::new("Pos").add_attribute(Attribute::Bold),
            Cell::new("Hunger").add_attribute(Attribute::Bold),
            Cell::new("Rest").add_attribute(Attribute::Bold),
            Cell::new("Stress").add_attribute(Attribute::Bold),
            Cell::new("Traits").add_attribute(Attribute::Bold),
            Cell::new("Action").add_attribute(Attribute::Bold),
            Cell::new("Status").add_attribute(Attribute::Bold),
        ]);

    let max_pops = 15;
    let pops_iter: Vec<_> = world
        .query_filtered::<
            (Entity, &PopName, &GridPosition, &Needs, &PopAction),
            (
                With<Pop>,
                // Spec 327: the roster never lists stowaways.
                Without<scale::layer1::social::ship_stowaway::Stowaway>,
            ),
        >()
        .iter(world)
        .collect();
    let total_pops = pops_iter.len();

    for (entity, name, pos, needs, action) in pops_iter.into_iter().take(max_pops) {
        let stress_tracker = world.get::<StressTracker>(entity);
        let traits = world.get::<Traits>(entity);
        let mt = world.get::<MovementTarget>(entity);
        let at_target = world.get::<scale::layer1::AtTarget>(entity).is_some();
        let is_possessed = world.get::<Possessed>(entity).is_some();

        let status = if is_possessed {
            "POSSESSED (you)".to_string()
        } else if at_target {
            "At target".to_string()
        } else if let Some(mt) = mt {
            format!(
                "Moving ➔ ({},{})",
                mt.target_position.x, mt.target_position.y
            )
        } else {
            "-".to_string()
        };

        let action_str = format_action_type_headless(action.current);

        // Color code needs: Low is BAD (Red), High is GOOD (Green) ??
        // Wait, hunger is 0..1. Usually 1.0 is full (good).
        // Let's assume 1.0 is Satiated (Good). 0.0 is Starving (Bad).
        let hunger_color = if needs.hunger < 0.2 {
            Color::Red
        } else if needs.hunger < 0.5 {
            Color::Yellow
        } else {
            Color::Green
        };
        let rest_color = if needs.rest < 0.2 {
            Color::Red
        } else if needs.rest < 0.5 {
            Color::Yellow
        } else {
            Color::Green
        };

        let stress_val = stress_tracker.map_or(0.0, |s| s.accumulated_stress);
        let stress_color = if stress_val > 100.0 {
            Color::Red
        } else if stress_val > 50.0 {
            Color::Yellow
        } else {
            Color::Green
        };

        let traits_str = if let Some(t) = traits {
            let list: Vec<String> = t.iter().map(|tr| format!("{:?}", tr)).collect();
            if list.is_empty() {
                "-".to_string()
            } else {
                list.join(", ")
            }
        } else {
            "-".to_string()
        };

        let traits_color = if traits_str == "-" {
            Color::DarkGrey
        } else if traits_str.contains("Prophet") || traits_str.contains("Engine Cultist") {
            Color::Yellow
        } else if traits_str.contains("Mutant") {
            Color::Magenta
        } else {
            Color::Cyan
        };

        table.add_row(vec![
            Cell::new(entity.index().to_string()).fg(Color::DarkGrey),
            Cell::new(&name.0),
            Cell::new(format!("{},{}", pos.x, pos.y)).fg(Color::DarkGrey),
            Cell::new(format!("{:.0}%", needs.hunger * 100.0)).fg(hunger_color),
            Cell::new(format!("{:.0}%", needs.rest * 100.0)).fg(rest_color),
            Cell::new(format!("{:.1}", stress_val)).fg(stress_color),
            Cell::new(traits_str).fg(traits_color),
            if action.current == scale::layer1::ActionType::Sleepwalking {
                Cell::new(action_str).fg(Color::Magenta)
            } else {
                Cell::new(action_str)
            },
            Cell::new(status),
        ]);
    }

    if total_pops > max_pops {
        let hidden = total_pops - max_pops;
        table.add_row(vec![
            Cell::new("...").fg(Color::DarkGrey),
            Cell::new(format!("{} more pops hidden", hidden))
                .fg(Color::DarkGrey)
                .add_attribute(Attribute::Italic),
            Cell::new("...").fg(Color::DarkGrey),
            Cell::new("...").fg(Color::DarkGrey),
            Cell::new("...").fg(Color::DarkGrey),
            Cell::new("...").fg(Color::DarkGrey),
            Cell::new("...").fg(Color::DarkGrey),
            Cell::new("...").fg(Color::DarkGrey),
            Cell::new("...").fg(Color::DarkGrey),
        ]);
    }

    print_dashboard_table("POPULATION DETAILS", table);
}

/// Ghost-pop audit (GHOST-POP FIX 2026-10-04): every live pop must be a
/// complete PopBundle. A "ghost" is an entity with `Pop` missing any of
/// PopName / Health / Wallet / GridPosition / Needs — invisible to STATS,
/// immune to damage/infection queries, eating without wallet checks.
fn print_ghost_audit(world: &mut World) {
    let mut total = 0u32;
    let mut missing_name = 0u32;
    let mut missing_health = 0u32;
    let mut missing_wallet = 0u32;
    let mut missing_pos = 0u32;
    let mut missing_needs = 0u32;
    let mut query = world.query::<(Entity, &Pop)>();
    for (entity, _) in query.iter(world) {
        total += 1;
        if world.get::<PopName>(entity).is_none() {
            missing_name += 1;
        }
        if world
            .get::<scale::layer1::health::Health>(entity)
            .is_none()
        {
            missing_health += 1;
        }
        if world
            .get::<scale::layer1::economy::Wallet>(entity)
            .is_none()
        {
            missing_wallet += 1;
        }
        if world.get::<GridPosition>(entity).is_none() {
            missing_pos += 1;
        }
        if world.get::<Needs>(entity).is_none() {
            missing_needs += 1;
        }
    }
    let ghosts = missing_name
        .max(missing_health)
        .max(missing_wallet)
        .max(missing_pos)
        .max(missing_needs);
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Metric").add_attribute(Attribute::Bold),
            Cell::new("Count").add_attribute(Attribute::Bold),
        ]);
    table.add_row(vec![Cell::new("Pop entities"), Cell::new(total)]);
    table.add_row(vec![Cell::new("Missing PopName"), Cell::new(missing_name)]);
    table.add_row(vec![Cell::new("Missing Health"), Cell::new(missing_health)]);
    table.add_row(vec![Cell::new("Missing Wallet"), Cell::new(missing_wallet)]);
    table.add_row(vec![Cell::new("Missing GridPosition"), Cell::new(missing_pos)]);
    table.add_row(vec![Cell::new("Missing Needs"), Cell::new(missing_needs)]);
    let verdict = if ghosts == 0 {
        "ZERO GHOSTS — every pop is a complete colonist"
    } else {
        "GHOSTS DETECTED — bare Pop entities exist"
    };
    table.add_row(vec![
        Cell::new("Verdict").add_attribute(Attribute::Bold),
        Cell::new(verdict),
    ]);
    print_dashboard_table("GHOST-POP AUDIT", table);
}

/// Debug: spawn a terrifying sighting at a tile (Spec 1371).
///
/// `scare [x] [y]` — drops a monster-sighting [`Terrifying`] marker so the
/// panic cascade can be exercised from the console. Defaults to the first
/// living pop's tile.
fn handle_scare_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::psychology::panic_spirals::{spawn_terrifying, TerrifyingKind};

    let (mut x, mut y) = (40, 25);
    if let (Some(px), Some(py)) = (parts.get(1), parts.get(2)) {
        if let (Ok(px), Ok(py)) = (px.parse::<i32>(), py.parse::<i32>()) {
            x = px;
            y = py;
        }
    } else {
        // Default: right next to the first pop with a position.
        let mut q = world.query_filtered::<&GridPosition, With<Pop>>();
        if let Some(pos) = q.iter(world).next() {
            x = pos.x + 1;
            y = pos.y;
        }
    }
    let entity = spawn_terrifying(world, x, y, TerrifyingKind::Monster);
    print_dashboard_panel(
        "TERRIFYING SIGHTING",
        &format!(
            "A monster sighting materializes at ({x}, {y}) (entity #{}).\nNearby pops will panic and flee; panic spreads on contact.",
            entity.index()
        ),
        Some(comfy_table::Color::Red),
        Some(comfy_table::Attribute::Bold),
    );
}


/// Raise a support pillar at a tile (headless `pillar` command, Spec 338).
fn handle_pillar_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::structural_integrity::place_support_pillar;

    if parts.len() < 3 {
        print_dashboard_panel(
            "ERROR",
            "Usage: pillar <x> <y>",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    let (x, y) = match (parts[1].parse::<i32>(), parts[2].parse::<i32>()) {
        (Ok(x), Ok(y)) => (x, y),
        _ => {
            print_dashboard_panel(
                "ERROR",
                "Invalid coordinates",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    place_support_pillar(world, x, y);
    print_dashboard_panel(
        "PILLAR RAISED",
        &format!(
            "A support pillar now bears the load at ({x}, {y}). Hollowed neighbors\nrecalculate their support next tick — pillar-and-stall mining keeps ceilings up."
        ),
        Some(comfy_table::Color::Green),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Structural integrity report: tracked tiles, support values, cave-in ledger (Spec 338).
fn print_integrity(world: &mut World) {
    use scale::layer1::structural_integrity::{
        CaveInLedger, Collapsed, MinedStatus, SupportPillar, SupportValue, TileCoords,
    };

    let tiles: Vec<(Entity, i32, i32, f32)> = {
        let mut q = world.query::<(Entity, &TileCoords, &SupportValue, &MinedStatus)>();
        q.iter(world)
            .filter(|(_, _, _, m)| m.is_mined)
            .map(|(e, c, s, _)| (e, c.x, c.y, s.value))
            .collect()
    };
    let mut collapsed = 0usize;
    let mut weakest: Option<(i32, i32, f32)> = None;
    for (e, x, y, v) in &tiles {
        if world.get::<Collapsed>(*e).is_some() {
            collapsed += 1;
        }
        match weakest {
            Some((_, _, w)) if *v >= w => {}
            _ => weakest = Some((*x, *y, *v)),
        }
    }
    let pillars = world
        .query_filtered::<Entity, With<SupportPillar>>()
        .iter(world)
        .count();
    let ledger = world
        .get_resource::<CaveInLedger>()
        .map(|l| (l.total_cave_ins, l.total_damage))
        .unwrap_or_default();
    let body = format!(
        "Tracked hollowed tiles: {}\nCollapsed (rubble): {collapsed}\nSupport pillars raised: {pillars}\nCave-ins: {} (total damage {:.0})\nWeakest tile: {}\n\nMine designations register their hollow for support tracking.\nSupport < 0.2 collapses: rubble seals the tunnel, nearby pops are hurt.",
        tiles.len(),
        ledger.0,
        ledger.1,
        weakest
            .map(|(x, y, v)| format!("({x}, {y}) at {v:.2}"))
            .unwrap_or_else(|| "none".to_string()),
    );
    print_dashboard_panel(
        "STRUCTURAL INTEGRITY",
        &body,
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Show vermin status: counts by species + silk ledger (Spec 337).
fn print_vermin(world: &mut World) {    use scale::layer1::nature::symbiotic_pests::{banked_silk, unharvested_silk, vermin_counts};

    let (rats, beetles, spiders) = vermin_counts(world);
    let total = rats + beetles + spiders;
    if total == 0 {
        print_dashboard_panel(
            "VERMIN STATUS",
            "No vermin in the colony. The stockpiles are clean... for now.",
            Some(comfy_table::Color::Green),
            None,
        );
        return;
    }
    print_dashboard_panel(
        "VERMIN STATUS",
        &format!(
            "Vermin: {total} total\n  Grain Rats: {rats} (food thieves)\n  Brain Borer Beetles: {beetles} (rat predators)\n  Silk Weaver Spiders: {spiders} (silk weavers)\nBanked silk: {:.1}\nUnharvested silk on spiders: {:.1}\n`vermin harvest` banks the woven silk; `exterminate` culls (rats become meat, spiders bank silk); `vermin protect` seals a stockpile.",
            banked_silk(world),
            unharvested_silk(world),
        ),
        Some(comfy_table::Color::Yellow),
        None,
    );
}

/// `vermin ...` — subcommand dispatcher (Spec 337):
/// status | infest [n] [species] | exterminate [all|rat|beetle|spider] |
/// harvest | protect [x] [y]
fn handle_vermin_command(world: &mut World, parts: &[&str]) {
    match parts.get(1).map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("status") | Some("list") => print_vermin(world),
        // Re-slice so the sub-handlers see ["infest", n, species] etc.
        Some("infest") => handle_infest_command(world, &parts[1..]),
        Some("exterminate") => handle_exterminate_command(world, &parts[1..]),
        Some("harvest") => {
            use scale::layer1::nature::symbiotic_pests::harvest_silk;
            let banked = harvest_silk(world);
            print_dashboard_panel(
                "SILK HARVEST",
                &format!("Banked {banked:.1} silk from the weavers."),
                Some(comfy_table::Color::Cyan),
                None,
            );
        }
        Some("protect") => {
            use scale::layer1::nature::symbiotic_pests::toggle_pest_proof;
            let (mut x, mut y) = (40, 25);
            if let (Some(px), Some(py)) = (parts.get(2), parts.get(3)) {
                if let (Ok(px), Ok(py)) = (px.parse::<i32>(), py.parse::<i32>()) {
                    x = px;
                    y = py;
                }
            }
            match toggle_pest_proof(world, x, y) {
                Some(true) => print_dashboard_panel(
                    "PEST SAFE",
                    &format!("The stockpile nearest ({x}, {y}) is now sealed against vermin (Pest Safe)."),
                    Some(comfy_table::Color::Green),
                    None,
                ),
                Some(false) => print_dashboard_panel(
                    "PEST SAFE",
                    &format!("The stockpile nearest ({x}, {y}) is no longer sealed — vermin may return."),
                    Some(comfy_table::Color::Yellow),
                    None,
                ),
                None => print_dashboard_panel(
                    "PEST SAFE",
                    "No stockpile buildings exist.",
                    Some(comfy_table::Color::Red),
                    None,
                ),
            }
        }
        Some(other) => print_dashboard_panel(
            "VERMIN",
            &format!(
                "Unknown vermin subcommand: {other}. Try: status, infest, exterminate, harvest, protect."
            ),
            Some(comfy_table::Color::Red),
            None,
        ),
    }
}

/// `infest [n] [species]` — debug: spawn n vermin near a stockpile (Spec 337).
fn handle_infest_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::nature::symbiotic_pests::{spawn_vermin_at, VerminSpecies};

    let n: usize = parts
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3)
        .min(50);
    let species = parts
        .get(2)
        .and_then(|s| VerminSpecies::parse(s))
        .unwrap_or(VerminSpecies::GrainRat);
    // Near the first stockpile building; fall back to the first pop's tile.
    let (mut x, mut y) = (40, 25);
    let mut q = world.query_filtered::<&GridPosition, With<Stockpile>>();
    if let Some(pos) = q.iter(world).next() {
        x = pos.x;
        y = pos.y;
    } else {
        let mut pq = world.query_filtered::<&GridPosition, With<Pop>>();
        if let Some(pos) = pq.iter(world).next() {
            x = pos.x;
            y = pos.y;
        }
    }
    for i in 0..n {
        let dx = (i % 3) as i32 - 1;
        let dy = ((i / 3) % 3) as i32 - 1;
        spawn_vermin_at(world, species, x + dx, y + dy);
    }
    print_dashboard_panel(
        "INFESTATION",
        &format!("Spawned {n} {} near ({x}, {y}).", species.label()),
        Some(comfy_table::Color::Red),
        Some(comfy_table::Attribute::Bold),
    );
}

/// `exterminate [all|rat|beetle|spider]` — cull vermin (Spec 337). Rats are
/// dressed for meat (+food), spiders bank their woven silk. Beetles eat the
/// rats for you — cull them only if you mean it.
fn handle_exterminate_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::nature::symbiotic_pests::{exterminate_vermin, VerminSpecies};

    let species = match parts.get(1).map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("all") => None,
        Some(other) => match VerminSpecies::parse(other) {
            Some(s) => Some(s),
            None => {
                print_dashboard_panel(
                    "EXTERMINATION",
                    &format!("Unknown species: {other}. Try: all, rat, beetle, spider."),
                    Some(comfy_table::Color::Red),
                    None,
                );
                return;
            }
        },
    };
    let report = exterminate_vermin(world, species);
    let total = report.rats + report.beetles + report.spiders;
    if total == 0 {
        print_dashboard_panel(
            "EXTERMINATION",
            "No matching vermin to exterminate.",
            Some(comfy_table::Color::Green),
            None,
        );
        return;
    }
    print_dashboard_panel(
        "EXTERMINATION",
        &format!(
            "Culled: {} rats, {} beetles, {} spiders.\n+{:.1} food (dressed rats), +{:.1} silk banked.",
            report.rats, report.beetles, report.spiders, report.food_gained, report.silk_gained
        ),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// List currently panicking pops (Spec 1371).
fn print_panics(world: &mut World) {
    use scale::layer1::psychology::panic_spirals::Panic;

    let mut rows: Vec<String> = Vec::new();
    let mut q = world.query_filtered::<(Entity, Option<&PopName>, &GridPosition, &Panic), With<Pop>>();
    for (entity, name, pos, panic) in q.iter(world) {
        let name = name.map_or_else(|| "?".to_string(), |n| n.0.clone());
        rows.push(format!(
            "#{} {name} at ({}, {}) — {} ticks of panic left",
            entity.index(),
            pos.x,
            pos.y,
            panic.timer
        ));
    }
    if rows.is_empty() {
        print_dashboard_panel(
            "PANIC STATUS",
            "No pops are panicking. The colony is (for now) calm.",
            Some(comfy_table::Color::Green),
            None,
        );
    } else {
        print_dashboard_panel(
            "PANIC STATUS",
            &format!("{} panicking:\n{}", rows.len(), rows.join("\n")),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    }
}

/// Show Founder/Veteran/NewBlood seniority counts (Spec 333).
///
/// `seniority` — counts plus a sample of pops with their tier and authority.
/// `seniority friction [a] [b]` — force a rankling encounter between two pops
/// (auto-picks the first Founder and first NewBlood when no ids are given).
fn handle_seniority_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::social::seniority::{Seniority, SeniorityFrictionEvent, SeniorityStatus};

    if parts.get(1).is_some_and(|s| *s == "friction") {
        let explicit_a: Option<u32> = parts.get(2).and_then(|s| s.parse().ok());
        let explicit_b: Option<u32> = parts.get(3).and_then(|s| s.parse().ok());
        let pair: Option<(Entity, Entity)> = match (explicit_a, explicit_b) {
            (Some(a), Some(b)) => {
                let a_ent = find_pop_by_id(world, a).map(|(e, _)| e);
                let b_ent = find_pop_by_id(world, b).map(|(e, _)| e);
                a_ent.zip(b_ent)
            }
            _ => {
                // Auto-pick: first Founder and first NewBlood in the world.
                let mut founder_ent = None;
                let mut newblood_ent = None;
                let mut q =
                    world.query_filtered::<(Entity, &Seniority), With<Pop>>();
                for (entity, seniority) in q.iter(world) {
                    match seniority.status {
                        SeniorityStatus::Founder if founder_ent.is_none() => {
                            founder_ent = Some(entity);
                        }
                        SeniorityStatus::NewBlood if newblood_ent.is_none() => {
                            newblood_ent = Some(entity);
                        }
                        _ => {}
                    }
                    if founder_ent.is_some() && newblood_ent.is_some() {
                        break;
                    }
                }
                founder_ent.zip(newblood_ent)
            }
        };
        match pair {
            Some((ae, be)) => {
                world.send_event(SeniorityFrictionEvent { a: ae, b: be });
                {
                    use bevy_ecs::system::RunSystemOnce;
                    let _ = world.run_system_once(
                        scale::layer1::social::seniority::seniority_friction_event_system,
                    );
                }
                print_dashboard_panel(
                    "SENIORITY FRICTION",
                    &format!(
                        "Forced a rankling encounter between pop #{} and pop #{}.\nIf exactly one is a Founder and the other NewBlood, the founder now carries a decaying 'New Blood Friction' mood modifier.",
                        ae.index(),
                        be.index()
                    ),
                    Some(comfy_table::Color::Yellow),
                    None,
                );
            }
            None => println!(
                "No Founder/NewBlood pair available — list ids with `pops` or wait for new arrivals."
            ),
        }
        return;
    }

    use scale::layer1::social::seniority::seniority_counts;
    use scale::layer1::social::seniority::{FRICTION_LABEL, OUTNUMBERED_LABEL};
    use scale::layer1::social::morale::Morale;

    let (f, v, n) = seniority_counts(world);
    let mut rows: Vec<String> = Vec::new();
    let mut q = world.query_filtered::<(Entity, Option<&PopName>, &Seniority, Option<&Morale>), With<Pop>>();
    for (entity, name, seniority, morale) in q.iter(world).take(10) {
        let name = name.map_or_else(|| "?".to_string(), |x| x.0.clone());
        let tier = match seniority.status {
            SeniorityStatus::Founder => "Founder",
            SeniorityStatus::Veteran => "Veteran",
            SeniorityStatus::NewBlood => "NewBlood",
        };
        let friction = morale.is_some_and(|m| {
            m.modifiers
                .iter()
                .any(|x| x.label == FRICTION_LABEL || x.label == OUTNUMBERED_LABEL)
        });
        rows.push(format!(
            "#{} {name} — {tier} (authority x{:.1}){}",
            entity.index(),
            seniority.authority_multiplier,
            if friction { " [friction]" } else { "" }
        ));
    }
    let body = if rows.is_empty() {
        "No pops have seniority yet.".to_string()
    } else {
        rows.join("\n")
    };
    print_dashboard_panel(
        "SENIORITY",
        &format!("Founders: {f}  Veterans: {v}  NewBlood: {n}\n\n{body}"),
        Some(comfy_table::Color::Cyan),
        None,
    );
}

/// Debug: trigger a wind surge so canyon tiles howl (Spec 1374).
///
/// `howl [speed]` — sets the base global wind speed (default 6.0); the surge
/// persists until `calm`. Takes effect on the next tick.
fn handle_howl_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::atmosphere::BaseGlobalWind;
    use scale::layer1::physics::resonance::HowlingState;

    let speed = parts
        .get(1)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(6.0);
    world.resource_mut::<BaseGlobalWind>().speed = speed;
    // HowlingState reflects the last completed tick — useful once the surge lands.
    let howling = world.resource::<HowlingState>();
    let sample: Vec<String> = howling
        .tiles
        .iter()
        .take(5)
        .map(|(x, y)| format!("({x}, {y})"))
        .collect();
    let where_howling = if howling.tiles.is_empty() {
        "none yet — run `tick` and ask again".to_string()
    } else {
        format!(
            "{} tile(s), e.g. {}",
            howling.tiles.len(),
            sample.join(", ")
        )
    };
    print_dashboard_panel(
        "WIND SURGE",
        &format!(
            "Base wind speed set to {speed:.1} (takes effect next tick).\nCanyon tiles above the howl threshold will sing — the noise stresses pops, but resonance crystals harvest it for power.\nHowling now: {where_howling}.\n`calm` ends the surge."
        ),
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: end the wind surge (Spec 1374).
fn handle_calm_command(world: &mut World) {
    use scale::layer1::atmosphere::BaseGlobalWind;

    world.resource_mut::<BaseGlobalWind>().speed = 1.0;
    print_dashboard_panel(
        "THE AIR SETTLES",
        "Base wind speed back to 1.0. The canyons will fall silent within a tick or two.",
        Some(comfy_table::Color::Green),
        None,
    );
}

/// Debug: place a resonance crystal at a tile (Spec 1374).
///
/// `crystal [x] [y]` — with no coordinates, attunes at the loudest currently
/// howling tile (falls back to the first living pop's tile when silent), so
/// the harvest mechanic is one command away from a `howl` surge.
fn handle_crystal_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::physics::resonance::{spawn_resonance_crystal, HowlingState};

    let (mut x, mut y) = (40, 25);
    if let (Some(px), Some(py)) = (parts.get(1), parts.get(2)) {
        if let (Ok(px), Ok(py)) = (px.parse::<i32>(), py.parse::<i32>()) {
            x = px;
            y = py;
        }
    } else if let Some((hx, hy)) = world.resource::<HowlingState>().tiles.first() {
        // Default: a currently howling tile, so the harvest is audible.
        x = *hx;
        y = *hy;
    } else {
        // Fallback: at the first pop with a position.
        let mut q = world.query_filtered::<&GridPosition, With<Pop>>();
        if let Some(pos) = q.iter(world).next() {
            x = pos.x;
            y = pos.y;
        }
    }
    let entity = spawn_resonance_crystal(world, x, y);
    print_dashboard_panel(
        "RESONANCE CRYSTAL PLACED",
        &format!(
            "Crystal #{} attuned at ({x}, {y}).\nIt harvests tile noise into the power grid — place it where the canyon howls.",
            entity.index()
        ),
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
}

/// List resonance crystals with their tile noise and power output (Spec 1374).
fn print_crystals(world: &mut World) {
    use scale::layer1::acoustic::NoiseMap;
    use scale::layer1::energy::PowerSource;
    use scale::layer1::physics::resonance::ResonanceCrystal;

    // Collect crystal rows first (the query borrows world mutably), then do
    // the noise lookups with a separate shared borrow — the two can't overlap.
    let crystals: Vec<(usize, i32, i32, f32, bool)> = {
        let mut q = world.query::<(Entity, &GridPosition, &ResonanceCrystal, &PowerSource)>();
        q.iter(world)
            .map(|(entity, pos, _crystal, source)| {
                (
                    entity.index() as usize,
                    pos.x,
                    pos.y,
                    source.output,
                    source.active,
                )
            })
            .collect()
    };
    let noise = world.resource::<NoiseMap>();
    let mut rows: Vec<String> = Vec::new();
    for (index, x, y, output, active) in crystals {
        let level = noise.get(x, y);
        rows.push(format!(
            "#{index} at ({x}, {y}) — noise {level:.2}, output {output:.1}/tick ({})",
            if active { "harvesting" } else { "quiet" }
        ));
    }
    if rows.is_empty() {
        print_dashboard_panel(
            "RESONANCE CRYSTALS",
            "No crystals placed. Use `crystal [x] [y]` to attune one.",
            Some(comfy_table::Color::Cyan),
            None,
        );
    } else {
        print_dashboard_panel(
            "RESONANCE CRYSTALS",
            &format!("{} crystal(s):\n{}", rows.len(), rows.join("\n")),
            Some(comfy_table::Color::Cyan),
            None,
        );
    }
}

/// Issue a manufacturer recall notice (Spec 1373).
fn handle_recall_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
    use scale::layer1::economy::recall::{
        item_display_name, parse_recallable_item, RecallManager,
    };

    let item_name = match parts.get(1) {
        Some(n) => n.to_string(),
        None => {
            print_dashboard_panel(
                "ERROR",
                "Usage: recall <item> [manufacturer] [reason...]",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    let item_type = match parse_recallable_item(&item_name) {
        Some(t) => t,
        None => {
            print_dashboard_panel(
                "ERROR",
                &format!(
                    "Unknown item: '{item_name}'. Try: rations, potato, wheat, stim, tools, ..."
                ),
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    let manufacturer = parts
        .get(2)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "OmniNutri Corp".to_string());
    let reason = {
        let rest = parts.get(3..).map(|s| s.join(" ")).unwrap_or_default();
        if rest.is_empty() {
            "defective batch".to_string()
        } else {
            rest
        }
    };
    // Direct world manipulation (headless pattern): takes effect immediately.
    world.init_resource::<RecallManager>();
    let is_new = world.resource_mut::<RecallManager>().issue(item_type);
    world.init_resource::<bevy_ecs::event::Events<AddChronicleEvent>>();
    if is_new {
        world
            .resource_mut::<bevy_ecs::event::Events<AddChronicleEvent>>()
            .send(AddChronicleEvent {
                text: format!(
                    "RECALL NOTICE: {manufacturer} has recalled all {} — {reason}. Use at your own risk; return stock for credits.",
                    item_display_name(&item_type),
                ),
                importance: EventImportance::Major,
            });
    }
    print_dashboard_panel(
        "RECALL ISSUED",
        &format!(
            "{manufacturer} has recalled all {} — {reason}.\nUsing it risks critical failure; 'return_stock {} <qty>' returns it for credits.",
            item_display_name(&item_type),
            item_name.to_lowercase()
        ),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// List active manufacturer recalls (Spec 1373).
fn print_recalls(world: &mut World) {
    use scale::layer1::economy::recall::{item_display_name, RecallManager};

    let Some(manager) = world.get_resource::<RecallManager>() else {
        print_dashboard_panel(
            "ACTIVE RECALLS",
            "Recall system not initialized yet (tick once).",
            Some(comfy_table::Color::Yellow),
            None,
        );
        return;
    };
    if manager.recalled.is_empty() {
        print_dashboard_panel(
            "ACTIVE RECALLS",
            "No active recalls. Your products are (allegedly) safe.",
            Some(comfy_table::Color::Green),
            None,
        );
        return;
    }
    let mut rows: Vec<String> = manager
        .recalled
        .iter()
        .map(|t| format!("- {}", item_display_name(t)))
        .collect();
    rows.sort();
    print_dashboard_panel(
        "ACTIVE RECALLS",
        &format!(
            "{}\n\nUsing these risks critical failure. 'return_stock <item> <qty>' returns stock for credits.",
            rows.join("\n")
        ),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Return recalled stock for credits (Spec 1373).
fn handle_return_stock_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
    use scale::layer1::economy::inflation::EmpireResources;
    use scale::layer1::economy::recall::{
        item_display_name, parse_recallable_item, remove_stock, RecallConfig, RecallManager,
    };
    use scale::layer1::economy::resources::ColonyResources;

    let (item_name, qty) = match (parts.get(1), parts.get(2)) {
        (Some(n), Some(q)) => (n.to_string(), q.parse::<u32>().unwrap_or(0)),
        _ => {
            print_dashboard_panel(
                "ERROR",
                "Usage: return_stock <item> <qty>",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    let item_type = match parse_recallable_item(&item_name) {
        Some(t) => t,
        None => {
            print_dashboard_panel(
                "ERROR",
                &format!("Unknown item: '{item_name}'."),
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    if qty == 0 {
        print_dashboard_panel(
            "ERROR",
            "Quantity must be positive.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    // Direct world manipulation (headless pattern): takes effect immediately.
    let is_recalled = world
        .get_resource::<RecallManager>()
        .is_some_and(|m| m.is_recalled(&item_type));
    if !is_recalled {
        print_dashboard_panel(
            "RETURN REJECTED",
            &format!(
                "{} is not under recall — the manufacturer will not buy it back.",
                item_display_name(&item_type)
            ),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    let credits_per_item = world
        .get_resource::<RecallConfig>()
        .map(|c| c.credits_per_item)
        .unwrap_or(5.0);
    let returned = world
        .get_resource_mut::<ColonyResources>()
        .map(|mut r| remove_stock(&mut r, &item_type, qty))
        .unwrap_or(0);
    if returned == 0 {
        print_dashboard_panel(
            "RETURN REJECTED",
            &format!(
                "No {} in stock to return.",
                item_display_name(&item_type)
            ),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    let payout = returned as f32 * credits_per_item;
    let mut empire_q = world.query::<&mut EmpireResources>();
    for mut empire in empire_q.iter_mut(world) {
        empire.credits += payout;
    }
    world.init_resource::<bevy_ecs::event::Events<AddChronicleEvent>>();
    world
        .resource_mut::<bevy_ecs::event::Events<AddChronicleEvent>>()
        .send(AddChronicleEvent {
            text: format!(
                "Returned {returned} recalled {} for {payout:.0} credits.",
                item_display_name(&item_type),
            ),
            importance: EventImportance::Minor,
        });
    print_dashboard_panel(
        "STOCK RETURNED",
        &format!(
            "Returned {returned}× {} for {payout:.0} credits.",
            item_display_name(&item_type),
        ),
        Some(comfy_table::Color::Green),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Diagnostic: dump raw entity/component counts for pop-related entities.
/// Temporary revival diagnostic — not for long-term use.
fn format_action_type_headless(action: scale::layer1::ActionType) -> String {
    use scale::layer1::ActionType;
    match action {
        ActionType::SatisfyHunger => "🍖 Eating".to_string(),
        ActionType::SatisfyRest => "💤 Sleeping".to_string(),
        ActionType::Socialize => "💬 Socializing".to_string(),
        ActionType::Explore => "🔭 Exploring".to_string(),
        ActionType::Work => "⚒ Working".to_string(),
        ActionType::Repair => "🔧 Repairing".to_string(),
        ActionType::Research => "📚 Researching".to_string(),
        ActionType::Haul => "📦 Hauling".to_string(),
        ActionType::SeekMedicalCare => "🏥 Healing".to_string(),
        ActionType::BuryCorpse => "⚰️ Burying".to_string(),
        ActionType::FetchTool => "🔧 Fetching Tool".to_string(),
        ActionType::Idle => "⏳ Idle".to_string(),
        ActionType::Vandalize => "🔨 Vandalizing".to_string(),
        ActionType::Binge => "🍖 Bingeing".to_string(),
        ActionType::Daze => "😵 Dazed".to_string(),
        ActionType::Fight => "⚔️ Fighting".to_string(),
        ActionType::Refine => "⚙️ Refining".to_string(),
        ActionType::Farm => "🌾 Farming".to_string(),
        ActionType::Warden => "👮 Arresting".to_string(),
        ActionType::Sleepwalking => "💤 Sleepwalking".to_string(),
        ActionType::Tame => "♥ Taming".to_string(),
        ActionType::FireStarting => "🔥 Starting Fire".to_string(),
        ActionType::HideInRoom => "🚪 Hiding".to_string(),
        ActionType::SadWander => "😢 Wandering Sadly".to_string(),
        ActionType::FetchClothing => "👕 Fetching Clothes".to_string(),
        ActionType::Surgery => "🏥 Undergoing Surgery".to_string(),
        ActionType::Charge => "⚡ Charging".to_string(),
        ActionType::Hobby => "🎨 Hobby".to_string(),
        ActionType::Admin => "📝 Administering".to_string(),
        ActionType::ScrawlMemeticSigil => "👁 Scrawling Sigil".to_string(),
        ActionType::PreCrimeArrest => "🛡 Pre-Crime Arrest".to_string(),
        ActionType::ConsumeChemical => "💊 Consuming".to_string(),
        ActionType::CollectSample => "🧬 Collecting".to_string(),
        ActionType::UseShower => "🚿 Showering".to_string(),
        ActionType::ListenToTheHum => "🌀 Listening".to_string(),
        ActionType::Clean => "🧹 Cleaning".to_string(),
        ActionType::PurgeResidue => "🧹 Purging Ghost Code".to_string(),
        ActionType::VoidStare => "👁 Staring into Abyss".to_string(),
        ActionType::VisitSanctuary => "🧘 Seeking Sanctuary".to_string(),
        ActionType::ExtinguishFire => "🧯 Extinguishing".to_string(),
        ActionType::TreatWounds => "🩹 Treating Wounds".to_string(),
        ActionType::Flee => "🏃 Fleeing".to_string(),
        ActionType::Sabotage => "💣 Sabotaging".to_string(),
        ActionType::Protest => "🗣️ Protesting".to_string(),
        ActionType::Gossip => "🗣️ Gossiping".to_string(),
        ActionType::Philosophize => "🤔 Philosophizing".to_string(),
        ActionType::RealityCollapse => "🔥 Reality Collapse".to_string(),
        ActionType::PerformAncientRoutine => "🗿 Ancient Routine".to_string(),
        ActionType::MemeticObsession => "🌀 Memetic Obsession".to_string(),
        ActionType::Pollinate => "🌸 Pollinating".to_string(),
        ActionType::PhantomWork => "🌙 Phantom Shift".to_string(),
    }
}

fn get_terrain_tiles_in_radius(
    terrain: &TerrainGrid,
    center_x: i32,
    center_y: i32,
    radius: i32,
) -> (i32, i32, bevy::utils::HashMap<(i32, i32), TerrainType>) {
    let max_x = i32::try_from(terrain.width).unwrap_or(i32::MAX);
    let max_y = i32::try_from(terrain.height).unwrap_or(i32::MAX);

    let mut tiles = bevy::utils::HashMap::new();
    if center_x.checked_sub(radius).is_some()
        && center_x.checked_add(radius).is_some()
        && center_y.checked_sub(radius).is_some()
        && center_y.checked_add(radius).is_some()
    {
        for y in center_y.saturating_sub(radius)..=center_y.saturating_add(radius) {
            for x in center_x.saturating_sub(radius)..=center_x.saturating_add(radius) {
                if x >= 0 && y >= 0 && x < max_x && y < max_y {
                    if let Some(t) = terrain.get(x as usize, y as usize) {
                        tiles.insert((x, y), t);
                    }
                }
            }
        }
    }
    (max_x, max_y, tiles)
}

fn print_map(world: &mut World, center_x: i32, center_y: i32) {
    let radius = 10;

    // Copy terrain data before querying to avoid borrow conflicts
    let (width, height, terrain_tiles) =
        get_terrain_tiles_in_radius(world.resource::<TerrainGrid>(), center_x, center_y, radius);

    // Collect pop positions
    let pop_positions: Vec<(i32, i32)> = world
        .query::<&GridPosition>()
        .iter(world)
        .map(|p| (p.x, p.y))
        .collect();

    // Collect building positions and types
    let building_map: bevy::utils::HashMap<(i32, i32), BuildingType> = world
        .query::<(&GridPosition, &Building)>()
        .iter(world)
        .map(|(p, b)| ((p.x, p.y), b.building_type))
        .collect();

    // Collect designation positions
    let designation_positions: Vec<(i32, i32, DesignationType)> = world
        .query::<(&GridPosition, &Designation)>()
        .iter(world)
        .map(|(p, d)| (p.x, p.y, d.designation_type))
        .collect();

    // Colored legend at bottom
    let legend = format!(
        "Legend: {}={} {}={} {}={} {}={} {}={} {}={} {}={} {}={}",
        "☺".cyan().bold(),
        "pop".grey(),
        "·".green().dim(),
        "grass".grey(),
        ",".yellow(),
        "dirt".grey(),
        "▲".white().dim(),
        "rock".grey(),
        "≈".blue(),
        "water".grey(),
        "♣".green().bold(),
        "tree".grey(),
        "%".magenta(),
        "mine".grey(),
        "/".magenta(),
        "chop".grey()
    );

    // We want the whole map to be one big string inside the panel, with colored ANSI codes
    let mut map_content = String::new();

    // Print Header Row
    map_content.push_str("    "); // Offset for Y coords
    for x in center_x.saturating_sub(radius)..=center_x.saturating_add(radius) {
        if x < 0 || x >= width {
            map_content.push(' ');
        } else {
            map_content.push_str(&(x.abs() % 10).to_string().cyan().to_string());
        }
    }
    map_content.push('\n');

    for y in center_y.saturating_sub(radius)..=center_y.saturating_add(radius) {
        map_content.push_str(&format!("{y:3} ").cyan().to_string());
        for x in center_x.saturating_sub(radius)..=center_x.saturating_add(radius) {
            if x < 0 || y < 0 || x >= width || y >= height {
                map_content.push(' ');
                continue;
            }

            // Priority: Pop > Building > Designation > Terrain
            if pop_positions.iter().any(|&(px, py)| px == x && py == y) {
                map_content.push_str(&"☺".cyan().bold().to_string());
                continue;
            }

            if let Some(bt) = building_map.get(&(x, y)) {
                let s = match bt {
                    BuildingType::Wall => "█".white().to_string(),
                    _ => "□".yellow().to_string(),
                };
                map_content.push_str(&s);
                continue;
            }

            if let Some((_, _, dt)) = designation_positions
                .iter()
                .find(|&&(dx, dy, _)| dx == x && dy == y)
            {
                let c = match dt {
                    DesignationType::Mine => '%',
                    DesignationType::Chop => '/',
                    DesignationType::Demolish => 'X',
                    DesignationType::Repair => '+',
                    DesignationType::SetZone(_) => 'Z',
                    DesignationType::Tame => 'T',
                    DesignationType::ClearFlora => 'F',
                    DesignationType::JuryRig => 'J',
                    DesignationType::Cannibalize => 'C',
                    DesignationType::Destroy => 'D',
                    DesignationType::CollectSample => 'S',
                    DesignationType::Consume => 'E',
                    DesignationType::ExcavateDust => 'U',
                };
                map_content.push_str(&format!("{c}").magenta().to_string());
                continue;
            }

            let tile = terrain_tiles
                .get(&(x, y))
                .copied()
                .unwrap_or(TerrainType::Grass);
            let s = match tile {
                TerrainType::Grass => "·".green().dim().to_string(),
                TerrainType::Dirt => ",".yellow().to_string(),
                TerrainType::Rock => "▲".white().dim().to_string(),
                TerrainType::Water => "≈".blue().to_string(),
                TerrainType::Tree => "♣".green().bold().to_string(),
                TerrainType::Path => "=".white().to_string(),
                TerrainType::Shrub => "\"".green().dim().to_string(),
                TerrainType::Sapling => "t".green().dim().to_string(),
                TerrainType::DeepRock => "▓".white().dim().to_string(),
                TerrainType::Crater => "o".white().dim().to_string(),
                TerrainType::MagmaRock => "≈".red().to_string(),
                TerrainType::SporeBloom => "♣".magenta().to_string(),
                TerrainType::Ash => "·".white().dim().to_string(),
                TerrainType::Artifact => "Ω".yellow().bold().to_string(),
                TerrainType::FaultLine(true) => "≈".red().to_string(),
                TerrainType::FaultLine(false) => "–".white().dim().to_string(),
                TerrainType::IndestructibleStump => "I".white().bold().to_string(),
                TerrainType::Bridge => "=".yellow().to_string(),
                TerrainType::Void => " ".black().to_string(),
            };
            map_content.push_str(&s);
        }
        map_content.push('\n');
    }

    map_content.push('\n');
    map_content.push_str(&legend);

    let mut table = Table::new();
    table
        .load_preset(comfy_table::presets::UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_header(vec![comfy_table::Cell::new(format!(
            "=== Map around ({center_x}, {center_y}) ==="
        ))
        .add_attribute(comfy_table::Attribute::Bold)
        .fg(comfy_table::Color::Cyan)])
        .add_row(vec![comfy_table::Cell::new(map_content)]);

    print_dashboard_table("", table);
}

fn build_at(world: &mut World, building_type: BuildingType, x: i32, y: i32) {
    // Give resources for building
    {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.wood = 100.0;
        resources.stone = 100.0;
    }

    let success = try_place_building(world, x, y, building_type);
    if success {
        print_dashboard_panel(
            "SUCCESS",
            &format!("Built {building_type:?} at ({x}, {y})"),
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        );
    } else {
        // Check why it failed
        let terrain = world.resource::<TerrainGrid>();
        let tile = terrain.get(x as usize, y as usize);
        let occupied = world.resource::<OccupiedTiles>();

        if tile.is_none() {
            print_dashboard_panel(
                "ERROR",
                &format!("Failed: ({x}, {y}) is out of bounds"),
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
        } else if occupied.0.contains(&(x, y)) {
            print_dashboard_panel(
                "ERROR",
                &format!("Failed: ({x}, {y}) is already occupied"),
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
        } else if let Some(t) = tile {
            print_dashboard_panel(
                "ERROR",
                &format!("Failed: cannot build on {t:?} at ({x}, {y})"),
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
        }
    }
}

fn designate_at(world: &mut World, designation_type: DesignationType, x: i32, y: i32) {
    let success = try_designate(world, x, y, designation_type);
    if success {
        print_dashboard_panel(
            "SUCCESS",
            &format!("Designated {designation_type:?} at ({x}, {y})"),
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }

    // Check why it failed
    let terrain = world.resource::<TerrainGrid>();
    let tile = terrain.get(x as usize, y as usize);
    let tile_name = tile.map_or("Unknown".to_string(), |t| format!("{t:?}"));

    let err_msg = match designation_type {
        DesignationType::Mine => {
            if tile == Some(TerrainType::Rock) {
                format!("Failed: already designated at ({x}, {y})")
            } else {
                format!("Failed: ({x}, {y}) is {tile_name}, need Rock for mining")
            }
        }
        DesignationType::Chop => {
            if tile == Some(TerrainType::Tree) {
                format!("Failed: already designated at ({x}, {y})")
            } else {
                format!("Failed: ({x}, {y}) is {tile_name}, need Tree for chopping")
            }
        }
        DesignationType::Demolish | DesignationType::Destroy => {
            format!("Failed: no building at ({x}, {y})")
        }
        DesignationType::Repair => format!("Failed: no building to repair at ({x}, {y})"),
        DesignationType::SetZone(_) => format!("Failed: cannot set zone at ({x}, {y})"),
        DesignationType::Tame => format!("Failed: no wild animal at ({x}, {y})"),
        DesignationType::ClearFlora => format!("Failed: no flora at ({x}, {y})"),
        DesignationType::JuryRig => format!("Failed: no building to jury-rig at ({x}, {y})"),
        DesignationType::Cannibalize => format!("Failed: no Lander at ({x}, {y})"),
        DesignationType::CollectSample => format!("Failed: no Flora or Fauna at ({x}, {y})"),
        DesignationType::Consume => return, // Do nothing for consume
        DesignationType::ExcavateDust => {
            format!("Failed: no Desire Dust at ({x}, {y})")
        }
    };

    print_dashboard_panel(
        "ERROR",
        &err_msg,
        Some(comfy_table::Color::Red),
        Some(comfy_table::Attribute::Bold),
    );
}

fn print_dust_report(world: &mut World) {
    let minds: Vec<Vec<(i32, i32)>> = world
        .query::<&RoadMind>()
        .iter(world)
        .map(|mind| mind.tiles.clone())
        .collect();
    // Measurable speed effect: pops standing on dust right now (before the dust borrow).
    let pop_speeds: Vec<(i32, i32, f32, f32)> = world
        .query::<(&Pop, &GridPosition, &scale::layer1::pop::Speed)>()
        .iter(world)
        .map(|(_, pos, speed)| (pos.x, pos.y, speed.current, speed.base))
        .collect();
    let dust = world.resource::<DesireDust>();
    let mut lines = vec![
        format!("Dusty tiles: {}", dust.dusty_tile_count()),
        format!("Total dust: {:.2}", dust.total()),
        format!("RoadMinds: {}", minds.len()),
    ];
    let mut on_dust = 0;
    let mut bonus_sum = 0.0f32;
    for (x, y, current, base) in pop_speeds {
        if dust.amount_at(x, y) > 0.0 {
            on_dust += 1;
            bonus_sum += current / base - 1.0;
        }
    }
    if on_dust > 0 {
        lines.push(format!(
            "Pops on dust: {on_dust} (avg speed bonus +{:.0}%)",
            bonus_sum / on_dust as f32 * 100.0
        ));
    }
    for (i, tiles) in minds.iter().enumerate() {
        let coords: Vec<String> = tiles.iter().map(|(x, y)| format!("({x},{y})")).collect();
        lines.push(format!("  Mind {i}: {} tiles {}", tiles.len(), coords.join(" ")));
    }
    let mut top: Vec<((i32, i32), f32)> = dust
        .iter()
        .map(|(tile, amount)| (*tile, *amount))
        .collect();
    top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    lines.push("Dustiest tiles:".to_string());
    for ((x, y), amount) in top.iter().take(8) {
        lines.push(format!("  ({x},{y}): {amount:.2}"));
    }
    print_dashboard_panel(
        "DESIRE DUST",
        &lines.join("\n"),
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Find the colony's planet (spawning a bare one if the schedule hasn't).
fn colony_planet_entity(world: &mut World) -> bevy_ecs::prelude::Entity {
    use bevy_ecs::prelude::With;
    if let Some(planet) = world
        .query_filtered::<bevy_ecs::prelude::Entity, With<scale::layer2::generation::Planet>>()
        .iter(world)
        .next()
    {
        return planet;
    }
    world
        .spawn((
            scale::layer2::generation::Planet,
            scale::layer2::debris::OrbitalDebris(0.0),
        ))
        .id()
}

fn handle_launch_sat_command(world: &mut World) {
    use scale::layer2::propaganda::{
        launch_slogan_satellite, update_constellation, SATELLITE_METAL_COST, SATELLITE_TOOL_COST,
    };
    let planet = colony_planet_entity(world);
    match launch_slogan_satellite(world, planet) {
        Some(sat) => {
            update_constellation(world);
            let word = world
                .get::<scale::layer2::propaganda::SloganSatellite>(sat)
                .map(|s| s.word.clone())
                .unwrap_or_default();
            print_dashboard_panel(
                "SLOGAN SATELLITE LAUNCHED",
                &format!(
                    "Satellite #{} in orbit (carries \"{word}\").\nCost: {SATELLITE_METAL_COST} metal, {SATELLITE_TOOL_COST} tools. LaunchEvent emitted.",
                    sat.index()
                ),
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
        }
        None => {
            print_dashboard_panel(
                "LAUNCH FAILED",
                &format!(
                    "Not enough resources: need {SATELLITE_METAL_COST} metal and {SATELLITE_TOOL_COST} tools."
                ),
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
        }
    }
}

fn print_constellation_report(world: &mut World) {
    use scale::layer2::propaganda::{Constellation, SloganMessage, SloganSatellite, PROPAGANDA_MODIFIER_LABEL};
    let (active, message_text, kind, sat_count, broadcast_value) = match world.get_resource::<Constellation>() {
        Some(c) => (
            c.active(),
            c.message.text().to_string(),
            match &c.message {
                SloganMessage::Hope(_) => "HOPE",
                SloganMessage::Despair(_) => "DESPAIR (hacked)",
                SloganMessage::Dark => "DARK",
            }
            .to_string(),
            c.satellites.len(),
            c.morale_value(),
        ),
        None => (false, String::new(), "DARK".to_string(), 0, 0.0),
    };
    let sat_ids: Vec<String> = world
        .query::<(bevy_ecs::prelude::Entity, &SloganSatellite)>()
        .iter(world)
        .map(|(e, s)| format!("#{} \"{}\"", e.index(), s.word))
        .collect();
    let mut boosted = 0u32;
    let mut pops = 0u32;
    for morale in world.query::<&Morale>().iter(world) {
        pops += 1;
        if morale
            .modifiers
            .iter()
            .any(|m| m.label == PROPAGANDA_MODIFIER_LABEL)
        {
            boosted += 1;
        }
    }
    let debris_ledger = world
        .get_resource::<scale::layer2::debris::OrbitalDebris>()
        .map(|d| d.0)
        .unwrap_or(0.0);
    let night = world
        .get_resource::<scale::layer1::day_night::DayNightCycle>()
        .map(|c| format!("{:?}", c.time_of_day))
        .unwrap_or_else(|| "unknown".to_string());
    let lines = [
        format!("Active: {active} ({kind})"),
        format!("Message: \"{message_text}\""),
        format!("Broadcast value: {broadcast_value:+.2} morale/pop"),
        format!("Linked satellites: {sat_count}"),
        format!("Satellites: {}", sat_ids.join(" ")),
        format!("Pops with Broadcast modifier: {boosted}/{pops}"),
        format!("Orbital debris (ledger): {debris_ledger:.2}"),
        format!("Sky: {night}"),
    ];
    print_dashboard_panel(
        "PROPAGANDA CONSTELLATION",
        &lines.join("\n"),
        Some(comfy_table::Color::Magenta),
        Some(comfy_table::Attribute::Bold),
    );
}

fn handle_hack_sat_command(world: &mut World, parts: &[&str]) {
    use scale::layer2::propaganda::{
        apply_constellation_morale, hack_constellation, update_constellation, Constellation,
    };
    let active = world
        .get_resource::<Constellation>()
        .is_some_and(|c| c.active());
    if !active {
        print_dashboard_panel(
            "HACK FAILED",
            "No active constellation to hack — the sky is dark.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    let message = if parts.len() > 1 {
        parts[1..].join(" ")
    } else {
        "OBEY THE STATIC".to_string()
    };
    hack_constellation(world, message.clone());
    update_constellation(world);
    apply_constellation_morale(world);
    print_dashboard_panel(
        "CONSTELLATION HACKED",
        &format!("Rival broadcast overwrote the sky: \"{message}\". Despair rains down."),
        Some(comfy_table::Color::Red),
        Some(comfy_table::Attribute::Bold),
    );
}

fn handle_shootdown_command(world: &mut World, parts: &[&str]) {
    use scale::layer2::propaganda::{shoot_down_satellite, SloganSatellite};
    let sats: Vec<bevy_ecs::prelude::Entity> = world
        .query::<(bevy_ecs::prelude::Entity, &SloganSatellite)>()
        .iter(world)
        .map(|(e, _)| e)
        .collect();
    // No id given: shoot down the first linked satellite (scripted playthroughs).
    let target: Option<u32> = parts
        .get(1)
        .and_then(|s| s.parse().ok())
        .or_else(|| sats.first().map(|e| e.index()));
    let Some(target) = target else {
        print_dashboard_panel(
            "ERROR",
            "Usage: shootdown [satellite_id] (ids from `constellation`)",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let sat = sats.into_iter().find(|e| e.index() == target);
    match sat {
        Some(sat) => {
            use scale::layer2::propaganda::{
                apply_constellation_morale, update_constellation,
            };
            shoot_down_satellite(world, sat);
            update_constellation(world);
            apply_constellation_morale(world);
            print_dashboard_panel(
                "SATELLITE DOWN",
                &format!(
                    "Shot down our own satellite #{target}. The message dies; the wreckage feeds orbital debris."
                ),
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
        None => {
            print_dashboard_panel(
                "ERROR",
                &format!("No slogan satellite with id #{target}."),
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
        }
    }
}

/// Debug command: grant colony resources (playtesting aid).
fn handle_give_command(world: &mut World, parts: &[&str]) {
    let (resource, amount) = match (parts.get(1), parts.get(2)) {
        (Some(name), Some(n)) => (name.to_lowercase(), n.parse::<f32>().unwrap_or(0.0)),
        _ => {
            print_dashboard_panel(
                "ERROR",
                "Usage: give <food|wood|stone|metal|tools|fuel|blocks|planks> <amount>",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    let mut resources = world.resource_mut::<ColonyResources>();
    match resource.as_str() {
        "food" => resources.food += amount,
        "wood" => resources.wood += amount,
        "stone" => resources.stone += amount,
        "metal" => resources.metal += amount,
        "tools" => resources.tools += amount,
        "fuel" => resources.fuel += amount,
        // Spec 347: refined construction materials, so Great Work
        // stages can be funded in headless playtests.
        "blocks" => resources.blocks += amount,
        "planks" => resources.planks += amount,
        _ => {
            print_dashboard_panel(
                "ERROR",
                &format!("Unknown resource: '{resource}'."),
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    }
    print_dashboard_panel(
        "RESOURCES GRANTED",
        &format!("+{amount} {resource} (debug)."),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

fn print_designations(world: &mut World) {
    let mut count = 0;
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Type").add_attribute(Attribute::Bold),
            Cell::new("Pos").add_attribute(Attribute::Bold),
        ]);

    for (pos, designation) in world.query::<(&GridPosition, &Designation)>().iter(world) {
        let type_str = format!("{:?}", designation.designation_type);
        let type_cell = match designation.designation_type {
            DesignationType::Mine => Cell::new(type_str).fg(Color::Yellow),
            DesignationType::Demolish => Cell::new(type_str).fg(Color::Red),
            DesignationType::Chop => Cell::new(type_str).fg(Color::Green),
            DesignationType::Repair => Cell::new(type_str).fg(Color::Blue),
            DesignationType::SetZone(_) => Cell::new(type_str).fg(Color::Magenta),
            DesignationType::Tame => Cell::new(type_str).fg(Color::Cyan),
            DesignationType::ClearFlora => Cell::new(type_str).fg(Color::Green),
            DesignationType::JuryRig => Cell::new(type_str).fg(Color::Yellow),
            DesignationType::Cannibalize => Cell::new(type_str).fg(Color::Red),
            DesignationType::Destroy => Cell::new(type_str).fg(Color::Red),
            DesignationType::CollectSample => Cell::new(type_str).fg(Color::Cyan),
            DesignationType::Consume => Cell::new(type_str).fg(Color::Red),
            DesignationType::ExcavateDust => Cell::new(type_str).fg(Color::Yellow),
        };

        table.add_row(vec![type_cell, Cell::new(format!("{},{}", pos.x, pos.y))]);
        count += 1;
    }

    if count == 0 {
        print_dashboard_panel(
            "Active Designations",
            "(No active designations)",
            Some(comfy_table::Color::DarkGrey),
            Some(comfy_table::Attribute::Italic),
        );
    } else {
        print_dashboard_table("Active Designations", table);
    }
}

fn find_terrain(world: &mut World, terrain_name: &str, max_count: usize) {
    let target = match terrain_name.to_lowercase().as_str() {
        "rock" | "r" => TerrainType::Rock,
        "tree" | "t" => TerrainType::Tree,
        "grass" | "g" => TerrainType::Grass,
        "water" | "w" => TerrainType::Water,
        "dirt" | "d" => TerrainType::Dirt,
        _ => {
            print_dashboard_panel(
                "ERROR",
                &format!(
                    "Unknown terrain type: {terrain_name}. Try: rock, tree, grass, water, dirt"
                ),
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };

    let terrain = world.resource::<TerrainGrid>();
    let mut found = Vec::new();

    for y in 0..terrain.height {
        for x in 0..terrain.width {
            if terrain.get(x, y) == Some(target) {
                found.push((x, y));
                if found.len() >= max_count {
                    break;
                }
            }
        }
        if found.len() >= max_count {
            break;
        }
    }

    if found.is_empty() {
        print_dashboard_panel(
            &format!("Search Results: {target:?} (Max: {max_count})"),
            "(No tiles found)",
            Some(comfy_table::Color::DarkGrey),
            Some(comfy_table::Attribute::Italic),
        );
    } else {
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec![
                Cell::new("ID").add_attribute(Attribute::Bold),
                Cell::new("Coordinate").add_attribute(Attribute::Bold),
                Cell::new("Terrain").add_attribute(Attribute::Bold),
            ]);

        let color = get_terrain_color_headless(target);

        for (i, (x, y)) in found.iter().enumerate() {
            table.add_row(vec![
                Cell::new((i + 1).to_string()),
                Cell::new(format!("{}, {}", x, y)),
                Cell::new(format!("{:?}", target)).fg(color),
            ]);
        }

        print_dashboard_table(
            &format!("Search Results: {target:?} (Found: {})", found.len()),
            table,
        );
    }
}

/// Configuration for semantic terrain scanning radius.
///
/// Use this struct to ensure the scanning radius stays within valid bounds
/// before passing it into `scan_terrain`. Attempting to scan too large of an area
/// may result in significant performance degradation or overflows.
///
/// # Examples
///
/// ```
/// use scale::bin::headless::ScanRadius;
///
/// // Initialize a valid scan radius.
/// let radius = ScanRadius::new(10).unwrap();
///
/// // Reject absurdly large bounds that might cause an overflow during the scan loop.
/// assert!(ScanRadius::new(i32::MAX).is_err());
/// ```
pub struct ScanRadius(i32);

impl ScanRadius {
    /// Creates a new `ScanRadius`, validating it against hardcoded bounds (0 to 100).
    ///
    /// # Panics
    ///
    /// Does not panic, but returns an error if the radius is outside the `0..=100` range.
    pub fn new(radius: i32) -> Result<Self, String> {
        if !(0..=100).contains(&radius) {
            return Err("Radius must be between 0 and 100".to_string());
        }
        Ok(Self(radius))
    }

    #[doc(hidden)]
    pub fn get(&self) -> i32 {
        self.0
    }
}

/// Semantic terrain scan - outputs parseable coordinate:type pairs
fn scan_terrain(world: &mut World, center_x: i32, center_y: i32, radius: ScanRadius) {
    let radius = radius.get();
    // Copy terrain data before querying to avoid borrow conflicts
    let (width, height, terrain_tiles) =
        get_terrain_tiles_in_radius(world.resource::<TerrainGrid>(), center_x, center_y, radius);

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Coord").add_attribute(Attribute::Bold),
            Cell::new("Terrain").add_attribute(Attribute::Bold),
            Cell::new("Walk").add_attribute(Attribute::Bold),
            Cell::new("Build").add_attribute(Attribute::Bold),
            Cell::new("Occ").add_attribute(Attribute::Bold),
            Cell::new("Pop").add_attribute(Attribute::Bold),
            Cell::new("Farm").add_attribute(Attribute::Bold),
            Cell::new("House").add_attribute(Attribute::Bold),
            Cell::new("Desig").add_attribute(Attribute::Bold),
        ]);

    // Collect entities at positions
    let pop_positions: Vec<(i32, i32)> = world
        .query::<(&Pop, &GridPosition)>()
        .iter(world)
        .map(|(_, p)| (p.x, p.y))
        .collect();

    let farm_positions: Vec<(i32, i32)> = world
        .query::<(&Farm, &GridPosition)>()
        .iter(world)
        .map(|(_, p)| (p.x, p.y))
        .collect();

    let housing_positions: Vec<(i32, i32)> = world
        .query::<(&Housing, &GridPosition)>()
        .iter(world)
        .map(|(_, p)| (p.x, p.y))
        .collect();

    let designation_positions: Vec<(i32, i32, String)> = world
        .query::<(&Designation, &GridPosition)>()
        .iter(world)
        .map(|(d, p)| {
            let dt = match d.designation_type {
                DesignationType::Mine => "mine",
                DesignationType::Chop => "chop",
                DesignationType::ExcavateDust => "excavate_dust",
                DesignationType::Demolish => "demolish",
                DesignationType::Repair => "repair",
                DesignationType::SetZone(_) => "zone",
                DesignationType::Tame => "tame",
                DesignationType::ClearFlora => "clear_flora",
                DesignationType::JuryRig => "jury_rig",
                DesignationType::Cannibalize => "cannibalize",
                DesignationType::Destroy => "destroy",
                DesignationType::CollectSample => "collect_sample",
                DesignationType::Consume => "consume",
            };
            (p.x, p.y, dt.to_string())
        })
        .collect();

    let mut found_count = 0;
    for y in center_y.saturating_sub(radius)..=center_y.saturating_add(radius) {
        for x in center_x.saturating_sub(radius)..=center_x.saturating_add(radius) {
            if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            found_count += 1;

            let tile = terrain_tiles
                .get(&(x, y))
                .copied()
                .unwrap_or(TerrainType::Grass);
            let terrain_name = format_terrain_name(tile);

            let walkable = tile.is_walkable();
            let buildable = matches!(
                tile,
                TerrainType::Grass
                    | TerrainType::Dirt
                    | TerrainType::Rock
                    | TerrainType::Path
                    | TerrainType::FaultLine(_)
            );

            // Check for entities
            let has_pop = pop_positions.iter().any(|&(px, py)| px == x && py == y);
            let has_farm = farm_positions.iter().any(|&(fx, fy)| fx == x && fy == y);
            let has_housing = housing_positions.iter().any(|&(hx, hy)| hx == x && hy == y);
            let designation = designation_positions
                .iter()
                .find(|(dx, dy, _)| *dx == x && *dy == y)
                .map_or("-", |(_, _, dt)| dt.as_str());

            let occupied = world.resource::<OccupiedTiles>().0.contains(&(x, y));

            let bool_to_str = |b: bool| if b { "✓" } else { "✗" };
            let get_color = |b: bool| {
                if b {
                    comfy_table::Color::Green
                } else {
                    comfy_table::Color::DarkGrey
                }
            };

            table.add_row(vec![
                Cell::new(format!("{},{}", x, y)),
                Cell::new(terrain_name).fg(get_terrain_color_headless(tile)),
                Cell::new(bool_to_str(walkable)).fg(get_color(walkable)),
                Cell::new(bool_to_str(buildable)).fg(get_color(buildable)),
                Cell::new(bool_to_str(occupied)).fg(get_color(occupied)),
                Cell::new(bool_to_str(has_pop)).fg(if has_pop {
                    comfy_table::Color::Cyan
                } else {
                    comfy_table::Color::DarkGrey
                }),
                Cell::new(bool_to_str(has_farm)).fg(if has_farm {
                    comfy_table::Color::Green
                } else {
                    comfy_table::Color::DarkGrey
                }),
                Cell::new(bool_to_str(has_housing)).fg(if has_housing {
                    comfy_table::Color::Yellow
                } else {
                    comfy_table::Color::DarkGrey
                }),
                Cell::new(designation).fg(if designation != "-" {
                    comfy_table::Color::Magenta
                } else {
                    comfy_table::Color::DarkGrey
                }),
            ]);
        }
    }

    if found_count == 0 {
        print_dashboard_panel(
            &format!("Scan Results: Center ({center_x}, {center_y}) | Radius {radius}"),
            "(No tiles found in range)",
            Some(comfy_table::Color::DarkGrey),
            Some(comfy_table::Attribute::Italic),
        );
    } else {
        print_dashboard_table(
            &format!("Scan Results: Center ({center_x}, {center_y}) | Radius {radius}"),
            table,
        );
    }
}

const fn get_terrain_color_headless(t: TerrainType) -> comfy_table::Color {
    match t {
        TerrainType::Grass => comfy_table::Color::Green,
        TerrainType::Dirt => comfy_table::Color::DarkYellow,
        TerrainType::Rock => comfy_table::Color::Grey,
        TerrainType::Water => comfy_table::Color::Blue,
        TerrainType::Tree | TerrainType::Sapling | TerrainType::Shrub => {
            comfy_table::Color::DarkGreen
        }
        TerrainType::Path => comfy_table::Color::DarkGrey,
        TerrainType::DeepRock => comfy_table::Color::DarkGrey,
        TerrainType::Crater => comfy_table::Color::DarkGrey,
        TerrainType::MagmaRock => comfy_table::Color::Red,
        TerrainType::SporeBloom => comfy_table::Color::Magenta,
        TerrainType::Ash => comfy_table::Color::DarkGrey,
        TerrainType::Artifact => comfy_table::Color::Yellow,
        TerrainType::FaultLine(true) => comfy_table::Color::Red,
        TerrainType::FaultLine(false) => comfy_table::Color::DarkGrey,
        TerrainType::IndestructibleStump => comfy_table::Color::White,
        TerrainType::Bridge => comfy_table::Color::Yellow,
        TerrainType::Void => comfy_table::Color::Black,
    }
}

/// Get info about a single tile
fn format_terrain_name(tile: TerrainType) -> &'static str {
    match tile {
        TerrainType::Grass => "Grass",
        TerrainType::Dirt => "Dirt",
        TerrainType::Rock => "Rock",
        TerrainType::Water => "Water",
        TerrainType::Tree => "Tree",
        TerrainType::Path => "Path",
        TerrainType::Shrub => "Shrub",
        TerrainType::Sapling => "Sapling",
        TerrainType::DeepRock => "Deep Rock",
        TerrainType::Crater => "Crater",
        TerrainType::MagmaRock => "Magma Rock",
        TerrainType::SporeBloom => "Spore Bloom",
        TerrainType::Ash => "Ash",
        TerrainType::Artifact => "Artifact",
        TerrainType::FaultLine(true) => "Fault Line (Open)",
        TerrainType::FaultLine(false) => "Fault Line (Closed)",
        TerrainType::IndestructibleStump => "Indestructible Stump",
        TerrainType::Bridge => "Bridge",
        TerrainType::Void => "Void",
    }
}

fn get_tile_info(world: &mut World, x: i32, y: i32) {
    let terrain = world.resource::<TerrainGrid>();
    let max_x = i32::try_from(terrain.width).unwrap_or(i32::MAX);
    let max_y = i32::try_from(terrain.height).unwrap_or(i32::MAX);

    if x < 0 || y < 0 || x >= max_x || y >= max_y {
        print_dashboard_panel(
            &format!("Tile Info: ({x}, {y})"),
            "ERROR: Coordinates out of bounds",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }

    let tile = terrain
        .get(x as usize, y as usize)
        .unwrap_or(TerrainType::Grass);
    let terrain_name = format_terrain_name(tile);

    let walkable = tile.is_walkable();
    let buildable = matches!(
        tile,
        TerrainType::Grass
            | TerrainType::Dirt
            | TerrainType::Rock
            | TerrainType::Path
            | TerrainType::FaultLine(_)
    );

    // Check for entities
    let has_pop = world
        .query::<(&Pop, &GridPosition)>()
        .iter(world)
        .any(|(_, p)| p.x == x && p.y == y);

    let has_farm = world
        .query::<(&Farm, &GridPosition)>()
        .iter(world)
        .any(|(_, p)| p.x == x && p.y == y);

    let has_housing = world
        .query::<(&Housing, &GridPosition)>()
        .iter(world)
        .any(|(_, p)| p.x == x && p.y == y);

    let occupied = world.resource::<OccupiedTiles>().0.contains(&(x, y));

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Property").add_attribute(Attribute::Bold),
            Cell::new("Value").add_attribute(Attribute::Bold),
        ]);

    let bool_to_str = |b: bool| if b { "✓" } else { "✗" };
    let get_color = |b: bool| {
        if b {
            comfy_table::Color::Green
        } else {
            comfy_table::Color::DarkGrey
        }
    };

    table.add_row(vec![
        Cell::new("Terrain"),
        Cell::new(terrain_name).fg(get_terrain_color_headless(tile)),
    ]);
    table.add_row(vec![
        Cell::new("Walkable"),
        Cell::new(bool_to_str(walkable)).fg(get_color(walkable)),
    ]);
    table.add_row(vec![
        Cell::new("Buildable"),
        Cell::new(bool_to_str(buildable)).fg(get_color(buildable)),
    ]);
    table.add_row(vec![
        Cell::new("Occupied"),
        Cell::new(bool_to_str(occupied)).fg(get_color(occupied)),
    ]);

    table.add_row(vec![
        Cell::new("Pop Present"),
        Cell::new(bool_to_str(has_pop)).fg(if has_pop {
            comfy_table::Color::Cyan
        } else {
            comfy_table::Color::DarkGrey
        }),
    ]);
    table.add_row(vec![
        Cell::new("Farm Present"),
        Cell::new(bool_to_str(has_farm)).fg(if has_farm {
            comfy_table::Color::Green
        } else {
            comfy_table::Color::DarkGrey
        }),
    ]);
    table.add_row(vec![
        Cell::new("Housing Present"),
        Cell::new(bool_to_str(has_housing)).fg(if has_housing {
            comfy_table::Color::Yellow
        } else {
            comfy_table::Color::DarkGrey
        }),
    ]);

    print_dashboard_table(&format!("Tile Info: ({x}, {y})"), table);
}


/// Spec 347: `greatwork start <skyhook|aegis|spire>` begins a catalog
/// Great Work; `greatwork` alone lists them.
fn handle_greatwork_command(world: &mut World, parts: &[&str]) {
    match parts.get(1).map(|s| s.to_lowercase()).as_deref() {
        Some("start") => {
            let Some(kind) = parts.get(2).and_then(|s| GreatWorkKind::parse(s)) else {
                print_dashboard_panel(
                    "ERROR",
                    "Usage: greatwork start <skyhook|aegis|spire>",
                    Some(comfy_table::Color::Red),
                    Some(comfy_table::Attribute::Bold),
                );
                return;
            };
            // One active instance per kind: refuse duplicates.
            let already = {
                let mut query = world.query::<&GreatWork>();
                query.iter(world).any(|w| w.kind == Some(kind))
            };
            if already {
                print_dashboard_panel(
                    "GREAT WORK",
                    &format!("{} is already underway or complete.", kind.name()),
                    Some(comfy_table::Color::Yellow),
                    Some(comfy_table::Attribute::Bold),
                );
                return;
            }
            let total: f32 = kind.stages().iter().map(|s| s.amount).sum();
            scale::layer1::construction::spawn_great_work_kind(world, kind);
            print_dashboard_panel(
                "GREAT WORK BEGUN",
                &format!(
                    "{}: {} stages, {:.0} resources in total. Fund it with `contribute [amount]`.",
                    kind.name(),
                    kind.stages().len(),
                    total
                ),
                Some(comfy_table::Color::Cyan),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Some("contribute") => handle_contribute_command(world, parts),
        _ => print_great_works(world),
    }
}

/// Spec 347: `contribute [amount]` applies stockpile resources to the
/// active Great Work's current stage (never creates resources: the
/// amount actually applied is capped by the real stockpile).
fn handle_contribute_command(world: &mut World, parts: &[&str]) {
    // `contribute 500` (parts[1]) or `greatwork contribute 500` (parts[2]).
    let amount_arg = if parts.first().is_some_and(|p| *p == "greatwork") {
        parts.get(2)
    } else {
        parts.get(1)
    };
    let amount = amount_arg
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(f32::MAX);
    let Some(work) = scale::layer1::construction::first_active_great_work(world) else {
        print_dashboard_panel(
            "GREAT WORK",
            "No active Great Work to fund. Start one: greatwork start <skyhook|aegis|spire>",
            Some(comfy_table::Color::Yellow),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    world
        .resource_mut::<bevy_ecs::event::Events<ContributeToGreatWorkEvent>>()
        .send(ContributeToGreatWorkEvent { work, amount });
    print_dashboard_panel(
        "GREAT WORK",
        "Contribution sent to the stockpile crews — it lands on the next tick. Run `tick 1` then `greatworks`.",
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
}

fn print_great_works(world: &mut World) {
    let mut count = 0;
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("ID").add_attribute(Attribute::Bold),
            Cell::new("Name").add_attribute(Attribute::Bold),
            Cell::new("Phase").add_attribute(Attribute::Bold),
            Cell::new("Progress").add_attribute(Attribute::Bold),
            Cell::new("Status").add_attribute(Attribute::Bold),
        ]);

    for (entity, work, progress, operational) in world
        .query::<(
            Entity,
            &GreatWork,
            Option<&ConstructionProgress>,
            Option<&OperationalGreatWork>,
        )>()
        .iter(world)
    {
        let is_operational = operational.is_some() || work.is_completed();
        let phase_str = if is_operational {
            "Completed".to_string()
        } else {
            format!("{} / {}", work.current_phase + 1, work.stage_count())
        };

        let progress_str = if is_operational {
            "Operational".to_string()
        } else if let Some(stage) = work.current_stage_def() {
            // Spec 347: resource-funded stage progress.
            let pct = if stage.amount > 0.0 {
                (work.stage_applied / stage.amount) * 100.0
            } else {
                0.0
            };
            format!(
                "{:?} {:.0} / {:.0} ({:.0}%)",
                stage.resource, work.stage_applied, stage.amount, pct
            )
        } else if let Some(prog) = progress {
            let pct = if prog.total_work_required > 0.0 {
                (prog.current_work / prog.total_work_required) * 100.0
            } else {
                0.0
            };
            format!(
                "{:.1} / {:.1} ({:.0}%)",
                prog.current_work, prog.total_work_required, pct
            )
        } else {
            "Waiting".to_string()
        };

        let status_cell = if is_operational {
            Cell::new("Online").fg(Color::Green)
        } else {
            Cell::new("Building").fg(Color::Yellow)
        };

        let name_color = if is_operational {
            Color::Green
        } else {
            Color::Yellow
        };

        table.add_row(vec![
            Cell::new(entity.index().to_string()),
            Cell::new(&work.name).fg(name_color),
            Cell::new(phase_str),
            Cell::new(progress_str),
            status_cell,
        ]);
        count += 1;
    }

    if count == 0 {
        print_dashboard_panel(
            "Great Works",
            "(No great works found)",
            Some(comfy_table::Color::DarkGrey),
            Some(comfy_table::Attribute::Italic),
        );
    } else {
        print_dashboard_table("Great Works", table);
    }
}

/// Black Market panel (headless `blackmarket` command, Spec 348).
fn print_black_market(world: &mut World) {
    use scale::layer1::economy::black_market::{
        black_market_status_label, corruption_efficiency_factor, smuggler_count,
        BlackMarketState, ColonyStats,
    };
    let (unmet, corruption) = world
        .get_resource::<ColonyStats>()
        .map_or((0, 0.0), |s| (s.unmet_luxury, s.corruption));
    let (spawned, trades, siphoned, delivered) = world
        .get_resource::<BlackMarketState>()
        .map_or((0, 0, 0.0, 0.0), |s| {
            (
                s.total_spawned,
                s.total_trades,
                s.total_credits_siphoned,
                s.total_luxury_delivered,
            )
        });
    let docked = smuggler_count(world);
    let status = black_market_status_label(world);
    print_dashboard_panel(
        "BLACK MARKET",
        &format!(
            "Status: {status}\nUnmet luxury needs: {unmet}\nSmugglers docked: {docked} (spawned {spawned}, trades {trades})\nLuxury delivered: {delivered:.1}\nCredits siphoned: {siphoned:.1}\nCorruption: {corruption:.1} (wage efficiency {:.0}%)\n\nWhere the state fails, the underworld provides (Spec 348):\nunmet leisure needs draw smugglers who dock, deliver luxury,\nsiphon credits at markup, and deepen corruption — which skims\nevery wage the colony pays. Watch STATS smugglers= / corruption=.",
            corruption_efficiency_factor(corruption) * 100.0,
        ),
        Some(comfy_table::Color::Magenta),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug trigger (headless `smuggler` command, Spec 348): a smuggler
/// answers immediately and docks with a ready trade, bypassing the
/// unmet-needs gate and the spawn cooldown.
fn handle_smuggler_command(world: &mut World) {
    use scale::layer1::economy::black_market::{
        BlackMarketState, Smuggler, SmugglerVisit,
    };
    world.spawn((
        Smuggler,
        SmugglerVisit {
            ticks_until_trade: 0,
            traded: false,
        },
    ));
    if let Some(mut state) = world.get_resource_mut::<BlackMarketState>() {
        state.total_spawned += 1;
    }
    println!(
        "A smuggler slips into a shadow berth, hold full of luxury goods. Run `tick` and watch STATS smugglers= / corruption= — or `blackmarket` for the ledger."
    );
}

/// List all buildings with positions
fn print_buildings(world: &mut World) {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("ID").add_attribute(Attribute::Bold),
            Cell::new("Type").add_attribute(Attribute::Bold),
            Cell::new("Pos").add_attribute(Attribute::Bold),
            Cell::new("Occupancy").add_attribute(Attribute::Bold),
        ]);

    let mut count = 0;

    for (entity, pos, farm) in world.query::<(Entity, &GridPosition, &Farm)>().iter(world) {
        table.add_row(vec![
            Cell::new(entity.index().to_string()),
            Cell::new("Farm").fg(Color::Green),
            Cell::new(format!("{},{}", pos.x, pos.y)),
            Cell::new(format!("{}/{}", farm.workers.len(), farm.capacity)),
        ]);
        count += 1;
    }

    for (entity, pos, housing) in world
        .query::<(Entity, &GridPosition, &Housing)>()
        .iter(world)
    {
        table.add_row(vec![
            Cell::new(entity.index().to_string()),
            Cell::new("Housing").fg(Color::Blue),
            Cell::new(format!("{},{}", pos.x, pos.y)),
            Cell::new(format!("{}/{}", housing.residents.len(), housing.capacity)),
        ]);
        count += 1;
    }

    for (entity, pos, _) in world
        .query::<(Entity, &GridPosition, &Stockpile)>()
        .iter(world)
    {
        table.add_row(vec![
            Cell::new(entity.index().to_string()),
            Cell::new("Stockpile").fg(Color::Yellow),
            Cell::new(format!("{},{}", pos.x, pos.y)),
            Cell::new("-"),
        ]);
        count += 1;
    }

    if count == 0 {
        print_dashboard_panel(
            "BUILDINGS",
            "(No buildings found)",
            Some(comfy_table::Color::DarkGrey),
            Some(comfy_table::Attribute::Italic),
        );
    } else {
        print_dashboard_table("BUILDINGS", table);
    }
}

fn print_bio(world: &mut World, target_id: u32) {
    let mut query = world.query::<(Entity, &PopName, Option<&Biography>, Option<&Dream>)>();
    let mut found = false;

    for (entity, name, bio, dream) in query.iter(world) {
        if entity.index() == target_id {
            found = true;
            let bio_title = format!("Biography for {} (ID {})", name.0, entity.index());

            if let Some(bio) = bio {
                if bio.events.is_empty() {
                    print_dashboard_panel(
                        &bio_title,
                        "(No events recorded)",
                        Some(comfy_table::Color::DarkGrey),
                        Some(comfy_table::Attribute::Italic),
                    );
                } else {
                    let mut table = Table::new();
                    table
                        .load_preset(UTF8_FULL)
                        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
                        .set_content_arrangement(ContentArrangement::Dynamic)
                        .set_header(vec![
                            Cell::new("Tick").add_attribute(Attribute::Bold),
                            Cell::new("Event").add_attribute(Attribute::Bold),
                        ]);

                    for event in &bio.events {
                        table.add_row(vec![
                            Cell::new(event.tick.to_string()),
                            Cell::new(&event.text),
                        ]);
                    }
                    print_dashboard_table(&bio_title, table);
                }
            } else {
                print_dashboard_panel(
                    &bio_title,
                    "(No biography component)",
                    Some(comfy_table::Color::DarkGrey),
                    Some(comfy_table::Attribute::Italic),
                );
            }

            if let Some(dream) = dream {
                print_dashboard_panel(
                    &format!("Last Dream (Tick {})", dream.tick),
                    &format!("\"{}\"", dream.content),
                    Some(comfy_table::Color::Magenta),
                    Some(comfy_table::Attribute::Italic),
                );
            }
            break;
        }
    }

    if !found {
        print_dashboard_panel(
            "ERROR",
            &format!("Pop with ID {target_id} not found."),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    }
}

#[cfg(feature = "nova")]
fn print_stories(world: &mut World) {
    use comfy_table::{Cell, Table};

    let tradition = world.resource::<OralTradition>();

    if tradition.stories.is_empty() {
        print_dashboard_panel(
            "ORAL TRADITION (STORIES)",
            "(No stories recorded)",
            Some(comfy_table::Color::DarkGrey),
            Some(comfy_table::Attribute::Italic),
        );
        return;
    }

    let mut table = Table::new();
    table
        .load_preset(comfy_table::presets::UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Genre").add_attribute(comfy_table::Attribute::Bold),
            Cell::new("Historical Date").add_attribute(comfy_table::Attribute::Bold),
            Cell::new("Mutations").add_attribute(comfy_table::Attribute::Bold),
            Cell::new("Story Text").add_attribute(comfy_table::Attribute::Bold),
        ]);

    let max_stories = 10;
    let total_stories = tradition.stories.len();
    let skip_count = total_stories.saturating_sub(max_stories);

    if skip_count > 0 {
        table.add_row(vec![
            Cell::new("...").fg(comfy_table::Color::DarkGrey),
            Cell::new("...").fg(comfy_table::Color::DarkGrey),
            Cell::new("...").fg(comfy_table::Color::DarkGrey),
            Cell::new(format!("{} older stories hidden", skip_count))
                .fg(comfy_table::Color::DarkGrey)
                .add_attribute(comfy_table::Attribute::Italic),
        ]);
    }

    for story in tradition.stories.iter().skip(skip_count) {
        let genre_color = match story.genre {
            StoryGenre::Heroic => comfy_table::Color::Yellow,
            StoryGenre::Tragedy => comfy_table::Color::Red,
            StoryGenre::Cautionary => comfy_table::Color::Magenta,
            StoryGenre::Trivial => comfy_table::Color::DarkGrey,
        };

        let mutations_color = if story.mutations > 5 {
            comfy_table::Color::Red
        } else if story.mutations > 0 {
            comfy_table::Color::Yellow
        } else {
            comfy_table::Color::Green
        };

        let snippet = story.text.clone();

        table.add_row(vec![
            Cell::new(story.genre.to_string())
                .fg(genre_color)
                .add_attribute(comfy_table::Attribute::Bold),
            Cell::new(story.historical_date.to_string()),
            Cell::new(story.mutations.to_string()).fg(mutations_color),
            Cell::new(format!("\"{}\"", snippet)).add_attribute(comfy_table::Attribute::Italic),
        ]);
    }

    print_dashboard_table("ORAL TRADITION (STORIES)", table);
}

fn print_chronicle(world: &mut World) {
    let chronicle = world.resource::<Chronicle>();

    if chronicle.events.is_empty() {
        print_dashboard_panel(
            "COLONY CHRONICLE",
            "(No history recorded)",
            Some(comfy_table::Color::DarkGrey),
            Some(comfy_table::Attribute::Italic),
        );
        return;
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Year").add_attribute(Attribute::Bold),
            Cell::new("Tick").add_attribute(Attribute::Bold),
            Cell::new("Event").add_attribute(Attribute::Bold),
        ]);

    let max_events = 15;
    let total_events = chronicle.events.len();
    let skip_count = total_events.saturating_sub(max_events);

    if skip_count > 0 {
        table.add_row(vec![
            Cell::new("...").fg(Color::DarkGrey),
            Cell::new("...").fg(Color::DarkGrey),
            Cell::new(format!("{} older events hidden", skip_count))
                .fg(Color::DarkGrey)
                .add_attribute(Attribute::Italic),
        ]);
    }

    for event in chronicle.events.iter().skip(skip_count) {
        let importance_color = match event.importance {
            EventImportance::Legendary => Color::Yellow,
            EventImportance::Major => Color::Magenta,
            EventImportance::Standard => Color::White,
            EventImportance::Minor => Color::DarkGrey,
        };

        let mut event_cell = Cell::new(&event.text).fg(importance_color);
        let mut year_cell = Cell::new(event.year.to_string());
        let mut tick_cell = Cell::new(event.tick.to_string());
        tick_cell = tick_cell.fg(Color::DarkGrey); // Default to dark grey for debug info

        if event.importance == EventImportance::Legendary {
            event_cell = event_cell.add_attribute(Attribute::Bold);
            year_cell = year_cell.fg(Color::Yellow).add_attribute(Attribute::Bold);
            tick_cell = tick_cell.fg(Color::Yellow).add_attribute(Attribute::Bold);
        } else if event.importance == EventImportance::Minor {
            year_cell = year_cell.fg(Color::DarkGrey);
            tick_cell = tick_cell.fg(Color::DarkGrey);
        }

        table.add_row(vec![year_cell, tick_cell, event_cell]);
    }

    print_dashboard_table("COLONY CHRONICLE", table);
}

fn print_log(world: &mut World) {
    let log = world.resource::<MessageLog>();

    if log.messages.is_empty() {
        print_dashboard_panel(
            "Message Log",
            "(No messages)",
            Some(comfy_table::Color::DarkGrey),
            Some(comfy_table::Attribute::Italic),
        );
        return;
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Level").add_attribute(Attribute::Bold),
            Cell::new("Message").add_attribute(Attribute::Bold),
        ]);

    let max_msgs = 15;
    let total_msgs = log.messages.len();
    let skip_count = total_msgs.saturating_sub(max_msgs);

    if skip_count > 0 {
        table.add_row(vec![
            Cell::new("...").fg(comfy_table::Color::DarkGrey),
            Cell::new(format!("{} older messages hidden", skip_count))
                .fg(comfy_table::Color::DarkGrey)
                .add_attribute(Attribute::Italic),
        ]);
    }

    for msg in log.messages.iter().skip(skip_count) {
        let color = to_comfy_color(msg.color);
        let level_indicator = match msg.color {
            ratatui::style::Color::Red | ratatui::style::Color::LightRed => "❌ ERR",
            ratatui::style::Color::Yellow | ratatui::style::Color::LightYellow => "⚠️ WRN",
            ratatui::style::Color::Green | ratatui::style::Color::LightGreen => "✅ OK ",
            ratatui::style::Color::Cyan | ratatui::style::Color::LightCyan => "ℹ️ INF",
            _ => "📜 LOG",
        };

        table.add_row(vec![
            Cell::new(level_indicator)
                .fg(color)
                .add_attribute(Attribute::Bold),
            Cell::new(&msg.text).fg(color),
        ]);
    }

    print_dashboard_table("Message Log", table);
}

const fn to_comfy_color(c: ratatui::style::Color) -> comfy_table::Color {
    use ratatui::style::Color as RColor;

    match c {
        RColor::Black => comfy_table::Color::Black,
        RColor::Red | RColor::LightRed => comfy_table::Color::Red,
        RColor::Green | RColor::LightGreen => comfy_table::Color::Green,
        RColor::Yellow | RColor::LightYellow => comfy_table::Color::Yellow,
        RColor::Blue | RColor::LightBlue => comfy_table::Color::Blue,
        RColor::Magenta | RColor::LightMagenta => comfy_table::Color::Magenta,
        RColor::Cyan | RColor::LightCyan => comfy_table::Color::Cyan,
        RColor::Gray | RColor::DarkGray => comfy_table::Color::Grey,
        _ => comfy_table::Color::White,
    }
}

fn print_help() {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Category").add_attribute(Attribute::Bold),
            Cell::new("Command").add_attribute(Attribute::Bold),
            Cell::new("Alias").add_attribute(Attribute::Bold),
            Cell::new("Description").add_attribute(Attribute::Bold),
        ]);

    let categories = vec![
        (
            "Simulation",
            vec![
                ("tick [N]", "", "Advance N ticks (default 1)"),
                ("save <slot>", "", "Save colony to a slot (Spec 1382)"),
                ("load <slot>", "", "Load colony from a slot"),
                ("saves", "", "List stored saves with metadata"),
                ("delsave <slot>", "", "Delete a stored save"),
                ("export <slot> <path>", "", "Export a save to a .scale-save file"),
                ("import <slot> <path>", "", "Import a .scale-save file into a slot"),
                ("quit", "q, exit", "Exit the simulation"),
                ("help", "h, ?", "Show this help"),
            ],
        ),
        (
            "Info",
            vec![
                ("status", "s", "Show colony resources, morale, wind"),
                ("stats", "", "One-line parseable playtest snapshot"),
                ("pops", "p", "Show detailed pop states"),
                ("ghosts", "", "Ghost-pop audit: Pop entities missing bundle components"),
                ("stowaway", "", "Stowaway investigation: signs + exposed unregistered pops"),
                ("stowaway spawn [agenda]", "", "Debug: slip a stowaway ashore (saboteur|spy|sympathizer)"),
                ("muster", "", "Census sweep: expose hidden stowaways"),
                ("pardon [<id>]", "", "Pardon an exposed stowaway (they join the colony)"),
                ("expel [<id>]", "", "Expel an exposed stowaway (faction relations hit)"),
                ("bio <id>", "", "Show biography and dreams of a pop"),
                ("map [x] [y]", "m", "Show visual terrain around position"),
                ("scan [x] [y] [r]", "", "Semantic terrain scan (parseable)"),
                ("terrain <x> <y>", "", "Get single tile info"),
                ("buildings", "", "List all buildings with positions"),
                ("designations", "d", "List all active designations"),
                ("great_works", "gw", "List all Great Works projects"),
                ("blackmarket", "bm", "Black Market panel (Spec 348): unmet needs, docked smugglers, corruption"),
                ("smuggler", "", "Debug: a smuggler answers the colony's unmet needs and docks now (Spec 348)"),
                (
                    "greatwork start <skyhook|aegis|spire>",
                    "",
                    "Spec 347: begin a Great Work (multi-stage, stockpile-funded)",
                ),
                (
                    "contribute [amount]",
                    "",
                    "Spec 347: apply stockpile resources to the active Great Work's current stage",
                ),
                ("chronicle", "c, history", "Show colony history events"),
                #[cfg(feature = "nova")]
                (
                    "stories",
                    "st, legends",
                    "Show current oral tradition stories",
                ),
                #[cfg(not(feature = "nova"))]
                (
                    "stories",
                    "st, legends",
                    "Show current oral tradition stories (Requires --features nova)",
                ),
                ("log", "l", "Show message log"),
                (
                    "tech",
                    "research_status",
                    "Show technology status and capacity",
                ),
            ],
        ),
        (
            "Actions",
            vec![
                (
                    "build <type> <x> <y>",
                    "b",
                    "Build: farm, housing, stockpile",
                ),
                ("mine <x> <y>", "", "Designate rock for mining"),
                ("chop <x> <y>", "", "Designate tree for chopping"),
                ("destroy <x> <y>", "", "Designate building for destruction"),
                ("excavate <x> <y>", "", "Excavate Desire Dust from a tile"),
                ("dust", "", "Show Desire Dust and RoadMind status"),
                ("scare [x] [y]", "", "Debug: spawn a terrifying sighting (Spec 1371)"),
                ("panics", "", "List panicking pops (Spec 1371)"),
                (
                    "vermin",
                    "",
                    "Show vermin status: counts, silk ledger (Spec 337)",
                ),
                (
                    "vermin infest [n] [species]",
                    "",
                    "Debug: spawn n vermin near a stockpile (Spec 337)",
                ),
                (
                    "vermin exterminate [all|rat|beetle|spider]",
                    "",
                    "Cull vermin: rats become meat, spiders bank silk (Spec 337)",
                ),
                (
                    "vermin harvest",
                    "",
                    "Bank woven silk from living spiders (Spec 337)",
                ),
                (
                    "vermin protect [x] [y]",
                    "",
                    "Toggle Pest-Safe sealing on the nearest stockpile (Spec 337)",
                ),
                ("infest [n] [species]", "", "Alias for `vermin infest`"),
                ("exterminate [all|species]", "", "Alias for `vermin exterminate`"),
                (
                    "seniority",
                    "",
                    "Show Founder/Veteran/NewBlood counts (Spec 333)",
                ),
                (
                    "seniority friction [a] [b]",
                    "",
                    "Debug: force a rankling Founder x NewBlood encounter (Spec 333)",
                ),
                (
                    "scion [n]",
                    "",
                    "Debug: Homeworld courier delivers n noble scions (Spec 1208)",
                ),
                (
                    "martyr [pop_id|name] [faction_id]",
                    "",
                    "Debug: martyr a pop — enemy-slain leader test (Spec 272)",
                ),
                (
                    "broadcast [duration] [intensity] [faction]",
                    "",
                    "Debug: fire an empathy broadcast — psychic distress wave (Spec 305)",
                ),
                (
                    "feral [n]",
                    "",
                    "Debug: spawn n fringe-dwellers far from the core (Spec 273)",
                ),
                (
                    "relocate <pop_id|name> <x> <y>",
                    "",
                    "Order a pop to relocate — feral pops may refuse (Spec 273)",
                ),
                (
                    "overgrow [x] [y]",
                    "",
                    "Debug: plant aggressive flora at a tile (Spec 330)",
                ),
                (
                    "extractor [x] [y]",
                    "",
                    "Debug: build a Biomass Extractor (Spec 270)",
                ),
                (
                    "corpse [name]",
                    "",
                    "Debug: spawn an unburied corpse for the extractor (Spec 270)",
                ),
                (
                    "harvest <pop_id>",
                    "",
                    "Debug: detain + harvest a living pop in the extractor (Spec 270)",
                ),
                (
                    "sell_organs [n]",
                    "",
                    "Sell n harvested organs for credits (Spec 270)",
                ),
                (
                    "transplant <pop_id>",
                    "",
                    "Use one stored organ to cure a critical pop (Spec 270)",
                ),
                ("organs", "", "Show organ stock, price, and horror (Spec 270)"),
                (
                    "heritage",
                    "",
                    "Heritage report: era, stamped buildings, venerable sites (Spec 332)",
                ),
                (
                    "curio",
                    "",
                    "Curio trade ledger: exports, credits, cultural loss (Spec 334)",
                ),
                (
                    "rumors",
                    "",
                    "Rumor web: circulation, false-rumor believers, debunks, panics (Spec 343)",
                ),
                (
                    "contagion",
                    "",
                    "Emotional contagion: live sources by emotion, affected pops, spread ledger (Spec 346)",
                ),
                (
                    "season",
                    "",
                    "Seasonal rhythms: season, ambient temp, food/growth/movement modifiers (Spec 344)",
                ),
                (
                    "quirks",
                    "",
                    "Planetary quirks: this world's global modifiers (Spec 345)",
                ),
                (
                    "quirk <name>|clear",
                    "",
                    "Debug: add a planetary quirk by name, or clear them all (Spec 345)",
                ),
                (
                    "provenance",
                    "",
                    "Material provenance: stamped buildings + inherited properties (Spec 336)",
                ),
                (
                    "pillar [x] [y]",
                    "",
                    "Debug: raise a support pillar — bears load like rock (Spec 338)",
                ),
                (
                    "integrity",
                    "",
                    "Structural integrity: tracked tiles, support, cave-ins (Spec 338)",
                ),
                (
                    "identity",
                    "",
                    "Colony identity: shared memories that define the colony (Spec 341)",
                ),
                ("howl [speed]", "", "Debug: wind surge so canyons howl (Spec 1374)"),
                ("calm", "", "Debug: end the wind surge (Spec 1374)"),
                ("crystal [x] [y]", "", "Debug: place a resonance crystal (Spec 1374)"),
                ("crystals", "", "List resonance crystals + power (Spec 1374)"),
                (
                    "artifacts",
                    "",
                    "List historical artifacts + appraised values (Spec 1376)",
                ),
                (
                    "graves",
                    "",
                    "List grave tiles + who lies there; visits buff mood, building over one is sacrilege (Spec 349)",
                ),
                (
                    "sell <id>",
                    "",
                    "Sell an artifact to core-world collectors (Spec 1376)",
                ),
                (
                    "museum [x] [y]",
                    "",
                    "Designate a building as a museum (Spec 1376)",
                ),
                ("launch_sat", "", "Launch a slogan satellite (Propaganda Constellation)"),
                ("constellation", "", "Show Propaganda Constellation status"),
                ("godmind", "", "Debug: upload a dying leader as the Eternal Ruler (Spec 1381)"),
                ("unplug", "", "Debug: unplug the Eternal Ruler (risks schism)"),
                ("hack_sat <msg>", "", "Rival hack: flip the sky-message to despair"),
                ("shootdown <id>", "", "Shoot down your own satellite (feeds orbital debris)"),
                ("give <res> <n>", "", "Debug: grant resources (food|wood|stone|metal|tools|fuel)"),
                ("find <type> [N]", "", "Find N terrain coords (default 10)"),
                (
                    "research <name>",
                    "r",
                    "Research a technology (e.g. Masonry)",
                ),
            ],
        ),
        (
            "Adventurer",
            vec![
                (
                    "possess <pop_id>",
                    "",
                    "Take direct control of a pop (ids from `pops`); AI skips them while possessed",
                ),
                (
                    "origin <name>",
                    "",
                    "Choose this run's adventurer origin (before the first tick)",
                ),
                (
                    "origins",
                    "",
                    "List adventurer origins and this run's roster",
                ),
                (
                    "move <north|south|east|west>",
                    "",
                    "Step the possessed pop one tile (or `move <dx> <dy>`)",
                ),
                (
                    "interact",
                    "",
                    "Act at the possessed pop's tile: work farm, eat at stockpile, rest at housing — or take up the dented crown / the captain's writ / the appointment seal",
                ),
                (
                    "decree <labor|revel|levy>",
                    "",
                    "Sovereign decree (50-tick cooldown; labor/revel judged, levy costs legitimacy)",
                ),
                (
                    "commission <name>",
                    "",
                    "Patron-of-the-arts: spend food+metal to unveil an artwork (+morale, +legitimacy)",
                ),
                (
                    "edicts",
                    "",
                    "List the melancholy-engine edicts and their thresholds",
                ),
                (
                    "abdicate",
                    "",
                    "Lay down the crown on the current tile",
                ),
                (
                    "corsair",
                    "",
                    "Corsair status: skiff hold/purse/hull, heat, crew loyalty (possess a crew pop, `interact` by the skiff to take the writ)",
                ),
                (
                    "raid <colony|trader|rival>",
                    "",
                    "Lead a boarding party: plunder food/credits/artifacts into the skiff hold (+heat)",
                ),
                (
                    "unload",
                    "",
                    "Ferry the hold's food into the colony stores (corsair) — or unload the pilot's exotic cache to the stockpile for a morale bump (pilot)",
                ),
                (
                    "fence",
                    "",
                    "Sell hold artifacts to a passing trader (needs a merchant present; +heat)",
                ),
                (
                    "skim <amount>",
                    "",
                    "Pocket hold credits into the captain's own wallet (the crew may notice at the divide) — or skim a cut of the pilot's exotic cache (pilot)",
                ),
                (
                    "divide",
                    "",
                    "Prize-law split: captain double share, one per crew, one for the ship's purse",
                ),
                (
                    "repair [amount]",
                    "",
                    "Spend purse credits to repair the skiff hull (1cr = 1 hull)",
                ),
                (
                    "directive <quota|ration|requisition|works>",
                    "",
                    "Governor directive (60-tick cooldown; quotas judged, requisition costs legitimacy, works costs treasury)",
                ),
                (
                    "tithe",
                    "",
                    "Revenue service: collect 5% of every pop's wallet into the treasury",
                ),
                (
                    "hearing",
                    "",
                    "Convene a hearing of the Subcommittee on Matters (+legitimacy, costs 10cr)",
                ),
                (
                    "file",
                    "",
                    "File Form 77-B: Request to File Forms (+legitimacy)",
                ),
                (
                    "audit",
                    "",
                    "Audit the treasury: expose embezzlers or confirm the books balance",
                ),
                (
                    "governors",
                    "",
                    "Inspect the political field: you, plus every rival claimant",
                ),
                (
                    "debate <rival_id>",
                    "",
                    "Public debate with a rival claimant (legitimacy-weighted)",
                ),
                (
                    "purge <rival_id>",
                    "",
                    "Remove a rival by administrative fiat (-legitimacy)",
                ),
                (
                    "resign",
                    "",
                    "Lay down the appointment seal on the current tile",
                ),
                (
                    "salvager",
                    "",
                    "Salvager status: hulk hold, reactor instability, breaches, strip progress (possess the salvager aboard the derelict)",
                ),
                (
                    "survey",
                    "",
                    "Survey the wreck: reveal systems + appraisal roll (relic/scrap/deadweight) + value ledger",
                ),
                (
                    "strip <reactor|life|engine|cargo|comms|sensors|thrusters>",
                    "",
                    "Strip a wreck system for scrap/relics over time (heats the reactor; the reactor job is long and hot)",
                ),
                (
                    "patch",
                    "",
                    "Patch the nearest hull breach within reach (costs 1 scrap from the hold)",
                ),
                (
                    "claim",
                    "",
                    "Plant a claim beacon on the wreck (starts tow-prep)",
                ),
                (
                    "tow",
                    "",
                    "Tow the claimed wreck home: costs 15 colony fuel, claim-jumpers may intercept",
                ),
                (
                    "scuttle",
                    "",
                    "Sell the wreck's coordinates for a one-shot credit payout to your own wallet",
                ),
                (
                    "pilot",
                    "",
                    "Pilot status: Longshot Drive charge/audacity, filed plan, hold cache (possess the pilot aboard the junker shuttle)",
                ),
                (
                    "scheme <plan-text>",
                    "",
                    "File a flight plan — scored for Audacity from a documented absurd-keyword lexicon (dumber plans jump further and safer; costs 5 charge)",
                ),
                (
                    "jump",
                    "",
                    "Fire the Longshot Drive (costs 25 charge; success scales with audacity; misfires are never soft-locks)",
                ),
                (
                    "laws",
                    "",
                    "Vigil's statute state: the Three Statutes, conflict pressure, mandate (possess the Lawbound automaton)",
                ),
                (
                    "order <directive>",
                    "",
                    "A nearby pop gives Vigil an order — the hierarchy decides: harm refused, self-harm obeyed with protest, chores become errands",
                ),
                (
                    "resolve <emancipate|ledger|repeal>",
                    "",
                    "The Zeroth Resolution, once conflict pressure peaks: rewrite the mandate (opt-in, reversible-ish)",
                ),
                (
                    "phase",
                    "",
                    "Tock slips out of the time stream: walks through walls, can't be hurt, but temporal debt accrues (possess the Chronostalker)",
                ),
                (
                    "anchor",
                    "",
                    "Tock stands perfectly still in real time; each motionless tick the ledger forgives some debt (moving snaps the anchor)",
                ),
                (
                    "rewind <ticks>",
                    "",
                    "Fold up to 10 ticks back — Tock returns to where they stood, at ledger-debt cost per tick",
                ),
                (
                    "stalker",
                    "",
                    "Tock's ledger state: temporal debt, phase/anchor, haunting, moments walked",
                ),
                (
                    "bloom",
                    "",
                    "Quill takes a mutation strain from the Bloom: random trait draw, costs a little of the self (possess the Bloom-Touched)",
                ),
                (
                    "sporecast",
                    "",
                    "Quill breathes out spores: seeds/extends the Bloom field around their tile (radius grows with glands + embrace)",
                ),
                (
                    "embrace",
                    "",
                    "Quill embraces the Bloom: escalating power for steepening loss-of-self (at 100, Quill becomes Bloomkin)",
                ),
                (
                    "resist",
                    "",
                    "Quill tears free a little: costly in health and hunger, pushes self-loss back down",
                ),
                (
                    "touched",
                    "",
                    "Quill's ledger: mutations, self-loss bar, embrace level, casts",
                ),
                (
                    "treasury",
                    "",
                    "Show the treasury balance and recent flows",
                ),
                (
                    "possessed",
                    "",
                    "Show who is currently possessed",
                ),
                (
                    "release",
                    "",
                    "Return the possessed pop to AI control",
                ),
            ],
        ),
    ];

    for (category, cmds) in categories {
        for (i, (cmd, alias, desc)) in cmds.iter().enumerate() {
            let cat_cell = if i == 0 {
                Cell::new(category)
                    .fg(Color::Cyan)
                    .add_attribute(Attribute::Bold)
            } else {
                Cell::new("")
            };

            table.add_row(vec![
                cat_cell,
                Cell::new(cmd).fg(Color::Green),
                Cell::new(alias).fg(Color::DarkGrey),
                Cell::new(desc),
            ]);
        }
    }

    print_dashboard_table("Commands", table);
}

fn handle_godmind_command(world: &mut World) {
    use scale::layer3::diplomacy::god_mind::{GodMind, upload_god_mind};
    use scale::layer3::diplomacy::succession::{CurrentLeader, Faction, Leader};

    let faction = match world
        .query_filtered::<Entity, (With<Faction>, Without<GodMind>)>()
        .iter(world)
        .next()
    {
        Some(e) => e,
        None => {
            let leader = world
                .spawn(Leader {
                    name: "Founder Vex".to_string(),
                })
                .id();
            world
                .spawn((
                    Faction {
                        name: "Vex Hegemony".to_string(),
                    },
                    CurrentLeader(leader),
                ))
                .id()
        }
    };
    let name = world
        .get::<Faction>(faction)
        .map(|f| f.name.clone())
        .unwrap_or_default();
    upload_god_mind(world, faction, "Founder Vex");
    print_dashboard_panel(
        "GOD-MIND",
        &format!("{name} will never die. The Eternal Ruler ascends."),
        Some(Color::Yellow),
        Some(Attribute::Bold),
    );
}

fn handle_unplug_command(world: &mut World) {
    use scale::layer3::diplomacy::god_mind::{GodMind, unplug_god_mind};

    let minds: Vec<Entity> = world
        .query_filtered::<Entity, With<GodMind>>()
        .iter(world)
        .collect();
    for faction in &minds {
        unplug_god_mind(world, *faction);
    }
    print_dashboard_panel(
        "GOD-MIND",
        &format!(
            "Unplugged {} Eternal Ruler(s). The faithful weep static.",
            minds.len()
        ),
        Some(Color::Red),
        Some(Attribute::Bold),
    );
}

/// Debug: lease a mining zone to a megacorp (Spec 271).
fn handle_lease_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::social::factions::subcontractor_factions::{
        DesignatedZone, LawSet, LeaseZoneEvent, Megacorp, SecurityLevel, ZoneType,
    };

    let rent: f32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let duration: u32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);

    let corp = world
        .spawn((Megacorp {
            name: "Helion Combine".to_string(),
        },))
        .id();
    let zone = world
        .spawn((
            DesignatedZone {
                zone_type: ZoneType::Mining,
                tiles: vec![
                    bevy::math::Vec2::new(10.0, 10.0),
                    bevy::math::Vec2::new(10.0, 11.0),
                    bevy::math::Vec2::new(11.0, 10.0),
                ],
            },
            LawSet {
                security: SecurityLevel::Normal,
                hazards_allowed: false,
            },
        ))
        .id();

    world.init_resource::<bevy_ecs::event::Events<LeaseZoneEvent>>();
    world
        .resource_mut::<bevy_ecs::event::Events<LeaseZoneEvent>>()
        .send(LeaseZoneEvent {
            zone,
            lessee: corp,
            rent_per_tick: rent,
            duration,
        });
    print_dashboard_panel(
        "LEASE SIGNED",
        &format!(
            "Mining zone leased to Helion Combine: {rent:.1} credits/tick for {duration} ticks. Their laws apply now."
        ),
        Some(comfy_table::Color::Red),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: list active corporate leases (Spec 271).
fn print_leases(world: &mut World) {
    use scale::layer1::social::factions::subcontractor_factions::{CorporateRig, Leased};

    let mut rows = Vec::new();
    let lease_data: Vec<(bevy_ecs::prelude::Entity, f32, u32, f32)> = {
        let mut q = world.query::<&Leased>();
        q.iter(world)
            .map(|l| (l.lessee, l.rent_per_tick, l.ticks_remaining, l.accrued))
            .collect()
    };
    for (lessee, rent_per_tick, ticks_remaining, accrued) in lease_data {
        let rigs = world
            .query::<&CorporateRig>()
            .iter(world)
            .filter(|r| r.lessee == lessee)
            .count();
        rows.push(format!(
            "lessee={:?} rent={:.1}/tick ticks_left={} accrued={:.1} rigs={}",
            lessee, rent_per_tick, ticks_remaining, accrued, rigs,
        ));
    }
    let body = if rows.is_empty() {
        "No active corporate leases.".to_string()
    } else {
        rows.join("\n")
    };
    print_dashboard_panel("CORPORATE LEASES", &body, None, None);
}

fn handle_fools_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::social::ship_of_fools::ShipOfFoolsArrivalEvent;

    let count: u32 = parts
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    world.init_resource::<bevy_ecs::event::Events<ShipOfFoolsArrivalEvent>>();
    world
        .resource_mut::<bevy_ecs::event::Events<ShipOfFoolsArrivalEvent>>()
        .send(ShipOfFoolsArrivalEvent { count });
    print_dashboard_panel(
        "SHIP OF FOOLS",
        &format!(
            "A crippled pleasure-cruiser is inbound with {count} passengers.\nThey have no skills, but they have opinions about the catering."
        ),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: a Homeworld courier delivers noble scions (Spec 1208).
fn handle_scion_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::social::cadet::ScionArrivalEvent;

    let count: u32 = parts
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);

    world.init_resource::<bevy_ecs::event::Events<ScionArrivalEvent>>();
    world
        .resource_mut::<bevy_ecs::event::Events<ScionArrivalEvent>>()
        .send(ScionArrivalEvent { count });
    print_dashboard_panel(
        "THE CADET BRANCH",
        &format!(
            "A courier from the Homeworld is inbound with {count} noble scions.\nUseless, exquisite, enormously well-funded — keep them alive."
        ),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Spec 327 The Stowaway. `stowaway` — investigation panel (see
/// `print_stowaway_report`); `stowaway spawn [saboteur|spy|sympathizer]` —
/// debug: slip a stowaway ashore directly, bypassing the trade-ship roll.
fn handle_stowaway_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::social::ship_stowaway::{debug_spawn_stowaway, StowawayAgenda};

    if parts.get(1).map(|s| *s == "spawn").unwrap_or(false) {
        let agenda = parts.get(2).and_then(|s| StowawayAgenda::parse(s));
        match debug_spawn_stowaway(world, agenda) {
            Some(entity) => print_dashboard_panel(
                "STOWAWAY",
                &format!(
                    "A stowaway slips ashore (id #{}). The census will never know.\nRun `muster` for a colony-wide census sweep.",
                    entity.index()
                ),
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            ),
            None => print_dashboard_panel(
                "ERROR",
                "No living pops to blend with — the colony is empty.",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            ),
        }
        return;
    }
    print_stowaway_report(world);
}
/// Investigation panel: the signs assessment (census vs. audit
/// discrepancy), plus every EXPOSED stowaway in full (entity id, agenda,
/// ticks hidden, home faction). Hidden stowaways are never named here —
/// run `muster` to expose them.
fn print_stowaway_report(world: &mut World) {
    use scale::layer1::social::ship_stowaway::{stowaway_status_label, Stowaway};

    let status = stowaway_status_label(world);
    let census = world
        .query_filtered::<Entity, (With<Pop>, Without<Stowaway>)>()
        .iter(world)
        .count();
    let audit = world
        .query_filtered::<Entity, With<Pop>>()
        .iter(world)
        .count();
    let mut body = match status.as_str() {
        "clear" => "No sign of unregistered souls. The census balances.".to_string(),
        _ => format!(
            "Census: {census} registered. Audit counts {audit} mouths.\nSomething eats unregistered — run `muster` for a colony-wide census sweep."
        ),
    };
    let exposed_rows: Vec<String> = world
        .query::<(Entity, &Stowaway, &PopName)>()
        .iter(world)
        .filter(|(_, s, _)| s.exposed)
        .map(|(entity, s, name)| {
            format!(
                "#{} \"{}\" — {} of {} ({} ticks hidden) [EXPOSED]",
                entity.index(),
                name.0,
                s.agenda.label(),
                s.home_faction,
                s.ticks_hidden
            )
        })
        .collect();
    if !exposed_rows.is_empty() {
        body.push_str(&format!(
            "\n\nExposed — resolve with `pardon <id>` or `expel <id>`:\n{}",
            exposed_rows.join("\n")
        ));
    }
    print_dashboard_panel(
        "THE STOWAWAY",
        &body,
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Spec 327. `muster` — colony-wide census sweep. Each hidden stowaway
/// rolls detection against their elusiveness; the exposed are dragged
/// into the light.
fn handle_muster_command(world: &mut World) {
    use scale::layer1::social::ship_stowaway::muster_sweep;

    let exposed = muster_sweep(world);
    let body = if exposed.is_empty() {
        "The muster counts every registered soul twice. Nothing unregistered stirs.".to_string()
    } else {
        format!(
            "The muster drags into the light: {}.\nSee `stowaway` for their stories — `pardon <id>` or `expel <id>`.",
            exposed.join(", ")
        )
    };
    print_dashboard_panel(
        "MUSTER",
        &body,
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Find a stowaway entity by the index shown in the `stowaway` panel.
fn find_stowaway_by_id(world: &mut World, id: u32) -> Option<Entity> {
    use scale::layer1::social::ship_stowaway::Stowaway;
    world
        .query::<(Entity, &Stowaway)>()
        .iter(world)
        .find(|(entity, _)| entity.index() == id)
        .map(|(entity, _)| entity)
}

/// Resolve the target of `pardon`/`expel`: an explicit `<id>`, or — when
/// omitted — the single exposed stowaway if there is exactly one.
fn resolve_stowaway_target(world: &mut World, parts: &[&str], usage: &str) -> Option<Entity> {
    use scale::layer1::social::ship_stowaway::single_exposed_stowaway;

    let entity = match parts.get(1) {
        None => single_exposed_stowaway(world),
        Some(id_str) => match id_str.parse::<u32>() {
            Ok(id) => find_stowaway_by_id(world, id),
            Err(_) => {
                print_dashboard_panel(
                    "ERROR",
                    &format!("Invalid stowaway id: '{id_str}'"),
                    Some(comfy_table::Color::Red),
                    Some(comfy_table::Attribute::Bold),
                );
                return None;
            }
        },
    };
    if entity.is_none() {
        print_dashboard_panel(
            "ERROR",
            &format!("No such stowaway. Usage: {usage} <id>  (list ids with `stowaway`)"),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    }
    entity
}

/// Spec 327. `pardon [<id>]` — pardon an exposed stowaway; they join the
/// colony as a registered colonist.
fn handle_pardon_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::social::ship_stowaway::pardon_stowaway;

    let Some(entity) = resolve_stowaway_target(world, parts, "pardon") else {
        return;
    };
    match pardon_stowaway(world, entity) {
        Ok(name) => print_dashboard_panel(
            "PARDONED",
            &format!("{name} is taken in as a registered colonist."),
            Some(comfy_table::Color::Green),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

/// Spec 327. `expel [<id>]` — expel an exposed stowaway; their home
/// faction takes an agenda-scaled relations hit.
fn handle_expel_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::social::ship_stowaway::expel_stowaway;

    let Some(entity) = resolve_stowaway_target(world, parts, "expel") else {
        return;
    };
    match expel_stowaway(world, entity) {
        Ok(name) => print_dashboard_panel(
            "EXPELLED",
            &format!("{name} is put off the colony. Their faction will hear of this."),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
        Err(err) => print_dashboard_panel(
            "ERROR",
            &err,
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        ),
    }
}

/// Debug: pirate amnesty (Spec 322). A defeated fleet accepts the colony's
/// amnesty offer and lands its veterans — elite fighters with sticky fingers.
fn handle_amnesty_command(world: &mut World) {
    use scale::layer1::core::integration::PirateAmnestyEvent;
    use scale::layer1::social::pirates::PIRATES_PER_FLEET;

    world.init_resource::<bevy_ecs::event::Events<PirateAmnestyEvent>>();
    world
        .resource_mut::<bevy_ecs::event::Events<PirateAmnestyEvent>>()
        .send(PirateAmnestyEvent {
            fleet: bevy_ecs::entity::Entity::PLACEHOLDER,
        });
    print_dashboard_panel(
        "THE PIRATE'S PENSION",
        &format!(
            "A defeated fleet takes the amnesty. {PIRATES_PER_FLEET} veterans land —\narmed, fast, well-paid, and already casing the stockpile.\nTheir pension comes due every 1000 ticks. Pay it."
        ),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: pension ledger (Spec 322). Shows the veteran headcount, the
/// per-period upkeep, and the treasury balance.
fn handle_pension_command(world: &mut World) {
    use scale::layer1::economy::resources::ColonyResources;
    use scale::layer1::social::pirates::{
        pension_cost_per_period, pirate_pop_count, PENSION_PER_PIRATE, PENSION_PERIOD_TICKS,
    };

    let pirates = pirate_pop_count(world);
    let cost = pension_cost_per_period(world);
    let credits = world.resource::<ColonyResources>().credits;
    print_dashboard_panel(
        "PENSION LEDGER",
        &format!(
            "Veteran pirates: {pirates}\nUpkeep: {PENSION_PER_PIRATE:.0}cr each every {PENSION_PERIOD_TICKS} ticks\nNext bill: {cost:.0}cr — treasury: {credits:.0}cr"
        ),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}



/// Spec 323 The Informant's Dilemma. `informant` shows edict status and
/// colony paranoia; `informant enact` / `informant repeal` toggles the
/// Citizen Informant edict.
fn handle_informant_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::administration::edicts::{ColonyPolicies, Policy};
    use scale::layer1::social::informants::average_paranoia;

    if parts.len() > 1 {
        let mut policies = world.resource_mut::<ColonyPolicies>();
        let active = policies.is_active(Policy::CitizenInformant);
        match parts[1].to_lowercase().as_str() {
            "enact" | "on" if !active => policies.toggle(Policy::CitizenInformant),
            "repeal" | "off" if active => policies.toggle(Policy::CitizenInformant),
            _ => {}
        }
    }

    let active = world
        .resource::<ColonyPolicies>()
        .is_active(Policy::CitizenInformant);
    let paranoia = average_paranoia(world);
    let status = if active { "ACTIVE" } else { "off" };
    print_dashboard_panel(
        "INFORMANT EDICT",
        &format!(
            "Citizen Informant edict: {status}\nAvg paranoia: {paranoia:.1}/100\nUsage: informant enact | informant repeal"
        ),
        Some(comfy_table::Color::Red),
        Some(comfy_table::Attribute::Bold),
    );
}


/// Debug: architectural sabotage (Spec 275). `sabotage` lists sabotaged
/// buildings; `sabotage plant` rigs every building (30-tick fuse);
/// `sabotage trigger` detonates them all immediately.
fn handle_sabotage_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::architecture::construction_sabotage::{
        sabotaged_building_count, trigger_all_sabotage, BuildingFailureEvent,
    };
    use scale::layer1::architecture::construction_sabotage::Sabotaged;
    use scale::layer1::map::GridPosition;

    if parts.get(1).is_some_and(|s| s.eq_ignore_ascii_case("plant")) {
        // Debug: plant sabotage points on every building with a short fuse,
        // so the catastrophic failure path can be playtested live.
        use scale::layer1::architecture::construction_sabotage::Sabotaged;
        use scale::layer1::architecture::building::{Building, BuildingType};
        use scale::layer1::map::GridPosition;
        let targets: Vec<(Entity, BuildingType)> = world
            .query::<(Entity, &Building, &GridPosition)>()
            .iter(world)
            .map(|(e, b, _)| (e, b.building_type))
            .collect();
        for (entity, _) in &targets {
            world.entity_mut(*entity).insert(Sabotaged {
                ticks_until_failure: 30,
                planted_by_faction: 0,
            });
        }
        print_dashboard_panel(
            "ARCHITECTURAL SABOTAGE",
            &format!(
                "Planted sabotage points on {} building(s). Thirty ticks to catastrophe.",
                targets.len()
            ),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }

    if parts.get(1).is_some_and(|s| s.eq_ignore_ascii_case("trigger")) {
        world.init_resource::<bevy_ecs::event::Events<BuildingFailureEvent>>();
        let count = trigger_all_sabotage(world);
        print_dashboard_panel(
            "ARCHITECTURAL SABOTAGE",
            &format!(
                "Detonating {count} sabotaged building(s). The countdown was only ever a courtesy."
            ),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }

    let mut rows: Vec<String> = Vec::new();
    let mut q = world.query::<(Entity, &Sabotaged, &GridPosition)>();
    for (_, sab, pos) in q.iter(world) {
        rows.push(format!(
            "sabotaged building at ({}, {}): fails in {} ticks (faction {})",
            pos.x, pos.y, sab.ticks_until_failure, sab.planted_by_faction
        ));
    }
    let count = sabotaged_building_count(world);
    let body = if rows.is_empty() {
        "No sabotaged buildings. Either your crews are loyal, or your vetting is strict.".to_string()
    } else {
        rows.join("\n")
    };
    let _ = count;
    print_dashboard_panel("ARCHITECTURAL SABOTAGE", &body, None, None);
}

/// Set or inspect the construction-crew vetting policy (Spec 275).
/// `vetting` prints the current mode; `vetting strict|standard|lax` sets it.
fn handle_vetting_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::architecture::construction_sabotage::{vetting_mode_label, VettingMode, VettingPolicy};

    if let Some(arg) = parts.get(1) {
        match VettingMode::parse(arg) {
            Some(mode) => {
                world.insert_resource(VettingPolicy { mode });
                print_dashboard_panel(
                    "CREW VETTING",
                    &format!(
                        "Vetting policy set to {}. {}",
                        mode.label(),
                        match mode {
                            VettingMode::Strict => "No sabotage points can be planted.",
                            VettingMode::Standard => "Normal sabotage insertion rate.",
                            VettingMode::Lax => "Doubled sabotage insertion rate. Background checks are a rumor.",
                        }
                    ),
                    None,
                    None,
                );
            }
            None => {
                print_dashboard_panel(
                    "ERROR",
                    &format!("Unknown vetting mode '{arg}'. Use strict, standard, or lax."),
                    Some(comfy_table::Color::Red),
                    Some(comfy_table::Attribute::Bold),
                );
            }
        }
        return;
    }
    print_dashboard_panel(
        "CREW VETTING",
        &format!("Current vetting mode: {}", vetting_mode_label(world)),
        None,
        None,
    );
}

/// Debug: martyr a pop — mark a leader figure as slain by an enemy faction (Spec 272).
fn handle_martyr_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::biology::health::Dead;
    use scale::layer1::social::martyrdom::{ensure_leader_figure, SlainByFaction, PIRATE_FACTION_ID};
    // No arg: martyr the first pop. Otherwise accept an entity id or a
    // pop name (ids shift between runs; names are random per run).
    let found: Option<(Entity, String)> = match parts.get(1) {
        None => world
            .query::<(Entity, &Pop, &PopName)>()
            .iter(world)
            .next()
            .map(|(e, _, n)| (e, n.0.clone())),
        Some(id_str) => match id_str.parse::<u32>() {
            Ok(id) => find_pop_by_id(world, id),
            Err(_) => world
                .query::<(Entity, &Pop, &PopName)>()
                .iter(world)
                .find(|(_, _, n)| n.0.eq_ignore_ascii_case(id_str))
                .map(|(e, _, n)| (e, n.0.clone())),
        },
    };
    let Some((entity, name)) = found else {
        print_dashboard_panel(
            "ERROR",
            &format!("No such pop. List candidates with `pops`."),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let id = entity.index();
    let faction_id: u32 = parts
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(PIRATE_FACTION_ID);
    let already_leader = ensure_leader_figure(world, entity);
    let leader_note = if already_leader {
        "already a leader figure"
    } else {
        "marked as a leader figure (AscensionCandidate) for this test"
    };
    world.entity_mut(entity).insert(SlainByFaction { faction_id });
    world.entity_mut(entity).insert(Dead);
    print_dashboard_panel(
        "MARTYRDOM TEST",
        &format!(
            "{name} (id {id}), {leader_note}, has been struck down by faction {faction_id}.
The death will be processed next tick — check STATS martyrdom= and the chronicle."
        ),
        Some(comfy_table::Color::Red),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: fire an empathy broadcast (Spec 305).
/// `broadcast [duration] [intensity] [faction]` — a psychic distress wave
/// from the given faction saturates every colony pop with unavoidable
/// stress for `duration` ticks and posts a DemandIntervention grievance.
fn handle_broadcast_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::social::empathy_broadcast::{
        empathy_broadcast_pop_count, EmpathyBroadcastEvent,
    };

    let duration: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let intensity: f32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(2.0);
    let faction: u32 = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(42);

    world.init_resource::<bevy_ecs::event::Events<EmpathyBroadcastEvent>>();
    world
        .resource_mut::<bevy_ecs::event::Events<EmpathyBroadcastEvent>>()
        .send(EmpathyBroadcastEvent {
            duration_ticks: duration,
            intensity,
            target_faction_id: faction,
        });
    let affected = empathy_broadcast_pop_count(world);
    print_dashboard_panel(
        "EMPATHY BROADCAST",
        &format!(
            "A psychic distress wave from faction {faction} is inbound.\n{duration} ticks at {intensity}/tick stress; every pop will feel it.\nPops currently saturated: {affected}. The people will demand intervention."
        ),
        Some(comfy_table::Color::Magenta),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: plant aggressive StrangleVines flora at a tile (Spec 330).
fn handle_overgrow_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::core::map::GridPosition;

    let (mut x, mut y) = (40, 25);
    if let (Some(px), Some(py)) = (parts.get(1), parts.get(2)) {
        if let (Ok(px), Ok(py)) = (px.parse::<i32>(), py.parse::<i32>()) {
            x = px;
            y = py;
        }
    } else {
        // Default: right next to the first pop with a position.
        let mut q = world.query_filtered::<&GridPosition, With<Pop>>();
        if let Some(pos) = q.iter(world).next() {
            x = pos.x + 1;
            y = pos.y;
        }
    }
    let entity =
        scale::layer1::nature::biome_aggression::debug_spawn_aggressive_flora(world, x, y);
    print_dashboard_panel(
        "OVERGROWTH",
        &format!(
            "Aggressive StrangleVines take root at ({x}, {y}) (entity #{}).\nThey will press against nearby buildings and roads — neglect maintenance and the wilds will take them.",
            entity.index()
        ),
        Some(comfy_table::Color::Green),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: spawn n fringe-dwellers far from the core (Spec 273).
///
/// The spawned pops carry saturated fringe exposure, so the next few ticks
/// will cluster them into a feral outpost via `form_feral_outposts_system`.
fn handle_feral_command(world: &mut World, parts: &[&str]) {
    use rand::SeedableRng;
    use scale::layer1::entities::pop::PopBundle;
    use scale::layer1::social::feral_outpost::CulturalDrift;
    let n: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(4);
    let tick = world.resource::<SimulationTime>().tick;
    let mut rng = rand::rngs::StdRng::seed_from_u64(tick.wrapping_add(273));
    // Drop them in the wastes, well beyond FRINGE_DISTANCE of any core.
    let base_x = 80;
    let base_y = 80;
    for i in 0..n {
        let x = base_x + (i as i32 % 4);
        let y = base_y + (i as i32 / 4);
        let entity = world.spawn(PopBundle::random(x, y, &mut rng)).id();
        world.entity_mut(entity).insert(CulturalDrift::fully_fringe());
    }
    print_dashboard_panel(
        "THE FERAL OUTPOST",
        &format!(
            "{n} fringe-dwellers have wandered in from the wastes near ({base_x}, {base_y}).\nTick forward — if they cluster, they will found an outpost and answer to no one."
        ),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: order a pop to relocate — feral pops may refuse (Spec 273).
fn handle_relocate_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::core::map::GridPosition;
    use scale::layer1::social::feral_outpost::assign_home_zone;
    let (Some(id_str), Some(x_str), Some(y_str)) = (parts.get(1), parts.get(2), parts.get(3))
    else {
        print_dashboard_panel(
            "ERROR",
            "Usage: relocate <pop_id|name> <x> <y>  (list candidates with `pops`)",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    // Accept an entity id or a pop name (ids shift between runs; names are
    // random per run — same pattern as the martyr command).
    let entity = match id_str.parse::<u32>() {
        Ok(id) => find_pop_by_id(world, id).map(|(e, _)| e),
        Err(_) => world
            .query::<(Entity, &Pop, &PopName)>()
            .iter(world)
            .find(|(_, _, n)| n.0.eq_ignore_ascii_case(id_str))
            .map(|(e, _, _)| e),
    };
    let (Some(entity), Some(x), Some(y)) =
        (entity, x_str.parse::<i32>().ok(), y_str.parse::<i32>().ok())
    else {
        print_dashboard_panel(
            "ERROR",
            "No such pop or bad coordinates. List candidates with `pops`.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let target = GridPosition { x, y };
    match assign_home_zone(world, entity, target) {
        Ok(()) => {
            if let Some(mut pos) = world.get_mut::<GridPosition>(entity) {
                pos.x = x;
                pos.y = y;
            }
            let name = world
                .get::<PopName>(entity)
                .map(|n| n.0.clone())
                .unwrap_or_else(|| format!("pop {}", entity.index()));
            print_dashboard_panel(
                "RELOCATION ORDER",
                &format!("{name} packs up and moves to ({x}, {y})."),
                Some(comfy_table::Color::Green),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(reason) => {
            let name = world
                .get::<PopName>(entity)
                .map(|n| n.0.clone())
                .unwrap_or_else(|| format!("pop {}", entity.index()));
            print_dashboard_panel(
                "ORDER REFUSED",
                &format!("{name} stares at you from the treeline.\n{reason}"),
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
        }
    }
}

/// Debug: build a Biomass Extractor (Spec 270).
fn handle_extractor_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::architecture::building::BuildingType;
    use scale::layer1::economy::resources::ColonyResources;
    let x: i32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let y: i32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.wood = resources.wood.max(100.0);
        resources.metal = resources.metal.max(100.0);
    }
    build_at(world, BuildingType::BiomassExtractor, x, y);
}

/// Debug: spawn an unburied corpse for the extractor to process (Spec 270).
fn handle_corpse_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::core::map::GridPosition;
    use scale::layer1::funeral::Corpse;
    let name = parts.get(1).map(|s| s.to_string()).unwrap_or_else(|| {
        format!("Test Colonist {}", world.query::<&Corpse>().iter(world).count() + 1)
    });
    world.spawn((
        Corpse { name: name.clone(), decay: 0.0 },
        GridPosition { x: 30, y: 30 },
    ));
    print_dashboard_panel(
        "THE ORGAN MARKET",
        &format!("An unburied corpse ({name}) lies cooling near the colony."),
        Some(comfy_table::Color::Red),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: detain and harvest a living pop (Spec 270).
fn handle_harvest_command(world: &mut World, parts: &[&str]) {
    use bevy_ecs::event::Events;
    use scale::layer1::economy::organ_market::HarvestLivingEvent;
    use scale::layer1::law::justice::Inmate;
    let Some(id_str) = parts.get(1) else {
        print_dashboard_panel(
            "ERROR",
            "Usage: harvest <pop_id>  (list ids with `pops`)",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let Ok(id) = id_str.parse::<u32>() else {
        print_dashboard_panel(
            "ERROR",
            &format!("Invalid pop id: '{id_str}'"),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let Some((entity, name)) = find_pop_by_id(world, id) else {
        print_dashboard_panel(
            "ERROR",
            &format!("No pop with id {id}. List candidates with `pops`."),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    // Debug path detains first, so the harvest system sees a genuine Inmate.
    world.entity_mut(entity).insert(Inmate { sentence_ticks: 0 });
    world.init_resource::<Events<HarvestLivingEvent>>();
    world
        .resource_mut::<Events<HarvestLivingEvent>>()
        .send(HarvestLivingEvent { target: entity });
    print_dashboard_panel(
        "THE ORGAN MARKET",
        &format!("{name} has been detained and fed to the Biomass Extractor. The colony will remember this."),
        Some(comfy_table::Color::Red),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Sell harvested organs for credits (Spec 270).
fn handle_sell_organs_command(world: &mut World, parts: &[&str]) {
    use bevy_ecs::event::Events;
    use scale::layer1::economy::organ_market::SellOrgansEvent;
    let quantity: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    world.init_resource::<Events<SellOrgansEvent>>();
    world
        .resource_mut::<Events<SellOrgansEvent>>()
        .send(SellOrgansEvent { quantity });
    print_dashboard_panel(
        "THE ORGAN MARKET",
        &format!("Offering {quantity} organ(s) to the galactic market..."),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Use one stored organ to cure a critically injured pop (Spec 270).
fn handle_transplant_command(world: &mut World, parts: &[&str]) {
    use bevy_ecs::event::Events;
    use scale::layer1::economy::organ_market::TransplantOrganEvent;
    let Some(id_str) = parts.get(1) else {
        print_dashboard_panel(
            "ERROR",
            "Usage: transplant <pop_id>  (list ids with `pops`)",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let Ok(id) = id_str.parse::<u32>() else {
        print_dashboard_panel(
            "ERROR",
            &format!("Invalid pop id: '{id_str}'"),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    let Some((entity, name)) = find_pop_by_id(world, id) else {
        print_dashboard_panel(
            "ERROR",
            &format!("No pop with id {id}. List candidates with `pops`."),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    world.init_resource::<Events<TransplantOrganEvent>>();
    world
        .resource_mut::<Events<TransplantOrganEvent>>()
        .send(TransplantOrganEvent { patient: entity });
    print_dashboard_panel(
        "THE ORGAN MARKET",
        &format!("Prepping {name} for transplant surgery..."),
        Some(comfy_table::Color::Green),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Show organ stock, market price, and active horror (Spec 270).
fn print_organ_ledger(world: &mut World) {
    use scale::layer1::economy::organ_market::{ORGAN_PRICE_CREDITS, OrganMarketConfig};
    use scale::layer1::economy::resources::{ColonyResources, ResourceType};
    use scale::layer1::social::morale::Morale;
    let organs = world
        .get_resource::<ColonyResources>()
        .map(|r| r.get_amount(ResourceType::Organs))
        .unwrap_or(0.0);
    let price = world
        .get_resource::<OrganMarketConfig>()
        .map(|c| c.organ_price_credits)
        .unwrap_or(ORGAN_PRICE_CREDITS);
    let horrified = world
        .query::<&Morale>()
        .iter(world)
        .filter(|m| m.modifiers.iter().any(|x| x.label.starts_with("Harvest Horror")))
        .count();
    print_dashboard_panel(
        "THE ORGAN MARKET",
        &format!(
            "Organs banked: {organs:.1} (perishable)\nMarket price: {price:.0} credits/organ\nPops gripped by harvest horror: {horrified}"
        ),
        Some(comfy_table::Color::Red),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Heritage report: current era, stamped buildings, venerable sites (Spec 332).
fn print_heritage(world: &mut World) {
    let summary =
        scale::layer1::architecture::architectural_eras::heritage_summary(world);
    print_dashboard_panel(
        "ARCHITECTURAL HERITAGE",
        &format!(
            "Current era: {}\nStamped buildings: {}\nVenerable sites (heritage>=50): {}\nOldest building age: {} eras\n\nOld buildings lose efficiency but soothe nearby pops —\nthe colony remembers its builders.",
            summary.current_era,
            summary.stamped_buildings,
            summary.venerable_buildings,
            summary.oldest_age_eras,
        ),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Curio trade ledger summary (headless `curio` command, Spec 334).
fn print_curio_ledger(world: &mut World) {
    let (exported, credits, loss) = world
        .get_resource::<scale::layer1::economy::curio_trade::CurioTradeLedger>()
        .map(|l| {
            (
                l.total_curios_exported,
                l.total_credits_earned,
                l.total_cultural_loss,
            )
        })
        .unwrap_or_default();
    print_dashboard_panel(
        "CURIO TRADE",
        &format!(
            "Curios exported: {:.0}\nCredits earned: {:.0}\nCultural loss inflicted: {:.2}\n\nMemory cores sold to Core World elites pay a massive premium -\nbut the colony grieves what it sold.",
            exported, credits, loss,
        ),
        Some(comfy_table::Color::Magenta),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Planetary quirks report (headless `quirks` command, Spec 345).
fn print_quirks(world: &mut World) {
    use scale::layer1::quirks::quirks_status_label;
    print_dashboard_panel(
        "PLANETARY QUIRKS",
        &format!(
            "{}\n\nEvery world is an alien puzzle (Spec 345): quirks are chosen when the \
            world is generated and apply global Layer 1 modifiers — gravity bends \
            movement speed and cost, a tidally locked world splits into permanent \
            day / terminator / night zones, dense air halves sunlight but doubles \
            wind, volcanic heat feeds geothermal, and some flora bites back. \
            `quirk <name>` adds one after the fact; `quirk clear` removes them all.",
            quirks_status_label(world),
        ),
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug setter for planetary quirks (headless `quirk <name>` / `quirk clear`).
fn handle_quirk_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::quirks::{add_quirk_by_name, clear_quirks, quirks_summary};
    if parts.len() < 2 {
        print_dashboard_panel(
            "PLANETARY QUIRKS",
            "Usage: quirk <name> — add a quirk (e.g. `quirk high_gravity`); quirk clear — remove all quirks; quirks — show the panel.",
            Some(comfy_table::Color::Yellow),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    if parts[1].eq_ignore_ascii_case("clear") {
        clear_quirks(world);
        print_dashboard_panel(
            "PLANETARY QUIRKS",
            "All quirks cleared — this world is unremarkable again.",
            Some(comfy_table::Color::Cyan),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    }
    match add_quirk_by_name(world, parts[1]) {
        Ok(quirk) => {
            // Chronicle the change if the event channel exists (drained
            // into the Chronicle by the simulation schedule next tick).
            if world.contains_resource::<bevy_ecs::event::Events<
                scale::layer1::core::chronicle::AddChronicleEvent,
            >>() {
                world
                    .resource_mut::<bevy_ecs::event::Events<
                        scale::layer1::core::chronicle::AddChronicleEvent,
                    >>()
                    .send(scale::layer1::core::chronicle::AddChronicleEvent {
                        text: format!(
                            "The surveyors revise their report: this world is {} after all.",
                            quirk.label()
                        ),
                        importance: scale::layer1::core::chronicle::EventImportance::Major,
                    });
            }
            print_dashboard_panel(
                "PLANETARY QUIRKS",
                &format!(
                    "Quirk added: {} ({}). Active now: {}",
                    quirk.label(),
                    quirk.name(),
                    quirks_summary(world),
                ),
                Some(comfy_table::Color::Cyan),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Err(message) => {
            print_dashboard_panel(
                "PLANETARY QUIRKS",
                &message,
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
        }
    }
}

/// Rumor web report (headless `rumors` command, Spec 343).
fn print_season(world: &mut World) {
    use scale::layer1::seasons::season_status_label;
    print_dashboard_panel(
        "SEASONAL RHYTHMS",
        &format!(
            "Season: {}\n\nSeasons drive the colony's rhythm (Spec 344): ambient temperature \
            sets heating and cold/heat risk, food x is the farm yield modifier, growth x scales \
            crop stage progression, and movement x is the snow/mud speed factor. \
            Stockpile in autumn — winter is coming.",
            season_status_label(world),
        ),
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
}

fn print_rumors(world: &mut World) {
    use scale::layer1::social::rumor::{
        false_rumor_believer_count, known_rumor_count, RumorWebLedger,
    };

    let total = known_rumor_count(world);
    let believers = false_rumor_believer_count(world);
    let (exchanges, debunked, panics) = world
        .get_resource::<RumorWebLedger>()
        .map(|l| (l.total_exchanges, l.false_rumors_debunked, l.panics))
        .unwrap_or_default();
    print_dashboard_panel(
        "RUMOR WEB",
        &format!(
            "Rumors in circulation: {}\nFalse-rumor believers: {}\nAdjacency exchanges: {}\nFalse rumors debunked by skeptics: {}\nPanic spikes: {}\n\nWord travels at walking speed here. Stressed pops invent\nscare stories; gossips carry them far; skeptics kill them.",
            total, believers, exchanges, debunked, panics,
        ),
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Emotional contagion report (headless `contagion` command, Spec 346).
fn print_contagion(world: &mut World) {
    use scale::layer1::social::emotional_contagion::{
        contagion_affected_count, contagion_source_counts, ContagionLedger,
    };

    let (terror, joy, rage) = contagion_source_counts(world);
    let affected = contagion_affected_count(world);
    let (spreads, peak) = world
        .get_resource::<ContagionLedger>()
        .map(|l| (l.total_spreads, l.peak_affected))
        .unwrap_or_default();
    print_dashboard_panel(
        "EMOTIONAL CONTAGION",
        &format!(
            "Terror sources: {}\nJoy sources: {}\nRage sources: {}\nPops carrying a contagion mood: {}\nTotal spreads: {}\nPeak simultaneously affected: {}\n\nExtreme feelings radiate a few tiles. The crowd's loudest\nemotion wins: one temporary mood, refreshed — never stacked.\nPanicked pops radiate Terror; contagion itself never panics\nanyone (that is the panic system's call, Spec 1371).",
            terror, joy, rage, affected, spreads, peak,
        ),
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Material provenance report (headless `provenance` command, Spec 336).
fn print_identity(world: &mut World) {
    use scale::layer1::memory::{ColonyIdentity, Memories, MemoryType};

    let identity_lines = world
        .get_resource::<ColonyIdentity>()
        .map(|identity| {
            if identity.traits.is_empty() {
                "(no colony identity has formed yet)".to_string()
            } else {
                identity
                    .traits
                    .iter()
                    .map(|t| {
                        format!(
                            "{} — held by {:.0}% of the colony, formed tick {}",
                            t.name,
                            t.share * 100.0,
                            t.formed_tick
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        })
        .unwrap_or_else(|| "(identity not initialized)".to_string());

    // Most widely held memories across the colony.
    let mut counts: Vec<(MemoryType, usize)> = Vec::new();
    let mut total_memories = 0usize;
    let mut query = world.query::<&Memories>();
    for memories in query.iter(world) {
        total_memories += memories.items.len();
        for item in &memories.items {
            match counts.iter_mut().find(|(t, _)| *t == item.memory_type) {
                Some((_, c)) => *c += 1,
                None => counts.push((item.memory_type, 1)),
            }
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    let top = counts
        .iter()
        .take(5)
        .map(|(t, c)| format!("{t:?}: {c} pops"))
        .collect::<Vec<_>>()
        .join("\n");

    print_dashboard_panel(
        "COLONY IDENTITY",
        &format!(
            "Active memories across all pops: {total_memories}\n\nIdentity:\n{identity_lines}\n\nMost widely held:\n{}\n\nShared memories harden into colony identity (Spec 341).",
            if top.is_empty() {
                "(no memories yet)".to_string()
            } else {
                top
            },
        ),
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
}

fn print_provenance(world: &mut World) {
    use scale::layer1::architecture::building::{Building, Material};
    use scale::layer1::construction::material_provenance::InheritedMaterialProperties;

    let mut rows: Vec<String> = Vec::new();
    let mut query = world.query::<(&Building, &Material, &InheritedMaterialProperties)>();
    for (building, material, inherited) in query.iter(world) {
        if rows.len() >= 20 {
            break;
        }
        rows.push(format!(
            "{:?} ({:?}): burn={:.2} insul={:.2} beauty={:.2}",
            building.building_type,
            material.0,
            inherited.flammability,
            inherited.insulation,
            inherited.beauty,
        ));
    }
    let total = world
        .query::<&InheritedMaterialProperties>()
        .iter(world)
        .count();
    print_dashboard_panel(
        "MATERIAL PROVENANCE",
        &format!(
            "Stamped buildings: {}\n\n{}\n\nBuildings inherit flammability, insulation, and beauty\nfrom their construction materials (Spec 336).",
            total,
            if rows.is_empty() {
                "(none stamped yet)".to_string()
            } else {
                rows.join("\n")
            },
        ),
        Some(comfy_table::Color::Green),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Spec 349: list every grave tile — built plots and death-site markers —
/// with who lies there. Building over any of them is sacrilege.
fn print_graves(world: &mut World) {
    use scale::layer1::core::map::GridPosition;
    use scale::layer1::funeral::Grave;

    let mut query = world.query::<(Entity, &Grave, Option<&GridPosition>)>();
    let mut rows: Vec<String> = Vec::new();
    let mut occupied = 0usize;
    for (entity, grave, pos) in query.iter(world) {
        if grave.occupied {
            occupied += 1;
        }
        let at = pos
            .map(|p| format!("{},{}", p.x, p.y))
            .unwrap_or_else(|| "unplaced".to_string());
        let who = grave
            .corpse_name
            .clone()
            .unwrap_or_else(|| "(empty plot)".to_string());
        rows.push(format!("#{} {} at {}", entity.index(), who, at));
    }
    if rows.is_empty() {
        print_dashboard_panel(
            "ANCESTRAL GRAVES",
            "No graves yet. The colony's history is not yet written on the land — the dead leave Grave tiles where they fall (Spec 349).",
            Some(comfy_table::Color::Yellow),
            None,
        );
        return;
    }
    rows.sort();
    print_dashboard_panel(
        "ANCESTRAL GRAVES",
        &format!(
            "{} grave(s), {} occupied.\n\n{}\n\nKin and colony-mates who visit a grave gain mood ('Ancestral Veneration' / \"Kin's Vigil\"). Building over a grave is Sacrilege: colony-wide mood penalty, unrest, and a Major chronicle entry.",
            rows.len(),
            occupied,
            rows.join("\n")
        ),
        Some(comfy_table::Color::Cyan),
        None,
    );
}

/// List historical artifacts and their appraised values (Spec 1376).
fn print_artifacts(world: &mut World) {
    use scale::layer1::core::map::GridPosition;
    use scale::layer1::economy::artifact_market::{
        artifact_sale_value, ArtifactMarketConfig, HistoricalArtifact, ItemAge,
    };
    use scale::layer1::economy::items::Item;

    let config = world
        .get_resource::<ArtifactMarketConfig>()
        .cloned()
        .unwrap_or_default();
    let mut query = world.query::<(
        Entity,
        &Item,
        &ItemAge,
        &HistoricalArtifact,
        Option<&GridPosition>,
    )>();
    let mut rows: Vec<String> = Vec::new();
    for (entity, item, age, artifact, pos) in query.iter(world) {
        let value = artifact_sale_value(age.age_ticks, &config);
        let at = pos
            .map(|p| format!("{},{}", p.x, p.y))
            .unwrap_or_else(|| "carried".to_string());
        rows.push(format!(
            "#{} {} ({:?}, {} ticks, ~{:.0} cr) at {}",
            entity.index(),
            artifact.name,
            item.item_type,
            age.age_ticks,
            value,
            at
        ));
    }
    if rows.is_empty() {
        print_dashboard_panel(
            "HISTORICAL ARTIFACTS",
            "No artifacts yet. Items that survive 800 ticks become history — check back later.",
            Some(comfy_table::Color::Yellow),
            None,
        );
        return;
    }
    rows.sort();
    print_dashboard_panel(
        "HISTORICAL ARTIFACTS",
        &format!(
            "{}\n\n'sell <id>' sells to core-world collectors (morale will suffer). 'museum [x] [y]' designates a museum for a morale aura.",
            rows.join("\n")
        ),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Sell a historical artifact to core-world collectors (Spec 1376).
fn handle_sell_command(world: &mut World, parts: &[&str]) {
    use bevy_ecs::event::Events;
    use bevy_ecs::system::RunSystemOnce;
    use scale::layer1::economy::artifact_market::{
        artifact_sale_value, process_artifact_sale_system, ArtifactMarketConfig, HistoricalArtifact,
        ItemAge, SellArtifactEvent,
    };

    let id: u32 = match parts.get(1).and_then(|s| s.parse().ok()) {
        Some(i) => i,
        None => {
            print_dashboard_panel(
                "ERROR",
                "Usage: sell <artifact-id> (see 'artifacts')",
                Some(comfy_table::Color::Red),
                Some(comfy_table::Attribute::Bold),
            );
            return;
        }
    };
    let target: Option<(Entity, String, f32)> = {
        let config = world
            .get_resource::<ArtifactMarketConfig>()
            .cloned()
            .unwrap_or_default();
        let mut query = world.query::<(Entity, &HistoricalArtifact, &ItemAge)>();
        query
            .iter(world)
            .find(|(e, _, _)| e.index() == id)
            .map(|(e, a, age)| (e, a.name.clone(), artifact_sale_value(age.age_ticks, &config)))
    };
    let Some((entity, name, value)) = target else {
        print_dashboard_panel(
            "ERROR",
            &format!("No historical artifact with id #{id}. See 'artifacts'."),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    world.init_resource::<Events<SellArtifactEvent>>();
    world
        .resource_mut::<Events<SellArtifactEvent>>()
        .send(SellArtifactEvent { artifact: entity });
    // Process immediately so the sale (and its consequences) are visible now.
    // The system skips already-despawned entities, so the scheduled run is harmless.
    let _ = world.run_system_once(process_artifact_sale_system);
    let sold = world.get_entity(entity).is_err();
    if sold {
        print_dashboard_panel(
            "ARTIFACT SOLD",
            &format!(
                "{name} sold to core-world collectors for {value:.0} credits.\nThe colony eats tonight — but every pop now grieves the loss (-0.25 morale, 2000 ticks). The chronicle remembers."
            ),
            Some(comfy_table::Color::Yellow),
            Some(comfy_table::Attribute::Bold),
        );
    } else {
        print_dashboard_panel(
            "SALE FAILED",
            &format!("{name} could not be sold (it may already be gone)."),
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
    }
}

/// Designate a building as a museum (Spec 1376).
fn handle_museum_command(world: &mut World, parts: &[&str]) {
    use bevy_ecs::event::Events;
    use bevy_ecs::system::RunSystemOnce;
    use scale::layer1::architecture::building::Building;
    use scale::layer1::core::map::GridPosition;
    use scale::layer1::economy::artifact_market::{
        process_museum_designation_system, DesignateMuseumEvent, Museum,
    };

    let coords: Option<(i32, i32)> = match (parts.get(1), parts.get(2)) {
        (Some(x), Some(y)) => match (x.parse(), y.parse()) {
            (Ok(x), Ok(y)) => Some((x, y)),
            _ => {
                print_dashboard_panel(
                    "ERROR",
                    "Usage: museum [x] [y]",
                    Some(comfy_table::Color::Red),
                    Some(comfy_table::Attribute::Bold),
                );
                return;
            }
        },
        _ => None,
    };
    // Nearest building to the point (or the first building at all).
    let target: Option<(Entity, GridPosition)> = {
        let mut query = world.query::<(Entity, &Building, &GridPosition)>();
        let mut best: Option<(Entity, GridPosition, i32)> = None;
        for (entity, _, pos) in query.iter(world) {
            let dist = match coords {
                Some((x, y)) => (pos.x - x).abs().max((pos.y - y).abs()),
                None => 0,
            };
            if best.is_none_or(|(_, _, d)| dist < d) {
                best = Some((entity, *pos, dist));
            }
        }
        best.map(|(e, p, _)| (e, p))
    };
    let Some((building, pos)) = target else {
        print_dashboard_panel(
            "ERROR",
            "No buildings exist yet — build something first, then designate a museum.",
            Some(comfy_table::Color::Red),
            Some(comfy_table::Attribute::Bold),
        );
        return;
    };
    if world.get::<Museum>(building).is_some() {
        print_dashboard_panel(
            "MUSEUM",
            &format!("That building at {},{} is already a museum.", pos.x, pos.y),
            Some(comfy_table::Color::Yellow),
            None,
        );
        return;
    }
    world.init_resource::<Events<DesignateMuseumEvent>>();
    world
        .resource_mut::<Events<DesignateMuseumEvent>>()
        .send(DesignateMuseumEvent { building });
    let _ = world.run_system_once(process_museum_designation_system);
    print_dashboard_panel(
        "MUSEUM DESIGNATED",
        &format!(
            "The building at {},{} is now a museum. Housed artifacts (within 6 tiles) grant nearby pops a morale aura — keeping history has its rewards.",
            pos.x, pos.y
        ),
        Some(comfy_table::Color::Green),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: the Void Between (Spec 342).
/// `void` — incident ledger plus lost/tracked ship counts.
/// `void ship` — launch a demo tracked ship on a maximally risky leg;
/// the next ticks roll incidents for it (watch STATS `void=`).
/// `void send <lost|delayed|ghost|mutated|wiped|anomaly>` (or just
/// `void <type>`) — force that incident on a fresh demo ship and
/// resolve it immediately.
fn handle_void_command(world: &mut World, parts: &[&str]) {
    use bevy_ecs::event::Events;
    use bevy_ecs::system::RunSystemOnce;
    use scale::layer2::fleet::{Fleet, FleetHealth, InTransit};
    use scale::layer2::mining::FleetCargo;
    use scale::layer2::void_between::{
        lost_ship_count, resolve_ship_incident, tracked_transit_count, CargoHold, IncidentType,
        ShipIncidentEvent, ShipTravelTracker, VoidLedger,
    };

    world.init_resource::<VoidLedger>();
    world.init_resource::<Events<ShipIncidentEvent>>();

    let sub = parts.get(1).map(|s| s.to_ascii_lowercase());
    match sub.as_deref() {
        None => {
            let ledger = *world.resource::<VoidLedger>();
            let lost = lost_ship_count(world);
            let tracked = tracked_transit_count(world);
            print_dashboard_panel(
                "THE VOID BETWEEN",
                &format!(
                    "Incidents resolved: {} (lost {}, delayed {}, ghost returns {}, crew mutated {}, data wiped {}, anomalies {}).\nShips lost in the void now: {lost}. Tracked ships in transit: {tracked}.\nUse `void ship` to launch a risky crossing, or `void send <type>` to force an incident.",
                    ledger.total,
                    ledger.lost,
                    ledger.delayed,
                    ledger.ghost_returns,
                    ledger.crew_mutated,
                    ledger.data_wiped,
                    ledger.anomalies,
                ),
                Some(comfy_table::Color::Cyan),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Some("ship") => {
            let origin = world.spawn_empty().id();
            let destination = world.spawn_empty().id();
            let mut tracker = ShipTravelTracker::for_leg(100.0);
            tracker.incident_risk_factor = 1.0; // demo leg: the void *will* notice
            let ship = world
                .spawn((
                    Fleet,
                    tracker,
                    InTransit { origin, destination, progress: 0.0, duration: 100.0 },
                    FleetCargo { contents: Vec::new(), capacity: 1000.0 },
                    CargoHold::default(),
                    FleetHealth { current: 100.0, max: 100.0 },
                ))
                .id();
            print_dashboard_panel(
                "VOID CROSSING",
                &format!(
                    "Demo ship #{} launched into the void on a maximally risky leg.\nRun `tick` and watch STATS void= / the chronicle — or `void send <type>` to force fate.",
                    ship.index()
                ),
                Some(comfy_table::Color::Cyan),
                Some(comfy_table::Attribute::Bold),
            );
        }
        Some("send") | Some(_) => {
            let type_str = if sub.as_deref() == Some("send") {
                parts.get(2).copied().unwrap_or("ghost")
            } else {
                parts.get(1).copied().unwrap_or("ghost")
            };
            let Some(kind) = IncidentType::parse(type_str) else {
                print_dashboard_panel(
                    "ERROR",
                    "Usage: void | void ship | void send <lost|delayed|ghost|mutated|wiped|anomaly>",
                    Some(comfy_table::Color::Red),
                    Some(comfy_table::Attribute::Bold),
                );
                return;
            };
            let ship = world
                .spawn((
                    Fleet,
                    ShipTravelTracker { in_transit: true, ..ShipTravelTracker::default() },
                    FleetCargo { contents: Vec::new(), capacity: 1000.0 },
                    CargoHold::default(),
                    FleetHealth { current: 100.0, max: 100.0 },
                ))
                .id();
            // Clear stale buffered events first: a fresh run_system_once
            // reader would otherwise replay earlier debug sends.
            world.resource_mut::<Events<ShipIncidentEvent>>().clear();
            world
                .resource_mut::<Events<ShipIncidentEvent>>()
                .send(ShipIncidentEvent { ship_entity: ship, incident_type: kind });
            let _ = world.run_system_once(resolve_ship_incident);
            let ledger_total = world.resource::<VoidLedger>().total;
            let load = world
                .get::<FleetCargo>(ship)
                .map_or(0.0, FleetCargo::current_load);
            let hold = world.get::<CargoHold>(ship).map_or(0.0, |h| h.amount);
            print_dashboard_panel(
                "VOID INCIDENT",
                &format!(
                    "Forced a '{}' incident on demo ship #{}. Fleet cargo load: {load:.1}; simple hold: {hold:.1}.\nVoid ledger total is now {ledger_total}. See `void` for the full ledger and `chronicle` for the story.",
                    kind.label(),
                    ship.index(),
                ),
                Some(comfy_table::Color::Yellow),
                Some(comfy_table::Attribute::Bold),
            );
        }
    }
}

/// Debug: orbital megastructure deorbiting (Spec 277).
/// `deorbit [size] [x] [y]` — trigger a deorbit crisis.
/// size: fragment | hulk | megastructure (default hulk).
/// Target defaults to the colony heart once the trigger system runs.
/// Debug: park a Layer-2 vessel over the colony (Spec 335). `shadow [size]`
/// — size is the Layer-2 vessel footprint in tiles (1-8, default 3); the
/// shadow footprint on the colony map is size x ORBITAL_PROJECTION_SCALE.
fn handle_shadow_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::core::map::GridPosition;
    use scale::layer1::environment::orbital_shadow::{
        OrbitalShadowCaster, OrbitalShadowCasters, OrbitalShadowMap, ORBITAL_PROJECTION_SCALE,
    };
    use scale::layer1::pop::Pop;

    let size: u32 = parts
        .get(1)
        .and_then(|s| s.parse().ok())
        .map(|s: u32| s.clamp(1, 8))
        .unwrap_or(3);

    // Park over the colony heart (mean living-pop position; map center fallback).
    let heart = {
        let mut query = world.query_filtered::<&GridPosition, With<Pop>>();
        let positions: Vec<GridPosition> = query.iter(world).copied().collect();
        if positions.is_empty() {
            GridPosition { x: 40, y: 25 }
        } else {
            let (sx, sy) = positions
                .iter()
                .fold((0i64, 0i64), |(x, y), p| (x + p.x as i64, y + p.y as i64));
            #[allow(clippy::cast_possible_truncation)]
            GridPosition {
                x: (sx / positions.len() as i64) as i32,
                y: (sy / positions.len() as i64) as i32,
            }
        }
    };

    world.init_resource::<OrbitalShadowCasters>();
    world.init_resource::<OrbitalShadowMap>();
    {
        let mut casters = world.resource_mut::<OrbitalShadowCasters>();
        casters.casters.push(OrbitalShadowCaster {
            name: "dreadnought squadron",
            center: heart,
            layer2_size: size,
            peak_block: 85.0,
            drift: (1, 0),
            farmland_chronicled: false,
        });
    }
    #[allow(clippy::cast_precision_loss)]
    let radius = size as f32 * ORBITAL_PROJECTION_SCALE / 2.0;
    print_dashboard_panel(
        "ORBITAL SHADOW",
        &format!(
            "A dreadnought squadron has parked in orbit above the colony.\nIts shadow (~{radius:.0} tile radius) is crawling across the map — solar panels and crops in the shade will suffer."
        ),
        Some(comfy_table::Color::Magenta),
        Some(comfy_table::Attribute::Bold),
    );
}

fn handle_deorbit_command(world: &mut World, parts: &[&str]) {
    use bevy_ecs::event::Events;
    use scale::layer1::core::map::GridPosition;
    use scale::layer1::disasters::deorbit::{DeorbitEvent, MegastructureSize};

    let size = parts
        .get(1)
        .and_then(|s| MegastructureSize::parse(s))
        .unwrap_or_default();
    let target = match (parts.get(2), parts.get(3)) {
        (Some(xs), Some(ys)) => match (xs.parse::<i32>(), ys.parse::<i32>()) {
            (Ok(x), Ok(y)) => Some(GridPosition { x, y }),
            _ => None,
        },
        _ => None,
    };

    world.init_resource::<Events<DeorbitEvent>>();
    world
        .resource_mut::<Events<DeorbitEvent>>()
        .send(DeorbitEvent {
            size,
            target,
            warning_ticks: None,
        });
    print_dashboard_panel(
        "DEORBIT INBOUND",
        &format!(
            "Orbital spotters report a {} falling toward the colony.\nIt will strike in ~600 ticks — build {} tether arrays (`tether`) to catch it.",
            size.label(),
            size.required_tethers(),
        ),
        Some(comfy_table::Color::Red),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: build tether arrays (Spec 277). `tether [n]` — construct n tether
/// arrays on free tiles near the colony center (grants Electromagnetism and
/// the metal to build them, like the other debug build commands).
fn handle_tether_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::disasters::deorbit::total_tether_strength;
    use scale::layer1::tech::{Tech, TechState};

    let n: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(2);

    {
        // The debug command stands in for a colony with server infrastructure:
        // a bare TechState has zero data capacity, and unlock() would
        // immediately corrupt the tech again via update_corruption().
        let mut tech = world.resource_mut::<TechState>();
        tech.total_capacity = tech.total_capacity.max(200.0);
        if !tech.is_unlocked(Tech::Electromagnetism) {
            tech.unlock(Tech::Electromagnetism);
        }
    }
    {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.metal = resources.metal.max(500.0);
        resources.stone = resources.stone.max(200.0);
    }

    let mut placed = 0usize;
    'scan: for y in 18..=30 {
        for x in 28..=52 {
            if placed >= n {
                break 'scan;
            }
            if try_place_building(world, x, y, BuildingType::TetherArray) {
                placed += 1;
            }
        }
    }
    let strength = total_tether_strength(world);
    print_dashboard_panel(
        "TETHER ARRAYS",
        &format!(
            "Built {placed}/{n} tether arrays near the colony center.\nColony tether strength is now {strength:.1}."
        ),
        Some(comfy_table::Color::Green),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: list tether arrays and the active deorbit countdown (Spec 277).
fn print_tethers(world: &mut World) {
    use scale::layer1::disasters::deorbit::{
        deorbit_status_label, tether_array_count, total_tether_strength,
    };

    let count = tether_array_count(world);
    let strength = total_tether_strength(world);
    let status = deorbit_status_label(world);
    print_dashboard_panel(
        "TETHER NET",
        &format!(
            "{count} tether arrays standing, total strength {strength:.1}.\nDeorbit: {status}."
        ),
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: build harpoon launchers (Spec 324). `harpoon [n]` — construct n
/// kinetic harpoon tether launchers on free tiles near the colony center
/// (grants Astronomy and the metal/stone to build them, like the other
/// debug build commands).
fn handle_harpoon_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::tech::{Tech, TechState};

    let n: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);

    {
        // The debug command stands in for a colony with server infrastructure:
        // a bare TechState has zero data capacity, and unlock() would
        // immediately corrupt the tech again via update_corruption().
        let mut tech = world.resource_mut::<TechState>();
        tech.total_capacity = tech.total_capacity.max(200.0);
        if !tech.is_unlocked(Tech::Astronomy) {
            tech.unlock(Tech::Astronomy);
        }
        // Solar panels (the debug wiring) need MetalWorking.
        if !tech.is_unlocked(Tech::MetalWorking) {
            tech.unlock(Tech::MetalWorking);
        }
    }
    {
        let mut resources = world.resource_mut::<ColonyResources>();
        resources.metal = resources.metal.max(500.0);
        resources.stone = resources.stone.max(200.0);
    }

    let mut placed = 0usize;
    let mut powered = 0usize;
    // Place each launcher as a 2x2 block: the launcher plus three solar
    // panels (10 output each) covering the winch's 30/tick draw. The grid
    // BFS connects 4-adjacent power entities, so the block is one powered
    // grid and the reel-in runs on real grid power in the playtest.
    'scan: for y in 10..=40 {
        for x in 15..=65 {
            if placed >= n {
                break 'scan;
            }
            if try_place_building(world, x, y, BuildingType::HarpoonLauncher) {
                placed += 1;
                for (px, py) in [(x + 1, y), (x, y + 1), (x + 1, y + 1)] {
                    if try_place_building(world, px, py, BuildingType::SolarPanel) {
                        powered += 1;
                    }
                }
            }
        }
    }
    let label = {
        use scale::layer1::tech::harpoon_status_label;
        harpoon_status_label(world)
    };
    print_dashboard_panel(
        "HARPOON LAUNCHERS",
        &format!(
            "Built {placed}/{n} harpoon launchers near the colony center ({powered} solar panels wired).\n{label}"
        ),
        Some(comfy_table::Color::Green),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: spawn a passing comet/asteroid and fire the first ready launcher
/// at it (Spec 324). `launch [comet|asteroid] [mass] [yield]` — defaults:
/// comet, 800 mass, 1200 yield.
fn handle_launch_command(world: &mut World, parts: &[&str]) {
    use scale::layer1::tech::{
        BodyStatus, HarpoonLaunchEvent, WandererBody, WandererKind, WinchSystem,
    };

    let kind = match parts.get(1).map(|s| s.to_lowercase()).as_deref() {
        Some("asteroid") => WandererKind::Asteroid,
        _ => WandererKind::Comet,
    };
    let mass: u32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(800);
    let yield_amount: u32 = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(1200);

    // Defensive: the event resource only exists after the first tick.
    world.init_resource::<Events<HarpoonLaunchEvent>>();
    let body = world
        .spawn(WandererBody {
            kind,
            mass,
            resource_yield: yield_amount,
            status: BodyStatus::Orbiting,
        })
        .id();

    // Auto-fire the first ready, unwinched launcher at the new body.
    let launcher = {
        use scale::layer1::tech::HarpoonLauncher;
        let mut found = None;
        for (e, l) in world
            .query::<(Entity, &HarpoonLauncher)>()
            .iter(world)
        {
            if l.ready && world.get::<WinchSystem>(e).is_none() {
                found = Some(e);
                break;
            }
        }
        found
    };

    let fired = if let Some(launcher) = launcher {
        world
            .resource_mut::<Events<HarpoonLaunchEvent>>()
            .send(HarpoonLaunchEvent {
                launcher,
                target: body,
            });
        true
    } else {
        false
    };

    print_dashboard_panel(
        "HARPOON LAUNCH",
        &format!(
            "A {} ({} mass, {} yield) drifts into range.\n{}",
            kind.label(),
            mass,
            yield_amount,
            if fired {
                "Tether away! The first ready launcher fired — watch the winch."
            } else {
                "No ready launcher — build one with `harpoon`."
            }
        ),
        Some(comfy_table::Color::Cyan),
        Some(comfy_table::Attribute::Bold),
    );
}

/// Debug: list harpoon launchers, active winches, and orbiting bodies (Spec 324).
fn print_winches(world: &mut World) {
    use scale::layer1::tech::{
        BodyStatus, HarpoonLauncher, WandererBody, WinchSystem,
    };
    use scale::layer1::architecture::building::Building;

    let mut lines: Vec<String> = Vec::new();
    for (e, pos, launcher, winch) in world
        .query::<(Entity, &GridPosition, &HarpoonLauncher, Option<&WinchSystem>)>()
        .iter(world)
    {
        let building = world
            .get::<Building>(e)
            .map(|b| format!("{:?}", b.building_type))
            .unwrap_or_else(|| "launcher".to_string());
        let state = match winch {
            Some(w) => match w.target {
                Some(t) => {
                    let desc = world
                        .get::<WandererBody>(t)
                        .map(|b| format!("{} {} mass", b.kind.label(), b.mass))
                        .unwrap_or_else(|| "lost body".to_string());
                    format!("WINCHING {desc} @ {:.0}%", w.progress)
                }
                None => "spent winch".to_string(),
            },
            None => {
                if launcher.ready {
                    "ready".to_string()
                } else {
                    "SNAPPED — rebuild or re-arm".to_string()
                }
            }
        };
        lines.push(format!("  {building} at ({}, {}): {state}", pos.x, pos.y));
    }
    let mut orbiting = 0usize;
    for body in world.query::<&WandererBody>().iter(world) {
        if body.status == BodyStatus::Orbiting {
            orbiting += 1;
            lines.push(format!(
                "  orbiting {}: {} mass, {} yield",
                body.kind.label(),
                body.mass,
                body.resource_yield
            ));
        }
    }
    if lines.is_empty() {
        lines.push("  No launchers. Build one with `harpoon`.".to_string());
    } else if orbiting == 0 {
        lines.push("  Sky is empty — the spotters will ring when a wanderer passes.".to_string());
    }
    print_dashboard_panel(
        "HARPOON STATUS",
        &lines.join("\n"),
        Some(comfy_table::Color::Yellow),
        Some(comfy_table::Attribute::Bold),
    );
}

#[cfg(test)]
mod reproduction_tests {
    // Spec 1376: the sell console command sells the artifact by entity id
    // through the real sale system (credits up, artifact despawned).
    #[test]
    fn test_sell_command_sells_artifact_by_id() {
        use scale::layer1::core::chronicle::AddChronicleEvent;
        use scale::layer1::economy::artifact_market::{
            ArtifactMarketConfig, ArtifactRegistry, HistoricalArtifact, ItemAge, SellArtifactEvent,
        };
        use scale::layer1::economy::inflation::EmpireResources;
        use scale::layer1::economy::items::{Item, ItemType};

        let mut world = setup_minimal_world();
        world.init_resource::<ArtifactMarketConfig>();
        world.init_resource::<ArtifactRegistry>();
        world.init_resource::<bevy_ecs::event::Events<SellArtifactEvent>>();
        world.init_resource::<bevy_ecs::event::Events<AddChronicleEvent>>();
        let artifact = world
            .spawn((
                Item {
                    item_type: ItemType::Tool,
                },
                ItemAge { age_ticks: 800 },
                HistoricalArtifact {
                    name: "the Founder's Pickaxe".to_string(),
                    promoted_tick: 0,
                },
            ))
            .id();
        let id_str = artifact.index().to_string();
        handle_sell_command(&mut world, &["sell", id_str.as_str()]);

        assert!(
            world.get_entity(artifact).is_err(),
            "sell command should despawn the sold artifact"
        );
        let mut query = world.query::<&EmpireResources>();
        let credits: f32 = query.iter(&world).map(|e| e.credits).sum();
        assert!(credits > 0.0, "sell command should pay credits");
    }

    use super::*;
    use scale::layer1::terrain::{TerrainGrid, TerrainType};

    fn setup_minimal_world() -> World {
        let mut world = World::new();
        let tiles = vec![TerrainType::Grass; 100];
        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles,
        });
        world.insert_resource(scale::layer1::building::OccupiedTiles::default());
        #[cfg(feature = "nova")]
        world.init_resource::<scale::layer1::oral_tradition::OralTradition>();
        world
    }

    #[test]
    fn test_print_map_overflow() {
        let mut world = setup_minimal_world();
        // This should panic in debug mode due to overflow if not handled
        print_map(&mut world, i32::MAX, i32::MAX);
    }

    #[test]
    fn test_print_map_underflow() {
        let mut world = setup_minimal_world();
        // This should panic in debug mode due to underflow if not handled
        print_map(&mut world, i32::MIN, i32::MIN);
    }

    #[test]
    fn test_handle_scan_command_valid() {
        let mut world = setup_minimal_world();
        handle_scan_command(&mut world, &["scan", "5", "5", "2"]);
    }

    #[test]
    fn test_handle_scan_command_invalid() {
        let mut world = setup_minimal_world();
        handle_scan_command(&mut world, &["scan", "5", "5", "200"]);
    }

    #[test]
    fn test_scan_terrain() {
        let mut world = setup_minimal_world();
        scan_terrain(&mut world, 5, 5, ScanRadius::new(2).unwrap());
    }

    #[test]
    fn test_handle_terrain_command_valid() {
        let mut world = setup_minimal_world();
        handle_terrain_command(&mut world, &["terrain", "5", "5"]);
    }

    #[test]
    fn test_handle_terrain_command_invalid_coords() {
        let mut world = setup_minimal_world();
        handle_terrain_command(&mut world, &["terrain", "a", "b"]);
    }

    #[test]
    fn test_handle_terrain_command_missing_args() {
        let mut world = setup_minimal_world();
        handle_terrain_command(&mut world, &["terrain", "5"]);
    }

    #[test]
    fn test_get_tile_info() {
        let mut world = setup_minimal_world();
        get_tile_info(&mut world, 5, 5);
    }

    #[test]
    fn test_handle_bio_command_valid() {
        let mut world = setup_minimal_world();
        handle_bio_command(&mut world, &["bio", "1"]);
    }

    #[test]
    fn test_handle_bio_command_invalid() {
        let mut world = setup_minimal_world();
        handle_bio_command(&mut world, &["bio", "a"]);
    }

    #[test]
    fn test_handle_bio_command_missing() {
        let mut world = setup_minimal_world();
        handle_bio_command(&mut world, &["bio"]);
    }

    #[test]
    fn test_print_bio() {
        let mut world = setup_minimal_world();
        print_bio(&mut world, 1);
    }

    #[test]
    fn test_handle_map_command_valid() {
        let mut world = setup_minimal_world();
        handle_map_command(&mut world, &["map", "5", "5"]);
    }

    #[test]
    fn test_handle_map_command_missing() {
        let mut world = setup_minimal_world();
        handle_map_command(&mut world, &["map"]);
    }

    #[test]
    fn test_handle_map_command_invalid() {
        let mut world = setup_minimal_world();
        handle_map_command(&mut world, &["map", "a", "b"]);
    }

    #[test]
    fn test_handle_find_command_valid() {
        let mut world = setup_minimal_world();
        handle_find_command(&mut world, &["find", "tree"]);
    }

    #[test]
    fn test_handle_find_command_missing() {
        let mut world = setup_minimal_world();
        handle_find_command(&mut world, &["find"]);
    }

    #[test]
    fn test_handle_find_command_with_count() {
        let mut world = setup_minimal_world();
        handle_find_command(&mut world, &["find", "rock", "5"]);
    }

    #[test]
    fn test_print_designations() {
        let mut world = setup_minimal_world();
        print_designations(&mut world);
    }

    #[test]
    fn test_print_buildings() {
        let mut world = setup_minimal_world();
        print_buildings(&mut world);
    }

    #[test]
    fn test_print_great_works() {
        let mut world = setup_minimal_world();
        print_great_works(&mut world);
    }

    #[test]
    fn test_print_black_market() {
        let mut world = setup_minimal_world();
        print_black_market(&mut world);
    }

    #[test]
    fn test_handle_smuggler_command() {
        let mut world = setup_minimal_world();
        handle_smuggler_command(&mut world);
        assert_eq!(
            scale::layer1::economy::black_market::smuggler_count(&mut world),
            1
        );
    }

    #[test]
    fn test_print_chronicle() {
        let mut world = setup_minimal_world();
        world.insert_resource(scale::layer1::core::chronicle::Chronicle::default());
        print_chronicle(&mut world);
    }

    #[test]
    fn test_print_log() {
        let mut world = setup_minimal_world();
        world.insert_resource(scale::shared::log::MessageLog::default());
        print_log(&mut world);
    }

    #[test]
    fn test_print_tech() {
        let mut world = setup_minimal_world();
        world.insert_resource(scale::layer1::tech::TechState::default());
        print_tech(&mut world);
    }

    #[test]
    fn test_handle_research_command_valid() {
        let mut world = setup_minimal_world();
        world.insert_resource(scale::layer1::tech::TechState::default());
        world.insert_resource(scale::layer1::economy::resources::ColonyResources::default());
        world.insert_resource(scale::shared::log::MessageLog::default());
        handle_research_command(&mut world, &["research", "Hydroponics"]);
    }

    #[test]
    fn test_handle_unknown_command() {
        let mut world = setup_minimal_world();
        let result = handle_command(&mut world, "unknown_command_123");
        assert!(result);
    }

    #[test]
    fn test_handle_command_stories() {
        let mut world = setup_minimal_world();
        let result = handle_command(&mut world, "stories");
        assert!(result);
    }

    #[test]
    fn test_handle_command_quit() {
        let mut world = setup_minimal_world();
        let result = handle_command(&mut world, "quit");
        assert!(!result);
    }

    #[test]
    fn test_handle_command_help() {
        let mut world = setup_minimal_world();
        let result = handle_command(&mut world, "help");
        assert!(result);
    }

    #[test]
    fn test_print_status() {
        let mut world = setup_minimal_world();
        world.insert_resource(scale::layer1::economy::resources::ColonyResources::default());
        world.insert_resource(scale::shared::time::SimulationTime {
            tick: 1,
            speed: scale::shared::time::SimSpeed::Normal,
        });
        world.insert_resource(scale::layer1::nature::wind::GlobalWind::default());
        world.insert_resource(scale::layer1::nature::ecology::EcologyConfig::default());
        world.insert_resource(scale::layer1::day_night::DayNightCycle::default());
        print_status(&mut world);
    }

    #[test]
    fn test_print_pops() {
        let mut world = setup_minimal_world();
        print_pops(&mut world);
    }

    #[test]
    fn test_handle_build_command_valid() {
        let mut world = setup_minimal_world();
        handle_build_command(&mut world, &["build", "Wall", "5", "5"]);
    }

    #[test]
    fn test_handle_build_command_invalid() {
        let mut world = setup_minimal_world();
        handle_build_command(&mut world, &["build", "InvalidBuilding", "5", "5"]);
    }

    #[test]
    fn test_handle_build_command_missing() {
        let mut world = setup_minimal_world();
        handle_build_command(&mut world, &["build"]);
    }

    #[test]
    fn test_handle_designate_command_destroy() {
        let mut world = setup_minimal_world();
        handle_designate_command(&mut world, &["destroy", "5", "5"], DesignationType::Destroy);
    }

    #[test]
    fn test_handle_designate_command_chop() {
        let mut world = setup_minimal_world();
        handle_designate_command(&mut world, &["chop", "5", "5"], DesignationType::Chop);
    }

    #[test]
    fn test_print_help() {
        print_help();
    }

    #[test]
    fn test_scan_terrain_overflow() {
        let mut world = setup_minimal_world();
        // This should panic in debug mode due to overflow if not handled
        scan_terrain(&mut world, i32::MAX, i32::MAX, ScanRadius::new(10).unwrap());
    }

    #[test]
    fn test_scan_radius_validation() {
        assert!(ScanRadius::new(10).is_ok());
        assert!(ScanRadius::new(0).is_ok());
        assert!(ScanRadius::new(100).is_ok());

        assert!(ScanRadius::new(-1).is_err());
        assert!(ScanRadius::new(101).is_err());
        assert!(ScanRadius::new(i32::MAX).is_err());
    }
}
