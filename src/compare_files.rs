use std::io;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;

pub fn file_contents_equal_cmd(a: &Path, b: &Path) -> io::Result<bool> {
    debug!("Comparing {:?} with {:?}", a, b);
    Ok(Command::new("cmp")
             .stdout(Stdio::null())
             .arg(format!("{}", a.to_str().unwrap()))
             .arg(format!("{}", b.to_str().unwrap()))
             .status()?.code().unwrap() == 0)
}

/*pub fn file_contents_equal(a: &Path, b: &Path) -> io::Result<bool> {
    debug!("Comparing {:?} with {:?}", a, b);
    let mut buf_a = vec![0; 4096];
    let mut buf_b = vec![0; 4096];
    let mut file_a = try!(File::open(a));
    let mut file_b = try!(File::open(b));
    let mut a_eof = false;
    let mut b_eof = false;
    let mut i = 0;

    loop {
        match file_a.read_exact(&mut*buf_a) {
            Ok(_) => {},
            Err(e) => match e.kind() {
                io::ErrorKind::UnexpectedEof => a_eof = true,
                _ => return Err(e)
            }
        }
         match file_b.read_exact(&mut*buf_b) {
            Ok(_) => {},
            Err(e) => match e.kind() {
                io::ErrorKind::UnexpectedEof => b_eof = true,
                _ => return Err(e)
            }
        }

        if buf_a != buf_b {
            return Ok(false);
        }

        // if one reaches eof and the other doesn't, then they aren't equal
        match (a_eof, b_eof) {
            (true, true) => return Ok(true),
            (true, false) | (false, true) => return Ok(false),
            _ => {}
        }

        i += 1;
        if i % 100 == 0 {
            trace!("Read {} blocks", i);
        }
    }
}*/