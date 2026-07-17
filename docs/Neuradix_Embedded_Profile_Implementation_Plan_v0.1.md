---
title: "Neuradix Embedded Profile — Implementation Plan"
author: "Engineering"
date: "17 July 2026"
version: "0.1 Draft"
status: "For review"
---

# Objective

Implement a bounded microcontroller profile that participates in the same Neuradix contracts, health, safety and simulation ecosystem without attempting to run the full Linux runtime.

# Tiers

- Embedded Tiny: generated Arduino/AVR C/C++.
- Embedded MCU: native Rust `no_std`.
- Embedded Connected: network-capable MCU.
- Embedded High: richer multi-component MCU/RTOS node.

# First targets

1. host simulation;
2. ESP32-C3;
3. RP2040;
4. STM32F4/G4;
5. Arduino Uno R3 generated C++;
6. nRF52;
7. ESP32-S3 / Uno R4.

# Work packages

## WP1 Contract projections

- Rust `no_std`;
- Arduino C++;
- embedded C;
- golden encode/decode vectors;
- schema and deployment identity.

## WP2 Embedded core

- static components;
- bounded ports;
- health;
- watchdog;
- authority lease;
- safe state;
- executor-neutral APIs.

## WP3 Executors

- host static loop;
- Embassy;
- RTIC.

## WP4 Transports

- serial;
- CAN;
- CRC and sequence handling;
- optional gateway adapters.

## WP5 Tooling

- `neuradix embedded` commands;
- build;
- size;
- flash;
- monitor;
- conformance.

## WP6 Studio

- firmware identity;
- reset/watchdog;
- resource usage;
- safety intervention.

# Reference demonstration

An AUV propulsion node receives a thrust request from Edge, validates the authority lease, enforces rate/current/thermal limits, applies output, reports health and enters safe state on communication loss.

# Exit criteria

- host simulation and one physical MCU use the same contracts;
- lease expiry triggers safe state;
- flash and RAM budgets are reported;
- firmware/deployment identity is visible;
- CLI builds, flashes and monitors;
- Arduino C++ projection passes golden-vector tests.
