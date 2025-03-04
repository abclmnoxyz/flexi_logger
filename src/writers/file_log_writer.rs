#![allow(clippy::module_name_repetitions)]
mod builder;
mod config;
mod state;
mod state_handle;

pub use self::builder::{ArcFileLogWriter, FileLogWriterBuilder, FileLogWriterHandle};

use self::{
    config::{Config, RotationConfig},
    state::State,
    state_handle::StateHandle,
};
use crate::{
    writers::LogWriter, DeferredNow, EffectiveWriteMode, FileSpec, FlexiLoggerError, FormatFunction,
};
use log::Record;
use std::path::PathBuf;

const WINDOWS_LINE_ENDING: &[u8] = b"\r\n";
const UNIX_LINE_ENDING: &[u8] = b"\n";

/// A configurable [`LogWriter`] implementation that writes to a file or a sequence of files.
///
/// See [writers](crate::writers) for usage guidance.
#[derive(Debug)]
pub struct FileLogWriter {
    // the state needs to be mutable; since `Log.log()` requires an unmutable self,
    // which translates into a non-mutating `LogWriter::write()`,
    // we need internal mutability and thread-safety.
    state_handle: StateHandle,
    max_log_level: log::LevelFilter,
}
impl FileLogWriter {
    pub(crate) fn new(
        state: State,
        max_log_level: log::LevelFilter,
        format_function: FormatFunction,
    ) -> FileLogWriter {
        let state_handle = match state.config().write_mode.inner() {
            EffectiveWriteMode::Direct
            | EffectiveWriteMode::BufferAndFlushWith(_, _)
            | EffectiveWriteMode::BufferDontFlushWith(_) => {
                StateHandle::new_sync(state, format_function)
            }

            #[cfg(feature = "async")]
            EffectiveWriteMode::AsyncWith {
                bufsize: _,
                pool_capa,
                message_capa,
                flush_interval: _,
            } => StateHandle::new_async(pool_capa, message_capa, state, format_function),
        };

        FileLogWriter {
            state_handle,
            max_log_level,
        }
    }

    /// Instantiates a builder for `FileLogWriter`.
    #[must_use]
    pub fn builder(file_spec: FileSpec) -> FileLogWriterBuilder {
        FileLogWriterBuilder::new(file_spec)
    }

    /// Returns a reference to its configured output format function.
    #[must_use]
    #[inline]
    pub fn format(&self) -> FormatFunction {
        self.state_handle.format_function()
    }

    #[must_use]
    #[doc(hidden)]
    pub fn current_filename(&self) -> PathBuf {
        self.state_handle.current_filename()
    }

    pub(crate) fn plain_write(&self, buffer: &[u8]) -> std::result::Result<usize, std::io::Error> {
        self.state_handle.plain_write(buffer)
    }

    /// Replaces parts of the configuration of the file log writer.
    ///
    /// Note that the write mode and the format function cannot be reset and
    /// that the provided `FileLogWriterBuilder` must have the same values for these as the
    /// current `FileLogWriter`.
    ///
    /// # Errors
    ///
    /// `FlexiLoggerError::Reset` if no file log writer is configured,
    ///  or if a reset was tried with a different write mode.
    /// `FlexiLoggerError::Io` if the specified path doesn't work.
    /// `FlexiLoggerError::Poison` if some mutex is poisoned.
    pub fn reset(&self, flwb: &FileLogWriterBuilder) -> Result<(), FlexiLoggerError> {
        self.state_handle.reset(flwb)
    }
}

impl LogWriter for FileLogWriter {
    #[inline]
    fn write(&self, now: &mut DeferredNow, record: &Record) -> std::io::Result<()> {
        self.state_handle.write(now, record)
    }

    #[inline]
    fn flush(&self) -> std::io::Result<()> {
        self.state_handle.flush()
    }

    #[inline]
    fn max_log_level(&self) -> log::LevelFilter {
        self.max_log_level
    }

