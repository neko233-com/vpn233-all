import { createCipheriv, createDecipheriv, randomBytes } from "node:crypto";

export function encrypt(data: Buffer, key: string): Buffer {
  const iv = randomBytes(12);
  const cipher = createCipheriv("aes-256-gcm", Buffer.alloc(32, key), iv);
  const encrypted = Buffer.concat([cipher.update(data), cipher.final()]);
  const tag = cipher.getAuthTag();
  // Format: IV (12) + Tag (16) + Data
  return Buffer.concat([iv, tag, encrypted]);
}

export function decrypt(data: Buffer, key: string): Buffer {
  const iv = data.subarray(0, 12);
  const tag = data.subarray(12, 28);
  const encrypted = data.subarray(28);
  const decipher = createDecipheriv("aes-256-gcm", Buffer.alloc(32, key), iv);
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(encrypted), decipher.final()]);
}

// Simple XOR obfuscation for headers or light data
export function xor(data: Buffer, key: string): Buffer {
  const keyBuf = Buffer.from(key);
  const result = Buffer.alloc(data.length);
  for (let i = 0; i < data.length; i++) {
    result[i] = data[i] ^ keyBuf[i % keyBuf.length];
  }
  return result;
}
