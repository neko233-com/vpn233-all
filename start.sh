#!/bin/bash
echo "Starting VPN233 System..."

# Start Server
go run server-vpn-neko233/main.go -c server-vpn-neko233/config.json > server.log 2>&1 &
SERVER_PID=$!

# Start Client
go run client-vpn-neko233/main.go -c client-vpn-neko233/config.json > client.log 2>&1 &
CLIENT_PID=$!

echo "VPN233 is running in background."
echo "Server PID: $SERVER_PID, Client PID: $CLIENT_PID"
echo "Client Web: http://127.0.0.1:8082"
echo "Server Web: http://127.0.0.1:8083"
