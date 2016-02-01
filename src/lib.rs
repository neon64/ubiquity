#![feature(plugin)]
#![feature(hashmap_hasher)]
#![feature(custom_derive)]
#![plugin(serde_macros)]

extern crate fnv;
#[macro_use]
extern crate bincode;
extern crate serde;
extern crate regex;
extern crate byteorder;
#[macro_use]
extern crate log;
extern crate walkdir;

pub mod conflict;
pub mod archive;
pub mod util;
pub mod state;
pub mod transfer;
pub mod error;
mod compare_files;