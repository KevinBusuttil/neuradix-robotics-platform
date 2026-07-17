#!/usr/bin/env python3
"""Build the self-contained Neuradix Functional Specification v0.5.

The generator integrates the v0.5 Embedded/CLI addendum into the complete v0.4
specification. It deliberately replaces the relevant normative sections instead
of concatenating two documents, leaving one authoritative specification.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs"
BASE_PATH = DOCS / "Neuradix_Robotics_Platform_Functional_Specification_v0.4.md"
ADDENDUM_PATH = DOCS / "Neuradix_Robotics_Platform_Functional_Specification_v0.5_Addendum.md"
OUTPUT_PATH = DOCS / "Neuradix_Robotics_Platform_Functional_Specification_v0.5.md"
README_PATH = ROOT / "README.md"


def extract(text: str, start_heading: str, end_heading: str) -> str:
    start = text.find(start_heading)
    end = text.find(end_heading, start + len(start_heading))
    if start < 0 or end < 0:
        raise RuntimeError(f"Unable to extract {start_heading!r} to {end_heading!r}")
    return text[start:end].strip()


def replace_numbered_section(text: str, number: int, replacement: str) -> str:
    next_number = number + 1
    pattern = re.compile(
        rf"(?ms)^# {number}\. [^\n]+\n.*?(?=^# {next_number}\. [^\n]+\n)"
    )
    updated, count = pattern.subn(replacement.rstrip() + "\n\n", text, count=1)
    if count != 1:
        raise RuntimeError(f"Expected exactly one section {number}; found {count}")
    return updated


def append_before_section(text: str, next_number: int, material: str) -> str:
    marker = re.search(rf"(?m)^# {next_number}\. ", text)
    if not marker:
        raise RuntimeError(f"Unable to locate section {next_number}")
    return text[: marker.start()].rstrip() + "\n\n" + material.strip() + "\n\n" + text[marker.start() :]


def transform_embedded(raw: str) -> str:
    output: list[str] = []
    for line in raw.splitlines():
        if line == "# 1. Updated platform boundary":
            output.append("## 20.1 Platform boundary and participation model")
        elif line == "# 2. Neuradix Embedded profiles":
            output.append("## 20.2 Embedded profiles")
        elif match := re.match(r"## 2\.(\d+) (.+)", line):
            output.append(f"### 20.2.{match.group(1)} {match.group(2)}")
        elif line == "# 3. Embedded board support policy":
            output.append("## 20.3 Board support policy")
        elif match := re.match(r"## 3\.(\d+) (.+)", line):
            output.append(f"### 20.3.{match.group(1)} {match.group(2)}")
        elif line == "# 4. Embedded normative requirements":
            output.append("## 20.4 Normative requirements")
        else:
            output.append(line)
    return "\n".join(output).strip()


def transform_cli(raw: str) -> str:
    output: list[str] = []
    for line in raw.splitlines():
        if line == "# 5. Neuradix CLI product contract":
            output.append("## 37.1 Product and automation contract")
        elif match := re.match(r"## 5\.(\d+) (.+)", line):
            output.append(f"### 37.1.{match.group(1)} {match.group(2)}")
        elif line == "# 6. CLI command groups":
            output.append("## 37.2 Command groups")
        elif match := re.match(r"## 6\.(\d+) (.+)", line):
            output.append(f"### 37.2.{match.group(1)} {match.group(2)}")
        elif line == "# 7. CLI normative requirements":
            output.append("## 37.3 Normative requirements")
        else:
            output.append(line)
    return "\n".join(output).strip()


def body_after_heading(raw: str) -> str:
    return raw.split("\n", 1)[1].strip() if "\n" in raw else ""


def main() -> None:
    base = BASE_PATH.read_text(encoding="utf-8")
    addendum = ADDENDUM_PATH.read_text(encoding="utf-8")

    # Document identity and canonical status.
    base = base.replace('date: "2 July 2026"', 'date: "17 July 2026"', 1)
    base = base.replace('version: "0.4 Draft"', 'version: "0.5 Draft"', 1)
    base = base.replace("| Version | 0.4 Draft |", "| Version | 0.5 Draft |", 1)
    base = base.replace("| Date | 2 July 2026 |", "| Date | 17 July 2026 |", 1)
    base = base.replace(
        "| Primary implementation languages | Rust and Python |",
        "| Primary implementation languages | Rust and Python, with generated C/C++ for constrained embedded targets |",
        1,
    )
    status_anchor = (
        "This document defines the product architecture, sub-platform functions, interfaces, "
    )
    canonical_note = (
        "This v0.5 document is the single authoritative functional specification. It supersedes "
        "Functional Specification v0.4 and the separate Embedded/CLI v0.5 addendum.\n\n"
    )
    if canonical_note not in base:
        base = base.replace(status_anchor, canonical_note + status_anchor, 1)

    # Replace the earlier overview image with the current light ecosystem map.
    mind_map = """# Platform overview mind map

