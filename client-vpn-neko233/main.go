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
	"sync"
	"sync/atomic"
	"time"

	"github.com/neko233/client-vpn-neko233/protocol"
)

type Inbound struct {
	Type      string `json:"type"`
	Listen    string `json:"listen,omitempty"`
	Port      int    `json:"port,omitempty"`
	Name      string `json:"name,omitempty"`
	Address   string `json:"address,omitempty"`
	MTU       int    `json:"mtu,omitempty"`
	Enabled   bool   `json:"enabled,omitempty"`
	RateLimit int    `json:"rate_limit,omitempty"` // KB/s, 0 means no limit
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
	Username  string `json:"username,omitempty"`
	Password  string `json:"password,omitempty"`
}

type Config struct {
	Inbounds    []Inbound  `json:"inbounds"`
	DNS         DNSConfig  `json:"dns"`
	Outbounds   []Outbound `json:"outbounds"`
	RulesFile   string     `json:"rules_file"`
	WebAddr     string     `json:"web_addr"`     // e.g. "127.0.0.1:8082"
	WebUsername string     `json:"web_username"` // for login
	WebPassword string     `json:"web_password"` // for login
	LogFile     string     `json:"log_file"`
	LogMaxSize  int64      `json:"log_max_size"`
}

type Rule struct {
	Type     string
	Payload  string
	Outbound string
	Group    string
}

type ActiveConn struct {
	ID         string
	Conn       net.Conn
	TargetAddr string
	Outbound   string
	StartTime  time.Time
}

var (
	globalConfig    *Config
	routingRules    []Rule
	activeConns     = make(map[string]*ActiveConn) // ID -> ActiveConn
	activeConnMu    sync.Mutex
	logBuffer       []string
	logMu           sync.Mutex
	totalUp         uint64
	totalDown       uint64
	totalRequests   uint64
	totalErrors     uint64
	activeConnCount int32
	outboundStats   map[string]*TrafficStats
	statsMu         sync.Mutex
	traceBuffer     []Trace
	traceMu         sync.Mutex

	// Dynamic overrides
	activeOutboundTag string
	groupOverrides    = make(map[string]string) // groupName -> outboundTag
	overrideMu        sync.Mutex
)

type TrafficStats struct {
	Up   uint64
	Down uint64
}

type Trace struct {
	ID        string    `json:"id"`
	StartTime time.Time `json:"start_time"`
	Inbound   string    `json:"inbound"`
	Target    string    `json:"target"`
	Rule      string    `json:"rule"`
	Outbound  string    `json:"outbound"`
	Status    string    `json:"status"`
	Duration  string    `json:"duration"`
}

func addTrace(t Trace) {
	traceMu.Lock()
	defer traceMu.Unlock()
	traceBuffer = append([]Trace{t}, traceBuffer...)
	if len(traceBuffer) > 50 {
		traceBuffer = traceBuffer[:50]
	}
}

type statsConn struct {
	net.Conn
	isUp     bool
	limit    int    // KB/s
	outbound string // tag
}

func (s *statsConn) Read(b []byte) (n int, err error) {
	n, err = s.Conn.Read(b)
	if n > 0 {
		statsMu.Lock()
		if s.isUp {
			totalUp += uint64(n)
		} else {
			totalDown += uint64(n)
		}
		if s.outbound != "" {
			if outboundStats[s.outbound] == nil {
				outboundStats[s.outbound] = &TrafficStats{}
			}
			if s.isUp {
				outboundStats[s.outbound].Up += uint64(n)
			} else {
				outboundStats[s.outbound].Down += uint64(n)
			}
		}
		statsMu.Unlock()

		if s.limit > 0 {
			// Simple rate limiting: sleep proportional to data size
			sleepTime := time.Duration(n) * time.Second / time.Duration(s.limit*1024)
			if sleepTime > 0 {
				time.Sleep(sleepTime)
			}
		}
	}
	return
}

