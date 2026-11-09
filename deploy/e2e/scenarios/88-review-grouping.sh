#!/bin/sh
# 88: group approval (design 7c, W-APPROVE). `sync-review --approve-all --device <id>` approves only the changes of that
# device that are held as `stale` and touch no dangerous field. Nothing in a live ring is stale (staleness is a
# checkpoint effect, and the e2e image has no clock hook), so the observable rule is the negative one: the dangerous
# change (a new address), a share release and every non-stale change stay in the queue, with and without --device.
# Plain `--approve-all` keeps its older rule: it approves what has no ceremony and leaves the doubtful reasons.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=88-review-grouping

ring2
check "peer-a adds host grp1" "peer peer-a $SSHUB host add --name grp1 --address grp1.internal"
check "peer-a adds host grp2" "peer peer-a $SSHUB host add --name grp2 --address grp2.internal"
snow peer-b >/dev/null
q=$(peer peer-b $SSHUB sync-review --format json 2>/dev/null || true)
echo "$q" | head -n 40
N=$(printf '%s' "$q" | pjq 'length')
check "the new addresses wait for review on peer-b" "[ '${N:-0}' -ge 2 ]"
expect "every waiting change names its reason" "$q" 'all(.[]; .reason != null)'
expect "the address changes are marked dangerous" "$q" '([.[] | .changes[]? | select(.field == "address")] | length) >= 2 and all(.[] | .changes[]? | select(.field == "address"); .dangerous)'
A_ID=$(devid peer-b peer-a)
check "peer-b can name peer-a" "test -n '$A_ID'"

show "--device without --approve-all is a usage error" peer peer-b $SSHUB sync-review --device "$A_ID" --format json
check "usage error exit status 2" "[ '$LAST_RC' -eq 2 ]"

show "peer-b sync-review --approve-all --device peer-a (group approval for one device)" peer peer-b $SSHUB sync-review --approve-all --device "$A_ID" --format json
expect "nothing is stale, so nothing is approved and everything is left" "$LAST_OUT" '.approved == 0 and .left >= 2'
q2=$(peer peer-b $SSHUB sync-review --format json 2>/dev/null || true)
check "the queue is as long as before" "[ \"$(printf '%s' "$q2" | pjq 'length')\" = '$N' ]"
check "grp1 is not applied on peer-b yet" "! peer peer-b $SSHUB host list --format json 2>/dev/null | grep -q grp1.internal"

show "peer-b sync-review --approve-all (the plain verb, no ceremony in the queue)" peer peer-b $SSHUB sync-review --approve-all --format json
expect "the plain verb approves the waiting changes" "$LAST_OUT" '.approved >= 2 and .left == 0'
check "grp1 is applied on peer-b now" "peer peer-b $SSHUB host list --format json 2>/dev/null | grep -q grp1.internal"

finish "$NAME"
