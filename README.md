# Ubiquity

Ubiquity is a new file syncing *library* written in Rust. It is loosely inspired by [Unison](https://www.cis.upenn.edu/~bcpierce/unison/), another great sync utility, but it is designed first and foremost as a library not a command line utility, and it also handles an unlimited number of replicas.

## Features

- Unlimited number of replicas
- Configurable update detection that could be powered by a file system watcher (eg: kqueue on Linux or FSEvents on OS X)
- Runs as a library inside another application, and has a clean API.
- Caches filesystem state in 'archive directory', speeding up subsequent sync operations.

## Drawbacks

It would be unfair to claim that Ubiquity is suitable for every task, as after a few weekends of work Ubiquity is far from battle-tested.

- Requires nightly Rust!
    + It uses `serde_macros` to generate serialization/deserialization code for archives
    + It (as of recently) uses the question mark operator to make the code cleaner
- Test coverage is not great
    + The basics are there, and I use it to keep my own files in sync.
    + However lesser used code paths might be buggy.
- Ubiquity might not handle symlinks
    + Some code paths in the `ubiqiuity::propagate` module are `unimplemented!()`, because I haven't used symlinks enough inside replicas to really need explicit 'support'.
- Change propagation is not atomic

## Basic Operation
The syncing process consists of three stages.
First you call `ubiquity::detect::find_updates` with a few of arguments
which tell Ubiquity where to look for changed files. It will return a list of files that differ between replicas.

You can do whatever you like with that list, but most often you want to resolve those differences.
You can use an algorithm, user input, or a hardcoded value to determine which replica is the 'master' replica for each difference.
The 'master' is the correct, most up to date version of the file that will be propagated to all other replicas.

Ubiquity comes with the `ubiquity::reconcile::guess_operation` function which will pick whichever file changed since the last run, or `None` if no files change, or if files changed on both sides.

Once you have the 'master' replica, you can propagate changes using `ubiquity::propagate::propagate`.

```rust
extern crate ubiquity;
extern crate regex;
extern crate typenum;
#[macro_use]
extern crate generic_array;

use std::path::{Path, PathBuf};
use std::fs;
use ubiquity::{archive, detect, reconcile, propagate};
use ubiquity::config::{SyncInfo};
use regex::Regex;

fn main() {
    let archive = archive::Archive::new(Path::new("tests/replicas/archives").to_path_buf()).unwrap();

    let a = Path::new("tests/replicas/path_a");
    let b = Path::new("tests/replicas/path_b");
    if !a.is_dir() {
        fs::create_dir(a).unwrap();
    }
    if !b.is_dir() {
        fs::create_dir(b).unwrap();
    }

    let mut config: SyncInfo = SyncInfo::new(arr![PathBuf; PathBuf::from("tests/replicas/path_a"), PathBuf::from("tests/replicas/path_b")]);
    config.ignore.regexes.push(Regex::new(r".DS_Store").unwrap());
    config.ignore.paths.push("Microsoft User Data".to_string());

    let mut search = detect::SearchDirectories::from_root();

    let result = detect::find_updates(&archive, &mut search, &config, &detect::EmptyProgressCallback).expect("Failed to find conflicts");

    if result.differences.is_empty() {
        println!("All in sync");
    }

    for difference in result.differences {
        let operation = reconcile::guess_operation(&difference);
        println!("Difference at {:?}, resolving using {:?}", difference.path, operation);
        if let reconcile::Operation::PropagateFromMaster(master) = operation {
            propagate::propagate(&difference, master, &archive, &propagate::DefaultPropagationOptions, &propagate::EmptyProgressCallback).unwrap();
        }
    }
}
```
