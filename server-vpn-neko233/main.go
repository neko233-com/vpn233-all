package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"os"

	"github.com/neko233/server-vpn-neko233/protocol"
)

type User struct {
	ID   string `json:"id"`
	UUID string `json:"uuid"`
}

type Config struct {
	ListenAddr string   `json:"listen_addr"`
	DestAddr   string   `json:"dest_addr"`
	PrivateKey string   `json:"private_key"`
	ShortIDs   []string `json:"short_ids"`
	Users      []User   `json:"users"`
}

func main() {
	configPath := flag.String("c", "config.json", "path to config file")
	help := flag.Bool("h", false, "show help")
	flag.Parse()

	if *help || (len(os.Args) == 1 && !fileExists(*configPath)) {
		fmt.Println("vpn233-server - A secure VPN server with DIY extensible architecture")
		fmt.Println("\nUsage:")
		fmt.Println("  vpn233-server [-c config.json]")
		fmt.Println("\nOptions:")
		flag.PrintDefaults()
		return
	}

	configFile, err := os.Open(*configPath)
	if err != nil {
		log.Fatalf("Failed to open config: %v", err)
	}
	defer configFile.Close()

	var config Config
	if err := json.NewDecoder(configFile).Decode(&config); err != nil {
		log.Fatalf("Failed to decode config: %v", err)
	}

	// Initialize default handshaker (DIY: others can replace this)
	handshaker := &protocol.RealityHandshaker{
		ShortIDs: config.ShortIDs,
		Key:      []byte("neko233-vpn-secure-key-32-bytes!"), // Should be from config
	}

	listener, err := net.Listen("tcp", config.ListenAddr)
	if err != nil {
		log.Fatalf("Failed to listen on %s: %v", config.ListenAddr, err)
	}
	log.Printf("Server listening on %s", config.ListenAddr)

	for {
		conn, err := listener.Accept()
		if err != nil {
			continue
		}
		go handleConnection(conn, handshaker, config.DestAddr)
	}
}

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

func handleConnection(conn net.Conn, h protocol.Handshaker, destAddr string) {
	defer conn.Close()

	// DIY Handshake
	vpnConn, targetAddr, err := h.ServerHandshake(conn)
	if err != nil {
		// Fallback to destination (REALITY behavior)
		log.Printf("Non-VPN or Unauthorized connection, forwarding to %s", destAddr)
		if vpnConn != nil {
			forwardToDest(vpnConn, nil, destAddr)
		} else {
			forwardToDest(conn, nil, destAddr)
		}
		return
	}

	log.Printf("VPN Secure Request to: %s", targetAddr)

	// Connect to target
	targetConn, err := net.Dial("tcp", targetAddr)
	if err != nil {
		log.Printf("Failed to connect to target %s: %v", targetAddr, err)
		return
	}
	defer targetConn.Close()

	// Bidirectional copy
	errChan := make(chan error, 2)
	go func() { _, e := io.Copy(targetConn, vpnConn); errChan <- e }()
	go func() { _, e := io.Copy(vpnConn, targetConn); errChan <- e }()
	<-errChan
}

func forwardToDest(conn net.Conn, initialData []byte, destAddr string) {
	destConn, err := net.Dial("tcp", destAddr)
	if err != nil {
		return
	}
	defer destConn.Close()

	if initialData != nil {
		destConn.Write(initialData)
	}
	
	errChan := make(chan error, 2)
	go func() { io.Copy(destConn, conn); errChan <- nil }()
	go func() { io.Copy(conn, destConn); errChan <- nil }()
	<-errChan
}


func forwardToDest(conn net.Conn, initialData []byte, destAddr string) {
	destConn, err := net.Dial("tcp", destAddr)
	if err != nil {
		log.Printf("Failed to connect to fallback dest %s: %v", destAddr, err)
		return
	}
	defer destConn.Close()

	destConn.Write(initialData)
	errChan := make(chan error, 2)
	go func() {
		_, err := io.Copy(destConn, conn)
		errChan <- err
	}()
	go func() {
		_, err := io.Copy(conn, destConn)
		errChan <- err
	}()
	<-errChan
}
