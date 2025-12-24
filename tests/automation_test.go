package tests

import (
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/cookiejar"
	"net/url"
	"os"
	"os/exec"
	"strings"
	"testing"
	"time"
)

func TestVPNAutomation(t *testing.T) {
	// 1. Start Mock Target Server
	targetMux := http.NewServeMux()
	targetMux.HandleFunc("/hello", func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprintf(w, "hello from target")
	})
	targetServer := &http.Server{Addr: "127.0.0.1:9999", Handler: targetMux}
	go targetServer.ListenAndServe()
	defer targetServer.Close()

	// 2. Build Client and Server (optional, but let's assume they are ready or we run them with go run)
	// For speed, we'll use 'go run'

	// 3. Start VPN Server
	serverCmd := exec.Command("go", "run", "main.go", "-c", "config.json")
	serverCmd.Dir = "../server-vpn-neko233"
	serverCmd.Stdout = os.Stdout
	serverCmd.Stderr = os.Stderr
	err := serverCmd.Start()
	if err != nil {
		t.Fatalf("Failed to start server: %v", err)
	}
	defer serverCmd.Process.Kill()

	// 4. Start VPN Client
	clientCmd := exec.Command("go", "run", "main.go", "-c", "config.json")
	clientCmd.Dir = "../client-vpn-neko233"
	clientCmd.Stdout = os.Stdout
	clientCmd.Stderr = os.Stderr
	err = clientCmd.Start()
	if err != nil {
		t.Fatalf("Failed to start client: %v", err)
	}
	defer clientCmd.Process.Kill()

	// Wait for startup (Non-blocking retry)
	start := time.Now()
	success := false
	for time.Since(start) < 15*time.Second {
		// Check Client Web GUI
		cConn, cErr := net.DialTimeout("tcp", "127.0.0.1:8082", 500*time.Millisecond)
		// Check Server Web Admin
		sConn, sErr := net.DialTimeout("tcp", "127.0.0.1:8083", 500*time.Millisecond)

		if cErr == nil && sErr == nil {
			cConn.Close()
			sConn.Close()
			success = true
			break
		}
		if cConn != nil {
			cConn.Close()
		}
		if sConn != nil {
			sConn.Close()
		}
		time.Sleep(1 * time.Second)
	}

	if !success {
		t.Fatalf("Client or Server failed to start within 15 seconds")
	}

	// 5. Verify Proxy Effectiveness
	t.Run("ProxyEffectiveness", func(t *testing.T) {
		proxyUrl, _ := url.Parse("http://127.0.0.1:1081") // Assuming HTTP inbound is on 1081
		client := &http.Client{
			Transport: &http.Transport{Proxy: http.ProxyURL(proxyUrl)},
			Timeout:   5 * time.Second,
		}

		resp, err := client.Get("http://127.0.0.1:9999/hello")
		if err != nil {
			t.Fatalf("Failed to fetch through proxy: %v", err)
		}
		defer resp.Body.Close()

		body, _ := io.ReadAll(resp.Body)
		if string(body) != "hello from target" {
			t.Errorf("Unexpected response: %s", string(body))
		}
		t.Log("Proxy effectiveness verified!")
	})

	// 6. Verify Web Interfaces (with Auth)
	t.Run("WebInterfaces", func(t *testing.T) {
		// Client Web GUI Login
		clientJar, _ := cookiejar.New(nil)
		httpClient := &http.Client{Jar: clientJar}

		resp, err := httpClient.PostForm("http://127.0.0.1:8082/login", url.Values{
			"username": {"admin"},
			"password": {"password123"},
		})
		if err != nil {
			t.Errorf("Client Web GUI login failed: %v", err)
		} else {
			defer resp.Body.Close()
			if resp.Request.URL.Path == "/login" {
				t.Errorf("Client Web GUI login failed (still on login page)")
			} else {
				t.Log("Client Web GUI login successful!")
			}
		}

		// Server Web Admin Login
		serverJar, _ := cookiejar.New(nil)
		httpServerClient := &http.Client{Jar: serverJar}
		resp, err = httpServerClient.PostForm("http://127.0.0.1:8083/login", url.Values{
			"username": {"admin"},
			"password": {"adminpassword"},
		})
		if err != nil {
			t.Errorf("Server Web Admin login failed: %v", err)
		} else {
			defer resp.Body.Close()
			if resp.Request.URL.Path == "/login" {
				t.Errorf("Server Web Admin login failed (still on login page)")
			} else {
				t.Log("Server Web Admin login successful!")
			}
		}

		// Verify Debug Panels
		resp, err = httpClient.Get("http://127.0.0.1:8082/debug")
		if err != nil || resp.StatusCode != 200 {
			t.Errorf("Client Debug Panel not accessible")
		}
		resp, err = httpServerClient.Get("http://127.0.0.1:8083/debug")
		if err != nil || resp.StatusCode != 200 {
			t.Errorf("Server Debug Panel not accessible")
		}

		// Verify Speed Test API
		resp, err = httpClient.Get("http://127.0.0.1:8082/debug/speedtest")
		if err != nil || resp.StatusCode != 200 {
			t.Errorf("Speed Test API failed")
		} else {
			body, _ := io.ReadAll(resp.Body)
			t.Logf("Speed Test Result: %s", string(body))
			resp.Body.Close()
		}
	})

	// 8. Verify Routing Strategies (IP-CIDR)
	t.Run("RoutingStrategies", func(t *testing.T) {
		clientJar, _ := cookiejar.New(nil)
		httpClient := &http.Client{Jar: clientJar}

		// Login first
		httpClient.PostForm("http://127.0.0.1:8082/login", url.Values{
			"username": {"admin"},
			"password": {"password123"},
		})

		// Test IP-CIDR Proxy
		resp, err := httpClient.Get("http://127.0.0.1:8082/debug/test?host=8.8.8.8")
		if err == nil {
			body, _ := io.ReadAll(resp.Body)
			if !strings.Contains(string(body), "Matched Outbound: proxy") {
				t.Errorf("IP-CIDR proxy rule failed: %s", string(body))
			}
			resp.Body.Close()
		}

		// Test IP-CIDR Direct
		resp, err = httpClient.Get("http://127.0.0.1:8082/debug/test?host=114.114.114.114")
		if err == nil {
			body, _ := io.ReadAll(resp.Body)
			if !strings.Contains(string(body), "Matched Outbound: direct") {
				t.Errorf("IP-CIDR direct rule failed: %s", string(body))
			}
			resp.Body.Close()
		}
	})

	// 9. Verify Path Tracing
	t.Run("PathTracing", func(t *testing.T) {
		clientJar, _ := cookiejar.New(nil)
		httpClient := &http.Client{Jar: clientJar}

		// Login first
		httpClient.PostForm("http://127.0.0.1:8082/login", url.Values{
			"username": {"admin"},
			"password": {"password123"},
		})

		// Make a request through proxy to generate a trace
		proxyUrl, _ := url.Parse("http://127.0.0.1:1081")
		proxyClient := &http.Client{
			Transport: &http.Transport{
				Proxy:             http.ProxyURL(proxyUrl),
				DisableKeepAlives: true,
			},
			Timeout: 5 * time.Second,
		}
		proxyClient.Get("http://127.0.0.1:9999/hello")

		// Wait for trace to be recorded (with retry)
		success := false
		for i := 0; i < 10; i++ {
			time.Sleep(200 * time.Millisecond)
			resp, err := httpClient.Get("http://127.0.0.1:8082/traces")
			if err != nil {
				continue
			}
			body, _ := io.ReadAll(resp.Body)
			resp.Body.Close()
			if strings.Contains(string(body), "127.0.0.1:9999") {
				success = true
				break
			}
		}

		if !success {
			t.Errorf("Trace for target 127.0.0.1:9999 not found in traces page after retries")
		}
		t.Log("Path tracing verified!")
	})

	// 10. Verify Operational Stats
	t.Run("OperationalStats", func(t *testing.T) {
		clientJar, _ := cookiejar.New(nil)
		httpClient := &http.Client{Jar: clientJar}
		httpClient.PostForm("http://127.0.0.1:8082/login", url.Values{
			"username": {"admin"},
			"password": {"password123"},
		})

		resp, err := httpClient.Get("http://127.0.0.1:8082/")
		if err != nil {
			t.Fatalf("Failed to access dashboard: %v", err)
		}
		defer resp.Body.Close()

		body, _ := io.ReadAll(resp.Body)
		if !strings.Contains(string(body), "Total Requests") {
			t.Errorf("Dashboard does not contain Total Requests stat. Body: %s", string(body))
		}
		if !strings.Contains(string(body), "Active Conns") {
			t.Errorf("Dashboard does not contain Active Conns stat")
		}
		if !strings.Contains(string(body), "Outbound Traffic Analysis") {
			t.Errorf("Dashboard does not contain Outbound Traffic Analysis table")
		}
		t.Log("Operational stats verified!")
	})

	// 11. Verify User Traffic Persistence
	t.Run("UserTrafficPersistence", func(t *testing.T) {
		serverJar, _ := cookiejar.New(nil)
		httpServerClient := &http.Client{Jar: serverJar}
		httpServerClient.PostForm("http://127.0.0.1:8083/login", url.Values{
			"username": {"admin"},
			"password": {"adminpassword"},
		})

		// Check users page for traffic
		resp, err := httpServerClient.Get("http://127.0.0.1:8083/users")
		if err != nil {
			t.Fatalf("Failed to access users page: %v", err)
		}
		defer resp.Body.Close()

		body, _ := io.ReadAll(resp.Body)
		if !strings.Contains(string(body), "neko233") {
			t.Errorf("User neko233 not found in users page")
		}
		if !strings.Contains(string(body), "Daily Breakdown") {
			t.Errorf("Daily Breakdown not found in users page")
		}

		// Verify file persistence
		userFile := "../server-vpn-neko233/data/users/neko233.json"
		if _, err := os.Stat(userFile); os.IsNotExist(err) {
			t.Errorf("User persistence file %s does not exist", userFile)
		} else {
			t.Logf("User persistence file %s verified!", userFile)
		}
	})

	// 12. Verify Dynamic Control (Proxy Switch & Group Override)
	t.Run("DynamicControl", func(t *testing.T) {
		clientJar, _ := cookiejar.New(nil)
		httpClient := &http.Client{Jar: clientJar}

		// Login
		httpClient.PostForm("http://127.0.0.1:8082/login", url.Values{
			"username": {"admin"},
			"password": {"password123"},
		})

		// Test Group Switch (Switch 'video' group to 'direct')
		resp, err := httpClient.Post("http://127.0.0.1:8082/groups/switch?group=video&tag=direct", "", nil)
		if err != nil || resp.StatusCode != 200 {
			t.Errorf("Failed to switch group: %v", err)
		}

		// Verify group override in debug test
		resp, err = httpClient.Get("http://127.0.0.1:8082/debug/test?host=youtube.com")
		if err == nil {
			body, _ := io.ReadAll(resp.Body)
			if !strings.Contains(string(body), "Matched Outbound: direct") {
				t.Errorf("Group override failed: %s", string(body))
			}
			resp.Body.Close()
		}

		// Test Global Proxy Switch (Switch 'proxy' to 'reject')
		resp, err = httpClient.Post("http://127.0.0.1:8082/proxy/switch?tag=reject", "", nil)
		if err != nil || resp.StatusCode != 200 {
			t.Errorf("Failed to switch global proxy: %v", err)
		}

		// Verify global proxy override in debug test
		resp, err = httpClient.Get("http://127.0.0.1:8082/debug/test?host=google.com")
		if err == nil {
			body, _ := io.ReadAll(resp.Body)
			if !strings.Contains(string(body), "Matched Outbound: reject") {
				t.Errorf("Global proxy override failed: %s", string(body))
			}
			resp.Body.Close()
		}
	})
}
