//! Core library of Hebra, the version control system built for humans and
//! agents working on the same code at the same time.

pub mod archive;
pub mod attributes;
pub mod blame;
pub mod claims;
mod delta;
pub mod diff;
pub mod error;
pub mod gitexport;
pub mod gitimport;
pub mod history;
pub mod identity;
pub mod ids;
pub mod merge;
pub mod model;
pub mod oplog;
mod pack;
pub mod rename;
pub mod repo;
pub mod store;
pub mod structural;
pub mod transparency;
pub mod transport;
pub mod wc;

pub use error::{Error, Result};
pub use ids::{ChangeId, ObjectId};
pub use repo::{FsckReport, Node, Tx, Workspace};
