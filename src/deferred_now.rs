use time::{Date, formatting::Formattable, OffsetDateTime, UtcOffset};

/// Deferred timestamp creation.
///
/// Is used to ensure that a log record that is sent to multiple outputs
/// (in maybe different formats) always uses the same timestamp.
#[derive(Debug)]
pub struct DeferredNow(Option<OffsetDateTime>);

impl Default for DeferredNow {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> DeferredNow {
    /// Constructs a new instance, but does not generate the timestamp.
    #[must_use]
    pub fn new() -> Self {
        Self(None)
    }

    /// Retrieve the timestamp.
    ///
    /// Requires mutability because the first caller will generate the timestamp.
    #[allow(clippy::missing_panics_doc)]
    pub fn now(&'a mut self) -> &'a OffsetDateTime {
        self.0.get_or_insert_with(now_local_or_utc)
    }

    /// Convert into a formatted String.
    ///
    /// # Panics
    ///
    /// if fmt has an inappropriate value
    pub fn format(&'a mut self, fmt: &(impl Formattable + ?Sized)) -> String {
        self.now().format(fmt).unwrap()
    }
}

pub(crate) fn now_local_or_utc() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}


/// a number of: year * 10000 + month * 100 + day
pub(crate) fn offset_date_time_to_year_month_day_number(offset_date_time: Date) -> i32 {
    let (year, month, day) = (offset_date_time.year(), offset_date_time.month() as i32, offset_date_time.day() as i32);
    year * 10000 + month * 100 + day
}

/// a number of: year * 10000 + month * 100 + day
pub(crate) fn now_as_year_month_day_number(utc_offset: UtcOffset) -> i32 {
    let now = now_local_or_utc().to_offset(utc_offset).date();
    offset_date_time_to_year_month_day_number(now)
}
