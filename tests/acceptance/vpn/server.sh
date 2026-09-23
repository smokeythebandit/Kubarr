#!/bin/sh
set -eu

test -r /run/kubarr-vpn/server-private
test -r /run/kubarr-vpn/client-public

ip link add wg0 type wireguard
ip address add 10.66.0.1/24 dev wg0
wg set wg0 private-key /run/kubarr-vpn/server-private listen-port 51820 \
  peer "$(cat /run/kubarr-vpn/client-public)" allowed-ips 10.66.0.2/32
ip link set wg0 up

iptables -P FORWARD DROP
iptables -A FORWARD -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -A FORWARD -i wg0 -s 10.66.0.2/32 -d 203.0.113.3/32 -p tcp --dport 8080 -j ACCEPT
iptables -A FORWARD -i wg0 -s 10.66.0.2/32 -d 10.77.0.2/32 -p tcp --dport 8080 -j ACCEPT
iptables -t nat -A POSTROUTING -s 10.66.0.2/32 -d 203.0.113.3/32 -j SNAT --to-source 203.0.113.2
iptables -t nat -A POSTROUTING -s 10.66.0.2/32 -d 10.77.0.2/32 -j SNAT --to-source 10.77.0.1

exec sleep infinity
