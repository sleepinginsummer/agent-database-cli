#!/usr/bin/env node
// MCP server for agent-database-cli.
//
// Stateless tool surface, warm connections underneath: every query/describe
// call names its target database explicitly (no active-database context to
// set, forget, or race on between parallel agents), and each result echoes the
// database it ran against. The first query against a database spawns a native
// `repl` child process that holds one open connection; later queries stream
// over that child's stdin/stdout (one JSON line per statement), skipping the
// per-call process spawn and connect/disconnect cost. Each database gets its
// own child, so queries to different databases run concurrently. Children are
// closed after an idle period and respawned on demand; the blocklist and
// read-only guard run inside the native binary on every statement, exactly as
// in one-shot mode.
//
// `describe` and `list` remain one-shot CLI invocations (metadata is not part
// of the repl line protocol and is comparatively rare).
import os from "node:os";
import { readFileSync, existsSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  ListToolsRequestSchema,
  CallToolRequestSchema
} from "@modelcontextprotocol/sdk/types.js";

const CONFIG_ENV = "AGENT_DATABASE_CLI_CONFIG";
const IDLE_MS = Number(process.env.AGENT_DATABASE_CLI_MCP_IDLE_MS || 5 * 60 * 1000);
const QUERY_TIMEOUT_MS = Number(process.env.AGENT_DATABASE_CLI_MCP_QUERY_TIMEOUT_MS || 5 * 60 * 1000);

// ---------------------------------------------------------------------------
// Config path (matches the CLI's resolve_config_path)
// ---------------------------------------------------------------------------
function configPath() {
  return (
    process.env[CONFIG_ENV] ||
    join(os.homedir(), ".agent-database-cli", "config.json")
  );
}

// Resolve the native binary (downloaded into bin/native by postinstall, with
// local dev-build fallbacks).
function nativeBinary() {
  const exe =
    process.platform === "win32"
      ? "agent-database-cli.exe"
      : "agent-database-cli";
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  const candidates = [
    join(root, "bin", "native", exe),
    join(root, "target", "release", exe),
    join(root, "target", "debug", exe)
  ];
  return candidates.find((c) => existsSync(c)) ?? null;
}

// ---------------------------------------------------------------------------
// One-shot CLI transport (test / meta / anything outside the repl protocol)
// ---------------------------------------------------------------------------
function runCli(args) {
  const bin = nativeBinary();
  if (!bin) throw new Error("agent-database-cli native binary not found");
  const result = spawnSync(bin, args, { encoding: "utf8" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      (result.stderr || result.stdout || "command failed").trim()
    );
  }
  const out = (result.stdout || "").trim();
  return out ? JSON.parse(out) : {};
}

function configuredDatabases() {
  try {
    const raw = JSON.parse(readFileSync(configPath(), "utf8"));
    return Object.keys(raw.databases || {});
  } catch {
    return [];
  }
}

// ---------------------------------------------------------------------------
// Warm repl sessions: one native `repl` child per database
//
// The repl protocol is line-based: one statement in, one JSON result line out,
// strictly in order, so a FIFO of pending resolvers is enough to match
// responses. Statements are flattened to a single line before sending.
// ---------------------------------------------------------------------------
const sessions = new Map();

class ReplSession {
  constructor(database) {
    const bin = nativeBinary();
    if (!bin) throw new Error("agent-database-cli native binary not found");
    this.database = database;
    this.pending = [];
    this.buffer = "";
    this.stderr = "";
    this.dead = false;
    this.idleTimer = null;
    this.child = spawn(bin, ["repl", "--db", database], {
      stdio: ["pipe", "pipe", "pipe"]
    });
    this.child.stdout.setEncoding("utf8");
    this.child.stdout.on("data", (chunk) => this.onStdout(chunk));
    this.child.stderr.setEncoding("utf8");
    this.child.stderr.on("data", (chunk) => {
      this.stderr += chunk;
    });
    this.child.on("error", (error) => this.fail(error));
    this.child.on("exit", (code) => {
      this.fail(
        new Error(
          this.stderr.trim() ||
            `repl session for "${database}" exited with code ${code}`
        )
      );
    });
    this.touch();
  }