The following light-theme ecosystem mind map summarises the complete Neuradix platform boundary, including common foundations, deployment profiles, autonomy, data, operations, human interaction, domain profiles, embedded systems and ecosystem stakeholders.

![Neuradix Robotics Platform ecosystem mind map.](assets/neuradix_platform_ecosystem_mind_map_light.svg)

*Figure 0 — Neuradix Robotics Platform ecosystem overview.*
"""
    base = re.sub(
        r"(?ms)^# Platform overview mind map\n.*?(?=^# 1\. Naming and product identity)",
        mind_map.rstrip() + "\n\n",
        base,
        count=1,
    )

    # Integrate Embedded into the existing numbered section.
    embedded_raw = extract(
        addendum,
        "# 1. Updated platform boundary",
        "# 5. Neuradix CLI product contract",
    )
    embedded_section = (
        "# 20. Embedded and microcontroller profile\n\n"
        "Neuradix Embedded allows microcontrollers to participate as first-class contract, "
        "health and safety endpoints without requiring the full Linux-class runtime. The profile "
        "uses static generation, bounded resources and target-appropriate runtimes.\n\n"
        + transform_embedded(embedded_raw)
    )
    base = replace_numbered_section(base, 20, embedded_section)

    # Integrate the stable CLI product surface into Section 37.
    cli_raw = extract(addendum, "# 5. Neuradix CLI product contract", "# 8. Repository additions")
    cli_section = (
        "# 37. Command-line interface\n\n"
        "The command-line interface is a stable developer, operator and automation API. Studio, "
        "CI and external orchestration SHOULD invoke the same underlying services and result models "
        "rather than implementing separate operational logic.\n\n"
        + transform_cli(cli_raw)
    )
    base = replace_numbered_section(base, 37, cli_section)

    # Embedded verification requirements belong with the common test architecture.
    embedded_tests = """## 38.6 Embedded conformance and hardware verification

Supported embedded targets SHALL have automated build and conformance coverage appropriate to their support level. The suite SHOULD include contract encoding compatibility, static memory budgets, queue overflow behaviour, watchdog and reset handling, command-lease expiry, communication loss, safe-state transitions, host-simulation parity and target-specific hardware-in-the-loop tests.
"""
    if "## 38.6 Embedded conformance and hardware verification" not in base:
        base = append_before_section(base, 39, embedded_tests)

    # Repository additions from the addendum.
    repository_raw = extract(addendum, "# 8. Repository additions", "# 9. Updated delivery sequence")
    repository_addition = "## 40.1 Embedded and CLI repository additions\n\n" + body_after_heading(repository_raw)
    if "## 40.1 Embedded and CLI repository additions" not in base:
        base = append_before_section(base, 41, repository_addition)

    # Delivery sequence, acceptance and decisions are integrated into their existing sections.
    delivery_raw = extract(addendum, "# 9. Updated delivery sequence", "# 10. Updated acceptance criteria")
    delivery_addition = "## 43.8 Embedded and CLI integration sequence\n\n" + body_after_heading(delivery_raw)
    if "## 43.8 Embedded and CLI integration sequence" not in base:
        base = append_before_section(base, 44, delivery_addition)

    acceptance_raw = extract(addendum, "# 10. Updated acceptance criteria", "# 11. Decisions required before implementation")
    acceptance_addition = "## 45.1 Embedded and CLI preview acceptance\n\n" + body_after_heading(acceptance_raw)
    if "## 45.1 Embedded and CLI preview acceptance" not in base:
        base = append_before_section(base, 46, acceptance_addition)

    decisions_raw = addendum[addendum.find("# 11. Decisions required before implementation") :].strip()
    decisions_addition = "## 47.1 Embedded and CLI decisions\n\n" + body_after_heading(decisions_raw)
    if "## 47.1 Embedded and CLI decisions" not in base:
        base = append_before_section(base, 48, decisions_addition)

    embedded_90_days = """## Embedded extension immediately after the core 90-day slice