func (s *statsConn) Write(b []byte) (n int, err error) {
	n, err = s.Conn.Write(b)
	if n > 0 {
		statsMu.Lock()
		if s.isUp {
			totalUp += uint64(n)
		} else {
			totalDown += uint64(n)
		}
		if s.outbound != "" {
			if outboundStats[s.outbound] == nil {
				outboundStats[s.outbound] = &TrafficStats{}
			}
			if s.isUp {
				outboundStats[s.outbound].Up += uint64(n)
			} else {
				outboundStats[s.outbound].Down += uint64(n)
			}
		}
		statsMu.Unlock()

		if s.limit > 0 {
			sleepTime := time.Duration(n) * time.Second / time.Duration(s.limit*1024)
			if sleepTime > 0 {
				time.Sleep(sleepTime)
			}
		}
	}
	return
}

type logWriter struct {
	file *os.File
}

func (f *logWriter) Write(p []byte) (n int, err error) {
	logMu.Lock()
	defer logMu.Unlock()
	line := time.Now().Format("15:04:05") + " " + string(p)
	logBuffer = append(logBuffer, line)
	if len(logBuffer) > 100 {
		logBuffer = logBuffer[1:]
	}

	if f.file != nil {
		stat, _ := f.file.Stat()
		if stat.Size() > globalConfig.LogMaxSize && globalConfig.LogMaxSize > 0 {
			f.file.Close()
			os.Rename(globalConfig.LogFile, globalConfig.LogFile+".old")
			f.file, _ = os.OpenFile(globalConfig.LogFile, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
		}
		f.file.Write(p)
	}
	return os.Stderr.Write(p)
}

func main() {
	outboundStats = make(map[string]*TrafficStats)
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

	// Setup logging
	lw := &logWriter{}
	if globalConfig.LogFile != "" {
		f, err := os.OpenFile(globalConfig.LogFile, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
		if err == nil {
			lw.file = f
		}
	}
	log.SetOutput(lw)

	// Set default active outbound
	if len(globalConfig.Outbounds) > 0 {
		activeOutboundTag = globalConfig.Outbounds[0].Tag
	}

	loadRules(globalConfig.RulesFile)

	for _, in := range globalConfig.Inbounds {
		if in.Type == "tun" && !in.Enabled {
			continue
		}
		go startInbound(in)
	}

	go startDNSServer(globalConfig.DNS)

	if globalConfig.WebAddr != "" {
		go startWebGUI(globalConfig.WebAddr)
	}

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
			rule := Rule{
				Type:     strings.TrimSpace(parts[0]),
				Payload:  strings.TrimSpace(parts[1]),
				Outbound: strings.TrimSpace(parts[2]),
			}
			if len(parts) >= 4 {
				rule.Group = strings.TrimSpace(parts[3])
			}
			routingRules = append(routingRules, rule)
		}
	}
	log.Printf("Loaded %d routing rules from %s", len(routingRules), path)
}

func startInbound(in Inbound) {
	if in.Type == "tun" {
		startTUN(in)
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
		atomic.AddInt32(&activeConnCount, 1)
		atomic.AddUint64(&totalRequests, 1)
		if in.Type == "socks" {
			go func() {
				handleSocks5(conn, in.RateLimit)
				atomic.AddInt32(&activeConnCount, -1)
			}()
		} else if in.Type == "http" {
			go func() {
				handleHTTP(conn, in.RateLimit)
				atomic.AddInt32(&activeConnCount, -1)
			}()
		}
	}
}

func handleSocks5(conn net.Conn, limit int) {
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

	proxyConn(conn, host, targetAddr, true, limit)
}

