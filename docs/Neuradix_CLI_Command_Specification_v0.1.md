---
title: "Neuradix CLI Command Specification"
author: "Engineering"
date: "17 July 2026"
version: "0.1 Draft"
status: "For review"
---

# Purpose

This document defines the stable command language and automation contract for the `neuradix` CLI.

# Command tree

```text
neuradix
├── new
├── init
├── build
├── check
├── contract
├── component
├── graph
├── run
├── stop
├── inspect
├── record
├── replay
├── explain
├── sim
├── test
├── embedded
├── package
├── deploy
├── registry
├── bridge
├── swarm
├── aero
├── ground
├── fleet
├── studio
├── config
├── context
├── auth
├── doctor
├── completion
└── version
```

# Global flags

```text
--context
--profile
--robot
--swarm
--output table|json|yaml|jsonl
--offline
--timeout
--at
--dry-run
--yes
--verbose
--quiet
```

# Result envelope

Machine-readable commands SHOULD return:

```json
{
  "apiVersion": "cli.neuradix.io/v1alpha1",
  "kind": "CommandResult",
  "command": "contract.validate",
  "status": "success",
  "startedAt": "2026-07-17T10:00:00Z",
  "finishedAt": "2026-07-17T10:00:00Z",
  "context": "local",
  "data": {},
  "warnings": [],
  "errors": []
}
```

# Exit codes

| Code | Meaning |
|---:|---|
| 0 | success |
| 1 | general failure |
| 2 | invalid use |
| 3 | contract validation |
| 4 | compatibility |
| 5 | connectivity |
| 6 | authentication |
| 7 | authorization |
| 8 | safety rejection |
| 9 | determinism/replay mismatch |
| 10 | deployment validation |
| 11 | partial operation |
| 12 | timeout |

# MVP commands

```bash
neuradix init
neuradix contract validate
neuradix contract generate
neuradix build
neuradix run
neuradix graph
neuradix component list
neuradix component health
neuradix inspect stream
neuradix record start
neuradix record stop
neuradix replay run
neuradix explain command
neuradix sim run
neuradix test determinism
neuradix doctor
```

# Embedded commands

```bash
neuradix embedded targets
neuradix embedded new <name> --target <target>
neuradix embedded check
neuradix embedded generate
neuradix embedded build
neuradix embedded size
neuradix embedded flash
neuradix embedded monitor
neuradix embedded inspect
neuradix embedded test
neuradix embedded provision
neuradix embedded update
```

# Safety

Live mutation commands must use authenticated authority and must not bypass onboard Safety. Direct actuator development commands require a hardware-test profile, reason, audit and explicit target.
