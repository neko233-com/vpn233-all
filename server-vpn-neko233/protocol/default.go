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
	ShortIDs []string
	Key      []byte
}

func (h *RealityHandshaker) ServerHandshake(conn net.Conn) (net.Conn, string, error) {
	br := bufio.NewReader(conn)
	
	// Peek to check ShortID without consuming
	maxLen := 0
	for _, sid := range h.ShortIDs {
		if len(sid) > maxLen {
			maxLen = len(sid)
		}
	}
	if maxLen == 0 {
		return nil, "", fmt.Errorf("no short ids configured")
	}

	peeked, err := br.Peek(maxLen)
	if err != nil {
		return &BufferedConn{Conn: conn, br: br}, "", err
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
		return &BufferedConn{Conn: conn, br: br}, "", fmt.Errorf("unauthorized: short id mismatch")
	}

	// Now we know it's a VPN request, consume the ShortID
	br.Discard(sidLen)

	// Read Nonce(12) + EncryptedHeader(32)
	headerData := make([]byte, 44)
	if _, err := io.ReadFull(br, headerData); err != nil {
		return &BufferedConn{Conn: conn, br: br}, "", err
	}

	nonce := headerData[:12]
	encryptedHeader := headerData[12:44]

	block, _ := aes.NewCipher(h.Key)
	aesgcm, _ := cipher.NewGCM(block)

	header, err := aesgcm.Open(nil, nonce, encryptedHeader, nil)
	if err != nil {
		return &BufferedConn{Conn: conn, br: br}, "", fmt.Errorf("decryption failed")
	}

	addrLen := binary.BigEndian.Uint16(header[:2])
	targetAddr := string(header[2 : 2+addrLen])

	return &BufferedConn{Conn: conn, br: br}, targetAddr, nil
}

type BufferedConn struct {
	net.Conn
	br *bufio.Reader
}

func (c *BufferedConn) Read(b []byte) (int, error) {
	return c.br.Read(b)
}
