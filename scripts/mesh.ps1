# Runs the agent mesh CLI on Windows. Usage: scripts/mesh.ps1 <command> [args].
$root = Split-Path -Parent $PSScriptRoot
$crate = Join-Path $root "crates/agent-mesh"
$env:CARGO_TARGET_DIR = Join-Path $crate "target"
if (-not $env:MESH_DIR) { $env:MESH_DIR = Join-Path $root ".mesh" }
cargo run --quiet --release --manifest-path (Join-Path $crate "Cargo.toml") -- @args
exit $LASTEXITCODE
