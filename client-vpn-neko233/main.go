package main

import (
	"bufio"
	"encoding/binary"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"strings"

	"github.com/neko233/client-vpn-neko233/protocol"
)

type Inbound struct {
	Type    string `json:"type"`
	Listen  string `json:"listen,omitempty"`
	Port    int    `json:"port,omitempty"`
	Name    string `json:"name,omitempty"`
	Address string `json:"address,omitempty"`
	MTU     int    `json:"mtu,omitempty"`
	Enabled bool   `json:"enabled,omitempty"`
}

type DNSConfig struct {
	Listen      string   `json:"listen"`
	Nameservers []string `json:"nameservers"`
}

type Outbound struct {
	Tag       string `json:"tag"`
	Type      string `json:"type"`
	Server    string `json:"server,omitempty"`
	Port      int    `json:"port,omitempty"`
	UUID      string `json:"uuid,omitempty"`
	ShortID   string `json:"short_id,omitempty"`
	PublicKey string `json:"public_key,omitempty"`
	SNI       string `json:"sni,omitempty"`
}

type Config struct {
	Inbounds  []Inbound  `json:"inbounds"`
	DNS       DNSConfig  `json:"dns"`
	Outbounds []Outbound `json:"outbounds"`
	RulesFile string     `json:"rules_file"`
}

type Rule struct {
	Type     string
	Payload  string
	Outbound string
}

var (
	globalConfig *Config
	routingRules []Rule
	handshaker   protocol.Handshaker
)

func main() {
	configPath := flag.String("c", "config.json", "path to config file")
	help := flag.Bool("h", false, "show help")
	flag.Parse()

	if *help || (len(os.Args) == 1 && !fileExists(*configPath)) {
		fmt.Println("vpn233-client - A secure VPN client with DIY extensible architecture")
		fmt.Println("\nUsage:")
		fmt.Println("  vpn233-client [-c config.json]")
		fmt.Println("\nOptions:")
		flag.PrintDefaults()
		return
	}

	loadConfig(*configPath)
	loadRules(globalConfig.RulesFile)

	// Initialize default handshaker (DIY: others can replace this)
	handshaker = &protocol.RealityHandshaker{
		ShortID: "sid12345",
		Key:     []byte("neko233-vpn-secure-key-32-bytes!"),
	}

	for _, in := range globalConfig.Inbounds {
		if in.Type == "tun" && !in.Enabled {
			continue
		}
		go startInbound(in)
	}

	go startDNSServer(globalConfig.DNS)

	log.Println("vpn233-client started. Press Ctrl+C to exit.")
	select {}
}

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

func loadConfig(path string) {
	configFile, err := os.Open(path)
	if err != nil {
		log.Fatalf("Failed to open config: %v", err)
	}
	defer configFile.Close()

	if err := json.NewDecoder(configFile).Decode(&globalConfig); err != nil {
		log.Fatalf("Failed to decode config: %v", err)
	}
}

func loadRules(path string) {
	file, err := os.Open(path)
	if err != nil {
		log.Printf("Warning: Failed to open rules file %s: %v", path, err)
		return
	}
	defer file.Close()

	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") || !strings.Contains(line, ",") {
			continue
		}
		parts := strings.Split(line, ",")
		if len(parts) >= 3 {
			routingRules = append(routingRules, Rule{
				Type:     strings.TrimSpace(parts[0]),
				Payload:  strings.TrimSpace(parts[1]),
				Outbound: strings.TrimSpace(parts[2]),
			})
		}
	}
	log.Printf("Loaded %d routing rules from %s", len(routingRules), path)
}

func startInbound(in Inbound) {
	if in.Type == "tun" {
		log.Printf("TUN mode requested for %s. (Requires admin/root)", in.Name)
		return
	}

	addr := fmt.Sprintf("%s:%d", in.Listen, in.Port)
	listener, err := net.Listen("tcp", addr)
	if err != nil {
		log.Fatalf("Failed to listen on %s: %v", addr, err)
	}
	log.Printf("Inbound %s listening on %s", in.Type, addr)

	for {
		conn, err := listener.Accept()
		if err != nil {
			continue
		}
		if in.Type == "socks" {
			go handleSocks5(conn)
		} else if in.Type == "http" {
			go handleHTTP(conn)
		}
	}
}

