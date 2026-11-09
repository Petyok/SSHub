#!/bin/sh
# timeout: 2400
# 72a: two laptops and the phone, Quorum. peer-a removes peer-b: only one other terminal remains, so the removal is
# degraded (design 3.2): phone and paper approve (the second device is waived), the credentials peer-b held a piece of
# are marked exposed (sync-status text and JSON), and a credential saved afterwards is not wrapped to peer-b.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72a-degraded-revoke

step "Quorum ring with the phone (ring2)" quorum_ring ring2
B_ID=$(devid peer-a peer-b)
check "peer-a can name peer-b" "test -n '$B_ID'"
check "quorum1 is wrapped to peer-b before the removal" "[ \"$(sq peer-a "SELECT count(*) FROM sync_cred_state WHERE wrapped_shares LIKE '%$B_ID%'")\" -ge 1 ]"

show "peer-a sync-revoke peer-b (degraded: one other terminal)" peer peer-a $SSHUB sync-revoke "$B_ID" --format json
check "the removal is proposed, not refused" "[ '$LAST_RC' -eq 0 ]"
expect "it is a Red change with a ceremony" "$LAST_OUT" '.tier == "Red" and (.ceremony != null)'
REV=$(opid "$LAST_OUT")
step "removal ceremony (phone and paper, no second device)" red_phone_paper "$REV"
expect "the removal ceremony is complete" "$LAST_OUT" '.ceremony.complete == true'
spread peer-a peer-a
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-b is removed on peer-a" "$st" '[.devices[] | select(.name=="peer-b") | .status] == ["revoked"]'
expect "the marker lists quorum1 as sharing a key with the removed device" "$st" '[.exposure[] | select(test("removed device"))] | length >= 1'
expect "the marker names the host" "$st" 'any(.exposure[]; test("quorum1"))'
show "peer-a sync-status (text)" peer peer-a $SSHUB sync-status
check "the text status carries the marker line" "printf '%s' \"\$LAST_OUT\" | grep -q 'share a key with a removed device'"

check "peer-a saves a new credential after the removal" "printf '%s\n' 'N3w-Secret-e2e' | peer peer-a $SSHUB host add --name quorum2 --address quorum2.internal --password-stdin"
check "two credentials are stored" "[ \"$(sq peer-a "SELECT count(*) FROM sync_cred_state")\" -ge 2 ]"
check "only the old credential is wrapped to the removed peer-b" "[ \"$(sq peer-a "SELECT count(*) FROM sync_cred_state WHERE wrapped_shares LIKE '%$B_ID%'")\" = 1 ]"
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
check "the new credential is not on the exposed list" "! printf '%s' '$st' | grep -q quorum2"
ring_view
finish "$NAME"
