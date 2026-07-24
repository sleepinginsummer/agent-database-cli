// Thorough MCP smoke test: drives bin/mcp.js over stdio, exercises every tool
// and key error paths, and asserts each tool-result text is MINIFIED (no
// newlines / indentation) so the compact format's token savings survive.
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const server = join(here, "..", "bin", "mcp.js");
const DB = "minted-edge-local";

const reqs = [
  { id: 1, method: "initialize", params: { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "smoke", version: "0" } } },
  { method: "notifications/initialized" },
  { id: 2, method: "tools/list", params: {} },
  { id: 3, method: "tools/call", params: { name: "query", arguments: { sql: "SELECT 1" } } },                            // error: missing database param
  { id: 4, method: "tools/call", params: { name: "query", arguments: { sql: "SELECT 1", database: "does-not-exist" } } }, // error: unknown db
  { id: 5, method: "tools/call", params: { name: "list_databases", arguments: {} } },
  { id: 9, method: "tools/call", params: { name: "query", arguments: { database: DB, sql: "SELECT 1 AS id, 'Alice' AS name UNION ALL SELECT 2,'Bob'" } } },
  { id: 10, method: "tools/call", params: { name: "query", arguments: { database: DB, sql: "SELECT id, provider_payload FROM accounts WHERE provider_payload IS NOT NULL LIMIT 1" } } }, // jsonb
  { id: 11, method: "tools/call", params: { name: "query", arguments: { database: DB, sql: "SELECT first_name, middle_initials, dob FROM persons WHERE middle_initials IS NULL LIMIT 1" } } }, // nulls
  { id: 12, method: "tools/call", params: { name: "query", arguments: { database: DB, sql: "SELECT * FROM no_such_table" } } },        // db error
  { id: 13, method: "tools/call", params: { name: "query", arguments: { database: DB, sql: "UPDATE accounts SET tier='x'" } } },       // read-only block
  { id: 14, method: "tools/call", params: { name: "describe", arguments: { database: DB, type: "tables" } } },
  { id: 15, method: "tools/call", params: { name: "describe", arguments: { database: DB, type: "columns", table: "roles" } } },
  { id: 16, method: "tools/call", params: { name: "query", arguments: { database: DB, sql: "SELECT pg_backend_pid() AS pid" } } },      // warm session
  { id: 17, method: "tools/call", params: { name: "query", arguments: { database: DB, sql: "SELECT pg_backend_pid() AS pid" } } },      // same pid = reused connection
  { id: 19, method: "tools/call", params: { name: "list_databases", arguments: {} } },                                    // warm list populated
  { id: 20, method: "tools/call", params: { name: "help", arguments: {} } },
  { id: 21, method: "tools/call", params: { name: "set_default_format", arguments: { format: "yaml" } } }, // error: invalid format (validated before any write)
];

const proc = spawn("node", [server], { stdio: ["pipe", "pipe", "pipe"] });
let buf = "";
const responses = new Map();
proc.stdout.on("data", (d) => {
  buf += d.toString();
  let nl;
  while ((nl = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, nl).trim();
    buf = buf.slice(nl + 1);
    if (!line) continue;
    try { const msg = JSON.parse(line); if (msg.id != null) responses.set(msg.id, msg); } catch {}
  }
});

for (const r of reqs) {
  proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", ...r }) + "\n");
}

// give the server time to process (each query spawns the native binary)
await new Promise((res) => setTimeout(res, 12000));
proc.stdin.end();
proc.kill();

let pass = 0, fail = 0;
const check = (cond, label, detail = "") => {
  if (cond) { pass++; console.log(`  ok   ${label}`); }
  else { fail++; console.log(`  FAIL ${label}${detail ? "  <- " + detail : ""}`); }
};
const textOf = (id) => responses.get(id)?.result?.content?.[0]?.text ?? "";
const isErr = (id) => responses.get(id)?.result?.isError === true;
const minified = (id) => { const t = textOf(id); return t.length > 0 && !t.includes("\n") && !/:\s\s/.test(t); };

console.log("\n=== protocol ===");
check(responses.get(1)?.result?.protocolVersion === "2024-11-05", "initialize handshake");
check((responses.get(2)?.result?.tools?.length ?? 0) === 5, "tools/list returns 5 tools", `got ${responses.get(2)?.result?.tools?.length}`);

console.log("\n=== error paths ===");
check(isErr(3), "query without database param -> isError");
check(isErr(21), "set_default_format invalid value -> isError");
check(textOf(3).includes(DB), "missing-database error lists available connections");
check(isErr(4), "query unknown database -> isError");
check(isErr(12), "query bad table -> isError");
check(/no_such_table|does not exist/i.test(textOf(12)), "bad-table error message surfaced");
check(isErr(13), "UPDATE blocked by read-only -> isError");

console.log("\n=== happy paths (content correct) ===");
check(textOf(5).includes(DB), "list_databases includes connection");
check(JSON.parse(textOf(9)).database === DB, "query result echoes target database");
check(JSON.parse(textOf(9)).rowCount === 2, "query rowCount=2");
check(JSON.parse(textOf(9)).fields.join(",") === "id,name", "query fields order preserved");
check(Array.isArray(JSON.parse(textOf(9)).rows[0]), "rows are positional arrays (compact shape)");
const jb = JSON.parse(textOf(10)).rows?.[0]?.[1];
check(jb && typeof jb === "object" && !Array.isArray(jb), "jsonb column is a nested object, not a string");
check(JSON.parse(textOf(11)).rows?.[0]?.includes(null), "NULL preserved as JSON null");
check(JSON.parse(textOf(14)).fields?.includes("table_name"), "describe tables shape");
check(JSON.parse(textOf(14)).database === DB, "describe result echoes target database");
check(JSON.parse(textOf(15)).rowCount > 0, "describe columns(roles) returns rows");

console.log("\n=== warm connection reuse ===");
const pidOf = (id) => JSON.parse(textOf(id)).rows?.[0]?.[0];
check(Number.isInteger(pidOf(16)), "pg_backend_pid query works over warm session");
check(pidOf(16) === pidOf(17), "same backend pid across queries -> connection reused", `${pidOf(16)} vs ${pidOf(17)}`);
check(JSON.parse(textOf(19)).warm?.includes(DB), "list_databases reports warm session");

console.log("\n=== help ===");
const help = JSON.parse(textOf(20));
check(Array.isArray(help.workflow) && help.workflow.length > 0, "help returns workflow steps");
check(typeof help.commandFormats?.mongodb === "string", "help covers per-engine command formats");
check(/read-only|blocklist/i.test(help.safety ?? ""), "help states the safety model");

console.log("\n=== minification (THE fix) ===");
for (const id of [5, 9, 10, 11, 14, 15]) check(minified(id), `tool result #${id} is minified (no newlines/indent)`);

console.log(`\n=== ${pass} passed, ${fail} failed ===`);
process.exit(fail ? 1 : 0);
