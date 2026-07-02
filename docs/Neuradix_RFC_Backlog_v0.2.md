# Neuradix RFC Backlog v0.2

This backlog supports the implementation plans aligned to Functional Specification v0.4.

## P0 RFCs — required before core implementation widens

### RFC-0001 — Component and Lifecycle Model

Define:

- component manifest;
- lifecycle states;
- execution classes;
- health states;
- restart policies;
- component identity;
- resource declarations;
- relationship to runtime supervision.

### RFC-0002 — Contract Format and Code Generation

Define:

- authored contract format;
- schema hash;
- Stream/State/Command/Task/Event/Query primitives;
- unit metadata;
- frame metadata;
- clock-domain metadata;
- generated Rust;
- generated Python;
- generated Protobuf;
- compatibility policy.

### RFC-0003 — Time, Clocks and Deterministic Replay

Define:

- clock domains;
- timestamp rules;
- simulation clock;
- replay clock;
- random seed policy;
- lints for deterministic profiles;
- replay equivalence tests.

### RFC-0004 — Transport-Neutral Data Plane

Define:

- transport API;
- in-process transport;
- shared-memory lease semantics;
- network backend abstraction;
- queue/buffer policy;
- payload priority;
- metrics.

### RFC-0005 — Safety Authority and Command Lineage

Define:

- authority leases;
- actuator command gateway;
- safety constraints;
- command modification/rejection;
- lineage event model;
- explain query.

## P1 RFCs — required before Swarm, Aero and XR implementation

### RFC-0006 — Swarm Membership and Task Allocation

Define:

- swarm identity;
- membership epoch;
- roles;
- capabilities;
- task allocation;
- partition handling;
- rejoin reconciliation.

### RFC-0007 — Federated Shared World Model

Define:

- local versus shared world models;
- provenance;
- uncertainty;
- revision;
- conflict resolution;
- communication-aware deltas.

### RFC-0008 — Studio XR Operator Intent and Authority

Define:

- XR interaction modes;
- operator intent schema;
- command preview;
- Ground authority;
- Safety validation;
- multi-user authority.

### RFC-0009 — Aero Airspace and Collision Safety Model

Define:

- airspace volumes;
- UAV trajectory contracts;
- collision-conflict model;
- local avoidance;
- lost-link behaviour;
- emergency landing/diversion.

### RFC-0010 — Multi-Vehicle Simulation and Replay

Define:

- distributed simulation topology;
- multiple clocks;
- network fault injection;
- multi-robot recording;
- branch replay;
- swarm incident reconstruction.

## P2 RFCs — later

### RFC-0011 — Flight Profile and Restricted Rust Policy

Define:

- static topology;
- restricted Rust subset;
- deterministic scheduler;
- no-Python critical path;
- FDIR;
- evidence generation.

### RFC-0012 — Ground/Fleet Identity and Deployment Manifest

Define:

- operator identity;
- vehicle registry;
- manifest signing;
- OTA plan;
- audit trail;
- command dictionary.
