package protocol

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"encoding/binary"
	"io"
	"net"
)

type RealityHandshaker struct {
	ShortID string
	Key     []byte
}

func (h *RealityHandshaker) ClientHandshake(conn net.Conn, targetAddr string) (net.Conn, error) {
	// 1. Send Short ID
	conn.Write([]byte(h.ShortID))

	// 2. Send Encrypted Header
	block, _ := aes.NewCipher(h.Key)
	aesgcm, _ := cipher.NewGCM(block)

	nonce := make([]byte, 12)
	io.ReadFull(rand.Reader, nonce)
	conn.Write(nonce)

	addrBytes := []byte(targetAddr)
	header := make([]byte, 16) // Padded
	binary.BigEndian.PutUint16(header[:2], uint16(len(addrBytes)))
	copy(header[2:], addrBytes)

	encryptedHeader := aesgcm.Seal(nil, nonce, header, nil)
	conn.Write(encryptedHeader)

	return conn, nil
}
