# Ubiquity

Ubiquity is a new file syncing *library* written in Rust. It is loosely inspired by Unison, another great sync utility, but it is designed to be driven by another program (for example a GUI app) instead of the console.

## Basic Operation

Ubiquity works in a very decoupled manner. First you call `ubiquity::detect::find_conflicts` with a plethora of arguments
which tell Ubiquity where to look for changed files. It will return a list of files that differ between replicas.

You can do whatever you like with that list, but most often you want to resolve those conflicts.
You can use an algorithm, user input, or a hardcoded value to determine which replica is the 'master' replica for each conflict.
The 'master' is the correct, most up to date version of the file that will be propagated to all other replicas.

Ubiquity comes with the `ubiquity::resolve::guess` function which will pick whichever file changed since the last run, or `None` if no files change, or if files changed on both sides.

Once you have the 'master' replica, you can resolve the conflict using `ubiquity::transfer::resolve_conflict`.

Here is a basic code example:

```rust
let archive = archive::Archive::new(Path::new("/path/to/archives").to_path_buf()).unwrap();

let config = SyncInfo {
    roots: vec![Path::new("/path/to/files").to_path_buf(), Path::new("/path/to/other/files").to_path_buf()],
    ignore_regex: vec![regex!(r".DS_Store")],
    ignore_path: vec!["Microsoft User Data".to_string()],
    compare_file_contents: true
};

let mut search = detect::SearchDirectories::from_root();

let (conflicts, stats) = detect::find_conflicts(&archive, &mut search, &config, &NoProgress).expect("Failed to find conflicts");

let total = stats.archive_hits + stats.archive_additions + stats.conflicts;
println!("{}/{} of entries were already in the archive and hadn't changed", stats.archive_hits, total);
println!("{}/{} of entries weren't in the archive, but were identical\n  (this should be as close to 0% as possible, if it is higher then that means that the archive files aren't working)", stats.archive_additions, total);
println!("{}/{} of entries were conflicting", conflicts.len(), total);

if conflicts.is_empty() {
    println!("All in sync");
}

for conflict in conflicts {
    let mut resolution = resolve::guess(&conflict);
    println!("Conflict {:?} (resolving using {:?})", conflict.path, resolution);
    if let Some(master) = resolution {
        transfer::resolve_conflict(&conflict, master, &archive).unwrap();
    }
}
````
