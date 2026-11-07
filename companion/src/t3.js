import { createHash, randomBytes } from "node:crypto";
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { homedir } from "node:os";
import { join } from "node:path";

const PROTOCOL_VERSION = "2025-06-18";

export class AuthError extends Error {}

/** The local T3 server's origin, from the runtime file the server writes while it runs. */
export function discoverOrigin(t3Home = process.env.T3CODE_HOME || join(homedir(), ".t3")) {
  const file = join(t3Home, "userdata", "server-runtime.json");
  try {
    const origin = JSON.parse(readFileSync(file, "utf8")).origin;
    if (typeof origin === "string" && origin.startsWith("http")) return origin.replace(/\/+$/, "");
  } catch {}
  return null;
}

const base64url = (buffer) => buffer.toString("base64url");

function openBrowser(url) {
  const [command, args] =
    process.platform === "win32"
      ? ["rundll32", ["url.dll,FileProtocolHandler", url]]
      : process.platform === "darwin"
        ? ["open", [url]]
        : ["xdg-open", [url]];
  try {
    spawn(command, args, { stdio: "ignore", detached: true }).unref();
  } catch {}
}

/**
 * T3's MCP sign-in: register a public client, send the user to the approval page,
 * receive the code on a loopback port, exchange it with PKCE. The approval page is
 * where the user picks the access (Full access lets it write to full-access threads).
 */
export async function login(origin, { log = console.log } = {}) {
  const state = base64url(randomBytes(16));
  const verifier = base64url(randomBytes(32));
  const challenge = base64url(createHash("sha256").update(verifier).digest());
  const resource = `${origin}/mcp`;

  let finish;
  const received = new Promise((resolve, reject) => (finish = { resolve, reject }));
  const server = createServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    if (url.pathname !== "/callback") {
      res.writeHead(404).end();
      return;
    }
    const error = url.searchParams.get("error");
    const ok = !error && url.searchParams.get("state") === state && url.searchParams.get("code");
    res.writeHead(ok ? 200 : 400, { "content-type": "text/html; charset=utf-8" });
    res.end(
      ok
        ? "<h2>T3 WoW is signed in.</h2><p>You can close this tab and go back to the game.</p>"
        : `<h2>Sign-in failed</h2><p>${error ?? "state mismatch"}</p>`,
    );
    if (ok) finish.resolve(url.searchParams.get("code"));
    else finish.reject(new Error(`Sign-in failed: ${url.searchParams.get("error_description") ?? error ?? "state mismatch"}`));
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const redirectUri = `http://127.0.0.1:${server.address().port}/callback`;

  try {
    const registration = await fetch(`${origin}/oauth/mcp/register`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        client_name: "T3 WoW",
        redirect_uris: [redirectUri],
        grant_types: ["authorization_code"],
        response_types: ["code"],
        token_endpoint_auth_method: "none",
      }),
    });
    if (!registration.ok) throw new Error(`Registration failed: ${registration.status} ${await registration.text()}`);
    const { client_id: clientId } = await registration.json();

    const authorize = new URL(`${origin}/oauth/mcp/authorize`);
    for (const [key, value] of Object.entries({
      response_type: "code",
      client_id: clientId,
      redirect_uri: redirectUri,
      code_challenge: challenge,
      code_challenge_method: "S256",
      state,
      resource,
      scope: "orchestration:read orchestration:operate",
    }))
      authorize.searchParams.set(key, value);

    log("Opening the T3 sign-in page. Choose Full access so you can message your full-access threads.");
    log("If it asks for a pairing code, create one in T3: Settings > Sharing & access (or `t3 auth pairing create`).");
    log(`If no browser opened, visit:\n  ${authorize.href}`);
    openBrowser(authorize.href);

    const timeout = setTimeout(() => finish.reject(new Error("Timed out waiting for approval (10 min).")), 600_000);
    const code = await received.finally(() => clearTimeout(timeout));

    const token = await fetch(`${origin}/oauth/mcp/token`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({
        grant_type: "authorization_code",
        code,
        redirect_uri: redirectUri,
        client_id: clientId,
        code_verifier: verifier,
        resource,
      }),
    });
    const body = await token.json().catch(() => ({}));
    if (!token.ok || !body.access_token) {
      throw new Error(`Token exchange failed: ${body.error_description ?? body.error ?? token.status}`);
    }
    return {
      token: body.access_token,
      scope: body.scope ?? "",
      expiresAt: Date.now() + (body.expires_in ?? 0) * 1000,
    };
  } finally {
    server.close();
  }
}

function parseEventStream(text, id) {
  for (const block of text.split(/\r?\n\r?\n/)) {
    const data = block
      .split(/\r?\n/)
      .filter((line) => line.startsWith("data:"))
      .map((line) => line.slice(5).trimStart())
      .join("\n");
    if (!data) continue;
    try {
      const message = JSON.parse(data);
      if (message.id === id) return message;
    } catch {}
  }
  return null;
}

/** Minimal MCP client over streamable HTTP: initialize once, then tools/call. */
export class McpClient {
  constructor(origin, token) {
    this.endpoint = `${origin}/mcp`;
    this.token = token;
    this.sessionId = null;
    this.nextId = 1;
    this.ready = null;
  }

  async post(body, { expectReply = true } = {}) {
    const headers = {
      authorization: `Bearer ${this.token}`,
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
    };
    if (this.sessionId) {
      headers["mcp-session-id"] = this.sessionId;
      headers["mcp-protocol-version"] = PROTOCOL_VERSION;
    }
    const response = await fetch(this.endpoint, { method: "POST", headers, body: JSON.stringify(body) });
    if (response.status === 401 || response.status === 403) {
      throw new AuthError(`T3 refused the sign-in (${response.status}). Run: t3wow login`);
    }
    const session = response.headers.get("mcp-session-id");
    if (session) this.sessionId = session;
    if (!expectReply) return null;
    if (response.status === 404 && this.sessionId) {
      this.sessionId = null;
      this.ready = null;
      throw new SessionExpired();
    }
    const text = await response.text();
    if (!response.ok) throw new Error(`MCP ${response.status}: ${text.slice(0, 300)}`);
    const type = response.headers.get("content-type") ?? "";
    const message = type.includes("text/event-stream") ? parseEventStream(text, body.id) : JSON.parse(text);
    if (!message) throw new Error("MCP reply missing");
    if (message.error) throw new Error(`MCP error ${message.error.code}: ${message.error.message}`);
    return message.result;
  }

  async initialize() {
    this.sessionId = null;
    await this.post({
      jsonrpc: "2.0",
      id: this.nextId++,
      method: "initialize",
      params: { protocolVersion: PROTOCOL_VERSION, capabilities: {}, clientInfo: { name: "t3wow", version: "0.1.0" } },
    });
    await this.post({ jsonrpc: "2.0", method: "notifications/initialized" }, { expectReply: false });
  }

  async call(name, args = {}) {
    for (let attempt = 0; ; attempt++) {
      this.ready ??= this.initialize().catch((error) => {
        this.ready = null;
        throw error;
      });
      await this.ready;
      try {
        const result = await this.post({
          jsonrpc: "2.0",
          id: this.nextId++,
          method: "tools/call",
          params: { name, arguments: args },
        });
        const text = result.content?.find((part) => part.type === "text")?.text;
        if (result.isError) throw new Error(text ?? `${name} failed`);
        if (result.structuredContent) return result.structuredContent;
        return text ? JSON.parse(text) : {};
      } catch (error) {
        if (error instanceof SessionExpired && attempt === 0) continue;
        throw error;
      }
    }
  }
}

class SessionExpired extends Error {}
