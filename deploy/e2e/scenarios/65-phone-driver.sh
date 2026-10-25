#!/bin/sh
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
show "the phone restores its fixed test identity" phone setup
expect "the page shows the identity the driver derived" "$LAST_OUT" '.device_id | test("^[0-9a-f]{16}$")'
PHONE_ID=$(pf "$LAST_OUT" device_id)

ring2

# ---- a Red op before the phone exists: the paper factor registration. A factor must be registered BEFORE the op it
# signs, and the registration needs the phone, so the registration is proposed first and the phone signs it once enrolled.
show "peer-a registers the paper factor (Red)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
expect "it is a Red change with a ceremony" "$LAST_OUT" '.tier == "Red" and (.ceremony != null)'
REG=$(opid "$LAST_OUT")

# ---- enrollment of the phone to peer-a, all text (lib-creds.sh: enroll_phone)
enroll_phone peer-a
show "peer-a status after the enrollment" peer peer-a $SSHUB sync-status --format json
expect "the phone is listed, still a mirror until its admission is in force" "$LAST_OUT" '[.devices[] | select(.name=="my phone") | .role] == ["mirror"]'

# ---- the phone signs both Red ops it can see: the registration and its own admission
phone_signs peer-a "$REG"
expect "the registration proof was taken in" "$LAST_OUT" '.received >= 1'
phone_signs peer-a "$ADD"
expect "the admission proof was taken in" "$LAST_OUT" '.received >= 1'

# ---- the second attended holder votes on both, the paper signs the admission
spread peer-b peer-a peer-b
show "peer-b sync-approve the registration" peer peer-b $SSHUB sync-approve "$REG" --format json
echo "registration ceremony on peer-b: $(pf "$LAST_OUT" ceremony | tr -d '\n ')"
show "peer-b sync-approve the phone admission" peer peer-b $SSHUB sync-approve "$ADD" --format json
echo "admission ceremony on peer-b: $(pf "$LAST_OUT" ceremony | tr -d '\n ')"
spread peer-a peer-b peer-a
show "peer-a adds the paper factor to the admission" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor $ADD --paper --format json"
expect "the admission ceremony shows the phone proof" "$LAST_OUT" '.ceremony.have | any(. == "phone")'
expect "the admission ceremony is complete (phone, second device, paper)" "$LAST_OUT" '.ceremony.complete == true'
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "the phone is a holder on peer-a" "$st" '[.devices[] | select(.name=="my phone") | .role] == ["holder"]'
spread peer-b peer-a peer-b
st=$(peer peer-b $SSHUB sync-status --format json 2>/dev/null || true)
echo "peer-b sees the phone as: $(printf '%s' "$st" | pjq -c '[.devices[] | select(.name=="my phone") | {role, status}]')"
echo "peer-b op states (kind|state|reason): $(sq peer-b "SELECT kind||'|'||state||'|'||coalesce(reason,'') FROM sync_ops WHERE kind IN ('DeviceAdd','FactorRegister','RedPhoneSign','RedApprove','RedFactor')" | tr '\n' ';')"

dump_state "after the phone's answer"
finish "$NAME"
