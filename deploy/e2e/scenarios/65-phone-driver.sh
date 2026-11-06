#!/bin/sh
# timeout: 1200
# 65: the ring's phone is the committed companion page (phone-driver.mjs, compose service `phone`). It acts through
# paste text, the text and QR the page shows, and buttons only. The terminal side is the real CLI: sync-phone-enroll
# (enrollment as text, SAS words), sync-notice (kind 3), sync-phone-ops (kind 5 in). Nothing is read from a database
# except to find an op id.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=65-phone-driver

check "the phone daemon answers" phone_ready
show "phone info" phone info
expect "the pinned page equals its FINGERPRINT" "$LAST_OUT" '.fingerprint_ok == true'

step "ring with the phone (ring2)" ring_with_phone ring2

spread peer-b peer-a peer-b
st=$(peer peer-b $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-b sees the phone as a holder too" "$st" '[.devices[] | select(.name=="my phone") | .role] == ["holder"]'
expect "peer-b is a holder in its own table" "$st" '[.devices[] | select(.name=="peer-b") | .role] == ["holder"]'

dump_state "after the phone's ring setup"
finish "$NAME"
