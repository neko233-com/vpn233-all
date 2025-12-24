package protocol

import (
	"bytes"
	"crypto/aes"
	"crypto/cipher"
	"encoding/binary"
	"net"
	"testing"
)

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

func TestRealityClientHandshake(t *testing.T) {
	key := []byte("neko233-vpn-secure-key-32-bytes!")
	shortID := "sid12345"
	targetAddr := "google.com:443"

	handshaker := &RealityHandshaker{
		ShortID: shortID,
		Key:     key,
	}

	mockConn := &MockConn{
		readBuf:  new(bytes.Buffer),
		writeBuf: new(bytes.Buffer),
	}

	_, err := handshaker.ClientHandshake(mockConn, targetAddr)
	if err != nil {
		t.Fatalf("ClientHandshake failed: %v", err)
	}

	data := mockConn.writeBuf.Bytes()
	
	// Check ShortID
	if string(data[:len(shortID)]) != shortID {
		t.Errorf("Expected shortID %s, got %s", shortID, string(data[:len(shortID)]))
	}

	// Check Encryption
	nonce := data[len(shortID) : len(shortID)+12]
	encrypted := data[len(shortID)+12:]

	block, _ := aes.NewCipher(key)
	aesgcm, _ := cipher.NewGCM(block)
	decrypted, err := aesgcm.Open(nil, nonce, encrypted, nil)
	if err != nil {
		t.Fatalf("Failed to decrypt header: %v", err)
	}

	addrLen := binary.BigEndian.Uint16(decrypted[:2])
	gotAddr := string(decrypted[2 : 2+addrLen])

	if gotAddr != targetAddr {
		t.Errorf("Expected target addr %s, got %s", targetAddr, gotAddr)
	}
}
