package main

import (
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
	"time"

	"github.com/neko233/server-vpn-neko233/protocol"
)

type User struct {
	Username string `json:"username"`
	Password string `json:"password"`
	UUID     string `json:"uuid"`
}

type DailyStats struct {
	Date string `json:"date"`
	Up   uint64 `json:"up"`
	Down uint64 `json:"down"`
}

type UserData struct {
	User  User         `json:"user"`
	Stats []DailyStats `json:"stats"`
}

type Config struct {
	ListenAddr  string   `json:"listen_addr"`
	DestAddr    string   `json:"dest_addr"`
	PrivateKey  string   `json:"private_key"`
	ShortIDs    []string `json:"short_ids"`
	Users       []User   `json:"users"`
	WebAddr     string   `json:"web_addr"`     // e.g. "0.0.0.0:8081"
	WebUsername string   `json:"web_username"` // for login
	WebPassword string   `json:"web_password"` // for login
	LogFile     string   `json:"log_file"`
	LogMaxSize  int64    `json:"log_max_size"` // in bytes
}

type ActiveConn struct {
	Conn       net.Conn
	TargetAddr string
	Username   string
	StartTime  time.Time
}

var (
	activeConns = make(map[string]*ActiveConn) // remoteAddr -> ActiveConn
	logBuffer   []string
	logMu       sync.Mutex
	config      Config
	userStore   *UserStore
)

type UserStore struct {
	mu    sync.Mutex
	dir   string
	users map[string]*UserData
}

type statsConn struct {
	net.Conn
	username string
	isUp     bool
}

func (s *statsConn) Read(b []byte) (n int, err error) {
	n, err = s.Conn.Read(b)
	if n > 0 {
		if s.isUp {
			userStore.AddTraffic(s.username, uint64(n), 0) // Read from client = Upload
		} else {
			userStore.AddTraffic(s.username, 0, uint64(n)) // Read from target = Download
		}
	}
	return
}

func (s *statsConn) Write(b []byte) (n int, err error) {
	n, err = s.Conn.Write(b)
	if n > 0 {
		if s.isUp {
			userStore.AddTraffic(s.username, 0, uint64(n)) // Write to client = Download
		} else {
			userStore.AddTraffic(s.username, uint64(n), 0) // Write to target = Upload
		}
	}
	return
}

func NewUserStore(dir string) *UserStore {
	os.MkdirAll(dir, 0755)
	s := &UserStore{
		dir:   dir,
		users: make(map[string]*UserData),
	}
	s.loadAll()
	return s
}

func (s *UserStore) loadAll() {
	files, _ := os.ReadDir(s.dir)
	for _, f := range files {
		if !f.IsDir() && strings.HasSuffix(f.Name(), ".json") {
			username := strings.TrimSuffix(f.Name(), ".json")
			data, err := os.ReadFile(s.dir + "/" + f.Name())
			if err == nil {
				var ud UserData
				if json.Unmarshal(data, &ud) == nil {
					s.users[username] = &ud
				}
			}
		}
	}
}

func (s *UserStore) SaveUser(ud *UserData) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.users[ud.User.Username] = ud
	data, _ := json.MarshalIndent(ud, "", "  ")
	os.WriteFile(s.dir+"/"+ud.User.Username+".json", data, 0644)
}

func (s *UserStore) GetUser(username string) *UserData {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.users[username]
}

