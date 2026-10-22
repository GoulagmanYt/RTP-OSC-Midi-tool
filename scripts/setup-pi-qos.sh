#!/usr/bin/env bash
# Configure WMM Voice (AC_VO / priority 6) and DSCP EF (Expedited Forwarding / 46)
# QoS rules on a Raspberry Pi sending RTP-MIDI to OSCMidi.
#
# Usage (on the Raspberry Pi):
#   chmod +x setup-pi-qos.sh
#   sudo ./setup-pi-qos.sh

set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root (e.g. sudo $0)" >&2
    exit 1
fi

echo "==> Configuring nftables QoS for RTP-MIDI..."

NFT_CONF="/etc/nftables.conf"
QOS_CONF="/etc/nftables.d/oscmidi-qos.nft"

if ! command -v nft >/dev/null 2>&1; then
    echo "Installing nftables..."
    apt-get update -qq
    apt-get install -y -qq nftables
fi

RTP_UID=$(id -u rtpmidid 2>/dev/null || echo "")

# Stage both files before touching persistent configuration. Only the dedicated
# table is applied live; loading the full config could flush unrelated rules.
mkdir -p "$(dirname "$QOS_CONF")"
qos_tmp=$(mktemp "${QOS_CONF}.XXXXXX")
config_tmp=''
trap 'rm -f -- "$qos_tmp" ${config_tmp:+"$config_tmp"}' EXIT
config_tmp=$(mktemp "${NFT_CONF}.oscmidi.XXXXXX")
if [ -e "$NFT_CONF" ]; then
    cp -p -- "$NFT_CONF" "$config_tmp"
else
    printf '#!/usr/sbin/nft -f\n' > "$config_tmp"
fi
if ! grep -Fqx "include \"$QOS_CONF\"" "$config_tmp"; then
    printf '\ninclude "%s"\n' "$QOS_CONF" >> "$config_tmp"
fi

cat << 'EOF' > "$qos_tmp"
# add is idempotent; the scoped flush prevents duplicate rules on reruns.
add table inet oscmidi_qos
flush table inet oscmidi_qos
table inet oscmidi_qos {
	chain output {
		type filter hook output priority mangle; policy accept;

		# RTP-MIDI UDP ports (5004 control/data, 5005 clock sync)
		udp dport { 5004, 5005 } counter ip dscp set ef
		udp dport { 5004, 5005 } counter ip6 dscp set ef
		udp dport { 5004, 5005 } counter meta priority set 0:6

		udp sport { 5004, 5005 } counter ip dscp set ef
		udp sport { 5004, 5005 } counter ip6 dscp set ef
		udp sport { 5004, 5005 } counter meta priority set 0:6
EOF

if [ -n "$RTP_UID" ]; then
cat << EOF >> "$qos_tmp"

		# rtpmidid daemon user (UID $RTP_UID)
		meta skuid $RTP_UID counter ip dscp set ef
		meta skuid $RTP_UID counter ip6 dscp set ef
		meta skuid $RTP_UID counter meta priority set 0:6
EOF
fi

cat << 'EOF' >> "$qos_tmp"
	}
}
EOF

# nft applies this file as one transaction. Failed validation/application leaves
# existing rules and both persistent files untouched. Enabling without --now
# avoids starting/reloading the service with the user's full firewall config.
nft -c -f "$qos_tmp"
systemctl enable nftables
nft -f "$qos_tmp"
chmod 644 "$qos_tmp"
mv -f -- "$qos_tmp" "$QOS_CONF"
mv -f -- "$config_tmp" "$NFT_CONF"
echo "==> nftables QoS configuration successfully applied and enabled at boot:"
nft list table inet oscmidi_qos
