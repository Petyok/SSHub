#!/bin/sh
# timeout: 2400
# 72b: one terminal, the phone and a Hub. The only terminal (peer-a) is lost; the replacement machine (peer-b) joins
# through the Hub, and `sync-replace-terminal` walks it through the three steps of design 3.3: enroll, promote
# (phone and paper), remove the old one (phone and paper). The paper phrase is typed once per step on stdin. peer-a
# is never contacted again after the setup: the Hub is the one reachable copy of the ring.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72b-replace-hub

phone_prep_page
reset_peers
check "peer-a founds the ring with hub" "enroll_peer peer-a hub || enroll_peer peer-a hub"
enroll_phone peer-a
phone_signs peer-a "$ADD"
show "peer-a registers the paper (phone only)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
phone_signs peer-a "$(opid "$LAST_OUT")"
check "peer-a syncs everything to the Hub" "snow peer-a >/dev/null"
check "peer-b joins via hub" "enroll_peer peer-b hub"
check "peer-a approves peer-b's admission" "snow peer-a >/dev/null; approve_all peer-a"
check "peer-a syncs the admission to the Hub" "snow peer-a >/dev/null"
snow peer-b >/dev/null; snow peer-a >/dev/null; snow peer-b >/dev/null
show "peer-b's view of the paper and the phone (FactorRegister, RedPhoneSign rows)" sq peer-b "SELECT kind||'|'||state||'|'||coalesce(reason,'') FROM sync_ops WHERE kind IN ('FactorRegister','RedPhoneSign','DeviceAdd')"
A_ID=$(devid peer-b peer-a)
check "peer-b sees peer-a through the Hub" "test -n '$A_ID'"
st=$(peer peer-b $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-b is a view-only copy so far" "$st" '[.devices[] | select(.name=="peer-b") | .role] == ["mirror"]'

show "peer-b sync-replace-terminal peer-a (step 1: no new computer named)" peer peer-b $SSHUB sync-replace-terminal "$A_ID" --format json
expect "the first step is the enrollment and lists peer-b" "$LAST_OUT" '.step == "enroll" and any(.candidates[]; . == "peer-b")'

show "peer-b sync-replace-terminal peer-a --new peer-b (step 2: promote)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-b $SSHUB sync-replace-terminal '$A_ID' --new peer-b --format json"
expect "the second step proposes the promotion" "$LAST_OUT" '.step == "promote" and (.op | length) > 0'
PRO=$(opid "$LAST_OUT")
phone_signs peer-b "$PRO"
show "peer-b adds the paper to the promotion again (evidence: what the ceremony has and what is missing)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-b $SSHUB sync-factor $PRO --paper --format json"
snow peer-b >/dev/null
st=$(peer peer-b $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-b is a full device now" "$st" '[.devices[] | select(.name=="peer-b") | .role] == ["holder"]'

show "peer-b sync-replace-terminal peer-a --new peer-b (step 3: remove the old one)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-b $SSHUB sync-replace-terminal '$A_ID' --new peer-b --format json"
expect "the third step proposes the removal" "$LAST_OUT" '.step == "revoke" and (.op | length) > 0'
REV=$(opid "$LAST_OUT")
phone_signs peer-b "$REV"
snow peer-b >/dev/null
st=$(peer peer-b $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-a is removed" "$st" '[.devices[] | select(.name=="peer-a") | .status] == ["revoked"]'
expect "peer-b and the phone are the full devices" "$st" '[.devices[] | select(.role=="holder" and .status != "revoked") | .name] | sort == ["my phone","peer-b"]'
ring_view
finish "$NAME"
