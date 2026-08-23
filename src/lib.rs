//! askable - a regression judge for ranked search.
//!
//! Nothing here opens a socket or a terminal. The binary supplies both, so the
//! logic can be tested without either.

pub mod candidate;
pub mod corpus;
pub mod log;
pub mod record;
pub mod replay;

// The log reader was the whole crate once. Keep its names at the root so the
// callers that learned them still compile.
pub use log::{Event, events, message_of, parse, time_of};
