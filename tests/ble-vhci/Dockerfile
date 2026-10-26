# Test container for the Bluetooth channel against a real BlueZ on virtual controllers (hci_vhci).
# Built FROM the CI image; adds only bluez (bluetoothd), bluez-test-tools (btvirt) and dbus.
# Run as root with --network host (AF_BLUETOOTH exists only in the initial network namespace),
# --device /dev/vhci and NET_ADMIN/NET_RAW; see run.sh and poc/results/ble.md.
FROM sshub-ci:local
USER root
RUN apt-get update \
 && apt-get install -y --no-install-recommends bluez bluez-test-tools dbus \
 && rm -rf /var/lib/apt/lists/*
