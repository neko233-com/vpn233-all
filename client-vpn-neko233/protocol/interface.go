package protocol

import (
	"net"
)

// Handshaker defines how to establish the initial VPN connection and obfuscation.
type Handshaker interface {
	// ClientHandshake handles the outgoing connection on the client side.
	// Returns the wrapped connection (encrypted) and an error.
	ClientHandshake(conn net.Conn, targetAddr string) (net.Conn, error)
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
