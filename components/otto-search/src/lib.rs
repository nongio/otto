//! Otto's file search: one query language over the desktop's file index.
//!
//! Files, the launcher, the command line and the agents all search through
//! this crate, so the same words find the same files everywhere.
//!
//! ```no_run
//! use otto_search::{index, parse, Clock, Plan, Scope};
//!
//! let query = parse("invoice kind:pdf modified:<30d");
//! let scope = Scope { roots: vec!["/home/u".into()], ..Scope::default() };
//! if let Some(plan) = Plan::new(&query, &scope, Clock::now()) {
//!     for hit in index::search(&plan.sparql(100))? {
//!         println!("{}", hit.path.display());
//!     }
//! }
//! # Ok::<(), otto_search::Unavailable>(())
//! ```
//!
//! - [`query`] turns text into terms, and never fails.
//! - [`Plan`] resolves a query against a place and a time, and renders it as
//!   SPARQL or checks a file against it.
//! - [`index`] asks LocalSearch over D-Bus.
//! - [`find`](mod@find) runs a plan to a ranked list of files that are
//!   really on disk, paging past the ones the index remembers wrongly.
//! - [`dates`] is the calendar arithmetic underneath, shared with anything
//!   else that shows a date without a date crate.
//! - [`matching`] scores names, and is shared with anything else that ranks
//!   typed text against names.

// Rust guideline compliant 2026-02-21

pub mod dates;
pub mod find;
pub mod index;
pub mod matching;
mod plan;
pub mod query;

pub use find::{find, Found, Progress, Results};
pub use index::{Hit, Snippet, State, Status, Unavailable};
pub use plan::{Clock, Facts, Plan, Scope, Sparql};
pub use query::{parse, Kind, Query, Sort};
