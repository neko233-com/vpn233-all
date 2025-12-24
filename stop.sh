#!/bin/bash
echo "Stopping VPN233 System..."

pkill -f "go run server-vpn-neko233/main.go"
pkill -f "go run client-vpn-neko233/main.go"

echo "VPN233 stopped."
