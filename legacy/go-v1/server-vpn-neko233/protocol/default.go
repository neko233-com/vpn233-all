package protocol

import (
	"bufio"
	"crypto/aes"
	"crypto/cipher"
	"encoding/binary"
	"fmt"
	"io"
	"net"
)

type RealityHandshaker struct {
	ShortIDs   []string
	Key        []byte
	VerifyUser func(username, password string) bool
}

func (h *RealityHandshaker) ServerHandshake(conn net.Conn) (net.Conn, string, string, error) {
	br := bufio.NewReader(conn)

	// Peek to check ShortID without consuming
	maxLen := 0
	for _, sid := range h.ShortIDs {
		if len(sid) > maxLen {
			maxLen = len(sid)
		}
	}
	if maxLen == 0 {
		return nil, "", "", fmt.Errorf("no short ids configured")
	}

	peeked, err := br.Peek(maxLen)
	if err != nil {
		return &BufferedConn{Conn: conn, br: br}, "", "", err
	}

	matched := false
	var sidLen int
	for _, sid := range h.ShortIDs {
		if len(peeked) >= len(sid) && string(peeked[:len(sid)]) == sid {
			matched = true
			sidLen = len(sid)
			break
		}
	}

	if !matched {
		return &BufferedConn{Conn: conn, br: br}, "", "", fmt.Errorf("unauthorized: short id mismatch")
	}

	// Now we know it's a VPN request, consume the ShortID
	br.Discard(sidLen)

	// Read Nonce(12)
	nonce := make([]byte, 12)
	if _, err := io.ReadFull(br, nonce); err != nil {
		return &BufferedConn{Conn: conn, br: br}, "", "", err
	}

	// Read encrypted header length (uint16) followed by encrypted header
	lenBuf := make([]byte, 2)
	if _, err := io.ReadFull(br, lenBuf); err != nil {
		return &BufferedConn{Conn: conn, br: br}, "", "", err
	}
	encLen := binary.BigEndian.Uint16(lenBuf)
	encryptedHeader := make([]byte, encLen)
	if _, err := io.ReadFull(br, encryptedHeader); err != nil {
		return &BufferedConn{Conn: conn, br: br}, "", "", err
	}

	block, _ := aes.NewCipher(h.Key)
	aesgcm, _ := cipher.NewGCM(block)

	header, err := aesgcm.Open(nil, nonce, encryptedHeader, nil)
	if err != nil {
		return &BufferedConn{Conn: conn, br: br}, "", "", fmt.Errorf("decryption failed")
	}

	// Header format: [addrLen(2)][addr][userLen(1)][user][passLen(1)][pass]
	offset := 0
	addrLen := int(binary.BigEndian.Uint16(header[offset : offset+2]))
	offset += 2
	targetAddr := string(header[offset : offset+addrLen])
	offset += addrLen

	userLen := int(header[offset])
	offset += 1
	username := string(header[offset : offset+userLen])
	offset += userLen

	passLen := int(header[offset])
	offset += 1
	password := string(header[offset : offset+passLen])

	if h.VerifyUser != nil && !h.VerifyUser(username, password) {
		return &BufferedConn{Conn: conn, br: br}, "", "", fmt.Errorf("unauthorized: invalid user or password")
	}

	return &BufferedConn{Conn: conn, br: br}, targetAddr, username, nil
}

type BufferedConn struct {
	net.Conn
	br *bufio.Reader
}

func (c *BufferedConn) Read(b []byte) (int, error) {
	return c.br.Read(b)
}
