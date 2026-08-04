# Agent Instructions

<!-- HARNESS:BEGIN -->
<!-- harness-version: 0.22.0 -->
<!-- harness-project-id: b63a11e70813469c2947bc74cbd2401b -->
## Harness

This repo uses **Harness** (`5harness`, bin `harness`).

### Install / day-to-day

```bash
# preferred on a machine that works on many projects
npm i -g 5harness
harness --help

# after cloning a repo that already has harness markdown history:
harness link
```

### Project identity for MCP

Each harnessed clone has a durable random project id in this managed block.
Discover it from `<!-- harness-project-id: ... -->` or run:

```bash
harness project id
```

For a single-project OAuth grant, MCP tools are forced to the project selected
at consent. For an all-projects grant, send that target id on every MCP request
as `X-Harness-Project: <id>` (preferred) or `?project=<id>`. Never rely on cwd or
registry order to select a project; a missing or invalid id fails closed.

### Before work — read

- `README.md` (if present)
- `docs/HARNESS.md`
- `docs/FEATURE_INTAKE.md`
- `docs/ARCHITECTURE.md`
- `docs/CONTEXT_RULES.md`
- Active story packet under `docs/stories/` when implementing a story

### Mutation rule (mandatory)

**Do not** create or edit operational durable markdown by hand
(stories / decisions / intakes / backlog entities).

Use the CLI only, for example:

```bash
harness intake --type … --summary "…" --lane normal
harness story add --id US-… --title "…" --lane normal
harness story update --id US-… --status implemented --unit 1 --integration 1 --e2e 0 --platform 0
harness decision add --id … --title "…" --doc docs/decisions/….md
harness query matrix
```

All mutation commands auto-reindex after writing. You do NOT need to call
`harness reindex` manually after mutations.

### HARD STOP — harness failure contract (decision 0017)

If the harness **CLI** or **MCP** fails, is missing, or returns a non-zero /
error result for a step you need:

1. **HARD STOP** that durable-write path. Do **not** continue as if it succeeded.
2. **Never** fall back to hand-editing story / decision / intake / backlog
   markdown to “fix” or bypass the failure.
3. **Recover**, then retry the harness command:

| Order | Command | Why |
| --- | --- | --- |
| 1 | `harness --version` | Confirm install / PATH |
| 2 | `harness doctor` or `harness doctor --json` | Workspace health |
| 3 | `harness link` | Register clone / registry pointer |
| 4 | `harness reindex` | Rebuild derived index from markdown |
| 5 | `harness status` / `harness next` | Confirm the project is usable |

**Exit codes:** `0` = success; `1` = usage / validation / operational error
(**stop**, fix, retry); `2` = reserved — treat as non-success unless that
command’s docs say otherwise. Non-zero exit is never success.

### Read with tools (prefer over dumping large trees)

```bash
harness search "…"
harness get <id>
harness links <id>
harness query matrix
harness query stats
```

Classify work with feature intake before large edits. Record durable decisions
when architecture or product rules change.

### Upgrade

When a newer harness CLI version is installed (`npm i -g 5harness`),
run `harness upgrade` to update the harness block in this AGENTS.md.
Only the harness-managed section (markers HARNESS:BEGIN through HARNESS:END)
is modified — all other content is preserved.
<!-- HARNESS:END -->
