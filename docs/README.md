# Neuradix documentation

The current direction is a unified robotics development and operations platform from Arduino to enterprise, with conventional control, simulation and optional AI sharing one engineering model.

## Current document set

| Document | Authority and purpose |
|---|---|
| [Functional Specification v0.6](Neuradix_Robotics_Platform_Functional_Specification_v0.6.md) | Current functional/technical planning requirements; retains detailed domain ambitions with separate release scopes |
| [Detailed Implementation Plan v0.4](Neuradix_Implementation_Plan_v0.4.md) | Work packages, dependencies, owner roles, engineer-week ranges, 90-day capacity plan, acceptance catalogue and traceability |
| [Review and Strategy v1.0](Neuradix_Robotics_Platform_Review_and_Strategy_v1.0.md) | Repository-native revised research report with pinned code findings and external citations |
| [Capability Status](Neuradix_Capability_Status.md) | Integrated implementation and evidence; partial work packages and open findings |
| [A05 heartbeat evidence](implementation/WP-A05-Worker-Heartbeat.md) | Responsiveness windows, idle scheduling, bounded recovery, migration and remaining acceptance |
| [A05 bounded worker I/O evidence](implementation/WP-A05-Bounded-Worker-IO.md) | Protocol/storage limits, total deadlines, Linux cleanup, API migration and remaining extension acceptance |
| [A04.3 slew alignment evidence](implementation/WP-A04.3-Slew-Alignment.md) | Shared physical rate semantics, changing-period conformance, migration and remaining acceptance |
| [A04.2 command freshness evidence](implementation/WP-A04.2-Command-Freshness.md) | Bounded freshness, deadlines, generations, sequence, scheduling and API migration |
| [A04.1 trusted evaluation evidence](implementation/WP-A04.1-Trusted-Evaluation.md) | Trusted runtime time, numeric invariants, API migration and remaining A04 scope |
| [Gate A codec evidence](implementation/Gate-A-Embedded-Wire-and-ABI.md) | Integrated wire/ABI changes, exact CI results, migration and remaining acceptance work |
| [Embedded Plan v0.2](Neuradix_Embedded_Profile_Implementation_Plan_v0.2.md) | Actual Uno/MCU milestones, target ABI, tooling and conformance |
| [Studio Plan v0.2](Neuradix_Studio_Implementation_Plan_v0.2.md) | Integrated authoring, simulation, deployment and diagnosis |
| [CLI Specification v0.2](Neuradix_CLI_Command_Specification_v0.2.md) | Proposed command surface and compatibility; implemented commands remain separately identified |
| [RFC Backlog v0.4](Neuradix_RFC_Backlog_v0.4.md) | Integrated RFC alignment and reserved new decisions |
| [Studio XR Plan v0.2](Neuradix_Studio_XR_Implementation_Plan_v0.2.md) | Supplemental later-profile design; current platform gates override older sequencing |

## Precedence and maturity

User-approved scope and current project decisions govern the work. Within this current planning document set, v0.6 owns normative behaviour, v0.4 owns sequencing/estimates, and Capability Status owns implementation claims. Specialist plans refine the same work packages; they do not create separate release commitments. The research report supplies rationale, not evidence that recommendations are already implemented. RFCs and ADRs keep their recorded status until explicitly amended.

The first supported release is the named cross-target workflow through Gate E. Enterprise HA, additional boards/protocols, domain packs, optional AI, Swarm, XR and Flight discovery are Gate F extensions with their own evidence. All requirements apply when the corresponding feature/profile is claimed; a future requirement does not make a current feature available.

## Historical versions

Functional Specifications [v0.4](Neuradix_Robotics_Platform_Functional_Specification_v0.4.md), [v0.5](Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) and its [addendum](Neuradix_Robotics_Platform_Functional_Specification_v0.5_Addendum.md); Implementation Plans [v0.1](Neuradix_Implementation_Plan_v0.1.md), [v0.2](Neuradix_Implementation_Plan_v0.2.md), [v0.3](Neuradix_Implementation_Plan_v0.3.md); earlier [Embedded](Neuradix_Embedded_Profile_Implementation_Plan_v0.1.md), [Studio](Neuradix_Studio_Implementation_Plan_v0.1.md), [CLI](Neuradix_CLI_Command_Specification_v0.1.md), and RFC backlogs [v0.2](Neuradix_RFC_Backlog_v0.2.md)/[v0.3](Neuradix_RFC_Backlog_v0.3.md) are retained for history and are superseded for current scope/sequencing.

The v0.5 generator is historical. It must not rewrite the current README or v0.6. Edit v0.6 directly and keep its contents links and requirement traceability consistent with the implementation plan.
