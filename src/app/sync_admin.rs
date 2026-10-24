//! The Sync tab's admin menu: one entry per operation that changes the ring.
//! Each flow lives in its own module and is listed here.

use super::sync_ui::MenuFn;
use super::*;

/// The menu entries, in order.
pub(crate) fn entries() -> Vec<(String, MenuFn)> {
    vec![("Close".to_string(), Box::new(|_: &mut App| {}) as MenuFn)]
}
