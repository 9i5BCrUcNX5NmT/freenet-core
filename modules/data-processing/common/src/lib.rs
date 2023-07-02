pub mod dispatcher;
pub mod segment;
pub mod utils;

pub use crate::segment::Segment;

pub mod constants {
    const MAXIMUM_JOB_ASSIGNMENT_TOKEN_AGE : std::time::Duration = std::time::Duration::from_secs(60 * 60 * 24 * 7);

    const MAX_JOB_OUTPUT_RECORDS : usize = 5;
}