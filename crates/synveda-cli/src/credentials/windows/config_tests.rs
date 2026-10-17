use super::*;

use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Barrier;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::windows_private as fixture;

const LIMIT: u64 = 256 * 1024;

/// A task-owned directory below the test temp root; ordinary configuration
/// needs no private ancestry, so the root keeps its inherited ACL.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("synveda config '文' {}", nonce().unwrap()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    /// Temporaries this module could have left, anywhere below the scratch.
    fn leftovers(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![self.0.clone()];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".synveda-")
                {
                    found.push(path);
                }
            }
        }
        found
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn version(byte: u8) -> Vec<u8> {
    vec![byte; 64 * 1024]
}

#[test]
fn first_creation_is_private_and_replacement_keeps_the_owners_dacl() {
    let scratch = Scratch::new();
    let path = scratch.0.join("vendor dir/mcp config 文.json");
    assert_eq!(read(&path, LIMIT).unwrap(), None);
    replace(&path, None, b"first", LIMIT).unwrap();
    assert_eq!(read(&path, LIMIT).unwrap().unwrap(), b"first");
    let created = fixture::dacl(&path);
    assert!(
        created.starts_with("D:P"),
        "first creation is not private: {created}"
    );
    assert!(!created.contains(";BU)"), "{created}");

    // The owner shares the file; Synveda's next edit must keep that choice.
    fixture::allow(&path, "S-1-5-32-545", "Read", "None");
    let shared = fixture::dacl(&path);
    replace(&path, Some(b"first"), b"second", LIMIT).unwrap();
    assert_eq!(read(&path, LIMIT).unwrap().unwrap(), b"second");
    assert_eq!(fixture::dacl(&path), shared);

    // An inherited (unprotected) DACL is reproduced too, including entries
    // inherited from the parent and the owner's explicit grant.
    let inherited = scratch.0.join("inherited.json");
    std::fs::write(&inherited, b"one").unwrap();
    fixture::allow(&inherited, "S-1-5-32-545", "Read", "None");
    let before = fixture::dacl(&inherited);
    assert!(!before.starts_with("D:P"), "{before}");
    replace(&inherited, Some(b"one"), b"two", LIMIT).unwrap();
    assert_eq!(fixture::dacl(&inherited), before);
    assert!(scratch.leftovers().is_empty(), "{:?}", scratch.leftovers());
}

#[test]
fn dacl_comparison_ignores_inheritance_bookkeeping_but_not_protection_or_entries() {
    assert_eq!(access("D:AI(A;ID;FA;;;SY)"), access("D:(A;ID;FA;;;SY)"));
    assert_eq!(access("D:PAI(A;;FA;;;SY)"), access("D:P(A;;FA;;;SY)"));
    assert_ne!(access("D:P(A;;FA;;;SY)"), access("D:(A;;FA;;;SY)"));
    assert_ne!(
        access("D:(A;;FA;;;SY)"),
        access("D:(A;;FA;;;SY)(A;;FR;;;WD)")
    );
}

#[test]
fn stale_snapshots_and_concurrent_creation_never_replace() {
    let scratch = Scratch::new();
    let path = scratch.0.join("config.json");
    std::fs::write(&path, b"theirs").unwrap();
    // Created by someone else after this command read "absent".
    assert!(replace(&path, None, b"ours", LIMIT).is_err());
    // Edited by someone else after this command read it.
    assert!(replace(&path, Some(b"stale"), b"ours", LIMIT).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"theirs");
    assert!(scratch.leftovers().is_empty());
}

#[test]
fn simultaneous_first_creators_publish_exactly_one_complete_version() {
    let scratch = Scratch::new();
    for round in 0..8 {
        let path = scratch.0.join(format!("race-{round}.json"));
        let barrier = Barrier::new(4);
        let published = std::thread::scope(|scope| {
            let writers: Vec<_> = (0..4u8)
                .map(|writer| {
                    let (path, barrier) = (&path, &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        replace(path, None, &version(b'a' + writer), LIMIT).is_ok()
                    })
                })
                .collect();
            writers
                .into_iter()
                .map(|writer| writer.join())
                .filter(|joined| matches!(joined, Ok(true)))
                .count()
        });
        assert_eq!(published, 1, "round {round}");
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 64 * 1024);
        assert!(bytes.iter().all(|byte| *byte == bytes[0]));
    }
    assert!(scratch.leftovers().is_empty(), "{:?}", scratch.leftovers());
}

