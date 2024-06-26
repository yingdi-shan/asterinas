// SPDX-License-Identifier: MPL-2.0

pub mod c_types;
pub mod capabilities;
mod credentials_;
mod group;
mod static_cap;
mod user;

use aster_rights::{FullOp, ReadOp, WriteOp};
use credentials_::Credentials_;
pub use group::Gid;
pub use user::Uid;

use super::posix_thread::PosixThreadExt;
use crate::prelude::*;

/// `Credentials` represents a set of associated numeric user ids (UIDs) and group identifiers (GIDs)
/// for a process.
/// These identifiers are as follows:
/// - real user ID and group ID;
/// - effective user ID and group ID;
/// - saved-set user ID and saved-set group ID;
/// - file system user ID and group ID (Linux-specific);
/// - supplementary group IDs;
/// - Linux capabilities.
pub struct Credentials<R = FullOp>(Arc<Credentials_>, R);

/// Gets read-only credentials of current thread.
///
/// # Panics
///
/// This method should only be called in process context.
pub fn credentials() -> Credentials<ReadOp> {
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    posix_thread.credentials()
}

/// Gets write-only credentials of current thread.
///
/// # Panics
///
/// This method should only be called in process context.
pub fn credentials_mut() -> Credentials<WriteOp> {
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    posix_thread.credentials_mut()
}
