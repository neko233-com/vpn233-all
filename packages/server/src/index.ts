import { Socket } from "node:net";
import { encrypt, decrypt } from "../../common/src/crypto";

const PORT = process.env.PORT || 3000;
const AUTH_TOKEN = process.env.AUTH_TOKEN || "neko233-secret";
const ENCRYPTION_KEY = process.env.ENCRYPTION_KEY || "neko233-vpn-key-32-chars-length!!!";

console.log(`VPN Server starting on port ${PORT}...`);

const server = Bun.serve({
  port: PORT,
  fetch(req, server) {
    const url = new URL(req.url);
    if (url.pathname === "/tunnel") {
      const success = server.upgrade(req, {
        data: {
          authToken: req.headers.get("authorization"),
        },
      });
      return success
        ? undefined
        : new Response("Upgrade failed", { status: 400 });
    }
    return new Response("Neko233 VPN Server");
  },
  websocket: {
    open(ws) {
      const { authToken } = ws.data as { authToken: string };
      if (authToken !== AUTH_TOKEN) {
        console.log("Unauthorized connection attempt");
        ws.close(1008, "Unauthorized");
        return;
      }
      console.log("Client connected");
      (ws as any).tcpSockets = new Map<string, Socket>();
    },
    message(ws, message) {
      try {
        const data = JSON.parse(message.toString());
        const { type, connectionId, payload } = data;

        if (type === "connect") {
          const { host, port } = payload;
          console.log(`Connecting to ${host}:${port} [${connectionId}]`);
          
          const tcpSocket = new Socket();
          (ws as any).tcpSockets.set(connectionId, tcpSocket);

          tcpSocket.connect(port, host, () => {
            ws.send(JSON.stringify({ type: "connected", connectionId }));
          });

          tcpSocket.on("data", (chunk) => {
            const encrypted = encrypt(chunk, ENCRYPTION_KEY);
            ws.send(JSON.stringify({
              type: "data",
              connectionId,
              payload: encrypted.toString("base64")
            }));
          });

          tcpSocket.on("close", () => {
            ws.send(JSON.stringify({ type: "close", connectionId }));
            (ws as any).tcpSockets.delete(connectionId);
          });

          tcpSocket.on("error", (err) => {
            console.error(`TCP Error [${connectionId}]:`, err.message);
            ws.send(JSON.stringify({ type: "error", connectionId, payload: err.message }));
          });

        } else if (type === "data") {
          const tcpSocket = (ws as any).tcpSockets.get(connectionId);
          if (tcpSocket) {
            const decrypted = decrypt(Buffer.from(payload, "base64"), ENCRYPTION_KEY);
            tcpSocket.write(decrypted);
          }
        } else if (type === "close") {
          const tcpSocket = (ws as any).tcpSockets.get(connectionId);
          if (tcpSocket) {
            tcpSocket.end();
            (ws as any).tcpSockets.delete(connectionId);
          }
        }
      } catch (err) {
        console.error("WS Message Error:", err);
      }
    },
    close(ws) {
      console.log("Client disconnected");
      const sockets = (ws as any).tcpSockets as Map<string, Socket>;
      if (sockets) {
        for (const socket of sockets.values()) {
          socket.destroy();
        }
        sockets.clear();
      }
    },
  },
});

console.log(`Server running at ${server.hostname}:${server.port}`);
