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

if ! command -v nft >/dev/null 2>&1; then
    echo "Installing nftables..."
    apt-get update -qq && apt-get install -y -qq nftables
fi

RTP_UID=$(id -u rtpmidid 2>/dev/null || echo "")

cat << 'EOF' > "$NFT_CONF"
#!/usr/sbin/nft -f

flush ruleset

table inet filter {
	chain input {
		type filter hook input priority filter; policy accept;
	}
	chain forward {
		type filter hook forward priority filter; policy accept;
	}
	chain output {
		type filter hook output priority filter; policy accept;
	}
}

table inet qos {
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
cat << EOF >> "$NFT_CONF"

		# rtpmidid daemon user (UID $RTP_UID)
		meta skuid $RTP_UID counter ip dscp set ef
		meta skuid $RTP_UID counter ip6 dscp set ef
		meta skuid $RTP_UID counter meta priority set 0:6
EOF
fi

cat << 'EOF' >> "$NFT_CONF"
	}
}
EOF

nft -f "$NFT_CONF"
systemctl enable --now nftables
echo "==> nftables QoS configuration successfully applied and enabled at boot:"
nft list table inet qos
