package protocol

import (
	"bytes"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"encoding/binary"
	"io"
	"net"
	"testing"
)

// MockConn is a simple net.Conn implementation for testing
type MockConn struct {
	readBuf  *bytes.Buffer
	writeBuf *bytes.Buffer
}

func (m *MockConn) Read(b []byte) (n int, err error)  { return m.readBuf.Read(b) }
func (m *MockConn) Write(b []byte) (n int, err error) { return m.writeBuf.Write(b) }
func (m *MockConn) Close() error                     { return nil }
func (m *MockConn) LocalAddr() net.Addr              { return nil }
func (m *MockConn) RemoteAddr() net.Addr             { return nil }
func (m *MockConn) SetDeadline(t net.Time) error      { return nil }
func (m *MockConn) SetReadDeadline(t net.Time) error { return nil }
func (m *MockConn) SetWriteDeadline(t net.Time) error { return nil }

func TestRealityHandshake(t *testing.T) {
	key := []byte("neko233-vpn-secure-key-32-bytes!")
	shortID := "sid12345"
	targetAddr := "google.com:443"

	// 1. Prepare Client Data
	clientBuf := new(bytes.Buffer)
	clientBuf.WriteString(shortID)

	block, _ := aes.NewCipher(key)
	aesgcm, _ := cipher.NewGCM(block)
	nonce := make([]byte, 12)
	io.ReadFull(rand.Reader, nonce)
	clientBuf.Write(nonce)

	addrBytes := []byte(targetAddr)
	header := make([]byte, 16)
	binary.BigEndian.PutUint16(header[:2], uint16(len(addrBytes)))
	copy(header[2:], addrBytes)
	encryptedHeader := aesgcm.Seal(nil, nonce, header, nil)
	clientBuf.Write(encryptedHeader)

	// 2. Test Server Handshake
	serverConn := &MockConn{
		readBuf:  clientBuf,
		writeBuf: new(bytes.Buffer),
	}

	handshaker := &RealityHandshaker{
		ShortIDs: []string{shortID},
		Key:      key,
	}

	_, gotAddr, err := handshaker.ServerHandshake(serverConn)
	if err != nil {
		t.Fatalf("ServerHandshake failed: %v", err)
	}

	if gotAddr != targetAddr {
		t.Errorf("Expected target addr %s, got %s", targetAddr, gotAddr)
	}
}
