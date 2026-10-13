# Runs the agent mesh CLI on Windows. Usage: scripts/mesh.ps1 <command> [args].
$root = Split-Path -Parent $PSScriptRoot
if (-not $env:MESH_DIR) { $env:MESH_DIR = Join-Path $root ".mesh" }
cargo run --quiet --release --manifest-path (Join-Path $root "crates/agent-mesh/Cargo.toml") -- @args
exit $LASTEXITCODE
