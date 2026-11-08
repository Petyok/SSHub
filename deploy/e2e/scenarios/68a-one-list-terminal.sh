#!/bin/sh
# 68a-one-list-terminal: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=68a-one-list-terminal
skeleton "$NAME" "68a one terminal: a restored phone with one terminal in the ring accepts one list, with the risk stated" "PH-core U13-5 (accept_head, single_source) and the terminals block of kind 5"