#[test]
fn readers_observe_only_complete_versions_during_replacement() {
    let scratch = Scratch::new();
    let path = scratch.0.join("config.json");
    replace(&path, None, &version(b'0'), LIMIT).unwrap();
    let done = AtomicBool::new(false);
    let (complete, moved) = std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            let (mut complete, mut moved) = (0_u32, 0_u32);
            while !done.load(Ordering::SeqCst) {
                match read(&path, LIMIT) {
                    Ok(Some(bytes)) => {
                        assert_eq!(bytes.len(), 64 * 1024, "torn read");
                        assert!(bytes.iter().all(|byte| *byte == bytes[0]), "mixed read");
                        complete += 1;
                    }
                    Ok(None) => panic!("the configuration name was briefly absent"),
                    // A rename between open and identity recheck is reported,
                    // never returned as bytes.
                    Err(error) if error.to_string().contains("changed") => moved += 1,
                    Err(error) => panic!("unexpected read failure: {error}"),
                }
            }
            (complete, moved)
        });
        let mut current = version(b'0');
        for step in 1..=40_u8 {
            let next = version(b'0' + step % 10);
            replace(&path, Some(&current), &next, LIMIT).unwrap();
            current = next;
        }
        done.store(true, Ordering::SeqCst);
        reader.join().unwrap()
    });
    assert!(complete > 0, "reader made no observations ({moved} moved)");
    assert!(scratch.leftovers().is_empty());
}

#[test]
fn sharing_conflicts_wait_within_their_bound_then_preserve_the_original() {
    let scratch = Scratch::new();
    let path = scratch.0.join("config.json");
    std::fs::write(&path, b"original").unwrap();
    // The replacement starts only once the handle is open; a sleep would let a
    // slow runner open it after the replacement had already succeeded.
    let opened = Barrier::new(2);
    let hold = |duration: Duration| {
        // An editor-style handle without delete sharing.
        let file = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        opened.wait();
        std::thread::sleep(duration);
        drop(file);
    };
    std::thread::scope(|scope| {
        let holder = scope.spawn(|| hold(Duration::from_millis(150)));
        opened.wait();
        replace(&path, Some(b"original"), b"after brief hold", LIMIT).unwrap();
        holder.join().unwrap();
    });
    assert_eq!(std::fs::read(&path).unwrap(), b"after brief hold");
    std::thread::scope(|scope| {
        let holder = scope.spawn(|| hold(Duration::from_millis(1500)));
        opened.wait();
        let error = replace(&path, Some(b"after brief hold"), b"never", LIMIT).unwrap_err();
        assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
        holder.join().unwrap();
    });
    assert_eq!(std::fs::read(&path).unwrap(), b"after brief hold");
    assert!(scratch.leftovers().is_empty());
}

#[test]
fn publication_and_creation_failures_keep_the_previous_version() {
    let scratch = Scratch::new();
    let path = scratch.0.join("config.json");
    std::fs::write(&path, b"read-only original").unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions.clone()).unwrap();
    // Publication is refused by the OS; no delete or copy fallback runs.
    assert!(replace(&path, Some(b"read-only original"), b"replacement", LIMIT).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"read-only original");
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&path, permissions).unwrap();
    // The temporary cannot be created below a file.
    let below_file = path.join("nested.json");
    assert!(replace(&below_file, None, b"never", LIMIT).is_err());
    // Oversized input or state is refused before publication.
    assert!(
        replace(
            &path,
            Some(b"read-only original"),
            &vec![b'x'; 300 * 1024],
            LIMIT
        )
        .is_err()
    );
    std::fs::write(&path, vec![b'y'; 300 * 1024]).unwrap();
    assert!(read(&path, LIMIT).is_err());
    assert!(scratch.leftovers().is_empty(), "{:?}", scratch.leftovers());
}

#[test]
fn links_and_directories_are_refused_without_touching_their_targets() {
    let scratch = Scratch::new();
    let path = scratch.0.join("config.json");
    let alias = scratch.0.join("alias.json");
    std::fs::write(&path, b"linked").unwrap();
    std::fs::hard_link(&path, &alias).unwrap();
    assert!(read(&path, LIMIT).is_err());
    assert!(replace(&path, Some(b"linked"), b"never", LIMIT).is_err());
    assert_eq!(std::fs::read(&alias).unwrap(), b"linked");
    let directory = scratch.0.join("directory.json");
    std::fs::create_dir(&directory).unwrap();
    assert!(read(&directory, LIMIT).is_err());
    assert!(replace(&directory, None, b"never", LIMIT).is_err());
    assert!(directory.is_dir());
    assert!(scratch.leftovers().is_empty());
}

#[test]
#[ignore = "needs SeCreateSymbolicLinkPrivilege or Developer Mode; CI runs it explicitly"]
fn symbolic_links_are_refused_without_touching_their_targets() {
    let scratch = Scratch::new();
    let target = scratch.0.join("target.json");
    let link = scratch.0.join("config.json");
    std::fs::write(&target, b"target").unwrap();
    std::os::windows::fs::symlink_file(&target, &link).expect("symbolic link privilege");
    assert!(read(&link, LIMIT).is_err());
    assert!(replace(&link, Some(b"target"), b"never", LIMIT).is_err());
    assert!(replace(&link, None, b"never", LIMIT).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"target");
    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    assert!(scratch.leftovers().is_empty());
}