  onStdout(chunk) {
    this.buffer += chunk;
    let nl;
    while ((nl = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, nl).trim();
      this.buffer = this.buffer.slice(nl + 1);
      if (!line) continue;
      const entry = this.pending.shift();
      if (!entry) continue;
      clearTimeout(entry.timer);
      try {
        entry.resolve(JSON.parse(line));
      } catch {
        entry.reject(new Error(`unparseable repl response: ${line}`));
      }
    }
  }

  // Terminal failure: reject everything in flight and retire the session so
  // the next query respawns a fresh child (e.g. after a db restart).
  fail(error) {
    if (this.dead) return;
    this.dead = true;
    clearTimeout(this.idleTimer);
    while (this.pending.length) {
      const entry = this.pending.shift();
      clearTimeout(entry.timer);
      entry.reject(error);
    }
    if (sessions.get(this.database) === this) sessions.delete(this.database);
    try {
      this.child.kill();
    } catch {}
  }

  touch() {
    clearTimeout(this.idleTimer);
    this.idleTimer = setTimeout(() => {
      this.fail(new Error(`repl session for "${this.database}" closed after idle timeout`));
    }, IDLE_MS);
    this.idleTimer.unref?.();
  }

  run(command) {
    const line = command.replace(/\s*\r?\n\s*/g, " ").trim();
    if (!line) return Promise.reject(new Error("empty command"));
    if (this.dead) return Promise.reject(new Error("repl session is closed"));
    this.touch();
    return new Promise((resolve, reject) => {
      const entry = { resolve, reject, timer: null };
      entry.timer = setTimeout(() => {
        // Connection state and response ordering are unknown after a timeout,
        // so retire the whole session rather than only this query.
        this.fail(new Error(`query timed out after ${QUERY_TIMEOUT_MS} ms`));
      }, QUERY_TIMEOUT_MS);
      entry.timer.unref?.();
      this.pending.push(entry);
      this.child.stdin.write(line + "\n");
    });
  }
}

function warmSession(database) {
  const existing = sessions.get(database);
  if (existing && !existing.dead) return existing;
  const session = new ReplSession(database);
  sessions.set(database, session);
  return session;
}

// The repl emits rows as objects keyed by column; one-shot exec emits the
// compact shape (positional row arrays). Convert so MCP results keep the same
// contract and token footprint regardless of transport.
function toCompact(result) {
  const { fields, rows } = result ?? {};
  if (
    !Array.isArray(fields) ||
    fields.length === 0 ||
    !Array.isArray(rows) ||
    !rows.every((r) => r && typeof r === "object" && !Array.isArray(r))
  ) {
    return result;
  }
  return { ...result, rows: rows.map((row) => fields.map((f) => row[f])) };
}

process.on("exit", () => {
  for (const session of sessions.values()) {
    try {
      session.child.kill();
    } catch {}
  }
});

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------
function requireDatabase(database) {
  const known = configuredDatabases();
  if (!database) {
    throw new Error(
      `missing parameter: database. Available: ${known.join(", ") || "(none configured)"}`
    );
  }
  if (known.length && !known.includes(database)) {
    throw new Error(
      `Unknown database "${database}". Available: ${known.join(", ") || "(none)"}`
    );
  }
  return database;
}

const HELP = {
  overview:
    "Unified read-oriented access to locally configured databases (MySQL, PostgreSQL, Redis, Oracle, MongoDB). Every call names its target database explicitly. The first query per database opens a connection that stays warm for the session; later queries reuse it.",
  workflow: [
    "list_databases -> see configured connection names",
    'query {database, sql} -> run a statement, e.g. {"database":"local-pg","sql":"select count(*) from users"}',
    "describe {database, type} -> metadata; type: tables | columns (needs table) | collections | keys (optional pattern)"
  ],
  commandFormats: {
    sql: "MySQL/PostgreSQL/Oracle take a SQL string: select id, name from users limit 10",
    redis: "Redis takes a command string: GET user:1",
    mongodb: 'MongoDB takes a JSON command: {"find":{"collection":"users","filter":{},"limit":1}}'
  },
  results:
    "Compact JSON: column names once in fields, each row as a positional array, plus rowCount; the target database is echoed back so results are self-identifying.",
  safety:
    "Connections are read-only by default and may define a blocklist (checked first). Rejected statements return an error before touching the database; this server cannot bypass either guard.",
  config:
    "Connections live in ~/.agent-database-cli/config.json (override with AGENT_DATABASE_CLI_CONFIG). Add a connection there, then it appears in list_databases. set_default_format {format} persists the default output format for terminal CLI runs; MCP results are always compact and unaffected.",
  performance:
    "Warm queries cost single-digit milliseconds. Idle connections close after 5 minutes and reopen on demand; queries to different databases run concurrently."
};