func handleSocks5(conn net.Conn) {
	defer conn.Close()
	buf := make([]byte, 256)
	if _, err := io.ReadFull(conn, buf[:2]); err != nil {
		return
	}
	nMethods := int(buf[1])
	io.ReadFull(conn, buf[:nMethods])
	conn.Write([]byte{0x05, 0x00})

	if _, err := io.ReadFull(conn, buf[:4]); err != nil {
		return
	}
	var host string
	switch buf[3] {
	case 0x01:
		ip := make([]byte, 4)
		io.ReadFull(conn, ip)
		host = net.IP(ip).String()
	case 0x03:
		var length byte
		binary.Read(conn, binary.BigEndian, &length)
		domain := make([]byte, length)
		io.ReadFull(conn, domain)
		host = string(domain)
	}
	var port uint16
	binary.Read(conn, binary.BigEndian, &port)
	targetAddr := fmt.Sprintf("%s:%d", host, port)

	proxyConn(conn, host, targetAddr, true)
}

func handleHTTP(conn net.Conn) {
	defer conn.Close()
	reader := bufio.NewReader(conn)
	req, err := http.ReadRequest(reader)
	if err != nil {
		return
	}
	host := req.URL.Hostname()
	port := req.URL.Port()
	if port == "" {
		port = "80"
		if req.Method == http.MethodConnect {
			port = "443"
		}
	}
	targetAddr := net.JoinHostPort(host, port)

	outboundTag := matchRoute(host)
	log.Printf("Route (HTTP %s): %s -> %s", req.Method, targetAddr, outboundTag)

	var remote net.Conn
	outbound := getOutbound(outboundTag)

	if outbound.Type == "vpn" {
		remote, err = connectToVPNServer(outbound, targetAddr)
	} else {
		remote, err = net.Dial("tcp", targetAddr)
	}

	if err != nil {
		return
	}
	defer remote.Close()

	if req.Method == http.MethodConnect {
		conn.Write([]byte("HTTP/1.1 200 Connection Established\r\n\r\n"))
	} else {
		// Forward the initial request
		req.Write(remote)
	}

	errChan := make(chan error, 2)
	go func() { _, e := io.Copy(remote, conn); errChan <- e }()
	go func() { _, e := io.Copy(conn, remote); errChan <- e }()
	<-errChan
}

func proxyConn(clientConn net.Conn, host, targetAddr string, isSocks bool) {
	outboundTag := matchRoute(host)
	log.Printf("Route: %s -> %s", targetAddr, outboundTag)

	var remote net.Conn
	var err error
	outbound := getOutbound(outboundTag)

	if outbound.Type == "vpn" {
		remote, err = connectToVPNServer(outbound, targetAddr)
	} else {
		remote, err = net.Dial("tcp", targetAddr)
	}

	if err != nil {
		if isSocks {
			clientConn.Write([]byte{0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0})
		}
		return
	}
	defer remote.Close()

	if isSocks {
		clientConn.Write([]byte{0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0})
	}

	errChan := make(chan error, 2)
	go func() { _, e := io.Copy(remote, clientConn); errChan <- e }()
	go func() { _, e := io.Copy(clientConn, remote); errChan <- e }()
	<-errChan
}

func matchRoute(host string) string {
	for _, rule := range routingRules {
		switch rule.Type {
		case "DOMAIN":
			if host == rule.Payload {
				return rule.Outbound
			}
		case "DOMAIN-SUFFIX":
			if strings.HasSuffix(host, rule.Payload) {
				return rule.Outbound
			}
		case "DOMAIN-KEYWORD":
			if strings.Contains(host, rule.Payload) {
				return rule.Outbound
			}
		case "MATCH":
			return rule.Outbound
		}
	}
	return "direct"
}

func getOutbound(tag string) Outbound {
	for _, o := range globalConfig.Outbounds {
		if o.Tag == tag {
			return o
		}
	}
	return Outbound{Type: "direct"}
}

func connectToVPNServer(o Outbound, targetAddr string) (net.Conn, error) {
	conn, err := net.Dial("tcp", fmt.Sprintf("%s:%d", o.Server, o.Port))
	if err != nil {
		return nil, err
	}

	// DIY Handshake
	return handshaker.ClientHandshake(conn, targetAddr)
}

func startDNSServer(dns DNSConfig) {
	addr, _ := net.ResolveUDPAddr("udp", dns.Listen)
	conn, err := net.ListenUDP("udp", addr)
	if err != nil {
		return
	}
	defer conn.Close()
	buf := make([]byte, 512)
	for {
		n, remoteAddr, err := conn.ReadFromUDP(buf)
		if err != nil {
			continue
		}
		go func(data []byte, addr *net.UDPAddr) {
			upstream, _ := net.Dial("udp", dns.Nameservers[0])
			defer upstream.Close()
			upstream.Write(data)
			resp := make([]byte, 512)
			rn, _ := upstream.Read(resp)
			conn.WriteToUDP(resp[:rn], addr)
		}(buf[:n], remoteAddr)
	}
}
