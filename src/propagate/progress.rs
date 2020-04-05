use std::io;
use std::io::BufRead;

/// Handles progress updates for the propagation step.
pub trait ProgressCallback {
    /// Transfer progress from rsync
    fn rsync_progress(
        &self,
        transferred_bytes: usize,
        progress: u8,
        speed: &str,
        elapsed_time: &str,
        transferred: Option<u32>,
        to_check: Option<ToCheck>,
    );
}

/// A zero-sized struct with an empty implementation of ProgressCallback
pub struct EmptyProgressCallback;

impl ProgressCallback for EmptyProgressCallback {
    fn rsync_progress(
        &self,
        _: usize,
        _: u8,
        _: &str,
        _: &str,
        _: Option<u32>,
        _: Option<ToCheck>,
    ) {
    }
}

#[derive(Debug)]
/// The amount of files left to check.
pub struct ToCheck {
    pub remaining: u32,
    pub total: u32,
}

pub fn parse_from_stdout<B: BufRead, P: ProgressCallback>(
    reader: B,
    progress: &P,
) -> io::Result<()> {
    // blocks until subrocess finishes
    for text in reader.split(b'\r') {
        let text = text?;
        let text = String::from_utf8(text).unwrap();

        if text == "" {
            continue;
        }

        let mut iter = text.split_whitespace();

        let bytes: usize = iter.next().unwrap().replace(",", "").parse().unwrap();
        let percent = iter.next().unwrap();
        let percent: u8 = (&percent[0..percent.len() - 1]).parse().unwrap();
        let speed = iter.next().unwrap();
        let elapsed_time = iter.next().unwrap();

        let transferred: Option<u32> = iter
            .next()
            .map(|string| string[5..string.len() - 1].parse().unwrap());
        let to_check = iter.next().map(|string| {
            let slice = &string[7..string.len() - 1];
            let mut split = slice.split('/');
            ToCheck {
                remaining: split.next().unwrap().parse().unwrap(),
                total: split.next().unwrap().parse().unwrap(),
            }
        });

        println!("rsync: {}", text);

        progress.rsync_progress(bytes, percent, speed, elapsed_time, transferred, to_check);
    }

    Ok(())
}
