//! `sshub identity …` — SSH identity CRUD and agent key removal.

use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use crate::credentials::identity_key;
use crate::store::{DeleteIdentityOutcome, Identity, IdentityUpdate, NewIdentity};

use super::context::{optional_path_flag, read_password_stdin, CliContext};
use super::parse::{
    fail, fail_code, parse_format, take_flag, take_opt, usage, OutputFormat, CONFIRM_YES,
};

#[derive(Serialize)]
struct IdentityRecord<'a> {
    id: i64,
    name: &'a str,
    username: Option<&'a str>,
    private_key: Option<String>,
    certificate: Option<String>,
    has_password: bool,
}

pub fn run(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    match rest.first().map(String::as_str) {
        Some("list") => {
            rest.remove(0);
            cmd_list(ctx, &rest)
        }
        Some("show") => {
            rest.remove(0);
            cmd_show(ctx, &rest)
        }
        Some("add") => {
            rest.remove(0);
            cmd_add(ctx, &rest)
        }
        Some("edit") => {
            rest.remove(0);
            cmd_edit(ctx, &rest)
        }
        Some("delete") => {
            rest.remove(0);
            cmd_delete(ctx, &rest)
        }
        Some("agent-remove") => {
            rest.remove(0);
            cmd_agent_remove(ctx, &rest)
        }
        Some(other) => {
            eprintln!("sshub: unknown identity subcommand '{other}'");
            eprintln!("       try: sshub identity list|show|add|edit|delete|agent-remove");
            Ok(2)
        }
        None => {
            usage("identity needs a subcommand (list|show|add|edit|delete|agent-remove)");
        }
    }
}

fn cmd_list(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let fmt = parse_format(args).map_err(anyhow::Error::msg)?;
    let identities = ctx.store.list_identities()?;

    match fmt {
        OutputFormat::Plain => {
            for i in &identities {
                println!("{:<6} {}", i.id, i.name);
            }
        }
        OutputFormat::Json => {
            let records: Vec<IdentityRecord<'_>> = identities.iter().map(identity_record).collect();
            println!("{}", serde_json::to_string(&records)?);
        }
    }
    Ok(0)
}

fn cmd_show(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let fmt = parse_format(args).map_err(anyhow::Error::msg)?;
    let name = identity_name_arg(args);
    let identity = ctx.identity_by_name(&name)?;

    match fmt {
        OutputFormat::Plain => print_identity_plain(&identity)?,
        OutputFormat::Json => {
            let record = identity_record(&identity);
            println!("{}", serde_json::to_string(&record)?);
        }
    }
    Ok(0)
}

fn cmd_add(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let name =
        take_opt(&mut rest, "--name").unwrap_or_else(|| usage("identity add requires --name"));
    if name.trim().is_empty() {
        fail("identity name cannot be empty");
    }

    let username = take_opt(&mut rest, "--username");
    let private_key = match take_opt(&mut rest, "--private-key") {
        Some(p) => optional_path_flag(&p)?,
        None => None,
    };
    let certificate = match take_opt(&mut rest, "--certificate") {
        Some(p) => optional_path_flag(&p)?,
        None => None,
    };
    let password_stdin = take_flag(&mut rest, "--password-stdin");
    // Read the secret before creating the row so has_password reflects reality:
    // an empty piped password must not leave has_password=true with no secret.
    let password = if password_stdin {
        let pw = read_password_stdin()?;
        (!pw.is_empty()).then_some(pw)
    } else {
        None
    };
    let sort_order = ctx.store.list_identities()?.len() as i32;

    let created = ctx.store.create_identity(&NewIdentity {
        name: name.trim().to_string(),
        username,
        private_key,
        certificate,
        sort_order,
        has_password: password.is_some(),
    })?;

    if let Some(pw) = password {
        ctx.password_store.set(&identity_key(created.id), &pw)?;
    }

    println!("created identity '{}'", created.name);
    Ok(0)
}

