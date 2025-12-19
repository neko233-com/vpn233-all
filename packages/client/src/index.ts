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

console.log(`VPN Client starting...`);
console.log(`Connecting to server: ${serverUrl}`);

let ws: WebSocket;
let isWsConnected = false;

function connectWS() {
  ws = new WebSocket(serverUrl, {
    headers: { "authorization": authToken }
  });

  ws.onopen = () => {
    console.log("Connected to VPN Server");
    isWsConnected = true;
  };

  ws.onmessage = (event) => {
    try {
      const rawData = JSON.parse(event.data.toString());
      const { type, connectionId, payload } = rawData;
      const localSocket = pendingConnections.get(connectionId);

      if (!localSocket) return;

      if (type === "connected") {
        console.log(`Tunnel established for ${connectionId}`);
      } else if (type === "data") {
        const decrypted = decrypt(Buffer.from(payload, "base64"), encryptionKey);
        localSocket.write(decrypted);
      } else if (type === "close" || type === "error") {
        localSocket.end();
        pendingConnections.delete(connectionId);
      }
    } catch (err) {
      console.error("Client WS Message Error:", err);
    }
  };

  ws.onclose = () => {
    console.log("Disconnected from VPN Server, retrying in 5s...");
    isWsConnected = false;
    setTimeout(connectWS, 5000);
  };
}

connectWS();

const pendingConnections = new Map<string, Socket>();

// Rule-based routing check with fuzzy matching (e.g., 192.168.*.*)
function shouldDirect(host: string): boolean {
  return rules.direct.some((rule: string) => {
    // Convert glob-like pattern to Regex
    // Escape special characters except *
    const pattern = rule.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*');
    const regex = new RegExp(`^${pattern}$`, 'i');
    
    // Also support simple substring match for domains if no * is present
    if (!rule.includes('*')) {
      return host.includes(rule);
    }
    
    return regex.test(host);
  });
}

// SOCKS5 Proxy Server
const localServer = createServer((socket) => {
  const connectionId = randomUUID();

  socket.once("data", (data) => {
    if (data[0] === 0x05) {
      socket.write(Buffer.from([0x05, 0x00]));
      
      socket.once("data", (request) => {
        if (request[1] === 0x01) {
          let host = "";
          let port = 0;
          let offset = 4;

          if (request[3] === 0x01) {
            host = request.slice(4, 8).join(".");
            offset = 8;
          } else if (request[3] === 0x03) {
            const len = request[4];
            host = request.slice(5, 5 + len).toString();
            offset = 5 + len;
          }
          port = request.readUInt16BE(offset);

          if (shouldDirect(host)) {
            console.log(`[DIRECT] ${host}:${port}`);
            const directSocket = tcpConnect(port, host, () => {
              socket.write(Buffer.from([0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]));
              socket.pipe(directSocket);
              directSocket.pipe(socket);
            });
            directSocket.on("error", () => socket.end());
          } else {
            console.log(`[TUNNEL] ${host}:${port}`);
            pendingConnections.set(connectionId, socket);
            
            ws.send(JSON.stringify({
              type: "connect",
              connectionId,
              payload: { host, port }
            }));

            socket.write(Buffer.from([0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]));

            socket.on("data", (chunk) => {
              if (isWsConnected) {
                const encrypted = encrypt(chunk, encryptionKey);
                ws.send(JSON.stringify({
                  type: "data",
                  connectionId,
                  payload: encrypted.toString("base64")
                }));
              }
            });

            socket.on("close", () => {
              ws.send(JSON.stringify({ type: "close", connectionId }));
              pendingConnections.delete(connectionId);
            });
          }
        }
      });
    }
  });
});

localServer.listen(localPort, () => {
  console.log(`SOCKS5 Proxy listening on 127.0.0.1:${localPort}`);
});

// Web UI Server
Bun.serve({
  port: webPort,
  fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === "/") return new Response(Bun.file(join(process.cwd(), "public/index.html")));
    
    if (url.pathname === "/api/config") {
      if (req.method === "POST") {
        return req.json().then(newConfig => {
          config = { ...config, ...newConfig };
          writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2));
          return new Response(JSON.stringify({ success: true }));
        });
      }
      return new Response(JSON.stringify(config));
    }

    if (url.pathname === "/api/status") {
      return new Response(JSON.stringify({ connected: isWsConnected }));
    }

    return new Response("Not Found", { status: 404 });
  }
});
console.log(`Web UI running on http://localhost:${webPort}`);
