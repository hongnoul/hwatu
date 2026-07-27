# Continuous improvement

Hwatu improves from observed agent workflows, not from feature volume. The
loop is deliberately small enough to run every week.

## The outcome

The activation event is a **first successful verification**: a new user
installs hwatu and gets a valid result from `hwatu doctor`, `hwatu demo`, or
`hwatu check <url>` through the CLI, MCP, or native socket.

For each onboarding, record only this aggregate funnel unless the user
voluntarily provides more detail:

1. installation attempted;
2. daemon started;
3. agent connected;
4. first verification succeeded;
5. another verification ran on a later day.

Hwatu does not phone home. Maintainers collect this evidence from issue forms,
release discussions, and consensual onboarding sessions. Do not request page
contents, screenshots, cookies, private URLs, or proprietary source code.

## The weekly loop

### 1. Collect

- Reply to every issue and launch response.
- Invite users to submit a [use report][use-report], including successful use.
- During onboarding, observe rather than coach until the user becomes stuck.
- Turn every reproducible failure into a minimal public fixture.

Ask three questions:

1. Which agent or harness invoked hwatu?
2. What were you trying to verify?
3. Where did you hesitate, fail, or fall back to another tool?

### 2. Classify

Label each signal by funnel stage: `install`, `start`, `connect`, `verify`, or
`repeat`. Also label its workflow (`cli`, `mcp`, `socket`) and environment when
known. One report may produce several independently testable problems.

### 3. Rank

Score candidate work from 0 to 3 on each dimension:

| Dimension | 0 | 3 |
| --- | --- | --- |
| Frequency | hypothetical | repeated by 3+ users |
| Activation impact | cosmetic | blocks first verification |
| Strategic fit | generic browser feature | strengthens agent verification or hand-off |
| Confidence | assumption | reproduced or directly observed |
| Cost | multi-week/redesign | less than one focused day |

Priority is `frequency + activation + fit + confidence + cost`. Security,
data-loss, and correctness defects bypass scoring. Record the evidence and the
reason whenever roadmap order changes so public claims remain explainable.

### 4. Ship and close the loop

- Add a regression test or fixture before fixing a reproducible defect.
- Run the repository checks named in `CONTRIBUTING.md`.
- Link the release or commit from the originating report.
- Ask the reporter to retry the exact workflow.
- Publish one concrete weekly note: problem, measured change, and reproducer.

## Monthly review

Review the five funnel counts, recurring fallback tools, unsupported
environments, and issues without a reproducer. Remove stale roadmap items that
have no observed demand. Re-run published benchmarks when changes affect a
performance claim.

The positioning guardrail is: **browser-agnostic, measurable UI verification
for autonomous coding agents**. Work that does not strengthen that promise or
the human hand-off loop needs unusually strong evidence.

[use-report]: https://github.com/hongnoul/hwatu/issues/new?template=use-report.yml
