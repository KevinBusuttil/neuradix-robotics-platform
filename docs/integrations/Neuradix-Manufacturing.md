# Pending Neuradix Manufacturing integration

Status: **planned integration; no runtime adapter or physical support added by this document.**
Owner: Kevin Busuttil (product/architecture); delivery engineering and automation/commissioning
roles to be assigned for each bounded increment. Recorded 27 September 2026; merge status updated
28 September 2026.

## Purpose and product boundary

Neuradix Manufacturing is a Flutter/Rust factory operations product with Storekeeper, Operator,
Quality, Maintenance and Supervisor workspaces. Its first commercial configuration must connect
one named physical machine and one robot or robot-cell workflow **through Neuradix Robotics Platform**.
ERPNext is the first ERP; Atlas Team, Odoo and Microsoft ERP connectors are later profiles.

Repositories:

- [Manufacturing](https://github.com/KevinBusuttil/neuradix-manufacturing): private application,
  operational domain, ERP clients and complete product/commercial plans.
- [ERPNext companion](https://github.com/KevinBusuttil/neuradix-manufacturing-erpnext): public
  Frappe application and self-contained [implementation plan](https://github.com/KevinBusuttil/neuradix-manufacturing-erpnext/blob/main/docs/implementation-plan.md).
- This repository: qualified local execution, authority, device adapters and equipment evidence.

Cross-repository links target main. The coordinated foundation PRs
([Manufacturing #1](https://github.com/KevinBusuttil/neuradix-manufacturing/pull/1),
[ERPNext companion #1](https://github.com/KevinBusuttil/neuradix-manufacturing-erpnext/pull/1) and
this repository's [#19](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/19)) merged on 27 September 2026. This note contains
the requirements needed by platform contributors even without access to the private application.

Manufacturing decides approved production intent, including work order, lot, quality and maintenance
context. Platform runtime admits permitted local tasks, supervises execution and reports evidence.
Existing PLCs, robot controllers and protective systems retain their commissioned control authority.
No Flutter or ERP connector may bypass this boundary or acquire a direct actuator path. AI is optional;
local protective behavior cannot depend on cloud/ERP availability or subscription checks.

## Current evidence and limits

Recorded 27 September 2026: the inspected default branch was
`b35de66a16df27af4187c0479ed6c8965d3c0239`, containing the PR #18 merge and offline delayed-feedback
graph validation.

Updated 28 September 2026: [PR #19](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/19) merged this note on 27 September 2026 as
`d4911fa38c9c7efa2cf006c4ae1a4c4694f92cb4`; [main CI 36310554902](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36310554902) passed.
[PR #20](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/20) then merged a host scalar actuator-permission adapter as
`7b2be979f94c9fdf7c9192c4a85f11cef1993340`; [post-merge CI 36361418592](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36361418592)
passed. Capability and plan entries record PR #18 and PR #20 as integrated. Offline graph validation
and that host adapter are not deployed graph supervision, embedded or physical driver enforcement,
authentication or a qualified manufacturing device adapter. WP-A08 remains partial. This
documentation increment does not change those implementation claims or close Gate A.

Consult [Capability Status](../Neuradix_Capability_Status.md) and actual current code before implementation.
A source file, graph acceptance, host test, simulated command or compiler check cannot establish
physical device readiness. Qualify the exact controller, firmware, adapter and deployment revision.

## Dependency mapping to the existing platform plan

| Existing area | Manufacturing need | Required evidence |
|---|---|---|
| WP-A04 / WP-A08 | Trusted identity/time, fresh commands, authority generations and runtime physical driver permission separation | Unauthorized/expired/replayed intent cannot reach a live driver; graph validation alone is insufficient |
| WP-B07 | Supervised local Edge execution and stable services | Admitted task ownership and local fault response survive UI/WAN loss within commissioned policy |
| WP-D03 and applicable transport prerequisites | Authenticated, bounded site transport and recoverable events | Partition/reconnect, duplicate delivery, stale state and backlog behavior measured |
| WP-F03 selected industrial adapter scope | One selected machine interface and robot/cell workflow | Actual hardware access, vendor contract, controller/firmware matrix and commissioning evidence |
| WP-E02, conditional | ROS bridge only when selected equipment uses it | Qualified ROS transport/payload behavior for that path; otherwise not a prerequisite |

This maps dependencies; it does not declare the whole packages implemented or require completion of
every broader platform feature. Existing platform A–F gates and acceptance remain in force when
claiming those platform releases. Manufacturing can start in parallel, and its selected profile has
its own qualification. Any pull-forward of F03 implementation capacity must be explicitly recorded
in the combined backlog/roadmap before displacing committed A–E work. This note records the demand,
not an automatic wholesale reprioritization of the existing platform programme.

## Proposed bounded integration increments

| ID | Manufacturing gate | Deliverable | Exit evidence |
|---|---|---|---|
| RI-01 | M0–M1 | Named devices, hardware access, capability gap review and task/event contract decision | Agreed models/firmware, permitted operations, principal binding, recovery policy and estimates |
| RI-02 | M2 | Required Edge identity, local supervision, transport and adapter foundations | Negative permission, expiry and provenance tests; no live bypass |
| RI-03 | M3 | Supervised physical machine and robot demonstration | Work order/task/device/output references correlated; durable outcome discovery and restart behavior |
| RI-04 | M4–M5 | Quality/maintenance admission integration, telemetry and fault qualification | Holds, restrictions, WAN loss, stale state, local override and controller restart exercised |
| RI-05 | M6–M8 | Sustained pilots and supported profile | Repeatable commissioning/install, diagnostics, support ownership and release compatibility matrix |

These RI identifiers coordinate work across repositories; they do not consume reserved RFC IDs or
replace existing WP identifiers. Each implementation increment must reference the applicable WP,
requirements and acceptance evidence. A draft Manufacturing JSON schema is only an integration
proposal until the platform contract is reviewed and versioned here.

## Contract decisions required before live tasks

Every task needs durable task and idempotency identity, site/equipment/capability identifiers,
authenticated principal binding, permitted operation/parameters, production/lot references,
authority version and a valid expiry/clock relation. Define permitted states and events separately:
requested, admitted, dispatched/accepted, running and completed, with rejected, expired, failed,
cancelled and unknown-outcome branches. A cancel request is not proof of a physical stop.

Events need identity, source session/sequence, equipment time with clock provenance, receipt time,
quality/freshness and execution mode (live, simulated or replayed). Define parameter ranges/units,
version negotiation, bounded buffering/backpressure and outcome lookup. Manufacturing wall-clock
or display time must not renew physical authority. A valid schema is not authentication.

For non-idempotent physical work, a lost reply triggers discovery/reconciliation; it must not blindly
repeat a move, start, dose or release. Record physical execution and ERP confirmation as distinct
outcomes. Existing platform validity and lineage primitives are reused where suitable, with their
trusted-startup, shared-time and periodic-evaluation assumptions kept explicit.

Quality holds and maintenance restrictions block new conflicting task admission. Running work follows
the approved local equipment policy; there is no universal cloud-disconnect emergency-stop rule.
Manual override, restart, clearance and return-to-service require named owners and audited evidence.
Maintenance completion in ERPNext alone does not authorize equipment release.

## Required profile qualification

Before M3: supervised engineering work on actual selected hardware, with an agreed local procedure.
Before M5/production pilot: all selected runtime/driver/transport dependencies qualified, including:

- Authenticated access and runtime physical permissions, not just topology validation.
- Durable task identity, duplicate delivery, uncertain outcomes and controller/Edge restart.
- Freshness/expiry, invalid clock relation, authority changes and stale/out-of-order telemetry.
- Quality/maintenance admission restrictions and commissioned behavior during an active task.
- Network/WAN loss, reconnect without automatic physical replay, local override and recovery.
- Isolation of simulation/replay from live dispatch; provenance visible to operators.
- Supported adapter/controller/deployment versions, traceability and bounded telemetry load.

These correspond to Manufacturing acceptance A21–A30. ERP reconciliation A01–A20 and UI/UX A31–A36
remain separate product evidence, not platform-only tests. Commercial v1.0 requires supported,
repeatable operation at the selected profile, not universal robot/device compatibility.

## Coordination and next action

Start RI-01 alongside Manufacturing discovery/design and companion transaction mapping. Review
remaining platform work against current code and actual equipment; record one accountable owner,
shared estimate, dependencies, contract version and evidence per item. Count shared work once.
Broader boards, fleet, unrelated robot families and optional AI can continue independently.
The manufacturing programme does not fund or require completion of the entire Robotics Platform.

This change is documentation only. The next code increment should follow the existing platform
contribution/validation workflow and be bounded by its approved WP, rather than implementing a
machine driver merely because this integration note exists.
