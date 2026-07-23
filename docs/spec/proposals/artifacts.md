# Part VIII — Proposed: artifact selection and channels

*[LiNix v7](../../SPEC.md) — the map is there; this is one part of it.*

**Status: BUILT. Migrated into Part II (artifact selection, V.48); header corrected 2026-07-22.**
Raised 2026-07-19 and proposed here; it read "Not built. Not in Part II." for three sessions
after it was both. What is built: `formats` as an ordered preference (`src/backends/artifact/`),
the `priority`-level block (D7), `channel` on the two backends that have one (`snap --channel`,
`flatpak name//branch`), `@asset=all` (session 7), and the resolved asset/url/format/hash in
`locks/github.toml`. **One deviation from the rulings is recorded in Part VII**: specificity
beats shortest-filename, contra D3.

This section stays as the reasoning — the alternatives weighed and refused, which Part II does
not carry. **A "Proposed / Not built" banner on shipped work is not a harmless stale line:** it
is read as capability that does not exist, or as absence by someone about to build it twice.

The problem: `github:sharkdp/fd` does not name a file. A release ships a `.deb`, an `.rpm`, an
`.AppImage`, a `.tar.gz`, a `.zip` and a bare binary, and today the backend picks one. **A
declaration that resolves to a different artifact on two machines — or on the same machine next
month — is not declarative.** The user has to be able to say which, and that answer varies by
machine and by backend.

## VIII.1 Two axes, deliberately not unified

| key | question it answers | backends |
|---|---|---|
| `formats` | *which of these files do I download* | `github`, `web` (when the spec isn't one URL) |
| `channel` | *which version stream do I track* | `snap`, `flatpak` |

**They look alike and they are not the same question, so they stay two keys.** A snap channel
is not an artifact — snap ships one artifact and several streams of it. Folding them into one
key would produce a value whose meaning depends on the backend, which is the same defect as the
old `backend_priority`/`enabled_backends`/`default_backend` cluster (V.15) in miniature. Two
keys, each meaningless on the wrong backend, each an error there.

## VIII.2 `formats` — ordered preference, not a filter

An ordered list. **First match wins; a later entry is a fallback, never an addition.** The
vocabulary is closed, and an unrecognised name is an error naming the legal set — the same rule
as every other unrecognised line (II.2):

```
deb  rpm  appimage  tarball  zip  exe  msi  pkg  dmg  binary
```

`tarball` covers `.tar.gz`/`.tar.xz`/`.tgz`/`.tar.bz2`; `binary` is an unarchived executable
asset. **`appimage` here is a format, not the `appimage:` backend** — a GitHub release that
ships an `.AppImage` is still installed by `github`.

### Where it is declared

**In the `priority` file, as an options body on the backend line.** No new file and no new block
kind: `priority` already takes `when` blocks (II.6), and II.2 already says a declaration's body
is options with a repeated key making a list.

```
apt
dnf

when family == debian {
  github {
    formats = deb
    formats = appimage
    formats = tarball
  }
}

github {
  formats = appimage
  formats = tarball
  formats = binary
}
```

Per-line override uses the options forms that already exist. Short form for one, block form for
several — **`@formats=deb,rpm` is the comma error (II.2), not a list**:

```
github:sharkdp/fd@formats=deb                 one, short form

github:BurntSushi/ripgrep {                   several, block form
  formats = rpm
  formats = tarball
}
```

**Precedence: the line beats `priority`, `priority` beats the built-in default.** A line's list
*replaces* the backend's list; it does not extend it. Half-overriding an ordered list is how you
get an order nobody wrote. *(Asserted, not ruled — D9.)*

### The default, when nothing is declared

**Derived from detected facts, not configured** (II.1). Debian family → `deb`, then `appimage`,
`tarball`, `binary`. Fedora/SUSE family → `rpm`, then the same tail. Everything else →
`appimage`, `tarball`, `binary`. Windows → `exe`, `msi`, `zip`.

**A fresh repo installs the right thing without a `formats` line anywhere**, which is the point:
if it *is* the command's job, it is automatic and it does not ask.

### Arch and OS are not preferences

LiNix filters the asset list to this machine's OS and architecture **before** `formats` is
consulted, from detected facts. `formats` only orders what already runs here. So `formats = deb`
on an arm64 box selects the arm64 `.deb` and never the amd64 one, and **there is no
`@arch=` option** — a declaration that lets you request an artifact your machine cannot execute
is a footgun with no use case.

### When nothing matches

**Error. Never a fallback to "whatever was first."** The error prints what the release actually
offered, so the fix is visible without opening a browser:

```
github:sharkdp/fd — no asset matches your formats.
  wanted:  deb, appimage
  release v10.2.0 offers, for linux/x86_64:
    fd-v10.2.0-x86_64-unknown-linux-gnu.tar.gz   tarball
    fd_10.2.0_amd64.deb                          deb  (arm64 only)
  add `formats = tarball`, or pin one with @formats=.
```

### The lock, and the guard

- **The resolved asset name, its URL and its format go in `locks/github`.** A lock that records
  only a version leaves the artifact free to change under a pinned declaration, which is the
  bug this whole section exists to close.
- **`@sha256` outranks everything.** When both are given, `formats` selects and the checksum
  verifies; a mismatch is an error, not a re-selection down the list. Selecting a *different*
  asset because the pinned one failed its hash would turn a supply-chain alarm into a silent
  substitution. This ties into the unimplemented SEC work in Phase 5 (`web`/`appimage`/`github`
  checksums) and must not land before it. **But one hash cannot cover an asset that varies by
  machine — see D6, which is unresolved and may move checksums into the lock entirely.**
- Changing `formats` changes the installed artifact, so it goes through the plan and the guard
  like any other change.

## VIII.3 `channel` — one value, no list

```
snap:code@channel=stable
flatpak:org.gimp.GIMP@channel=stable
```

**Singular, and not ordered.** There is no "try edge, fall back to stable" — a fallback across
version streams would silently downgrade a machine, and the user asked for a stream, not a
best-effort. A channel the backend does not publish is an error naming the ones it does.

Declarable in `priority` the same way, for a machine-wide default:

```
flatpak {
  channel = stable
}
```

**Default when unset: the backend's own default** (`stable` for snap, the remote's default branch
for flatpak) — detected, not typed into the file. Snap's `--classic` confinement is a **third**
axis and is not `channel`; it is deliberately left out of this section rather than smuggled in.

## VIII.4 Backends this does not apply to, and why

- **`appimage:`** — no `formats`. The backend name *is* the format; `appimage:foo@formats=deb`
  is a contradiction and is an error, not an ignored key.
- **`web:URL`** — when the spec is one explicit URL there is nothing to choose, and `formats`
  there is an error. It applies only if a `web:` spec ever resolves to several candidates.
- **`apt`/`dnf`/`cargo`/…** — the ecosystem already decided the artifact. No `formats`, no
  `channel`.

**The general rule, so a future backend inherits it without an edit here:** `formats` is legal on
a backend that resolves one name to several downloadable artifacts; `channel` is legal on a
backend that publishes one artifact in several version streams. **On any other backend both are
errors.** Silently ignoring an option the user wrote is how a config grows lines that do nothing.


---

**Decisions: D1–D17.** They live in [the decision register](../decisions.md), with a status on
each — this part states the shape, the register states what is still unanswered.
