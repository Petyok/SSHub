#!/bin/sh
# timeout: 1500
# 71: the paper phrase is mandatory (design 3.1) for a switch to Quorum and for the third full device, and removing a
# computer names it. Part 1, two computers, no phone: peer-b becomes a full device (two full devices need no paper),
# then a Quorum switch and the removal of peer-b are refused with the paper sentence. Part 2, a ring with the phone and
# no paper: the third full device and the Quorum switch are refused; the paper is registered (the CLI form of the
# wizard: the phrase on stdin, the first registration needs the phone only); the same verbs then go through.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=71-paper-gate
GATE='register the paper phrase first'

echo "=== part 1: two computers, no phone"
ring2
check "peer-a sets up the Vault" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-a $SSHUB sync-import-kit"
show "peer-a sync-mode quorum without a paper" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-mode quorum --format json"
check "the Quorum switch is refused" "[ '$LAST_RC' -ne 0 ]"
check "the refusal names the paper phrase" "printf '%s' '$LAST_OUT' | grep -q '$GATE'"

show "peer-a sync-role peer-b holder (the second full device, no paper needed)" peer peer-a $SSHUB sync-role "$(devid peer-a peer-b)" holder --format json
ROLEOP=$(opid "$LAST_OUT")
if [ -n "$ROLEOP" ]; then
  spread peer-b peer-a peer-b
  show "peer-b sync-approve the promotion" peer peer-b $SSHUB sync-approve "$ROLEOP" --format json
  spread peer-a peer-b peer-a
fi
echo "holders as peer-a sees them: $(holders peer-a)"
check "two full devices now" "[ \"$(holders peer-a)\" -ge 2 ]"
show "peer-a sync-revoke peer-b without a paper" peer peer-a $SSHUB sync-revoke "$(devid peer-a peer-b)" --format json
check "the removal is refused" "[ '$LAST_RC' -ne 0 ]"
check "the refusal names the paper phrase" "printf '%s' '$LAST_OUT' | grep -qi 'paper phrase'"

echo "=== part 2: the phone, no paper"
phone_prep_page
ring2
enroll_phone peer-a
phone_signs peer-a "$ADD"
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "the phone is a full device on peer-a" "$st" '[.devices[] | select(.name=="my phone") | .role] == ["holder"]'
spread peer-b peer-a peer-b
show "peer-a sync-role peer-b holder (the third full device)" peer peer-a $SSHUB sync-role "$(devid peer-a peer-b)" holder --format json
check "the third full device is refused" "[ '$LAST_RC' -ne 0 ]"
check "the refusal names the paper phrase" "printf '%s' '$LAST_OUT' | grep -q '$GATE'"
check "peer-b is still not a full device" "[ \"$(peer peer-a $SSHUB sync-status --format json | pjq -r '[.devices[] | select(.name==\"peer-b\") | .role][0]')\" = mirror ]"

show "peer-a registers the paper (the first registration needs the phone only)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
expect "registering the paper is a Red change with a ceremony" "$LAST_OUT" '.tier == "Red" and (.ceremony != null)'
REG=$(opid "$LAST_OUT")
phone_signs peer-a "$REG"

show "peer-a sync-role peer-b holder, with the paper in force" peer peer-a $SSHUB sync-role "$(devid peer-a peer-b)" holder --format json
check "the promotion is proposed now" "[ '$LAST_RC' -eq 0 ]"
expect "it is a Red change with a ceremony" "$LAST_OUT" '.tier == "Red" and (.ceremony != null)'
ROLEOP=$(opid "$LAST_OUT")
step "promotion ceremony (phone, peer-b, paper)" red_complete "$ROLEOP"
expect "the promotion ceremony is complete" "$LAST_OUT" '.ceremony.complete == true'

check "peer-a sets up the Vault" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-a $SSHUB sync-import-kit"
show "peer-a sync-mode quorum with the paper in force" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-mode quorum --format json"
check "the Quorum switch is not refused for the paper any more" "! printf '%s' '$LAST_OUT' | grep -q '$GATE'"

finish "$NAME"