Once the single-AUV contract/runtime/record/replay path is stable, the next narrowly bounded increment SHALL:

- generate one contract for Rust `std`, Rust `no_std`, Python and Arduino C++;
- run the embedded component against a host-simulated hardware capability;
- deploy the component to one native Rust MCU target, initially ESP32-C3 or RP2040;
- generate and run one constrained Arduino C++ endpoint;
- demonstrate command-lease expiry and a local safe state;
- report flash, RAM, watchdog, reset and contract identity through the CLI and Studio.

This work proves the shared-contract thesis and MUST NOT expand into broad board support before the initial vertical slice is complete.
"""
    if "## Embedded extension immediately after the core 90-day slice" not in base:
        base = append_before_section(base, 49, embedded_90_days)

    # Correct backlog priority: core embedded architecture is not a late P4 afterthought.
    base = base.replace(
        "- CLI and testkit.\n",
        "- CLI command/output contract and testkit;\n- embedded contract projections and static-topology architecture.\n",
        1,
    )
    base = base.replace(
        "- Studio graph and timeline.\n",
        "- Studio graph and timeline;\n- first native `no_std` MCU target and generated Arduino C++ endpoint.\n",
        1,
    )
    base = base.replace("- embedded profile;\n", "", 1)

    # Add risks that are introduced by the broader embedded/CLI boundary.
    risk_anchor = "| Swarm and domain scope expands too quickly | programme dilution | staged AUV and UAV reference demonstrations with shared primitives |"
    extra_risks = (
        risk_anchor
        + "\n| Attempting identical runtime capability on every microcontroller | unusable or unsafe embedded design | explicit Embedded Tiny/MCU/Connected/High tiers and generated static profiles |"
        + "\n| Direct CLI or Studio hardware commands bypass authority | unsafe actuation and weak auditability | semantic intent, test-only profiles, Ground/onboard Safety enforcement and mandatory audit |"
        + "\n| Supporting too many boards before conformance exists | fragmented maintenance and false compatibility claims | one native MCU and one Arduino C++ target first; published support levels |"
    )
    base = base.replace(risk_anchor, extra_risks, 1)

    # Validate that the result is self-contained and correctly integrated.
    required_fragments = [
        "version: \"0.5 Draft\"",
        "# 20. Embedded and microcontroller profile",
        "NRX-EMB-020",
        "# 37. Command-line interface",
        "NRX-CLI-020",
        "neuradix embedded targets",
        "neuradix_platform_ecosystem_mind_map_light.svg",
        "## 45.1 Embedded and CLI preview acceptance",
    ]
    missing = [fragment for fragment in required_fragments if fragment not in base]
    if missing:
        raise RuntimeError(f"Generated specification is missing: {missing}")
    if base.count("# 20. Embedded and microcontroller profile") != 1:
        raise RuntimeError("Section 20 was not replaced cleanly")
    if base.count("# 37. Command-line interface") != 1:
        raise RuntimeError("Section 37 was not replaced cleanly")

    OUTPUT_PATH.write_text(base.rstrip() + "\n", encoding="utf-8")

    # Point new readers at the single canonical functional specification.
    readme = README_PATH.read_text(encoding="utf-8")
    readme = re.sub(
        r"- \[Product, Functional and Technical Specification v0\.4\]\([^\n]+\)\n"
        r"- \[Embedded and CLI Functional Addendum v0\.5\]\([^\n]+\)\n",
        "- [Product, Functional and Technical Specification v0.5](docs/Neuradix_Robotics_Platform_Functional_Specification_v0.5.md)\n",
        readme,
        count=1,
    )
    README_PATH.write_text(readme, encoding="utf-8")

    print(f"Generated {OUTPUT_PATH.relative_to(ROOT)} ({len(base.splitlines())} lines)")


if __name__ == "__main__":
    main()
