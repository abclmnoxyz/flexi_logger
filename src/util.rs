use crate::{deferred_now::DeferredNow, FormatFunction};
use log::Record;
use std::cell::RefCell;
use std::io::Write;

#[cfg(test)]
use std::io::Cursor;
#[cfg(test)]
use std::sync::{Arc, Mutex};

#[cfg(feature = "async")]
pub(crate) const ASYNC_FLUSH: &[u8] = b"F";
#[cfg(feature = "async")]
pub(crate) const ASYNC_SHUTDOWN: &[u8] = b"S";

#[derive(Copy, Clone, Debug)]
pub(crate) enum ERRCODE {
    Write,
    Flush,
    Format,
    Poison,
    LogFile,
    WriterSpec,
    #[cfg(feature = "specfile")]
    LogSpecFile,
    #[cfg(target_os = "linux")]
    Symlink,
}
impl ERRCODE {
    fn as_index(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Flush => "flush",
            Self::Format => "format",
            Self::Poison => "poison",
            Self::LogFile => "logfile",
            Self::WriterSpec => "writerspec",
            #[cfg(feature = "specfile")]
            Self::LogSpecFile => "logspecfile",
            #[cfg(target_os = "linux")]
            Self::Symlink => "symlink",
        }
    }
}

pub(crate) fn eprint_err(errcode: ERRCODE, msg: &str, err: &dyn std::error::Error) {
    let s = format!(
        "[flexi_logger][ERRCODE::{code:?}] {msg}, caused by {err}\n\
         See https://docs.rs/flexi_logger/latest/flexi_logger/error_info/index.html#{code_lc}",
        msg = msg,
        err = err,
        code = errcode,
        code_lc = errcode.as_index(),
    );
    try_to_write(&s);
}

pub(crate) fn eprint_msg(errcode: ERRCODE, msg: &str) {
    let s = format!(
        "[flexi_logger][ERRCODE::{code:?}] {msg}\n\
         See https://docs.rs/flexi_logger/latest/flexi_logger/error_info/index.html#{code_lc}",
        msg = msg,
        code = errcode,
        code_lc = errcode.as_index(),
    );
    try_to_write(&s);
}

fn try_to_write(s: &str) {
    eprintln!("{}", s);
    // TODO DOES THIS MAKE SENSE (for issue#75)? NEEDS SOME TESTING
    // let w = std::io::stderr();
    // let mut wl = w.lock();
    // let result = wl.write(s.as_bytes());
    // result.ok();
}

pub(crate) fn io_err(s: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, s)
}

// Thread-local buffer
pub(crate) fn buffer_with<F>(f: F)
where
    F: FnOnce(&RefCell<Vec<u8>>),
{
    thread_local! {
        static BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(200));
    }
    BUFFER.with(f);
}

// Use the thread-local buffer for formatting before writing into the given writer
pub(crate) fn write_buffered(
    format_function: FormatFunction,
    now: &mut DeferredNow,
    record: &Record,
    w: &mut dyn Write,
    #[cfg(test)] o_validation_buffer: Option<&Arc<Mutex<Cursor<Vec<u8>>>>>,
) -> Result<(), std::io::Error> {
    let mut result: Result<(), std::io::Error> = Ok(());

    buffer_with(|tl_buf| match tl_buf.try_borrow_mut() {
        Ok(mut buffer) => {
            (format_function)(&mut *buffer, now, record)
                .unwrap_or_else(|e| eprint_err(ERRCODE::Format, "formatting failed", &e));
            buffer
                .write_all(b"\n")
                .unwrap_or_else(|e| eprint_err(ERRCODE::Write, "writing failed", &e));

            result = w.write_all(&*buffer).map_err(|e| {
                eprint_err(ERRCODE::Write, "writing failed", &e);
                e
            });

            #[cfg(test)]
            if let Some(valbuf) = o_validation_buffer {
                valbuf.lock().unwrap().write_all(&*buffer).ok();
            }
            buffer.clear();
        }
        Err(_e) => {
            // We arrive here in the rare cases of recursive logging
            // (e.g. log calls in Debug or Display implementations)
            // we print the inner calls, in chronological order, before finally the
            // outer most message is printed
            let mut tmp_buf = Vec::<u8>::with_capacity(200);
            (format_function)(&mut tmp_buf, now, record)
                .unwrap_or_else(|e| eprint_err(ERRCODE::Format, "formatting failed", &e));
            tmp_buf
                .write_all(b"\n")
                .unwrap_or_else(|e| eprint_err(ERRCODE::Write, "writing failed", &e));

            result = w.write_all(&tmp_buf).map_err(|e| {
                eprint_err(ERRCODE::Write, "writing failed", &e);
                e
            });

            #[cfg(test)]
            if let Some(valbuf) = o_validation_buffer {
                valbuf.lock().unwrap().write_all(&tmp_buf).ok();
            }
        }
    });
    result
}
