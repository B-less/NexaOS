#!/bin/sh
# SETUP for test-app-headless.sh when shooting Settings' Bluetooth/VPN pages:
# a fake BlueZ on a private bus (standing in for the system bus), a
# pass-through doas, the real Settings helper, and one imported VPN.
apk add -q dbus wireguard-tools iproute2 >/dev/null 2>&1
DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address | head -1)
export DBUS_SESSION_BUS_ADDRESS
printf 'export DBUS_SESSION_BUS_ADDRESS=%s DBUS_SYSTEM_BUS_ADDRESS=%s\n' \
    "$DBUS_SESSION_BUS_ADDRESS" "$DBUS_SESSION_BUS_ADDRESS" > /tmp/app.env
python3 /src/common/tests/mock_bluez.py >/dev/null 2>&1 &
for i in 1 2 3 4 5 6 7 8 9 10; do
    dbus-send --session --print-reply --dest=org.bluez /org/bluez/hci0 org.freedesktop.DBus.Properties.Set \
        string:org.bluez.Adapter1 string:Powered variant:boolean:true >/dev/null 2>&1 && break
    sleep 0.5
done
printf '#!/bin/sh\nexec "$@"\n' > /usr/local/bin/doas
printf '#!/bin/sh\nPYTHONPATH=/src/settings-app:/src/common exec python3 -m nexa_settings.helper\n' \
    > /usr/local/bin/nexa-settings-helper
chmod +x /usr/local/bin/doas /usr/local/bin/nexa-settings-helper
key=$(wg genkey)
printf '{"action":"vpn_import","name":"Sweden","config":"[Interface]\\nPrivateKey = %s\\nAddress = 10.2.0.2/32\\nDNS = 10.2.0.1\\n[Peer]\\nPublicKey = %s\\nAllowedIPs = 0.0.0.0/0\\nEndpoint = se.example.net:51820\\n"}' \
    "$key" "$(echo "$key" | wg pubkey)" | /usr/local/bin/nexa-settings-helper
