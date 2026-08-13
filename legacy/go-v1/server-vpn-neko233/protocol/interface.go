package protocol

import (
	"net"
)

// Handshaker defines how to establish the initial VPN connection and obfuscation.
type Handshaker interface {
	// ServerHandshake handles the incoming connection on the server side.
	// Returns the wrapped connection (encrypted), the target address requested, username, and an error.
	ServerHandshake(conn net.Conn) (net.Conn, string, string, error)
}

// Cipher defines the encryption/decryption interface.
type Cipher interface {
	WrapConn(conn net.Conn) net.Conn
}

// Registry allows registering different implementations.
var (
	Handshakers = make(map[string]Handshaker)
	Ciphers     = make(map[string]Cipher)
)
