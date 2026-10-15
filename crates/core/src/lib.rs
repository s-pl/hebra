//! Core library of Hebra, the version control system built for humans and
//! agents working on the same code at the same time.

pub mod claims;
pub mod diff;
pub mod error;
pub mod gitexport;
pub mod gitimport;
pub mod identity;
pub mod ids;
pub mod merge;
pub mod model;
pub mod oplog;
pub mod repo;
pub mod store;
pub mod transparency;
pub mod transport;
pub mod wc;

pub use error::{Error, Result};
pub use ids::{ChangeId, ObjectId};
pub use repo::{Tx, Workspace};
