#!/bin/sh
# Inside the sshub-ble-vhci container: a private D-Bus, two virtual controllers (btvirt over /dev/vhci),
# bluetoothd on them, then the ignored vhci test binary given as $1 (the rest are its arguments).
# Everything started here is stopped on exit; the virtual controllers vanish with btvirt.
set -u
test_bin=$1
shift
mkdir -p /run/dbus /var/lib/bluetooth
dbus-uuidgen --ensure
dbus-daemon --system --fork --print-pid > /tmp/dbus.pid
btvirt -l2 > /tmp/btvirt.log 2>&1 &
btvirt_pid=$!
cleanup() {
  kill "$btvirt_pid" 2>/dev/null
  [ -n "${bluetoothd_pid:-}" ] && kill "$bluetoothd_pid" 2>/dev/null
  kill "$(cat /tmp/dbus.pid)" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT
i=0
while [ ! -e /sys/class/bluetooth/hci1 ] && [ "$i" -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
echo "controllers: $(ls /sys/class/bluetooth | tr '\n' ' ')"
[ -e /sys/class/bluetooth/hci1 ] || { echo "no second virtual controller"; cat /tmp/btvirt.log; exit 3; }
bluetoothd_bin=$(command -v bluetoothd || ls /usr/libexec/bluetooth/bluetoothd /usr/lib/bluetooth/bluetoothd 2>/dev/null | head -1)
[ -n "$bluetoothd_bin" ] || { echo "no bluetoothd in the image"; exit 4; }
"$bluetoothd_bin" -n ${BLUETOOTHD_ARGS:--d -E} > /tmp/bluetoothd.log 2>&1 &
bluetoothd_pid=$!
sleep 2
for i in 0 1; do echo "--- btmgmt info hci$i"; btmgmt --index $i info 2>&1 | head -20; done
export SSHUB_BLE_ADAPTER=${SSHUB_BLE_ADAPTER:-hci0} SSHUB_BLE_TEST_CLIENT=${SSHUB_BLE_TEST_CLIENT:-hci1}
"$test_bin" "$@"
rc=$?
echo "--- bluetoothd log (adv / gatt lines, tail)"
grep -v -E "gatt-database|vcp|mcp|bap|shared/mgmt" /tmp/bluetoothd.log | tail -n 60
exit $rc
