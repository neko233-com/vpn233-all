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
	ShortID  string
	Key      []byte
	Username string
	Password string
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
	userBytes := []byte(h.Username)
	passBytes := []byte(h.Password)

	// Header format: [addrLen(2)][addr][userLen(1)][user][passLen(1)][pass]
	header := make([]byte, 2+len(addrBytes)+1+len(userBytes)+1+len(passBytes))
	offset := 0
	binary.BigEndian.PutUint16(header[offset:offset+2], uint16(len(addrBytes)))
	offset += 2
	copy(header[offset:], addrBytes)
	offset += len(addrBytes)

	header[offset] = byte(len(userBytes))
	offset += 1
	copy(header[offset:], userBytes)
	offset += len(userBytes)

	header[offset] = byte(len(passBytes))
	offset += 1
	copy(header[offset:], passBytes)

	encryptedHeader := aesgcm.Seal(nil, nonce, header, nil)
	var lenBuf [2]byte
	binary.BigEndian.PutUint16(lenBuf[:], uint16(len(encryptedHeader)))
	conn.Write(lenBuf[:])
	conn.Write(encryptedHeader)

	return conn, nil
}