    #[doc(hidden)]
    fn validate_logs(&self, expected: &[(&'static str, &'static str, &'static str)]) {
        self.state_handle.validate_logs(expected);
    }

    fn shutdown(&self) {
        self.state_handle.shutdown();
    }
}

impl Drop for FileLogWriter {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod test {
    use crate::deferred_now::now_local_or_utc;
    use crate::writers::LogWriter;
    use crate::{Cleanup, Criterion, DeferredNow, FileSpec, Naming, WriteMode};
    use std::ops::Add;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use time::format_description;

    const DIRECTORY: &str = r"log_files/rotate";
    const ONE: &str = "ONE";
    const TWO: &str = "TWO";
    const THREE: &str = "THREE";
    const FOUR: &str = "FOUR";
    const FIVE: &str = "FIVE";
    const SIX: &str = "SIX";
    const SEVEN: &str = "SEVEN";
    const EIGHT: &str = "EIGHT";
    const NINE: &str = "NINE";

    // cargo test --lib -- --nocapture

    #[test]
    fn test_rotate_no_append_numbers() {
        // we use timestamp as discriminant to allow repeated runs
        let ts = now_local_or_utc()
            .format(
                &format_description::parse(
                    "false-numbers-[year]-[month]-[day]_[hour]-[minute]-[second]",
                )
                .unwrap(),
            )
            .unwrap();
        let naming = Naming::Numbers;

        // ensure we start with -/-/-
        assert!(not_exists("00000", &ts));
        assert!(not_exists("00001", &ts));
        assert!(not_exists("CURRENT", &ts));

        // ensure this produces -/-/ONE
        write_loglines(false, naming, &ts, &[ONE]);
        assert!(not_exists("00000", &ts));
        assert!(not_exists("00001", &ts));
        assert!(contains("CURRENT", &ts, ONE));

        // ensure this produces ONE/-/TWO
        write_loglines(false, naming, &ts, &[TWO]);
        assert!(contains("00000", &ts, ONE));
        assert!(not_exists("00001", &ts));
        assert!(contains("CURRENT", &ts, TWO));

        // ensure this also produces ONE/-/TWO
        remove("CURRENT", &ts);
        assert!(not_exists("CURRENT", &ts));
        write_loglines(false, naming, &ts, &[TWO]);
        assert!(contains("00000", &ts, ONE));
        assert!(not_exists("00001", &ts));
        assert!(contains("CURRENT", &ts, TWO));

        // ensure this produces ONE/TWO/THREE
        write_loglines(false, naming, &ts, &[THREE]);
        assert!(contains("00000", &ts, ONE));
        assert!(contains("00001", &ts, TWO));
        assert!(contains("CURRENT", &ts, THREE));
    }

    #[allow(clippy::cognitive_complexity)]
    #[test]
    fn test_rotate_with_append_numbers() {
        // we use timestamp as discriminant to allow repeated runs
        let ts = now_local_or_utc()
            .format(
                &format_description::parse(
                    "true-numbers-[year]-[month]-[day]_[hour]-[minute]-[second]",
                )
                .unwrap(),
            )
            .unwrap();
        let naming = Naming::Numbers;

        // ensure we start with -/-/-
        assert!(not_exists("00000", &ts));
        assert!(not_exists("00001", &ts));
        assert!(not_exists("CURRENT", &ts));

        // ensure this produces 12/-/3
        write_loglines(true, naming, &ts, &[ONE, TWO, THREE]);
        assert!(contains("00000", &ts, ONE));
        assert!(contains("00000", &ts, TWO));
        assert!(not_exists("00001", &ts));
        assert!(contains("CURRENT", &ts, THREE));

        // ensure this produces 12/34/56
        write_loglines(true, naming, &ts, &[FOUR, FIVE, SIX]);
        assert!(contains("00000", &ts, ONE));
        assert!(contains("00000", &ts, TWO));
        assert!(contains("00001", &ts, THREE));
        assert!(contains("00001", &ts, FOUR));
        assert!(contains("CURRENT", &ts, FIVE));
        assert!(contains("CURRENT", &ts, SIX));

        // ensure this also produces 12/34/56
        remove("CURRENT", &ts);
        remove("00001", &ts);
        assert!(not_exists("CURRENT", &ts));
        write_loglines(true, naming, &ts, &[THREE, FOUR, FIVE, SIX]);
        assert!(contains("00000", &ts, ONE));
        assert!(contains("00000", &ts, TWO));
        assert!(contains("00001", &ts, THREE));
        assert!(contains("00001", &ts, FOUR));
        assert!(contains("CURRENT", &ts, FIVE));
        assert!(contains("CURRENT", &ts, SIX));

        // ensure this produces 12/34/56/78/9
        write_loglines(true, naming, &ts, &[SEVEN, EIGHT, NINE]);
        assert!(contains("00002", &ts, FIVE));
        assert!(contains("00002", &ts, SIX));
        assert!(contains("00003", &ts, SEVEN));
        assert!(contains("00003", &ts, EIGHT));
        assert!(contains("CURRENT", &ts, NINE));
    }

    #[test]
    fn test_rotate_no_append_timestamps() {
        // we use timestamp as discriminant to allow repeated runs
        let ts = now_local_or_utc()
            .format(
                &format_description::parse(
                    "false-timestamps-[year]-[month]-[day]_[hour]-[minute]-[second]",
                )
                .unwrap(),
            )
            .unwrap();

        let basename = String::from(DIRECTORY).add("/").add(
            &Path::new(&std::env::args().next().unwrap())
                .file_stem().unwrap(/*cannot fail*/)
                .to_string_lossy().to_string(),
        );
        let naming = Naming::Timestamps;

        // ensure we start with -/-/-
        assert!(list_rotated_files(&basename, &ts).is_empty());
        assert!(not_exists("CURRENT", &ts));

        // ensure this produces -/-/ONE
        write_loglines(false, naming, &ts, &[ONE]);
        assert!(list_rotated_files(&basename, &ts).is_empty());
        assert!(contains("CURRENT", &ts, ONE));

        std::thread::sleep(Duration::from_secs(2));
        // ensure this produces ONE/-/TWO
        write_loglines(false, naming, &ts, &[TWO]);
        assert_eq!(list_rotated_files(&basename, &ts).len(), 1);
        assert!(contains("CURRENT", &ts, TWO));

        std::thread::sleep(Duration::from_secs(2));
        // ensure this produces ONE/TWO/THREE
        write_loglines(false, naming, &ts, &[THREE]);
        assert_eq!(list_rotated_files(&basename, &ts).len(), 2);
        assert!(contains("CURRENT", &ts, THREE));
    }

    #[test]
    fn test_rotate_with_append_timestamps() {
        // we use timestamp as discriminant to allow repeated runs
        let ts = now_local_or_utc()
            .format(
                &format_description::parse(
                    "true-timestamps-[year]-[month]-[day]_[hour]-[minute]-[second]",
                )
                .unwrap(),
            )
            .unwrap();

        let basename = String::from(DIRECTORY).add("/").add(
            &Path::new(&std::env::args().next().unwrap())
                .file_stem().unwrap(/*cannot fail*/)
                .to_string_lossy().to_string(),
        );
        let naming = Naming::Timestamps;

        // ensure we start with -/-/-
        assert!(list_rotated_files(&basename, &ts).is_empty());
        assert!(not_exists("CURRENT", &ts));

        // ensure this produces 12/-/3
        write_loglines(true, naming, &ts, &[ONE, TWO, THREE]);
        assert_eq!(list_rotated_files(&basename, &ts).len(), 1);
        assert!(contains("CURRENT", &ts, THREE));

        // ensure this produces 12/34/56
        write_loglines(true, naming, &ts, &[FOUR, FIVE, SIX]);
        assert!(contains("CURRENT", &ts, FIVE));
        assert!(contains("CURRENT", &ts, SIX));
        assert_eq!(list_rotated_files(&basename, &ts).len(), 2);

        // ensure this produces 12/34/56/78/9
        write_loglines(true, naming, &ts, &[SEVEN, EIGHT, NINE]);
        assert_eq!(list_rotated_files(&basename, &ts).len(), 4);
        assert!(contains("CURRENT", &ts, NINE));
    }

    #[test]
    fn issue_38() {
        const NUMBER_OF_FILES: usize = 5;
        const NUMBER_OF_PSEUDO_PROCESSES: usize = 11;
        const ISSUE_38: &str = "issue_38";
        const LOG_FOLDER: &str = "log_files/issue_38";

        for _ in 0..NUMBER_OF_PSEUDO_PROCESSES {
            let flwb = crate::writers::file_log_writer::FileLogWriter::builder(
                FileSpec::default()
                    .directory(LOG_FOLDER)
                    .discriminant(ISSUE_38),
            )
            .rotate(
                Criterion::Size(500),
                Naming::Timestamps,
                Cleanup::KeepLogFiles(NUMBER_OF_FILES),
            )
            .o_append(false);

            #[cfg(feature = "async")]
            let flwb = flwb.write_mode(WriteMode::AsyncWith {
                bufsize: 5,
                pool_capa: 5,
                message_capa: 400,
                flush_interval: Duration::from_secs(0),
            });

            let flw = flwb.try_build().unwrap();

            // write some lines, but not enough to rotate
            for i in 0..4 {
                flw.write(
                    &mut DeferredNow::new(),
                    &log::Record::builder()
                        .args(format_args!("{}", i))
                        .level(log::Level::Error)
                        .target("myApp")
                        .file(Some("server.rs"))
                        .line(Some(144))
                        .module_path(Some("server"))
                        .build(),
                )
                .unwrap();
            }
            flw.flush().ok();
        }

        // give the cleanup thread a short moment of time
        std::thread::sleep(Duration::from_millis(50));

        let fn_pattern = String::with_capacity(180)
            .add(
                &String::from(LOG_FOLDER).add("/").add(
                    &Path::new(&std::env::args().next().unwrap())
            .file_stem().unwrap(/*cannot fail*/)
            .to_string_lossy().to_string(),
                ),
            )
            .add("_")
            .add(ISSUE_38)
            .add("_r[0-9]*")
            .add(".log");

        assert_eq!(
            glob::glob(&fn_pattern)
                .unwrap()
                .filter_map(Result::ok)
                .count(),
            NUMBER_OF_FILES
        );
    }

    #[test]
    fn test_reset() {
        #[cfg(not(feature = "async"))]
        let write_mode = WriteMode::BufferDontFlushWith(4);
        #[cfg(feature = "async")]
        let write_mode = WriteMode::AsyncWith {
            bufsize: 6,
            pool_capa: 7,
            message_capa: 8,
            flush_interval: Duration::from_secs(0),
        };
        let flw = super::FileLogWriter::builder(
            FileSpec::default()
                .directory(DIRECTORY)
                .discriminant("test_reset-1"),
        )
        .rotate(
            Criterion::Size(28),
            Naming::Numbers,
            Cleanup::KeepLogFiles(20),
        )
        .append()
        .write_mode(write_mode)
        .try_build()
        .unwrap();

        flw.write(
            &mut DeferredNow::new(),
            &log::Record::builder()
                .args(format_args!("{}", "test_reset-1"))
                .level(log::Level::Error)
                .target("test_reset")
                .file(Some("server.rs"))
                .line(Some(144))
                .module_path(Some("server"))
                .build(),
        )
        .unwrap();

        println!("FileLogWriter {:?}", flw);

        flw.reset(
            &super::FileLogWriter::builder(
                FileSpec::default()
                    .directory(DIRECTORY)
                    .discriminant("test_reset-2"),
            )
            .rotate(
                Criterion::Size(28),
                Naming::Numbers,
                Cleanup::KeepLogFiles(20),
            )
            .write_mode(write_mode),
        )
        .unwrap();
        flw.write(
            &mut DeferredNow::new(),
            &log::Record::builder()
                .args(format_args!("{}", "test_reset-2"))
                .level(log::Level::Error)
                .target("test_reset")
                .file(Some("server.rs"))
                .line(Some(144))
                .module_path(Some("server"))
                .build(),
        )
        .unwrap();
        println!("FileLogWriter {:?}", flw);

        assert!(flw
            .reset(
                &super::FileLogWriter::builder(
                    FileSpec::default()
                        .directory(DIRECTORY)
                        .discriminant("test_reset-3"),
                )
                .rotate(
                    Criterion::Size(28),
                    Naming::Numbers,
                    Cleanup::KeepLogFiles(20),
                )
                .write_mode(WriteMode::Direct),
            )
            .is_err());
    }

    fn remove(s: &str, discr: &str) {
        std::fs::remove_file(get_hackyfilepath(s, discr)).unwrap();
    }

    fn not_exists(s: &str, discr: &str) -> bool {
        !get_hackyfilepath(s, discr).exists()
    }

    fn contains(s: &str, discr: &str, text: &str) -> bool {
        match std::fs::read_to_string(get_hackyfilepath(s, discr)) {
            Err(_) => false,
            Ok(s) => s.contains(text),
        }
    }

    fn get_hackyfilepath(infix: &str, discr: &str) -> Box<Path> {
        let arg0 = std::env::args().next().unwrap();
        let mut s_filename = Path::new(&arg0)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        s_filename += "_";
        s_filename += discr;
        s_filename += "_r";
        s_filename += infix;
        s_filename += ".log";
        let mut path_buf = PathBuf::from(DIRECTORY);
        path_buf.push(s_filename);
        path_buf.into_boxed_path()
    }

    fn write_loglines(append: bool, naming: Naming, discr: &str, texts: &[&'static str]) {
        let flw = get_file_log_writer(append, naming, discr);
        for text in texts {
            flw.write(
                &mut DeferredNow::new(),
                &log::Record::builder()
                    .args(format_args!("{}", text))
                    .level(log::Level::Error)
                    .target("myApp")
                    .file(Some("server.rs"))
                    .line(Some(144))
                    .module_path(Some("server"))
                    .build(),
            )
            .unwrap();
        }
    }

    fn get_file_log_writer(
        append: bool,
        naming: Naming,
        discr: &str,
    ) -> crate::writers::FileLogWriter {
        super::FileLogWriter::builder(FileSpec::default().directory(DIRECTORY).discriminant(discr))
            .rotate(
                Criterion::Size(if append { 28 } else { 10 }),
                naming,
                Cleanup::Never,
            )
            .o_append(append)
            .try_build()
            .unwrap()
    }

    fn list_rotated_files(basename: &str, discr: &str) -> Vec<String> {
        let fn_pattern = String::with_capacity(180)
            .add(basename)
            .add("_")
            .add(discr)
            .add("_r2[0-9]*") // Year 3000 problem!!!
            .add(".log");

        glob::glob(&fn_pattern)
            .unwrap()
            .map(|r| r.unwrap().into_os_string().to_string_lossy().to_string())
            .collect()
    }
}
