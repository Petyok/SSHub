//! OS auto-detection: probe a host's `/etc/os-release`, map it to a canonical
//! id, look up a vendored ANSI logo, and render it in the TUI.
//!
//! Flow: [`detect`] spawns a background worker that shells out over SSH and
//! feeds raw output to [`parse::parse_os`]; the resulting canonical id is
//! stored in `hosts.os_icon` and rendered via [`logos::logo_for`] +
//! [`widget::OsLogoWidget`].

pub mod detect;
pub mod logos;
pub mod parse;
pub mod widget;

/// Canonical OS id — one of the interned literals produced by [`parse_os`]
/// (e.g. `"arch"`, `"ubuntu"`, `"macos"`). Feeds directly into [`logo_for`]
/// and is the value stored in `hosts.os_icon`.
pub type CanonicalOs = &'static str;

pub use detect::{spawn_os_detect_worker, OsDetectCmd, OsDetectEvent, ProbeRunner, SshProbeRunner};
pub use logos::{large_logo_for, logo_for, OsLogo, OsLogoLine, OsLogoSpan};
pub use parse::parse_os;
pub use widget::OsLogoWidget;