func (s *UserStore) AddTraffic(username string, up, down uint64) {
	s.mu.Lock()
	defer s.mu.Unlock()
	ud, ok := s.users[username]
	if !ok {
		return
	}
	today := time.Now().Format("2006-01-02")
	found := false
	for i, st := range ud.Stats {
		if st.Date == today {
			ud.Stats[i].Up += up
			ud.Stats[i].Down += down
			found = true
			break
		}
	}
	if !found {
		ud.Stats = append(ud.Stats, DailyStats{Date: today, Up: up, Down: down})
	}
	// Save periodically or on change
	data, _ := json.MarshalIndent(ud, "", "  ")
	os.WriteFile(s.dir+"/"+username+".json", data, 0644)
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
		if stat.Size() > config.LogMaxSize && config.LogMaxSize > 0 {
			f.file.Close()
			os.Rename(config.LogFile, config.LogFile+".old")
			f.file, _ = os.OpenFile(config.LogFile, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
		}
		f.file.Write(p)
	}
	return os.Stderr.Write(p)
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

	if err := json.NewDecoder(configFile).Decode(&config); err != nil {
		log.Fatalf("Failed to decode config: %v", err)
	}

	// Setup logging
	lw := &logWriter{}
	if config.LogFile != "" {
		f, err := os.OpenFile(config.LogFile, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
		if err == nil {
			lw.file = f
		}
	}
	log.SetOutput(lw)

	userStore = NewUserStore("data/users")
	// Sync config users to store if not exists
	for _, u := range config.Users {
		if userStore.GetUser(u.Username) == nil {
			userStore.SaveUser(&UserData{User: u, Stats: []DailyStats{}})
		}
	}

	// Initialize default handshaker (DIY: others can replace this)
	handshaker := &protocol.RealityHandshaker{
		ShortIDs: config.ShortIDs,
		Key:      []byte("neko233-vpn-secure-key-32-bytes!"), // Should be from config
		VerifyUser: func(username, password string) bool {
			ud := userStore.GetUser(username)
			return ud != nil && ud.User.Password == password
		},
	}

	listener, err := net.Listen("tcp", config.ListenAddr)
	if err != nil {
		log.Fatalf("Failed to listen on %s: %v", config.ListenAddr, err)
	}
	log.Printf("Server listening on %s", config.ListenAddr)

	if config.WebAddr != "" {
		go startWebAdmin(config.WebAddr)
	}

	for {
		conn, err := listener.Accept()
		if err != nil {
			continue
		}
		go handleConnection(conn, handshaker, userStore)
	}
}

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

func handleConnection(conn net.Conn, h protocol.Handshaker, userStore *UserStore) {
	defer conn.Close()

	// DIY Handshake
	vpnConn, targetAddr, username, err := h.ServerHandshake(conn)
	if err != nil {
		// Fallback to destination (REALITY behavior)
		log.Printf("Non-VPN or Unauthorized connection, forwarding to %s", config.DestAddr)
		if vpnConn != nil {
			forwardToDest(vpnConn, nil, config.DestAddr)
		} else {
			forwardToDest(conn, nil, config.DestAddr)
		}
		return
	}

	log.Printf("VPN Secure Request from %s to: %s", username, targetAddr)

	remoteAddr := conn.RemoteAddr().String()
	activeConns[remoteAddr] = &ActiveConn{
		Conn:       conn,
		TargetAddr: targetAddr,
		Username:   username,
		StartTime:  time.Now(),
	}
	defer delete(activeConns, remoteAddr)

	// Connect to target
	targetConn, err := net.Dial("tcp", targetAddr)
	if err != nil {
		log.Printf("Failed to connect to target %s: %v", targetAddr, err)
		return
	}
	defer targetConn.Close()

	sClient := &statsConn{Conn: vpnConn, username: username, isUp: true}
	sTarget := &statsConn{Conn: targetConn, username: username, isUp: false}

	// Bidirectional copy
	errChan := make(chan error, 2)
	go func() { _, e := io.Copy(sTarget, sClient); errChan <- e }()
	go func() { _, e := io.Copy(sClient, sTarget); errChan <- e }()
	<-errChan
}

func forwardToDest(conn net.Conn, initialData []byte, destAddr string) {
	destConn, err := net.Dial("tcp", destAddr)
	if err != nil {
		log.Printf("Failed to connect to fallback dest %s: %v", destAddr, err)
		return
	}
	defer destConn.Close()

	if len(initialData) > 0 {
		if _, err := destConn.Write(initialData); err != nil {
			log.Printf("Failed to write initial data to dest %s: %v", destAddr, err)
			return
		}
	}

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

func startWebAdmin(addr string) {
	const sessionCookieName = "vpn233_admin_session"
	const secretToken = "neko233_admin_token_654321"

	checkAuth := func(r *http.Request) bool {
		if config.WebUsername == "" {
			return true
		}
		cookie, err := r.Cookie(sessionCookieName)
		return err == nil && cookie.Value == secretToken
	}

	renderLayout := func(w http.ResponseWriter, title string, activeTab string, content string) {
		fmt.Fprintf(w, `<!DOCTYPE html>
<html>
<head>
    <title>%s - VPN233 Admin</title>
    <style>
        body { font-family: 'Segoe UI', sans-serif; margin: 0; display: flex; height: 100vh; background: #f0f2f5; }
        .sidebar { width: 240px; background: #1e293b; color: white; display: flex; flex-direction: column; }
        .sidebar h2 { padding: 25px; margin: 0; background: #0f172a; font-size: 1.2em; text-align: center; letter-spacing: 1px; }
        .sidebar a { padding: 15px 30px; color: #94a3b8; text-decoration: none; transition: 0.2s; border-left: 4px solid transparent; }
        .sidebar a:hover { background: #334155; color: white; }
        .sidebar a.active { background: #334155; color: white; border-left: 4px solid #60a5fa; }
        .content { flex: 1; padding: 40px; overflow-y: auto; }
        .card { background: white; padding: 30px; border-radius: 12px; box-shadow: 0 4px 20px rgba(0,0,0,0.08); margin-bottom: 30px; }
        .debug-log { background: #1e1e1e; color: #d4d4d4; padding: 15px; border-radius: 5px; font-family: 'Consolas', monospace; font-size: 0.9em; height: 300px; overflow-y: scroll; white-space: pre-wrap; }
        h1 { margin-top: 0; color: #1e293b; font-size: 1.8em; }
        table { width: 100%%; border-collapse: collapse; }
        th, td { text-align: left; padding: 18px; border-bottom: 1px solid #f1f5f9; }
        th { background: #f8fafc; color: #475569; font-weight: 600; text-transform: uppercase; font-size: 0.85em; }
        .status-on { color: #059669; font-weight: 600; }
        .badge { padding: 4px 10px; border-radius: 20px; background: #dcfce7; color: #166534; font-size: 0.85em; }
        .btn { padding: 10px 20px; border-radius: 8px; border: none; cursor: pointer; background: #2563eb; color: white; font-weight: 500; }
    </style>
</head>
<body>
    <div class="sidebar">
        <h2>VPN233 ADMIN</h2>
        <a href="/" class="%s">Dashboard</a>
        <a href="/connections" class="%s">Connections</a>
        <a href="/users" class="%s">Users</a>
        <a href="/debug" class="%s">Debug Panel</a>
        <div style="flex:1"></div>
        <a href="/logout" style="background:#ef4444; color:white; text-align:center">Logout</a>
    </div>
    <div class="content">
        %s
    </div>
</body>
</html>`, title, (map[bool]string{true: "active"})[activeTab == "dashboard"], (map[bool]string{true: "active"})[activeTab == "connections"], (map[bool]string{true: "active"})[activeTab == "users"], (map[bool]string{true: "active"})[activeTab == "debug"], content)
	}

	http.HandleFunc("/login", func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" {
			user := r.FormValue("username")
			pass := r.FormValue("password")
			if user == config.WebUsername && pass == config.WebPassword {
				http.SetCookie(w, &http.Cookie{Name: sessionCookieName, Value: secretToken, Path: "/"})
				http.Redirect(w, r, "/", http.StatusSeeOther)
				return
			}
		}
		fmt.Fprintf(w, `<html><body style="font-family:sans-serif; background:#f0f2f5; display:flex; justify-content:center; align-items:center; height:100vh">
            <form method="POST" style="background:white; padding:50px; border-radius:15px; box-shadow:0 20px 40px rgba(0,0,0,0.1); width:350px">
                <h2 style="text-align:center; color:#1e293b; margin-bottom:30px">Admin Portal</h2>
                <input type="text" name="username" placeholder="Admin Username" style="width:100%%; padding:14px; margin:12px 0; border:1px solid #e2e8f0; border-radius:8px; box-sizing:border-box">
                <input type="password" name="password" placeholder="Admin Password" style="width:100%%; padding:14px; margin:12px 0; border:1px solid #e2e8f0; border-radius:8px; box-sizing:border-box">
                <button type="submit" style="width:100%%; padding:14px; background:#2563eb; color:white; border:none; border-radius:8px; cursor:pointer; font-weight:bold; margin-top:20px">Sign In</button>
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
		content := `<h1>Server Status</h1>
            <div class="card">
                <p><strong>Status:</strong> <span class="badge">Online</span></p>
                <p><strong>Listen Address:</strong> ` + config.ListenAddr + `</p>
                <p><strong>Active Connections:</strong> ` + fmt.Sprint(len(activeConns)) + `</p>
            </div>`
		renderLayout(w, "Dashboard", "dashboard", content)
	})

	http.HandleFunc("/connections", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) {
			http.Redirect(w, r, "/login", http.StatusSeeOther)
			return
		}
		rows := ""
		for addr, ac := range activeConns {
			duration := time.Since(ac.StartTime).Round(time.Second)
			rows += fmt.Sprintf(`<tr>
                <td>%s</td>
                <td>%s</td>
                <td>%s</td>
                <td>%s</td>
                <td><span class="status-on">Active</span></td>
                <td><button class="btn" style="background:#ef4444; padding:5px 10px" onclick="killConn('%s')">Kill</button></td>
            </tr>`, ac.Username, addr, ac.TargetAddr, duration, addr)
		}
		content := `<h1>Active Connections</h1>
            <div class="card">
                <table>
                    <thead><tr><th>User</th><th>Client IP</th><th>Target</th><th>Duration</th><th>Status</th><th>Action</th></tr></thead>
                    <tbody>` + rows + `</tbody>
                </table>
            </div>
            <script>
                function killConn(addr) {
                    if(confirm('Kill connection ' + addr + '?')) {
                        fetch('/connections/kill?addr=' + encodeURIComponent(addr), {method: 'POST'})
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
		addr := r.URL.Query().Get("addr")
		if ac, ok := activeConns[addr]; ok {
			ac.Conn.Close()
			log.Printf("Connection %s killed by admin", addr)
		}
		w.WriteHeader(http.StatusOK)
	})

	http.HandleFunc("/users", func(w http.ResponseWriter, r *http.Request) {
		if !checkAuth(r) {
			http.Redirect(w, r, "/login", http.StatusSeeOther)
			return
		}
		rows := ""
		userStore.mu.Lock()
		for _, ud := range userStore.users {
			totalUp := uint64(0)
			totalDown := uint64(0)
			dailyRows := ""
			for _, s := range ud.Stats {
				totalUp += s.Up
				totalDown += s.Down
				dailyRows += fmt.Sprintf(`<div>%s: Up %.2f MB, Down %.2f MB</div>`, s.Date, float64(s.Up)/1024/1024, float64(s.Down)/1024/1024)
			}
			rows += fmt.Sprintf(`
                <div class="card">
                    <h3>User: %s</h3>
                    <p><strong>UUID:</strong> <code>%s</code></p>
                    <p><strong>Total Traffic:</strong> Up %.2f MB, Down %.2f MB</p>
                    <details>
                        <summary>Daily Breakdown</summary>
                        <div style="padding:10px; font-size:0.9em; color:#64748b">%s</div>
                    </details>
                </div>`, ud.User.Username, ud.User.UUID, float64(totalUp)/1024/1024, float64(totalDown)/1024/1024, dailyRows)
		}
		userStore.mu.Unlock()
		content := `<h1>User Management & Traffic</h1>` + rows
		renderLayout(w, "Users", "users", content)
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
                <h3>Server Environment</h3>
                <p><strong>Listen Addr:</strong> ` + config.ListenAddr + `</p>
                <p><strong>Fallback Dest:</strong> ` + config.DestAddr + `</p>
                <p><strong>Active Conns:</strong> ` + fmt.Sprint(len(activeConns)) + `</p>
            </div>
            <div class="card">
                <h3>Recent Logs</h3>
                <div class="debug-log">` + logs + `</div>
                <button class="btn" style="margin-top:10px" onclick="location.reload()">Refresh Logs</button>
            </div>`
		renderLayout(w, "Debug Panel", "debug", content)
	})

	log.Printf("Web Admin started at http://%s", addr)
	http.ListenAndServe(addr, nil)
}
