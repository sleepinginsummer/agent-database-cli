# Performance

`agent-database-cli` opens a **direct connection per command**: each invocation spawns the process, connects to the database, runs the statement, and disconnects. There is no background daemon and no connection pool. This document explains where the time goes and how to get the lowest latency for high-frequency (agent) workloads.

## Measured latency (Windows, local Postgres over TLS, `passwordRef` credential)

Numbers are environment-specific — they depend on your OS, where the database lives, and whether the connection uses TLS / an SSH tunnel. Measure your own setup with `scripts/bench-launch.ps1`.

| Approach | Latency | What it pays for |
| --- | --- | --- |
| `list` (no DB connection) | ~28 ms | Process spawn + config load only |
| `exec` (one-off command) | ~130 ms | Spawn **+ a fresh DB connection** + query + disconnect |
| `repl` — fixed setup | ~130 ms once | Spawn + the one connection it reuses (≈ one `exec`) |
| `repl` — per statement (large batch) | **~0.7 ms** | The query over the warm connection (setup amortized away) |

The headline: for a **single** command, establishing the connection (TCP + auth, plus decrypting the stored credential, plus a TLS handshake when the server requires it) dominates — roughly **100 ms** of the ~130 ms `exec` total, versus only ~28 ms for process spawn. Reusing one connection (`repl`) amortizes that fixed cost away: across a large batch each statement costs under a millisecond.

> Numbers measured against a local plaintext Postgres. A remote or TLS-required server (e.g. AWS RDS) adds network round-trips and a TLS handshake to the connection step, so the one-off `exec` cost grows accordingly — which makes reusing the connection (`repl`, or the MCP server's warm sessions) matter even more.

## Core principle: reuse the connection for high-frequency work

```
One-off (exec):            spawn ─► connect (TCP/TLS/auth) ─► query ─► disconnect
                                    └──────── dominant ───────┘   (paid every call)

Batch (repl / MCP query):  spawn ─► connect ─► query ─► query ─► query ─► ...
                                    (paid once)        └─ ~0.3 ms each ─┘
```

The single biggest lever is **how many times you establish a connection**. A one-off `exec` (or a single MCP `query`) pays a full connect every time. Feeding many statements through one `repl` process pays it once.

Secondary, already-applied optimizations on the per-command path:

1. **Native launcher.** During install, `postinstall` rewrites the launcher shim to call the platform's native binary directly instead of going through Node, keeping process spawn around tens of milliseconds rather than ~145 ms. `bin/agent-database-cli.js` remains as a fallback (e.g. `--ignore-scripts`).
2. **Cached safety-check regexes.** The read-only / blocklist check compiles its keyword regexes once per process instead of on every command (previously ~28 ms of regex compilation per `exec`). This is now a negligible part of the query path.

## How to go fastest

- **One-off query**: `agent-database-cli exec --db <name> --command "<sql>"`. Simplest; pays a fresh connection each call (~130 ms here, mostly connection setup).
- **Many queries in a row (scripts / pipelines)**: use `repl` and feed multiple statements to a single process so the connection is established once:
  ```bash
  printf 'select 1\nselect count(*) from accounts\n' | agent-database-cli repl --db <name>
  ```
  Under a millisecond per statement once the connection is set up (~0.7 ms each across a large batch).
- **Agent that queries continuously**: the MCP server (`agent-database-cli-mcp`) holds a warm `repl` child per database — the first `query` against a database pays the spawn + connect once, and every later `query` streams over that reused connection (single-digit milliseconds, measured ~3 ms). Different databases get their own children, so multi-db queries run concurrently. Idle children close after 5 minutes (`AGENT_DATABASE_CLI_MCP_IDLE_MS`) and respawn on demand. `describe` remains a one-shot CLI invocation.

> Always measure before optimizing. On this project the connection cost — invisible until measured — is now the dominant term for one-off commands; without the daemon it is paid per call rather than amortized across a warm pool. Choose `repl` when that matters.
