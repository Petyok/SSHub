#!/bin/sh
# 73-solo-recovery: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=73-solo-recovery
skeleton "$NAME" "one-terminal recovery: the clock starts on the requester's ack; the live phone cancels" "C solo recovery (5.2) and the lock-shortening env of G8"