const TOOLS = [
  {
    name: "help",
    description:
      "Usage guide for this server: workflow, per-engine command formats, result shape, safety model, and configuration.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false }
  },
  {
    name: "list_databases",
    description:
      "List configured local database connections, supported database types, and which connections currently hold a warm session.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false }
  },
  {
    name: "query",
    description:
      "Run a SQL / Redis / MongoDB command against a configured database (read-only unless that connection is configured otherwise). The first query to a database opens its connection; later queries reuse it. Returns rows as JSON, with the target database echoed back.",
    inputSchema: {
      type: "object",
      properties: {
        sql: { type: "string", description: "The statement to execute." },
        database: {
          type: "string",
          description: "Configured connection name (see list_databases)."
        }
      },
      required: ["sql", "database"],
      additionalProperties: false
    }
  },
  {
    name: "describe",
    description:
      "Read metadata from a configured database: tables, columns (needs table), collections, or keys (optional pattern). The target database is echoed back.",
    inputSchema: {
      type: "object",
      properties: {
        type: { type: "string", enum: ["tables", "columns", "collections", "keys"] },
        table: { type: "string" },
        pattern: { type: "string" },
        database: {
          type: "string",
          description: "Configured connection name (see list_databases)."
        }
      },
      required: ["type", "database"],
      additionalProperties: false
    }
  },
  {
    name: "set_default_format",
    description:
      "Persist the CLI's default output format (defaultFormat in the config file), used by terminal `agent-database-cli` runs when --format is not passed. MCP query/describe results are always compact JSON and unaffected.",
    inputSchema: {
      type: "object",
      properties: {
        format: { type: "string", enum: ["compact", "json", "table"] }
      },
      required: ["format"],
      additionalProperties: false
    }
  }
];

async function handleTool(name, args) {
  switch (name) {
    case "help":
      return HELP;
    case "list_databases":
      return {
        configured: configuredDatabases(),
        warm: [...sessions.keys()],
        supported: ["mysql", "postgres", "redis", "oracle", "mongodb"]
      };
    case "query": {
      const sql = args?.sql;
      if (!sql) throw new Error("missing parameter: sql");
      const database = requireDatabase(args?.database);
      const result = await warmSession(database).run(sql);
      // Statement-level failures (blocklist, read-only, SQL errors) arrive as
      // an {"error": ...} line; surface them as tool errors like one-shot mode.
      if (result && typeof result === "object" && result.error) {
        throw new Error(result.error);
      }
      return { database, ...toCompact(result) };
    }
    case "describe": {
      const type = args?.type;
      if (!type) throw new Error("missing parameter: type");
      const database = requireDatabase(args?.database);
      // Pin --format: one-shot output is parsed as JSON here, so the config
      // file's defaultFormat (which may be `table`) must not apply.
      const cliArgs = ["--format", "compact", "meta", "--db", database, "--type", type];
      if (args?.table) cliArgs.push("--table", args.table);
      if (args?.pattern) cliArgs.push("--pattern", args.pattern);
      return { database, ...runCli(cliArgs) };
    }
    case "set_default_format": {
      const format = args?.format;
      if (!["compact", "json", "table"].includes(format)) {
        throw new Error('invalid parameter: format must be "compact", "json", or "table"');
      }
      return runCli(["--format", "json", "format", format]);
    }
    default:
      throw new Error(`Unknown tool: ${name}`);
  }
}

// ---------------------------------------------------------------------------
// Wire up the MCP server
// ---------------------------------------------------------------------------
const server = new Server(
  { name: "agent-database-cli", version: "1.3.0" },
  { capabilities: { tools: {} } }
);

server.setRequestHandler(ListToolsRequestSchema, async () => ({ tools: TOOLS }));

server.setRequestHandler(CallToolRequestSchema, async (req) => {
  try {
    const result = await handleTool(req.params.name, req.params.arguments || {});
    return { content: [{ type: "text", text: JSON.stringify(result) }] };
  } catch (error) {
    return {
      content: [{ type: "text", text: `Error: ${error?.message ?? error}` }],
      isError: true
    };
  }
});

const transportLayer = new StdioServerTransport();
await server.connect(transportLayer);
