# Ubiquity

Ubiquity is a new file syncing *library* written in Rust. It is loosely inspired by Unison, another great sync utility, but it is designed to be driven by another program (for example a GUI app) instead of the console.

### Differences from Unison

- Ubiquity can sync any number of replicas, not just 2
- Ubiquity can be easily run as a library, and has a smaller and simpler codebase
- Ubiquity's archive format is a number of small files inside a folder, whereas Unison's is one big file.
- Ubiquity is a hobby project and may break at any time.

- Unison handles network syncing, Windows support etc.
- Unison is battle-tested and its operation been mathematically verified (I think)

## Basic Operation

Ubiquity works in a very decoupled manner.
First you call `ubiquity::detect::find_updates` with a few of arguments
which tell Ubiquity where to look for changed files. It will return a list of files that differ between replicas.

You can do whatever you like with that list, but most often you want to resolve those differences.
You can use an algorithm, user input, or a hardcoded value to determine which replica is the 'master' replica for each difference.
The 'master' is the correct, most up to date version of the file that will be propagated to all other replicas.

Ubiquity comes with the `ubiquity::reconcile::guess_operation` function which will pick whichever file changed since the last run, or return an error if no files changed, or if files changed on both sides.

Once you have the 'master' replica, you can propagate changes using `ubiquity::propagate::propagate`.

# Examples
```
extern crate ubiquity;
extern crate regex;
extern crate typenum;
#[macro_use]
extern crate generic_array;

use std::path::{Path, PathBuf};
use std::fs;
use ubiquity::{archive, detect, reconcile, propagate};
use ubiquity::config::{SyncInfo, Ignore};
use regex::Regex;

fn main() {
    let archive = archive::Archive::new(Path::new("tests/docs/archives").to_path_buf()).unwrap();

    fs::create_dir(Path::new("tests/docs/path_a")).unwrap();
    cafs::create_dir(Path::new("tests/docs/path_b")).unwrap();

    let mut config: SyncInfo = SyncInfo::new(arr![PathBuf; PathBuf::from("tests/docs/path_a"), PathBuf::from("tests/docs/path_b")]);
    config.ignore.regexes.push(Regex::new(r".DS_Store").unwrap());
    config.ignore.paths.push("Microsoft User Data".to_string());

    let mut search = detect::SearchDirectories::from_root();

    let result = detect::find_updates(&archive, &mut search, &config, &detect::EmptyProgressCallback).expect("Failed to find conflicts");

    if result.differences.is_empty() {
        println!("All in sync");
    }

    for difference in result.differences {
        let mut operation = reconcile::guess_operation(&difference);
        println!("Difference at {:?}, resolving using {:?}", difference.path, operation);
        if let reconcile::Operation::PropagateFromMaster(master) = operation {
            propagate::propagate(&difference, master, &archive, &propagate::DefaultPropagationOptions, &propagate::EmptyProgressCallback).unwrap();
        }
    }
}
```