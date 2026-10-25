#!/bin/sh
# 65: the ring's phone is the committed companion page (phone-driver.mjs, compose service `phone`). It acts through
# paste text, the text and QR the page shows, and buttons only. The terminal side is the real CLI wherever the CLI
# has a verb; where it has none, the exact missing verb is printed as a GAP and a shim built from the peer's own
# database (read only) stands in, marked SHIM. 60, 70 and 95 get their phone proof from this driver once the CLI
# verbs exist (see phone-README.md).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=65-phone-driver

check "the phone daemon answers" phone_ready
show "phone info" phone info
expect "the pinned page equals its FINGERPRINT" "$LAST_OUT" '.fingerprint_ok == true'
show "the phone restores its fixed test identity" phone setup
expect "the page shows the identity the driver derived" "$LAST_OUT" '.device_id | test("^[0-9a-f]{16}$")'
PHONE_ID=$(printf '%s' "$LAST_OUT" | pjq -r .device_id)

# ---- which CLI verbs exist for the phone (read from the real binary's own command list)
HELP=$(peer peer-a $SSHUB --help 2>&1 || true)
echo "sync verbs the CLI lists: $(printf '%s\n' "$HELP" | grep -o 'sync-[a-z-]*' | sort -u | tr '\n' ' ')"
for v in sync-phone-enroll sync-notice sync-phone-ops sync-share-request sync-share-reply sync-cert-request; do
  if printf '%s\n' "$HELP" | grep -q -- "$v"; then echo "ok   CLI verb $v exists"; else echo "--- GAP: CLI verb $v does not exist"; fi
done

ring2

# ---- one Red operation: the paper factor registration is proposed on peer-a (real CLI)
show "peer-a registers the paper factor (a Red change)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
expect "it is a Red change with a ceremony" "$LAST_OUT" '.tier == "Red" and (.ceremony != null)'
OP=$(opid "$LAST_OUT")
echo "missing right after the proposal: $(printf '%s' "$LAST_OUT" | pjq -c '.ceremony.missing')"
check "the op is known to peer-a's database" "test -n \"\$(oprow peer-a '$OP')\""

# SHIM: no verb prints the kind 3 notice of a pending Red op, so the frame is built from the stored op bytes.
ROW=$(oprow peer-a "$OP")
CAN=${ROW% *}; SIG=${ROW#* }
show "SHIM: kind 3 notice frame from the stored op (verb sync-notice is missing)" phone frame-notice "{\"canonicalHex\":\"$CAN\",\"signatureHex\":\"$SIG\"}"
FRAME=$(printf '%s' "$LAST_OUT" | pjq -r .frame)
show "the phone reads the notice" phone red-open "{\"frame\":\"$FRAME\"}"
expect "the page names the operation and its three proofs" "$LAST_OUT" '.ok == true and (.banner | test("FactorRegister")) and (.banner | test("three proofs"))'
expect "the page shows every leaf of the operation" "$LAST_OUT" '(.rows | length) >= 4 and (.rows | map(select(.[2])) | length) >= 1'
expect "the hash on the screen starts with the op id the CLI printed" "$LAST_OUT" ".op_hash | startswith(\"$OP\")"
OPH=$(printf '%s' "$LAST_OUT" | pjq -r .op_hash)
show "the phone approves (RedPhoneSign)" phone red-approve
expect "the ops text is also the QR the page draws" "$LAST_OUT" '.ok == true and .qr_equals_text == true'
OPS=$(printf '%s' "$LAST_OUT" | pjq -r .ops)
show "check the phone's ops text" phone verify-ops "{\"ops\":\"$OPS\",\"kind\":\"RedPhoneSign\",\"opHash\":\"$OPH\"}"
expect "RedPhoneSign names the exact op hash, signed by the phone key, chain from seq 1" "$LAST_OUT" '.ok == true and .device_id != null and .op_hash != null'

# ---- what the terminal would do next, and cannot
echo "--- GAP: the phone's ops text ($(printf '%s' "$OPS" | wc -c) chars of base64, kind 5) has no CLI verb to be ingested (sync-phone-ops): ceremony of $OP still misses the phone proof"
echo "--- GAP: enrolling the phone ($PHONE_ID) needs sync-phone-enroll (PhoneEnroll begin/scan/answer + admit_phone); the page half is ready: phone enroll-start, enroll-commit, enroll-reveal, enroll-match, enroll-finish"
echo "--- GAP: a share release for the phone needs sync-share-request (prints the kind 1 challenge) and sync-share-reply (reads the kind 2 response); the page half is ready: phone share-open, share-approve"

dump_state "after the phone's answer"
finish "$NAME"
