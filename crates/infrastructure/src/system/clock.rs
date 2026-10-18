//! Wall clock behind the `Clock` port.

use chrono::{DateTime, Utc};

use application::ports::Clock;

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
