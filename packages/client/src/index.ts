import { createServer, Socket, connect as tcpConnect, Server as NetServer } from "node:net";
import { randomUUID } from "node:crypto";
import { encrypt, decrypt } from "../../common/src/crypto";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";

// --- Types & Config ---
interface Config {
  serverUrl: string;
  authToken: string;
  localPort: number;
  webPort: number;
  encryptionKey: string;
  rules: { direct: string[] };
}

const CONFIG_PATH = join(process.cwd(), "config.json");

class VPNClient {
  private config: Config;
  private ws: WebSocket | null = null;
  private socksServer: NetServer | null = null;
  private isWsConnected = false;
  private pendingConnections = new Map<string, Socket>();
  private logs: string[] = [];
  private maxLogs = 100;

  constructor() {
    this.config = this.loadConfig();
    this.log("Client initialized");
  }

  private loadConfig(): Config {
    if (!existsSync(CONFIG_PATH)) {
      const defaultConfig: Config = {
        serverUrl: "ws://localhost:3000/tunnel",
        authToken: "neko233-secret",
        localPort: 1080,
        webPort: 23333,
        encryptionKey: "neko233-vpn-key-32-chars-length!!!",
        rules: { direct: ["localhost", "127.0.0.1"] }
      };
      writeFileSync(CONFIG_PATH, JSON.stringify(defaultConfig, null, 2));
      return defaultConfig;
    }
    return JSON.parse(readFileSync(CONFIG_PATH, "utf-8"));
  }

  private log(msg: string) {
    const entry = `[${new Date().toLocaleTimeString()}] ${msg}`;
    console.log(entry);
    this.logs.push(entry);
    if (this.logs.length > this.maxLogs) this.logs.shift();
  }

  public getStatus() {
    return {
      connected: this.isWsConnected,
      activeConnections: this.pendingConnections.size,
      config: this.config,
      logs: this.logs
    };
  }

  public async updateConfig(newConfig: Partial<Config>) {
    this.config = { ...this.config, ...newConfig };
    writeFileSync(CONFIG_PATH, JSON.stringify(this.config, null, 2));
    this.log("Config updated, restarting services...");
    await this.restart();
  }

  public async start() {
    this.startWS();
    this.startSocks();
  }

  public async restart() {
    this.stop();
    await this.start();
  }

  private stop() {
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
    if (this.socksServer) {
      this.socksServer.close();
      this.socksServer = null;
    }
    this.pendingConnections.forEach(s => s.destroy());
    this.pendingConnections.clear();
    this.isWsConnected = false;
  }

  private startWS() {
    this.log(`Connecting to VPN Server: ${this.config.serverUrl}`);
    this.ws = new WebSocket(this.config.serverUrl, {
      headers: { "authorization": this.config.authToken }
    });

    this.ws.onopen = () => {
      this.isWsConnected = true;
      this.log("WebSocket Connected");
    };

    this.ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data.toString());
        const { type, connectionId, payload } = data;
        const localSocket = this.pendingConnections.get(connectionId);

        if (!localSocket) return;

        if (type === "data") {
          const decrypted = decrypt(Buffer.from(payload, "base64"), this.config.encryptionKey);
          localSocket.write(decrypted);
        } else if (type === "close" || type === "error") {
          localSocket.end();
          this.pendingConnections.delete(connectionId);
        }
      } catch (err: any) {
        this.log(`WS Error: ${err.message}`);
      }
    };

    this.ws.onclose = () => {
      if (this.ws) {
        this.isWsConnected = false;
        this.log("WebSocket Disconnected, retrying...");
        setTimeout(() => this.startWS(), 5000);
      }
    };
  }

  private shouldDirect(host: string): boolean {
    return this.config.rules.direct.some((rule: string) => {
      const pattern = rule.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*');
      const regex = new RegExp(`^${pattern}$`, 'i');
      return !rule.includes('*') ? host.includes(rule) : regex.test(host);
    });
  }

  private startSocks() {
    this.socksServer = createServer((socket) => {
      const connectionId = randomUUID();

      socket.once("data", (data) => {
        if (data[0] === 0x05) {
          socket.write(Buffer.from([0x05, 0x00]));
          socket.once("data", (request) => {
            if (request[1] === 0x01) {
              let host = "";
              let port = 0;
              let offset = 4;
              if (request[3] === 0x01) { host = request.slice(4, 8).join("."); offset = 8; }
              else if (request[3] === 0x03) { const len = request[4]; host = request.slice(5, 5 + len).toString(); offset = 5 + len; }
              port = request.readUInt16BE(offset);

              if (this.shouldDirect(host)) {
                this.log(`[DIRECT] ${host}:${port}`);
                const directSocket = tcpConnect(port, host, () => {
                  socket.write(Buffer.from([0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]));
                  socket.pipe(directSocket);
                  directSocket.pipe(socket);
                });
                directSocket.on("error", () => socket.end());
              } else {
                this.log(`[TUNNEL] ${host}:${port}`);
                this.pendingConnections.set(connectionId, socket);
                this.ws?.send(JSON.stringify({ type: "connect", connectionId, payload: { host, port } }));
                socket.write(Buffer.from([0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]));

                socket.on("data", (chunk) => {
                  if (this.isWsConnected) {
                    const encrypted = encrypt(chunk, this.config.encryptionKey);
                    this.ws?.send(JSON.stringify({ type: "data", connectionId, payload: encrypted.toString("base64") }));
                  }
                });

                socket.on("close", () => {
                  this.ws?.send(JSON.stringify({ type: "close", connectionId }));
                  this.pendingConnections.delete(connectionId);
                });
              }
            }
          });
        }
      });
    });

    this.socksServer.listen(this.config.localPort, () => {
      this.log(`SOCKS5 Proxy listening on 127.0.0.1:${this.config.localPort}`);
    });
  }
}

// --- Web Server (The "Client" Entry Point) ---
const vpn = new VPNClient();
vpn.start();

// Get webPort from config
const initialConfig = JSON.parse(readFileSync(CONFIG_PATH, "utf-8"));
const webPort = initialConfig.webPort || 23333;

Bun.serve({
  port: webPort,
  fetch(req) {
    const url = new URL(req.url);
    
    // Static Files
    if (url.pathname === "/") return new Response(Bun.file(join(process.cwd(), "public/index.html")));
    
    // API Endpoints
    if (url.pathname === "/api/status") {
      return new Response(JSON.stringify(vpn.getStatus()), { headers: { "Content-Type": "application/json" } });
    }

    if (url.pathname === "/api/config" && req.method === "POST") {
      return req.json().then(async (newConfig) => {
        await vpn.updateConfig(newConfig);
        return new Response(JSON.stringify({ success: true }));
      });
    }

    if (url.pathname === "/api/restart" && req.method === "POST") {
      vpn.restart();
      return new Response(JSON.stringify({ success: true }));
    }

    return new Response("Not Found", { status: 404 });
  }
});

console.log(`Web UI running on http://localhost:${webPort}`);
console.log(`Web UI running on http://localhost:${webPort}`);
