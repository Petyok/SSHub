#!/bin/sh
# timeout: 2700
# 73: one-terminal recovery of a lost phone (design 5.2). The ring is peer-a (the only attended terminal), the Hub (an
# unattended mirror and the sender of the emailed code) and the phone. peer-a starts the recovery with the paper phrase
# and proves the third factor; nobody else can acknowledge, so the requester's own ack starts the clock. The 14 day lock
# is shortened to SOLO seconds by the e2e image (--features e2e-hooks, SSHUB_E2E_SOLO_LOCK_SECS, set on every call below).
# The live phone cancels a second recovery during its lock; the first one, left alone, becomes ready after the lock.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=73-solo-recovery
SOLO=60
rec() { peer peer-a env SSHUB_E2E_SOLO_LOCK_SECS=$SOLO $SSHUB sync-recovery "$@"; }
verdict_of() { rec status --format json 2>/dev/null | pjq -r ".recoveries[] | select(.op | startswith(\"$1\")) | .verdict"; }

phone_prep_page
reset_peers
check "peer-a founds the ring with hub" "enroll_peer peer-a hub || enroll_peer peer-a hub"
enroll_phone peer-a
phone_signs peer-a "$ADD"
show "peer-a registers the paper (phone only)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
phone_signs peer-a "$(opid "$LAST_OUT")"
snow peer-a >/dev/null
show "hub registers the emailed-code factor (it is the sender)" peer hub $SSHUB sync-factor-register email --format json
CODE=$(pf "$LAST_OUT" code)
check "the Hub printed its code" "test -n '$CODE'"
REGE=$(opid "$LAST_OUT")
echo "--- the email factor registration needs the phone's signature"
snow peer-a >/dev/null
phone_signs peer-a "$REGE"
snow hub >/dev/null; snow peer-a >/dev/null
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-a is the only full terminal (the Hub is a view-only copy)" "$st" '[.devices[] | select(.role=="holder" and .name != "my phone") | .name] == ["peer-a"]'

# ---- recovery 1: left alone, becomes ready after the lock
show "peer-a sync-recovery start --paper (paper, then the emailed code)" sh -c "printf '%s\n%s\n' '$PAPER' '$CODE' | $C exec -T peer-a env SSHUB_E2E_SOLO_LOCK_SECS=$SOLO $SSHUB sync-recovery start --paper --format json"
check "the recovery is started" "[ '$LAST_RC' -eq 0 ]"
R1=$(opid "$LAST_OUT")
V=$(verdict_of "$(printf '%s' "$R1" | cut -c1-8)"); echo "verdict before any ack: $V"
check "it waits for an acknowledgement (nobody else can give one)" "printf '%s' '$V' | grep -q AwaitingAck"
show "peer-a sync-recovery ack (the requester's own ack, solo)" rec ack "$R1" --format json
check "the requester can acknowledge its own recovery" "[ '$LAST_RC' -eq 0 ]"
V=$(verdict_of "$(printf '%s' "$R1" | cut -c1-8)"); echo "verdict right after the ack: $V"
check "the clock runs: needs more time" "printf '%s' '$V' | grep -q NeedsMoreTime"
echo "--- waiting for the shortened lock ($SOLO s)"; sleep $((SOLO + 5))
V=$(verdict_of "$(printf '%s' "$R1" | cut -c1-8)"); echo "verdict after the lock: $V"
check "the recovery is ready after the shortened lock" "printf '%s' '$V' | grep -q Ready"
show "peer-a sync-status (the requester itself gets no alert)" peer peer-a $SSHUB sync-status --format json
ring_view
dump_state "after the first solo recovery"

# ---- recovery 2: the live phone cancels it during the lock
show "peer-a starts a second recovery" sh -c "printf '%s\n%s\n' '$PAPER' '$CODE' | $C exec -T peer-a env SSHUB_E2E_SOLO_LOCK_SECS=$SOLO $SSHUB sync-recovery start --paper --format json"
R2=$(opid "$LAST_OUT")
if [ -n "$R2" ]; then
  show "peer-a acknowledges it" rec ack "$R2" --format json
  show "peer-a shows the notice for the phone" peer peer-a $SSHUB sync-notice "$R2" --format json
  show "the live phone reads the recovery notice" phone recovery-open "{\"frame\":\"$(pf "$LAST_OUT" notice)\"}"
  expect "the page shows the recovery with a cancel button" "$LAST_OUT" '.ok == true and .can_approve == true'
  show "the live phone cancels" phone recovery-cancel
  expect "the page gives the cancellation" "$LAST_OUT" '.ok == true'
  show "peer-a takes the cancellation in" sh -c "printf '%s\n' '$(pf "$LAST_OUT" ops)' | $C exec -T peer-a $SSHUB sync-phone-ops --format json"
  V=$(verdict_of "$(printf '%s' "$R2" | cut -c1-8)"); echo "verdict of the second recovery: $V"
  check "the live phone's cancellation took effect" "printf '%s' '$V' | grep -qi Cancel"
else
  echo "FAIL the second recovery was not started"; fail=$((fail + 1))
fi
finish "$NAME"
