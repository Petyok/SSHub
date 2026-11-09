#!/bin/sh
# 88-review-grouping: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=88-review-grouping
skeleton "$NAME" "group approve leaves dangerous and non-stale ops pending" "T W-APPROVE (sync-review --approve-all)"
