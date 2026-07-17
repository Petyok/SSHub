//! `sshub audit …` — placeholder. Phase 1 compile-only stub.

use anyhow::Result;

use super::CliContext;

pub fn run(_ctx: &mut CliContext, _args: &[String]) -> Result<i32> {
    eprintln!("sshub: audit is not yet implemented");
    Ok(2)
}
