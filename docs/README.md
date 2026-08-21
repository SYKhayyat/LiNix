# The `docs/` directory

There is a lot in here, and most of it is not for you on day one. This page says what each file
is and whether you need it.

## Start here

| file | what it is |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | How the code is arranged, and how a `sync` flows through it. **Read this first.** |
| [`DEVELOPMENT.md`](DEVELOPMENT.md) | Build, run without wrecking your own machine, test, debug. |
| [`TAKING-OVER.md`](TAKING-OVER.md) | For an inheritor rather than a contributor: read the CI board, tell an ecosystem failure from a code one, and know which reds are not yours. |
| [`../CONTRIBUTING.md`](../CONTRIBUTING.md) | The working agreement: conventions, verification, what review asks. |
| [`../README.md`](../README.md) | The user-facing manual. |
| [`../examples/`](../examples/) | Working config files — a module, a resources module, a profile, a commented `preferences.toml`. Parsed by the test suite with the real grammar, so they cannot rot. |

## The specification

`SPEC.md` is the entry point and holds the map; the spec itself is one file per part under
`spec/`. It is the authority on design — where code and `spec/target-state.md` disagree, the code
is wrong.

| file | part | when to read it |
|---|---|---|
| [`SPEC.md`](SPEC.md) | — | The map. Start here for anything design-shaped. |
| [`spec/principles.md`](spec/principles.md) | I | The eight principles that settle arguments. |
| [`spec/target-state.md`](spec/target-state.md) | II | **Canonical.** What is supposed to exist. |
| [`spec/why.md`](spec/why.md) | V | The bug behind every Part II rule. **Read the entry before changing its rule.** |
| [`spec/plan.md`](spec/plan.md) | III + IV | The work in dependency order, with exit conditions. |
| [`spec/bugs.md`](spec/bugs.md) | VI | Defects killed by this design, and defects carried forward. |
| [`spec/decisions.md`](spec/decisions.md) | — | Every decision the design forces, with a status. **Do not answer an open one in code.** |

`scripts/decision-count.sh --check` verifies that every number written about the register matches
the register — including the ones inside the register.

## Process documents

Shall is built with heavy AI assistance, and these are the role briefs and review records that
process produces. They are honest history rather than instructions to a contributor, and you can
work on this repo without reading any of them.

| file | what it is |
|---|---|
| `BUILDER.md` | The brief for an agent implementing work orders (`B1`, `B2`, …). |
| `GRADER.md` | The brief for an agent doing adversarial review — the reviewer is told to be the adversary, not the author. |
| `GRADE-<date>.md` | What a grading round found, with measurements. Useful as a record of what has actually been driven versus merely argued. |
| `HANDOFF-<date>.md` | End-of-session state, so the next session resumes without re-deriving. The newest one is the current position. |
| `attic/lessons.md` | Residue of shipped defects. Its own header says it is for a person, once. |

If you want a sense of where the real risk sits, the newest `GRADE-*.md` is the most useful thing
in this list: it is written by someone trying to break the program and it says where it succeeded.
