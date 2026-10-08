# SCALE

*From pebble to empire. Every world remembers.*

[![CI](https://github.com/madmax983/scale/workflows/CI/badge.svg)](https://github.com/madmax983/scale/actions)

A 4X colony simulation where you begin Dwarf-Fortress-style on a single planet and end commanding a galactic civilization—but every world is still simulating underneath.

## Status

**Pre-Alpha** — AI agents are building this game.

## Quick Start

### Native (Terminal)

```bash
cargo run --bin scale

cargo test

cargo criterion
```

### Browser (WASM)

```bash
# Install prerequisites
rustup target add wasm32-unknown-unknown
cargo install --locked trunk

# Serve locally at http://localhost:8080
trunk serve
```

### Pre-commit Hooks

```bash
.\install-hooks.ps1  # Windows
./install-hooks.sh   # Linux/Mac
```

## Usage as a Library

### Procedural Generation (Narrative)


To use SCALE's procedural generation in your own Rust code:

```rust
// In Cargo.toml:
// [dependencies]
// scale = "0.1.0"

use scale::prelude::*;

fn main() {
    // 1. Initialize Generator (loads embedded lore by default)
    let generator = NarrativeGenerator::from_embedded();

    // Or load from a directory (must contain TEMPLATES.md and FRAGMENTS.md)
    // let mut generator = NarrativeGenerator::default();
    // generator.load_from_files("./lore").unwrap();

    // 2. Prepare Context
    let mut context = NarrativeContext::default();
    context.insert("CIV_NAME", "Terran Dominion");
    // Note: If required context variables are missing, `generate` will return an error.
    context.insert("ORIGIN_STAR", "Sol Prime");
    context.insert("YEAR", "2150");
    context.insert("CIV_EPITHET", "The First Ones");

    // 3. Generate Story
    match generator.generate("CIVILIZATION_RISE", &context) {
        Ok(story) => println!("{}", story),
        Err(e) => {
            eprintln!("{}", e); // Prints a helpful missing context message
        }
    }
}
```

See `examples/narrative_demo.rs` for a complete example.

**Note:** This is the **base narrative system** (mad-libs style text generation). It is available in the default build.
For the **advanced simulation of legends** (Oral Tradition), see the [Oral Tradition](#oral-tradition-nova-feature) section below.

### Oral Tradition (Nova Feature)

> # ℹ️ OPTIONAL FEATURE: NOVA
>
> **NOTE:** The snippet below uses the `nova` feature. If the feature is not enabled, the code will still compile and run, but `OralTradition` will act as a stub and print a warning instead of processing stories.
>
> *Enable `features = ["nova"]` in your `Cargo.toml` to activate the actual storytelling mechanics.*

The "Nova" feature (Oral Tradition) builds upon the base narrative system to create living legends that evolve in taverns. It is located in `scale::layer1::oral_tradition`.

**Usage:**

> **Run with:** `cargo run --features nova`

```toml
# In Cargo.toml:
[dependencies]
scale = { version = "0.1.0", features = ["nova"] }
```

```rust
use scale::prelude::*;

fn main() {
    let mut tradition = OralTradition::default();
    let mut chronicle = Chronicle::default();

    // 1. Add a historical event
    chronicle.add_event(
        100, // tick
        "The colony survived the Great Frost.".to_string(),
        EventImportance::Major,
    );

    // 2. Process events into stories
    tradition.process_chronicles(&chronicle);

    // Inspect
    println!("{:?}", tradition.stories);
}
```

**Running the Demos:**

```bash
# Minimal API usage
cargo run --features nova --example minimal_nova_demo

# Full Interactive TUI Demo
cargo run --features nova --example oral_tradition_demo
```

### Headless Simulation

To run the full simulation loop without a window or GPU (e.g. for servers or AI training):

```rust
use scale::prelude::*;

fn main() {
    // 1. Setup the world with headless configuration
    let config = SetupConfig {
        headless: true,
        ..Default::default()
    };
    let mut world = setup_world_with_config(config);

    // 2. Run a few ticks
    for _ in 0..10 {
        run_simulation_tick(&mut world);
    }

    // 3. Inspect state
    let time = world.resource::<SimulationTime>();
    println!("Current Tick: {}", time.tick);

    // You can also query game components easily using the prelude
    let mut query = world.query::<&Building>();
    let _buildings = query.iter(&world).count();
}
```

See `examples/headless_demo.rs` for a complete example.

## Controls

| Key | Action |
|-----|--------|
| **WASD / Arrows** | Scroll map (or move cursor in build mode) |
| **Space** | Pause/unpause |
| **1/2/3** | Speed (1x/3x/5x) |
| **B** | Toggle build mode |
| **M** | Toggle mine designation mode |
| **X** | Toggle demolish designation mode |
| **Tab** | Cycle building type (in build mode) |
| **Enter** | Place building / confirm |
| **L** | Toggle chronicle overlay |
| **Escape** | Exit current mode / close overlay |
| **Q** | Quit |
| **Mouse click** | Select entity or tile |

## Display

```
Terrain:   , grass   . dirt   # rock   ~ water   ^ tree
Pops:      @ idle/moving
Buildings: H housing   F farm   S stockpile   T tavern
```

## Architecture

Three simulation layers, each abstracting the one below:

1. **Colony** (Dwarf Fortress) — Individual pops, buildings, needs, jobs
2. **System** (Planetary) — Planets as nodes, ships, orbital stations
3. **Galaxy** (Stellaris) — Star systems, civilizations, diplomacy

**Tech stack:** Rust, `bevy_ecs` for simulation, `ratatui` for UI, `ratzilla` for browser support.

Runs natively in a terminal via crossterm, or in any browser via WASM + Ratzilla's DOM backend. Both targets share identical game logic — only the input translation and render backend differ.

See **[DESIGN.md](DESIGN.md)** for full architecture details and **[docs/adr/](docs/adr/)** for architecture decision records.

## Procedural History

Every playthrough generates unique lore:

- **Pre-history:** 500+ years of galactic events generated at game start
- **Chronicle:** Ongoing events recorded as you play
- **Discovery:** Find artifacts, ruins, and legends with generated backstories

See `lore/` for the building blocks.

## Project Structure

```
scale/
├── src/
│   ├── main.rs           # Native entry point (crossterm)
│   ├── lib.rs            # Library exports
│   ├── bin/
│   │   ├── wasm_app.rs   # WASM entry point (ratzilla)
│   │   └── headless.rs   # Headless simulation runner
│   ├── platform/         # Input abstraction (native ↔ wasm)
│   ├── setup.rs          # Shared world initialization
│   ├── simulation.rs     # Shared simulation tick
│   ├── layer1/           # Colony simulation (pops, buildings, needs, AI)
│   ├── shared/           # Input routing, selection, time, narrative
│   └── ui/               # Terminal UI rendering (backend-agnostic)
├── index.html            # Trunk HTML entry point for WASM
├── e2e/                  # Playwright E2E browser tests
├── specs/                # Feature specifications
├── lore/                 # Procedural lore system
├── design/               # Design docs and task tracking
├── docs/                 # Architecture documentation + ADRs
├── benches/              # Performance benchmarks
└── prompts/              # AI agent prompts
```

## Technical Details

**Language:** Rust Edition 2021

**Dependencies:**
- `bevy_ecs` — Entity Component System for simulation
- `ratatui` — Terminal UI framework (backend-agnostic)
- `crossterm` — Native terminal backend (optional, `native` feature)
- `ratzilla` — Browser WASM backend (optional, `wasm` feature)
- `anyhow` — Error handling
- `rand` — Random generation
- `criterion` — Benchmarking (dev)

**Feature flags:**
- `native` (default) — Terminal mode via crossterm
- `wasm` — Browser mode via ratzilla

**Profiles:**
- `dev` — Optimized for fast iteration (opt-level = 1)
- `release` — Fully optimized (LTO, single codegen unit)
- `bench` — Inherits from release

## CI/CD

- **Format** + **Clippy** (pedantic + nursery) + **Tests** + **Coverage** (native)
- **WASM Build** via Trunk
- **E2E Tests** via Playwright against the WASM build
- **85%+ test coverage** enforced

See **[CI-SETUP.md](CI-SETUP.md)** for full CI/CD documentation.

## Development

This project is built by AI agents coordinating through git. See **[AGENTS.md](AGENTS.md)** for the protocol.

See **[docs/guides/EXTENDING.md](docs/guides/EXTENDING.md)** for a guide on adding new buildings.

## Contributing

Follow the existing spec-driven workflow and ensure all changes pass CI quality gates.

## License

MIT
