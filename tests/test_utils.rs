#![allow(dead_code)]

use std::path::PathBuf;
use time::{format_description, OffsetDateTime};

const CTRL_INDEX: &str = "CTRL_INDEX";

pub fn file(filename: &str) -> PathBuf {
    let mut f = dir();
    f.push(filename);
    f
}

pub fn dir() -> PathBuf {
    let mut d = PathBuf::new();
    d.push("log_files");
    add_prog_name(&mut d);
    d.push(now_local_or_utc().format(
        &format_description::parse("[year]-[month]-[day]_[hour]-[minute]-[second]").unwrap(/*ok*/),
    ).unwrap());
    d
}
pub fn add_prog_name(pb: &mut PathBuf) {
    let path = PathBuf::from(std::env::args().next().unwrap());
    let filename = path.file_stem().unwrap(/*ok*/).to_string_lossy();

    // rsplit_once not available with rustc 1.51.0
    // let (progname, _) = filename.rsplit_once('-').unwrap_or((&filename, ""));
    let filename = filename.to_string();
    let progname = match filename.rfind('-') {
        Some(idx) => &filename[0..idx],
        None => filename.as_str(),
    };

    pb.push(progname);
}

// launch child process from same executable and sets there an additional environment variable
// or finds this environment variable and returns its value
pub fn dispatch(count: u8) -> Option<u8> {
    match std::env::var(CTRL_INDEX) {
        Err(_) => {
            println!("dispatcher");
            let progname = std::env::args().next().unwrap();
            let nocapture = std::env::args().any(|a| a == "--nocapture");
            for value in 0..count {
                let mut command = std::process::Command::new(progname.to_string());
                if nocapture {
                    command.arg("--nocapture");
                }
                let status = command
                    .env(CTRL_INDEX, value.to_string())
                    .status()
                    .expect("Command failed to start");
                assert!(status.success());
            }
            None
        }
        Ok(value) => {
            println!("executor {}", value);
            Some(value.parse().unwrap())
        }
    }
}

pub fn now_local_or_utc() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}