fn cmd_edit(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let name =
        take_opt(&mut rest, "--name").unwrap_or_else(|| usage("identity edit requires --name"));
    let identity = ctx.identity_by_name(&name)?;

    let set_name = take_opt(&mut rest, "--set-name");
    if let Some(n) = &set_name {
        if n.trim().is_empty() {
            fail("identity name cannot be empty");
        }
    }
    let set_username = if take_flag(&mut rest, "--clear-username") {
        Some(None)
    } else {
        take_opt(&mut rest, "--set-username").map(Some)
    };
    let set_private_key: Option<Option<PathBuf>> = if take_flag(&mut rest, "--clear-private-key") {
        Some(None)
    } else {
        match take_opt(&mut rest, "--set-private-key") {
            Some(p) => Some(optional_path_flag(&p)?),
            None => None,
        }
    };
    let set_certificate: Option<Option<PathBuf>> = if take_flag(&mut rest, "--clear-certificate") {
        Some(None)
    } else {
        match take_opt(&mut rest, "--set-certificate") {
            Some(p) => Some(optional_path_flag(&p)?),
            None => None,
        }
    };
    let password_stdin = take_flag(&mut rest, "--password-stdin");
    let clear_password = take_flag(&mut rest, "--clear-password");

    let mut has_password = identity.has_password;
    if clear_password {
        ctx.password_store.delete(&identity_key(identity.id))?;
        has_password = false;
    }
    if password_stdin {
        let pw = read_password_stdin()?;
        if pw.is_empty() {
            ctx.password_store.delete(&identity_key(identity.id))?;
            has_password = false;
        } else {
            ctx.password_store.set(&identity_key(identity.id), &pw)?;
            has_password = true;
        }
    }

    let updated = ctx.store.update_identity(
        identity.id,
        &IdentityUpdate {
            name: set_name,
            username: set_username,
            private_key: set_private_key,
            certificate: set_certificate,
            has_password: if password_stdin || clear_password {
                Some(has_password)
            } else {
                None
            },
            ..Default::default()
        },
    )?;
    if updated.is_none() {
        fail(&format!("identity '{name}' not found"));
    }
    println!("updated identity '{name}'");
    Ok(0)
}

fn cmd_delete(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let name =
        take_opt(&mut rest, "--name").unwrap_or_else(|| usage("identity delete requires --name"));
    if !take_flag(&mut rest, CONFIRM_YES) {
        fail(&format!(
            "refusing to delete identity '{name}' without {CONFIRM_YES}"
        ));
    }

    let identity = ctx.identity_by_name(&name)?;
    match ctx.store.delete_identity(identity.id)? {
        DeleteIdentityOutcome::Deleted => {
            let _ = ctx.password_store.delete(&identity_key(identity.id));
            println!("deleted identity '{name}'");
            Ok(0)
        }
        DeleteIdentityOutcome::NotFound => fail(&format!("identity '{name}' not found")),
        DeleteIdentityOutcome::InUse { host_count } => fail_code(
            &format!("identity '{name}' is used by {host_count} host(s)"),
            2,
        ),
    }
}

fn cmd_agent_remove(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let name = take_opt(&mut rest, "--name")
        .unwrap_or_else(|| usage("identity agent-remove requires --name"));

    let identity = match ctx.store.get_identity_by_name(&name)? {
        Some(i) => i,
        None => fail(&format!("identity '{name}' not found")),
    };

    let Some(ref key_path) = identity.private_key else {
        fail(&format!("identity '{name}' has no private key path"));
    };

    let expanded = crate::ssh::expand_tilde(&key_path.to_string_lossy());
    let key_display = expanded.to_string_lossy();

    match crate::ssh::agent::remove_key(&key_display) {
        Ok(()) => {
            let _ = ctx.store.log_auth_event(
                &name,
                None,
                "agent",
                "ok",
                &format!("key removed from agent: {key_display}"),
                None,
            );
            println!("removed '{}' from agent", name);
            Ok(0)
        }
        Err(e) => {
            let _ = ctx.store.log_auth_event(
                &name,
                None,
                "agent",
                "fail",
                &format!("remove from agent failed: {e:#}"),
                None,
            );
            fail(&format!("ssh-add -d failed: {e:#}"));
        }
    }
}

fn identity_name_arg(args: &[String]) -> String {
    let mut rest = args.to_vec();
    if let Some(n) = take_opt(&mut rest, "--name") {
        return n;
    }
    match super::parse::positional(args).first() {
        Some(n) => (*n).to_string(),
        None => usage("identity show requires an identity name"),
    }
}

fn identity_record(i: &Identity) -> IdentityRecord<'_> {
    IdentityRecord {
        id: i.id,
        name: &i.name,
        username: i.username.as_deref(),
        private_key: i
            .private_key
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        certificate: i
            .certificate
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        has_password: i.has_password,
    }
}

fn print_identity_plain(identity: &Identity) -> Result<()> {
    println!("id:           {}", identity.id);
    println!("name:         {}", identity.name);
    if let Some(u) = &identity.username {
        println!("username:     {u}");
    }
    if let Some(k) = &identity.private_key {
        println!("private_key:  {}", k.display());
    }
    if let Some(c) = &identity.certificate {
        println!("certificate:  {}", c.display());
    }
    println!("has_password: {}", identity.has_password);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_record_serializes() {
        let i = Identity {
            id: 1,
            name: "work".into(),
            username: Some("alice".into()),
            private_key: Some(PathBuf::from("/home/alice/.ssh/id_ed25519")),
            certificate: None,
            has_password: true,
        };
        let json = serde_json::to_string(&identity_record(&i)).unwrap();
        assert!(json.contains("\"name\":\"work\""));
    }
}
