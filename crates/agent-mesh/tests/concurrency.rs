//! Many writers at once: one claim wins, no event is lost.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command as Process;
use std::sync::{Arc, Barrier};
use std::thread;

use agent_mesh::event::TaskSpec;
use agent_mesh::ids::{AgentId, Skill, TaskId};
use agent_mesh::ledger::Command;
use agent_mesh::store::{Store, MESH_DIR};

fn tid(s: &str) -> TaskId {
    TaskId::new(s).unwrap_or_else(|e| panic!("{e}"))
}

fn add(id: &str) -> Command {
    Command::Add(TaskSpec::new(
        tid(id),
        "T",
        Skill::new("build").unwrap_or_else(|e| panic!("{e}")),
    ))
}

fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let dir = tmp.path().join(MESH_DIR);
    Store::init(&dir).unwrap_or_else(|e| panic!("{e}"));
    (tmp, dir)
}

#[test]
fn threads_racing_for_one_task_produce_one_winner() {
    let (_tmp, dir) = setup();
    Store::open(&dir)
        .and_then(|s| s.execute(1, add("a")))
        .unwrap_or_else(|e| panic!("{e}"));
    let n = 12;
    let barrier = Arc::new(Barrier::new(n));
    #[allow(
        clippy::needless_collect,
        reason = "start all threads before the first join"
    )]
    let handles: Vec<_> = (0..n)
        .map(|i| {
            let dir = dir.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let store = Store::open(&dir).unwrap_or_else(|e| panic!("{e}"));
                barrier.wait();
                store
                    .execute(
                        2,
                        Command::Claim {
                            task: Some(tid("a")),
                            agent: AgentId::new("builder").unwrap_or_else(|e| panic!("{e}")),
                            worker: Some(format!("w{i}")),
                            ttl: 60,
                        },
                    )
                    .is_ok()
            })
        })
        .collect();
    let wins = handles
        .into_iter()
        .map(|h| h.join().unwrap_or(false))
        .filter(|w| *w)
        .count();
    assert_eq!(wins, 1);
    let r = Store::open(&dir)
        .and_then(|s| s.load())
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(r.issues.is_empty());
    assert_eq!(r.envelopes.len(), 2);
}

#[test]
fn parallel_writers_lose_no_events() {
    let (_tmp, dir) = setup();
    let threads = 8;
    let per = 15;
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let dir = dir.clone();
            thread::spawn(move || {
                let store = Store::open(&dir).unwrap_or_else(|e| panic!("{e}"));
                for i in 0..per {
                    store
                        .execute(1, add(&format!("t{t}-{i}")))
                        .unwrap_or_else(|e| panic!("{e}"));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap_or_else(|_| panic!("thread panicked"));
    }
    let r = Store::open(&dir)
        .and_then(|s| s.load())
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(r.issues.is_empty(), "{:?}", r.issues);
    assert_eq!(r.ledger.len(), threads * per);
    let seqs: BTreeSet<u64> = r.envelopes.iter().map(|e| e.seq).collect();
    assert_eq!(seqs.len(), threads * per, "sequence numbers are unique");
    assert_eq!(
        seqs.iter().next_back().copied(),
        Some((threads * per) as u64)
    );
}

fn run(root: &Path, args: &[&str]) -> std::process::Child {
    Process::new(env!("CARGO_BIN_EXE_mesh"))
        .current_dir(root)
        .env("MESH_NOW", "5")
        .env_remove("MESH_DIR")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn processes_racing_for_one_task_produce_one_winner() {
    let (tmp, dir) = setup();
    Store::open(&dir)
        .and_then(|s| s.execute(1, add("a")))
        .unwrap_or_else(|e| panic!("{e}"));
    #[allow(
        clippy::needless_collect,
        reason = "start all processes before the first wait"
    )]
    let children: Vec<_> = (0..8)
        .map(|_| run(tmp.path(), &["claim", "a", "--agent", "builder"]))
        .collect();
    let wins = children
        .into_iter()
        .map(|c| c.wait_with_output().is_ok_and(|o| o.status.success()))
        .filter(|w| *w)
        .count();
    assert_eq!(wins, 1);
}