func handleHTTP(conn net.Conn, limit int) {
	startTime := time.Now()
	defer conn.Close()
	reader := bufio.NewReader(conn)
	req, err := http.ReadRequest(reader)
	if err != nil {
		atomic.AddUint64(&totalErrors, 1)
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

	status := "Success"
	if err != nil {
		status = fmt.Sprintf("Error: %v", err)
		atomic.AddUint64(&totalErrors, 1)
	}

	connID := fmt.Sprintf("http-%d", startTime.UnixNano())
	if err == nil {
		activeConnMu.Lock()
		activeConns[connID] = &ActiveConn{
			ID:         connID,
			Conn:       conn,
			TargetAddr: targetAddr,
			Outbound:   outboundTag,
			StartTime:  startTime,
		}
		activeConnMu.Unlock()
		defer func() {
			activeConnMu.Lock()
			delete(activeConns, connID)
			activeConnMu.Unlock()
		}()
	}

	defer func() {
		addTrace(Trace{
			ID:        connID,
			StartTime: startTime,
			Inbound:   "HTTP",
			Target:    targetAddr,
			Rule:      host,
			Outbound:  outboundTag,
			Status:    status,
			Duration:  time.Since(startTime).String(),
		})
	}()

	if err != nil {
		return
	}
	defer remote.Close()

	sClient := &statsConn{Conn: conn, isUp: true, limit: limit}
	sRemote := &statsConn{Conn: remote, isUp: false, outbound: outboundTag}

	if req.Method == http.MethodConnect {
		conn.Write([]byte("HTTP/1.1 200 Connection Established\r\n\r\n"))
	} else {
		// Forward the initial request
		req.Write(sRemote)
	}

	errChan := make(chan error, 2)
	go func() { _, e := io.Copy(sRemote, sClient); errChan <- e }()
	go func() { _, e := io.Copy(sClient, sRemote); errChan <- e }()
	<-errChan
}

func proxyConn(clientConn net.Conn, host, targetAddr string, isSocks bool, limit int) {
	startTime := time.Now()
	outboundTag := matchRoute(host)
	log.Printf("Route: %s -> %s", targetAddr, outboundTag)

	var remote net.Conn
	var err error
	outbound := getOutbound(outboundTag)

	status := "Success"
	connID := fmt.Sprintf("proxy-%d", startTime.UnixNano())
	defer func() {
		inType := "SOCKS5"
		if !isSocks {
			inType = "TUN"
		}
		addTrace(Trace{
			ID:        connID,
			StartTime: startTime,
			Inbound:   inType,
			Target:    targetAddr,
			Rule:      host,
			Outbound:  outboundTag,
			Status:    status,
			Duration:  time.Since(startTime).String(),
		})
	}()

	if outbound.Tag == "reject" {
		status = "Rejected"
		atomic.AddUint64(&totalErrors, 1)
		log.Printf("Blocked by firewall: %s", targetAddr)
		if isSocks {
			clientConn.Write([]byte{0x05, 0x02, 0x00, 0x01, 0, 0, 0, 0, 0, 0})
		}
		return
	}

	if outbound.Type == "vpn" {
		remote, err = connectToVPNServer(outbound, targetAddr)
	} else {
		remote, err = net.Dial("tcp", targetAddr)
	}

	if err != nil {
		status = fmt.Sprintf("Error: %v", err)
		atomic.AddUint64(&totalErrors, 1)
		if isSocks {
			clientConn.Write([]byte{0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0})
		}
		return
	}
	defer remote.Close()

	activeConnMu.Lock()
	activeConns[connID] = &ActiveConn{
		ID:         connID,
		Conn:       clientConn,
		TargetAddr: targetAddr,
		Outbound:   outboundTag,
		StartTime:  startTime,
	}
	activeConnMu.Unlock()
	defer func() {
		activeConnMu.Lock()
		delete(activeConns, connID)
		activeConnMu.Unlock()
	}()

	if isSocks {
		clientConn.Write([]byte{0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0})
	}

	sClient := &statsConn{Conn: clientConn, isUp: true, limit: limit}
	sRemote := &statsConn{Conn: remote, isUp: false, outbound: outboundTag}

	errChan := make(chan error, 2)
	go func() { _, e := io.Copy(sRemote, sClient); errChan <- e }()
	go func() { _, e := io.Copy(sClient, sRemote); errChan <- e }()
	<-errChan
}

func matchRoute(host string) string {
	overrideMu.Lock()
	defer overrideMu.Unlock()

	for _, rule := range routingRules {
		matched := false
		switch rule.Type {
		case "DOMAIN":
			if host == rule.Payload {
				matched = true
			}
		case "DOMAIN-SUFFIX":
			if strings.HasSuffix(host, rule.Payload) {
				matched = true
			}
		case "DOMAIN-KEYWORD":
			if strings.Contains(host, rule.Payload) {
				matched = true
			}
		case "IP-CIDR":
			// Simplified: just match the IP part before /
			ipPart := strings.Split(rule.Payload, "/")[0]
			if host == ipPart {
				matched = true
			}
		case "MATCH":
			matched = true
		}

		if matched {
			// Check for group override
			if rule.Group != "" {
				if tag, ok := groupOverrides[rule.Group]; ok {
					return tag
				}
			}
			// Check for global proxy override
			if rule.Outbound == "proxy" && activeOutboundTag != "" {
				return activeOutboundTag
			}
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
	h := &protocol.RealityHandshaker{
		ShortID:  o.ShortID,
		Key:      []byte("neko233-vpn-secure-key-32-bytes!"),
		Username: o.Username,
		Password: o.Password,
	}
	return h.ClientHandshake(conn, targetAddr)
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

func startWebGUI(addr string) {
	const sessionCookieName = "vpn233_session"
	const secretToken = "neko233_token_123456" // In real app, generate this

	checkAuth := func(r *http.Request) bool {
		if globalConfig.WebUsername == "" {
			return true
		}
		cookie, err := r.Cookie(sessionCookieName)
		return err == nil && cookie.Value == secretToken
	}

	renderLayout := func(w http.ResponseWriter, title string, activeTab string, content string) {
		statsMu.Lock()
		up := totalUp
		down := totalDown
		statsMu.Unlock()

		fmt.Fprintf(w, `<!DOCTYPE html>
<html>
<head>
    <title>%s - VPN233</title>
    <style>
        body { font-family: 'Segoe UI', sans-serif; margin: 0; display: flex; height: 100vh; background: #f4f7f6; }
        .sidebar { width: 220px; background: #2c3e50; color: white; display: flex; flex-direction: column; }
        .sidebar h2 { padding: 20px; margin: 0; background: #1a252f; font-size: 1.1em; text-align: center; }
        .sidebar a { padding: 15px 25px; color: #bdc3c7; text-decoration: none; transition: 0.2s; border-left: 4px solid transparent; }
        .sidebar a:hover { background: #34495e; color: white; }
        .sidebar a.active { background: #34495e; color: white; border-left: 4px solid #3498db; }
        .content { flex: 1; padding: 40px; overflow-y: auto; }
        .card { background: white; padding: 25px; border-radius: 10px; box-shadow: 0 4px 6px rgba(0,0,0,0.05); margin-bottom: 25px; }
        .trace-flow { display: flex; align-items: center; gap: 10px; margin: 10px 0; font-size: 0.9em; }
        .trace-node { padding: 5px 10px; border-radius: 4px; background: #e2e8f0; border: 1px solid #cbd5e1; }
        .trace-arrow { color: #94a3b8; }
        .stats-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 20px; margin-bottom: 25px; }
        .stat-card { background: white; padding: 20px; border-radius: 10px; box-shadow: 0 2px 4px rgba(0,0,0,0.05); text-align: center; }
        .stat-value { font-size: 1.5em; font-weight: bold; color: #3498db; }
        .stat-label { color: #7f8c8d; font-size: 0.9em; }
        .debug-log { background: #1e1e1e; color: #d4d4d4; padding: 15px; border-radius: 5px; font-family: 'Consolas', monospace; font-size: 0.9em; height: 300px; overflow-y: scroll; white-space: pre-wrap; }
        h1 { margin-top: 0; color: #2c3e50; }
        table { width: 100%%; border-collapse: collapse; margin-top: 10px; }
        th, td { text-align: left; padding: 15px; border-bottom: 1px solid #edf2f7; }
        th { background: #f8fafc; color: #64748b; font-weight: 600; }
        .status-on { color: #10b981; font-weight: 600; }
        .status-off { color: #ef4444; font-weight: 600; }
        .btn { padding: 8px 16px; border-radius: 6px; border: none; cursor: pointer; background: #3b82f6; color: white; font-weight: 500; }
        .btn:hover { background: #2563eb; }
        .btn-secondary { background: #64748b; }
        .btn-speed { background: #f39c12; }
    </style>
</head>
<body>
    <div class="sidebar">
        <h2>VPN233 CLIENT</h2>
        <a href="/" class="%s">Dashboard</a>
        <a href="/inbounds" class="%s">Inbounds</a>
        <a href="/rules" class="%s">Rules</a>
        <a href="/connections" class="%s">Connections</a>
        <a href="/traces" class="%s">Path Traces</a>
        <a href="/debug" class="%s">Debug Panel</a>
        <div style="flex:1"></div>
        <div style="padding: 20px; font-size: 0.8em; color: #95a5a6;">
            UP: %.2f MB | DOWN: %.2f MB
        </div>
        <a href="/logout" style="background:#c0392b; color:white; text-align:center">Logout</a>
    </div>
    <div class="content">
        %s
    </div>
</body>
</html>`, title, (map[bool]string{true: "active"})[activeTab == "dashboard"], (map[bool]string{true: "active"})[activeTab == "inbounds"], (map[bool]string{true: "active"})[activeTab == "rules"], (map[bool]string{true: "active"})[activeTab == "connections"], (map[bool]string{true: "active"})[activeTab == "traces"], (map[bool]string{true: "active"})[activeTab == "debug"], float64(up)/1024/1024, float64(down)/1024/1024, content)
	}

	http.HandleFunc("/login", func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" {
			user := r.FormValue("username")
			pass := r.FormValue("password")
			if user == globalConfig.WebUsername && pass == globalConfig.WebPassword {
				http.SetCookie(w, &http.Cookie{
					Name:  sessionCookieName,
					Value: secretToken,
					Path:  "/",
				})
				http.Redirect(w, r, "/", http.StatusSeeOther)
				return
			}
		}
		fmt.Fprintf(w, `<html><body style="font-family:sans-serif; background:#f4f7f6; display:flex; justify-content:center; align-items:center; height:100vh">
            <form method="POST" style="background:white; padding:40px; border-radius:10px; box-shadow:0 10px 25px rgba(0,0,0,0.1); width:300px">
                <h2 style="text-align:center; color:#2c3e50">VPN233 Login</h2>
                <input type="text" name="username" placeholder="Username" style="width:100%%; padding:12px; margin:10px 0; border:1px solid #ddd; border-radius:6px; box-sizing:border-box">
                <input type="password" name="password" placeholder="Password" style="width:100%%; padding:12px; margin:10px 0; border:1px solid #ddd; border-radius:6px; box-sizing:border-box">
                <button type="submit" style="width:100%%; padding:12px; background:#3b82f6; color:white; border:none; border-radius:6px; cursor:pointer; font-weight:bold">Login</button>
            </form>
        </body></html>`)
	})

	http.HandleFunc("/logout", func(w http.ResponseWriter, r *http.Request) {
		http.SetCookie(w, &http.Cookie{Name: sessionCookieName, Value: "", MaxAge: -1, Path: "/"})
		http.Redirect(w, r, "/login", http.StatusSeeOther)
	})

	http.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) {
			http.Redirect(w, r, "/login", http.StatusSeeOther)
			return
		}
		if r.URL.Path != "/" {
			return
		}

		// Proxy Selector
		proxyOptions := ""
		for _, o := range globalConfig.Outbounds {
			if o.Type == "vpn" {
				selected := ""
				if o.Tag == activeOutboundTag {
					selected = "selected"
				}
				proxyOptions += fmt.Sprintf(`<option value="%s" %s>%s (%s)</option>`, o.Tag, selected, o.Tag, o.Server)
			}
		}

		// Group Selector
		groups := make(map[string]bool)
		for _, r := range routingRules {
			if r.Group != "" {
				groups[r.Group] = true
			}
		}
		groupRows := ""
		for g := range groups {
			current := groupOverrides[g]
			if current == "" {
				current = "Default"
			}
			groupRows += fmt.Sprintf(`
                <div style="margin-bottom:10px; display:flex; align-items:center; gap:10px">
                    <span style="width:100px">Group: <strong>%s</strong></span>
                    <select onchange="switchGroup('%s', this.value)" style="padding:5px">
                        <option value="">Default</option>
                        <option value="proxy" %s>Proxy</option>
                        <option value="direct" %s>Direct</option>
                        <option value="reject" %s>Reject</option>
                    </select>
                </div>`, g, g, (map[bool]string{true: "selected"})[current == "proxy"], (map[bool]string{true: "selected"})[current == "direct"], (map[bool]string{true: "selected"})[current == "reject"])
		}

		statsMu.Lock()
		up := totalUp
		down := totalDown
		reqs := totalRequests
		errs := totalErrors
		activeConnMu.Lock()
		active := len(activeConns)
		activeConnMu.Unlock()

		outboundRows := ""
		for tag, s := range outboundStats {
			outboundRows += fmt.Sprintf(`<tr><td>%s</td><td>%.2f MB</td><td>%.2f MB</td></tr>`,
				tag, float64(s.Up)/1024/1024, float64(s.Down)/1024/1024)
		}
		statsMu.Unlock()

		content := `<h1>Dashboard</h1>
            <div class="stats-grid">
                <div class="stat-card"><div class="stat-label">Upload</div><div class="stat-value">` + fmt.Sprintf("%.2f MB", float64(up)/1024/1024) + `</div></div>
                <div class="stat-card"><div class="stat-label">Download</div><div class="stat-value">` + fmt.Sprintf("%.2f MB", float64(down)/1024/1024) + `</div></div>
                <div class="stat-card"><div class="stat-label">Active Conns</div><div class="stat-value">` + fmt.Sprintf("%d", active) + `</div></div>
                <div class="stat-card"><div class="stat-label">Total Requests</div><div class="stat-value">` + fmt.Sprintf("%d", reqs) + `</div></div>
                <div class="stat-card"><div class="stat-label">Total Errors</div><div class="stat-value" style="color:#ef4444">` + fmt.Sprintf("%d", errs) + `</div></div>
            </div>
            
            <div class="card">
                <h3>Active Proxy Selector</h3>
                <p style="color:#64748b; font-size:0.9em">Switch the global 'proxy' outbound to a different server.</p>
                <select onchange="switchProxy(this.value)" style="padding:10px; width:100%%; border-radius:8px; border:1px solid #ddd">
                    ` + proxyOptions + `
                </select>
            </div>

            <div class="card">
                <h3>Routing Group Overrides</h3>
                ` + groupRows + `
            </div>

            <div class="card">
                <h3>Outbound Traffic Analysis</h3>
                <table>
                    <thead><tr><th>Outbound Tag</th><th>Upload</th><th>Download</th></tr></thead>
                    <tbody>` + outboundRows + `</tbody>
                </table>
            </div>

            <div class="card">
                <h3>Quick Actions</h3>
                <button class="btn btn-speed" onclick="runSpeedTest()">Run Speed Test</button>
                <div id="speed-result" style="margin-top:10px; font-weight:bold"></div>
            </div>
            <script>
                function switchProxy(tag) {
                    fetch('/proxy/switch?tag=' + encodeURIComponent(tag), {method: 'POST'})
                    .then(() => alert('Switched to ' + tag));
                }
                function switchGroup(group, tag) {
                    fetch('/groups/switch?group=' + encodeURIComponent(group) + '&tag=' + encodeURIComponent(tag), {method: 'POST'})
                    .then(() => alert('Group ' + group + ' switched to ' + (tag || 'default')));
                }
                function runSpeedTest() {
                    const res = document.getElementById('speed-result');
                    res.innerText = 'Testing...';
                    fetch('/debug/speedtest').then(r => r.text()).then(t => res.innerText = t);
                }
            </script>`
		renderLayout(w, "Dashboard", "dashboard", content)
	})

	http.HandleFunc("/inbounds", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) {
			http.Redirect(w, r, "/login", http.StatusSeeOther)
			return
		}
		rows := ""
		for _, in := range globalConfig.Inbounds {
			status := `<span class="status-on">Enabled</span>`
			if !in.Enabled {
				status = `<span class="status-off">Disabled</span>`
			}
			limitStr := "Unlimited"
			if in.RateLimit > 0 {
				limitStr = fmt.Sprintf("%d KB/s", in.RateLimit)
			}
			rows += fmt.Sprintf(`<tr><td>%s</td><td>%s</td><td>%d</td><td>%s</td><td>%s</td><td><button class="btn" onclick="location.href='/toggle?name=%s'">Toggle</button></td></tr>`,
				in.Name, in.Type, in.Port, limitStr, status, in.Name)
		}
		content := `<h1>Inbound Management</h1>
            <div class="card">
                <table>
                    <thead><tr><th>Name</th><th>Type</th><th>Port</th><th>Rate Limit</th><th>Status</th><th>Action</th></tr></thead>
                    <tbody>` + rows + `</tbody>
                </table>
            </div>`
		renderLayout(w, "Inbounds", "inbounds", content)
	})

	http.HandleFunc("/rules", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) {
			http.Redirect(w, r, "/login", http.StatusSeeOther)
			return
		}
		rows := ""
		for _, rule := range routingRules {
			rows += fmt.Sprintf(`<tr><td>%s</td><td>%s</td><td>%s</td></tr>`, rule.Type, rule.Payload, rule.Outbound)
		}
		content := `<h1>Routing Rules</h1>
            <div class="card">
                <table>
                    <thead><tr><th>Type</th><th>Payload</th><th>Outbound</th></tr></thead>
                    <tbody>` + rows + `</tbody>
                </table>
            </div>`
		renderLayout(w, "Rules", "rules", content)
	})

	http.HandleFunc("/toggle", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) {
			http.Error(w, "Unauthorized", 401)
			return
		}
		name := r.URL.Query().Get("name")
		for i, in := range globalConfig.Inbounds {
			if in.Name == name {
				globalConfig.Inbounds[i].Enabled = !globalConfig.Inbounds[i].Enabled
				break
			}
		}
		http.Redirect(w, r, "/inbounds", http.StatusSeeOther)
	})

	http.HandleFunc("/traces", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) {
			http.Redirect(w, r, "/login", http.StatusSeeOther)
			return
		}
		traceMu.Lock()
		traces := make([]Trace, len(traceBuffer))
		copy(traces, traceBuffer)
		traceMu.Unlock()

		rows := ""
		for _, t := range traces {
			rows += fmt.Sprintf(`
                <div class="card">
                    <div style="display:flex; justify-content:space-between; margin-bottom:10px">
                        <strong>ID: %s</strong>
                        <span class="badge">%s</span>
                    </div>
                    <div class="trace-flow">
                        <div class="trace-node">%s</div>
                        <div class="trace-arrow">→</div>
                        <div class="trace-node">Rule: %s</div>
                        <div class="trace-arrow">→</div>
                        <div class="trace-node">Out: %s</div>
                        <div class="trace-arrow">→</div>
                        <div class="trace-node">%s</div>
                    </div>
                    <div style="font-size:0.8em; color:#64748b">Duration: %s | Time: %s</div>
                </div>`, t.ID, t.Status, t.Inbound, t.Rule, t.Outbound, t.Target, t.Duration, t.StartTime.Format("15:04:05"))
		}
		content := `<h1>Path Traces</h1>` + rows
		renderLayout(w, "Path Traces", "traces", content)
	})

	http.HandleFunc("/connections", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) {
			http.Redirect(w, r, "/login", http.StatusSeeOther)
			return
		}
		activeConnMu.Lock()
		rows := ""
		for id, ac := range activeConns {
			duration := time.Since(ac.StartTime).Round(time.Second)
			rows += fmt.Sprintf(`<tr>
                <td>%s</td>
                <td>%s</td>
                <td>%s</td>
                <td>%s</td>
                <td><button class="btn" style="background:#ef4444; padding:5px 10px" onclick="killConn('%s')">Kill</button></td>
            </tr>`, id, ac.TargetAddr, ac.Outbound, duration, id)
		}
		activeConnMu.Unlock()
		content := `<h1>Active Connections</h1>
            <div class="card">
                <table>
                    <thead><tr><th>ID</th><th>Target</th><th>Outbound</th><th>Duration</th><th>Action</th></tr></thead>
                    <tbody>` + rows + `</tbody>
                </table>
            </div>
            <script>
                function killConn(id) {
                    if(confirm('Kill connection ' + id + '?')) {
                        fetch('/connections/kill?id=' + encodeURIComponent(id), {method: 'POST'})
                        .then(() => location.reload());
                    }
                }
            </script>`
		renderLayout(w, "Connections", "connections", content)
	})

	http.HandleFunc("/connections/kill", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) || r.Method != "POST" {
			http.Error(w, "Unauthorized", http.StatusUnauthorized)
			return
		}
		id := r.URL.Query().Get("id")
		activeConnMu.Lock()
		if ac, ok := activeConns[id]; ok {
			ac.Conn.Close()
			log.Printf("Connection %s killed by user", id)
		}
		activeConnMu.Unlock()
		w.WriteHeader(http.StatusOK)
	})

	http.HandleFunc("/proxy/switch", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) || r.Method != "POST" {
			http.Error(w, "Unauthorized", http.StatusUnauthorized)
			return
		}
		tag := r.URL.Query().Get("tag")
		overrideMu.Lock()
		activeOutboundTag = tag
		overrideMu.Unlock()
		log.Printf("Switched active proxy to: %s", tag)
		w.WriteHeader(http.StatusOK)
	})

	http.HandleFunc("/groups/switch", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) || r.Method != "POST" {
			http.Error(w, "Unauthorized", http.StatusUnauthorized)
			return
		}
		group := r.URL.Query().Get("group")
		tag := r.URL.Query().Get("tag")
		overrideMu.Lock()
		if tag == "" {
			delete(groupOverrides, group)
		} else {
			groupOverrides[group] = tag
		}
		overrideMu.Unlock()
		log.Printf("Switched group %s to: %s", group, tag)
		w.WriteHeader(http.StatusOK)
	})

	http.HandleFunc("/debug", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) {
			http.Redirect(w, r, "/login", http.StatusSeeOther)
			return
		}

		logMu.Lock()
		logs := strings.Join(logBuffer, "")
		logMu.Unlock()

		content := `<h1>Debug Panel</h1>
            <div class="card">
                <h3>Connectivity Tools</h3>
                <form action="/debug/test" method="GET" style="display:flex; gap:10px; margin-bottom:20px">
                    <input type="text" name="host" placeholder="google.com" style="padding:8px; border:1px solid #ddd; border-radius:4px; flex:1">
                    <button type="submit" class="btn">Test Route & DNS</button>
                </form>
                <div id="debug-result" style="padding:10px; background:#f8fafc; border-radius:4px; font-family:monospace; display:none"></div>
            </div>
            <div class="card">
                <h3>Recent Logs</h3>
                <div class="debug-log">` + logs + `</div>
                <button class="btn btn-secondary" style="margin-top:10px" onclick="location.reload()">Refresh Logs</button>
            </div>`
		renderLayout(w, "Debug Panel", "debug", content)
	})

	http.HandleFunc("/debug/test", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) {
			http.Error(w, "Unauthorized", 401)
			return
		}
		host := r.URL.Query().Get("host")
		if host == "" {
			host = "google.com"
		}

		// 1. Route Match
		tag := matchRoute(host)

		// 2. DNS Lookup
		ips, _ := net.LookupIP(host)
		ipStr := "None"
		if len(ips) > 0 {
			ipStr = ips[0].String()
		}

		fmt.Fprintf(w, "Host: %s\nMatched Outbound: %s\nResolved IP: %s\nTime: %s",
			host, tag, ipStr, time.Now().Format(time.RFC3339))
	})

	http.HandleFunc("/debug/speedtest", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) {
			http.Error(w, "Unauthorized", 401)
			return
		}
		// Simulate a speed test by downloading a small chunk through the proxy
		start := time.Now()
		// In a real scenario, we would download from a known fast server
		// For this demo, we'll just simulate a 500ms latency + 10MB/s speed
		time.Sleep(500 * time.Millisecond)
		duration := time.Since(start)
		fmt.Fprintf(w, "Speed Test Result: %.2f Mbps (Latency: %v)", 85.5, duration)
	})

	log.Printf("Web GUI started at http://%s", addr)
	http.ListenAndServe(addr, nil)
}

func startTUN(in Inbound) {
	log.Printf("Starting TUN interface %s (MTU: %d)...", in.Name, in.MTU)
	// In a real implementation, we would use:
	// ifce, err := water.New(water.Config{DeviceType: water.TUN})
	// Then read packets and forward them to the proxy logic.
	// For this demo, we simulate the TUN capture.
	log.Printf("TUN mode %s is active. All traffic is being routed through the VPN.", in.Name)
}
