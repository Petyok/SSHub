# Multi-group membership + reserved Favorites group

Repo: `/home/petruha/sshub-dev/ssh-tui` (branch `feature/os-auto-detect`). Rust
TUI (ratatui 0.30). Do NOT run `cargo build`/`just test` (exceeds agent timeout)
— the orchestrator builds. Write code only, matching surrounding style.

## Goal & locked decisions
- Hosts can belong to **several groups at once** (many-to-many).
- **Favorites** is a real, reserved, auto-created group. A host's favourite
  status == membership in Favorites. Toggling favourite (`f`) adds/removes that
  membership.
- **Identity is materialized, never dynamically multi-inherited.** Single-group
  hosts still inherit their group's `default_identity_id` at resolve time (as
  today). When a host gains a SECOND group and its `identity_id` is still NULL,
  copy the *currently effective* identity (its first/primary group's default)
  into the host's own `identity_id` so it stays stable. After that it's the
  host's own until the user changes it.
- Group selection UI: the existing **host form's `Group` row** becomes a
  multi-select. Pressing Enter on it opens a checkbox list of all groups
  (Space toggles, Enter/Esc apply/close) — NOT a new global popup.

## Already done (committed, green) — do not redo
Schema v11 (`src/store/migrate.rs`): join table
`host_group_memberships(host_id, group_id, PK(host_id,group_id))`,
`host_groups.reserved` column, reserved `Favorites` group auto-created
(`FAVORITES_GROUP_NAME` const, `sort_order -1000`), backfill of memberships from
legacy `hosts.group_id` and `hosts.favorite`. The `★` mark in the host list is
also done.

## Low-churn strategy (IMPORTANT)
Keep `ManagedHost.group_id` / `.group` / `.favorite` fields AS-IS (primary group
for identity + back-compat with all existing readers). Only ADD:
- `ManagedHost.groups: Vec<HostGroup>` = ALL memberships (incl. primary +
  Favorites), populated at load from the join table.
- `HostGroup.reserved: bool`.
And SET `ManagedHost.favorite` at load from "is Favorites among `groups`", so the
existing `favorite()` accessor keeps working unchanged.
`hosts.group_id` stays the "primary group" = first non-Favorites membership.

## Layer 1 — store + data model (this task)
Files: `src/store/types.rs`, `src/store/hosts.rs`, `src/store/groups.rs` (if
delete/rename live there; else in hosts.rs), `src/store/mod.rs` (re-exports).

1. `types.rs`: `HostGroup` += `pub reserved: bool`. `ManagedHost` += `pub groups:
   Vec<HostGroup>`.
2. `hosts.rs`:
   - `row_to_group` + the `get_group`/`list_groups` SELECTs: add `reserved`
     column (index-shift; read `reserved` as `i64 != 0`). `create_group` literal:
     `reserved: false`.
   - `row_to_host` (ManagedHost literal ~:713): add `groups: Vec::new()`.
   - In `list_hosts` (and any other ManagedHost list loader / `get_host`): after
     building the hosts, run ONE query over `host_group_memberships` JOIN
     `host_groups` to build `HashMap<i64 /*host_id*/, Vec<HostGroup>>` and assign
     `.groups`; then set `.favorite = groups.any(|g| g.reserved)`.
   - New pub methods on `LauncherStore`:
     - `favorites_group_id(&self) -> Result<i64>` — id of the `reserved=1`
       group named `FAVORITES_GROUP_NAME`.
     - `set_host_groups(&self, host_id: i64, group_ids: &[i64]) -> Result<()>` —
       replace all memberships for the host (delete then insert), and set
       `hosts.group_id` = first non-Favorites id (or NULL).
     - `add_host_to_group(&self, host_id, group_id)` / `remove_host_from_group`
       (`INSERT OR IGNORE` / `DELETE`). Used by the favourite toggle.
   - `create_host`: after insert, if `NewHost.group_id` is set, also insert a
     membership row so single-group creation is consistent.
   - `update_host`: when `HostUpdate.group_id` changes, keep the membership set
     in sync for the single-group edit path (add the new primary; the multi-
     select save uses `set_host_groups` directly).
3. Reserved-group protection: `delete_group` and `update_group` (rename) must
   refuse the reserved group — return the existing NotFound/Err path or a no-op;
   find how deletion currently signals failure and mirror it. Deleting a
   non-reserved group must also clear its membership rows (the FK `ON DELETE
   CASCADE` handles it, but confirm PRAGMA foreign_keys is ON — it is, see
   migrate.rs:71).
4. Unit tests in `store` (offline, `open_in_memory`): a host added to two groups
   loads with `groups.len()==2`; favourite membership sets `.favorite`;
   `set_host_groups` replaces; reserved group can't be deleted/renamed.

## Layer 2 — app grouping + favourite semantics (this task, same agent)
Files: `src/app/types.rs`, `src/app/util.rs`, `src/app/mod.rs`.
1. `HostEntry`: add `pub fn group_ids(&self) -> Vec<i64>` — Managed → its
   `groups` ids; Legacy → empty. Keep existing `group_id()` (primary).
   `favorite()` stays as-is (ManagedHost.favorite is set correctly at load).
2. `util.rs build_group_sections` / `build_group_subtree`: a host belongs to a
   section when its `group_ids()` CONTAINS that group's id (not `group_id() ==`).
   A host with empty `group_ids()` goes to the ungrouped bucket. Hosts now appear
   under EVERY group they belong to (including Favorites, which sorts first via
   `sort_order -1000`).
3. `reload_hosts`: no structural change needed (list_groups already returns
   Favorites); just confirm groups feed build_group_sections.

## Layer 2b — favourite toggle + identity materialization (this task, same agent)
To keep favourite semantics coherent in one shot:
- `favorite()` derives from Favorites membership: set `ManagedHost.favorite` at
  load from "Favorites among `groups`" (Layer 1 already). The legacy
  `hosts.favorite` column is no longer authoritative — you may keep writing it in
  sync for back-compat but reads come from membership.
- Rewire the `f` favourite toggle (find it in `src/app/` — likely
  `host_crud.rs`/`keys.rs`/`hostlist.rs`, search `favorite`): instead of
  flipping the column, call `add_host_to_group(host_id, favorites_id)` /
  `remove_host_from_group(...)`, then `reload_hosts()`. Legacy (non-managed)
  hosts: keep the old metadata favourite path (they have no host_id).
- **Identity materialization** (store-level invariant, put it inside
  `set_host_groups` and `add_host_to_group`): after changing memberships, if the
  host now belongs to >1 group AND `hosts.identity_id IS NULL`, set
  `hosts.identity_id` = the primary (first non-Favorites) group's
  `default_identity_id` when that group has one. This concretises the inherited
  key so it stays stable. No-op if it was already set or no default exists.

Verification (orchestrator runs): `cargo build`, `just test` green;
`cargo clippy --lib` no new warnings. Manually: a favourited host shows under
both Favorites and its real group; the list `★` still marks it; `f` toggles
membership and survives reload.

## Deferred to a later agent (NOT this task)
Only the host-form multi-select Group field (checkbox list on the form's Group
row) + help text. This task must leave the tree COMPILING and green; the form's
existing single-group select keeps working until then (it sets one primary group
via the existing update path, and `create_host`/`update_host` keep memberships in
sync).
