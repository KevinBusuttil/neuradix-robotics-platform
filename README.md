# Neuradix Robotics Platform

Neuradix is a Rust-first, contract-driven platform for dependable autonomous robots across marine, aerial, ground, embedded and space domains.

![Neuradix Robotics Platform ecosystem mind map.](docs/assets/neuradix_platform_ecosystem_mind_map_light.svg)

## Current architecture documents

- [Product, Functional and Technical Specification v0.4](docs/Neuradix_Robotics_Platform_Functional_Specification_v0.4.md)
- [Embedded and CLI Functional Addendum v0.5](docs/Neuradix_Robotics_Platform_Functional_Specification_v0.5_Addendum.md)
- [Platform Implementation Plan v0.3](docs/Neuradix_Implementation_Plan_v0.3.md)
- [Studio XR Implementation Plan v0.2](docs/Neuradix_Studio_XR_Implementation_Plan_v0.2.md)
- [Embedded Profile Implementation Plan v0.1](docs/Neuradix_Embedded_Profile_Implementation_Plan_v0.1.md)
- [CLI Command Specification v0.1](docs/Neuradix_CLI_Command_Specification_v0.1.md)

## Current implementation priority

The first target is one complete AUV vertical slice:

```text
contracts → Rust/Python APIs → runtime → simulation → safety
→ record/replay → explain → Studio inspection
```

A narrow embedded extension then proves the same contracts on host simulation, one native Rust MCU and one generated Arduino C++ endpoint.
