# Part II — The target state

*[LiNix v7](../SPEC.md) — the map is there; this is one part of it.*

## II.1 Files on disk

**Your repo** — `$LINIX_CONFIG_DIR` or `~/.config/linix`. **This is a git repo.**

```
modules/            your lists              lowercase names       *.txt
profiles/           your choices           Capitalized names
active              which profiles are on
priority            which backends, in order
vars                your own names for conditions
schedules           when LiNix runs itself
locks/              what everything resolved to    one file per backend
preferences.toml    refusals and behaviour
```

**LiNix's data** — `$LINIX_DATA_DIR` or the platform data dir. **Never in git. Never in a
folder LiNix scans.**

```
registry.json       what LiNix currently owns
snapshots/          snapshot metadata, tagged with commit hashes
```

**Secrets** — the environment only. `GITHUB_TOKEN`. Never a file.

*This said `LINIX_GITHUB_TOKEN` until 2026-07-20, and the code never matched it. Ruled the
other way: `GITHUB_TOKEN` is the name `gh` and CI already set, so a machine that has one gets
the higher rate limit without being told to export it twice. The namespacing argument does not
apply to a value that is unambiguously a GitHub credential — and one name either way, never
both.*

**Facts about this machine** — **detected, never configured.** Core count, whether btrfs /
ZFS / Timeshift exists and where, which backends are installed. LiNix looks; it does not
ask you to maintain them by hand on every machine forever. **One deliberate exception:
`max_parallel` (owner ruling, 2026-07-17).** The core count is detected and is the default,
but you may set `max_parallel` by hand to cap concurrency *below* it — a preference (spare the
machine while it works), not a fact LiNix could look up. See V.41.

## II.2 Grammar

### Lines

A file is lines. A line is blank, a comment, a statement, or a block.

```
# whole-line comment                          anywhere
apt:curl                    # trailing        anywhere on a statement
```

**An unrecognised line is an error.** Not a package name. The error names the file, the
line number, and what was expected.

### Statements

```
NAME                          bare package — short for `list:NAME`
BACKEND:NAME                  this manager or nothing (a pin)
BACKEND,BACKEND:NAME          these managers, in this order, and nowhere else
BACKEND,list:NAME             these first, then the rest of `priority`
list:NAME                     every manager in `priority`, in order — then locked (II.7b)
BACKEND:re:PATTERN            regex — matches names in that backend. Must pin one
absent:BACKEND:NAME           declare it must not exist
repo:BACKEND:SPEC             a repository, for that backend
shim:NAME                     a shim
schedule:NAME                 a scheduled task (only in `schedules`)
service:NAME                  a service
link:SOURCE                   a managed file
setting:SCHEMA/KEY            a desktop setting (`@value=…`), read-before-write
use NAME                      reference a module (lowercase) or profile (Capitalized)
```

`use` takes **a name. Never a path, never a URL.** A file from the internet is a fetch step
that puts a module on disk; then you `use` it by name like everything else.

### Options — two forms

**Short form.** `@key=value,key2=value2`.
**A comma in a value is an error**, not a guess: *"commas need the block form."*

```
apt:jq@version=1.6                    ok
apt:jq@version=>=1.0,<2.0             ERROR → "commas need the block form"
apt:curl@2.0                          ERROR → "did you mean @version=2.0?"
```

**Block form.** Everything after the first `=` to end of line is the value: **verbatim,
trimmed.** No escaping is possible and none is needed.

```
apt:nginx {
  after_install = ./setup.sh --flag=a,b
  requires      = apt:libfoo
  requires      = apt:libbar          # a key given twice makes a list
}
```

- **A value cannot contain a newline.** If you need one, that's a file, not an option.
- **`#` does not start a comment inside a block value.** The value includes it.
- A block value containing ` # ` triggers a hint: *"block values are verbatim — did you mean
  a comment? Put it on its own line."*

### Blocks

**The header decides what the body is.** `module` and `when` are keywords; their bodies are
lines. Anything else is a declaration; its body is options.

```
module fancy {          keyword → body is lines
  apt:neovim
}

when os == linux {      keyword → body is lines
  apt:htop
}

apt:nginx {             declaration → body is options
  after_install = ./setup.sh
}
```

**`when` gates the lines inside it. One rule, everywhere** — in a module those lines are
packages; in a profile they're imports; in `priority` they're backends; in `active` they're
profile names. To gate a whole file, wrap it. Keys: `os`, `arch`, `host`, `hostname`, `family`. Operators: `==`, `!=`,
`in [a, b]`.

**`os` is the kernel** (`linux`, `windows`, `macos`); **`family` is the distribution**
(`debian`, `fedora`, `arch`, `suse`, `alpine`), read from `/etc/os-release` and falling back to
the OS name where there are no distributions to tell apart. They are two questions and neither
stands in for the other: `apt` is a `family == debian` fact, not a `linux` one.

### Option keys

| key | meaning |
|---|---|
| `version` | exact or range |
| `hold` | never upgrade. **`@hold` + `@version=` is a contradiction → error** |
| `expires` | **absolute** datetime. Present now, absent after |
| `until` | **absolute** datetime, on `absent:` only. Absent now, present after |
| `requires` | `BACKEND:NAME` — install that first. **A bare name is an error** |
| `after_install`, `before_install`, … | a hook. Hashed and locked |
| `source` | on `shim:` |
| `cron`, `run` | on `schedule:` |
| `target`, `content`, `template`, `decrypt`, `identity` | on `link:` |
| `enabled`, `status` | on `service:` |
| `formats` | ordered artifact preference. Repeated key makes the list. Backends that offer a choice only |
| `asset` | filename or glob narrowing the choice; `all` takes every match |
| `bin` | the executable inside an archive |
| `channel` | one version stream. Backends that publish channels only |
| `sha256` | checksum the resolved artifact must match. Not with `@asset=all` — one hash cannot verify several files |
| `allow_http` | bare flag: this URL may be `http://`. Downloading backends only (SEC2) |
| `unverified` | bare flag: no `@sha256` required on this line. Downloading backends only. **Never implied by `allow_http`** — over HTTP the checksum is the only thing left (SEC2) |

### Artifact selection (V.48)

`github:sharkdp/fd` names a repo, not a file. A release ships a `.deb`, an `.rpm`, an
`.AppImage`, a `.tar.gz` and a bare binary, and **a declaration that resolves to a different
file on two machines is not declarative.**

**`formats` is an ordered preference over a closed vocabulary.** First match wins; a later
entry is a fallback, never an addition. An unrecognised name is an error naming the legal set:

```
deb  rpm  appimage  tarball  zip  exe  msi  pkg  dmg  binary
```

**Arch and OS are not preferences.** The asset list is filtered to what this machine can run
*before* `formats` is consulted, from detected facts. There is no `@arch=`: a declaration that
requests an artifact your machine cannot execute is a footgun with no use case. A filename that
names a foreign OS or architecture is rejected; one that names neither is kept, because absent
evidence is not evidence of mismatch.

**When two assets are still equally legal, the tie-break is one rule in one place:** the format
you asked for first, then the asset that names this machine most explicitly, then the shortest
filename, then alphabetical. **The choice and what it passed over are reported and recorded in
the lock** — a guess that is printed and locked is not the guess that drifts.

**`@asset=` narrows; it does not select.** It takes a filename or a glob (`@asset=*musl*`, which
survives a version bump where an exact name does not). When the pattern still matches several,
the same tie-break applies. **`@asset=all` installs every match** rather than choosing.

**One artifact is deployed under the repo's name; several each keep their own.** A line that
resolves to one file puts it on `PATH` as the repo is called (`github:sharkdp/fd` → `fd`), and
`@bin=` overrides that as it always has. A line that resolves to several cannot: one name
cannot hold two files. Each then keeps the name of the program found inside it, and **two that
would land on the same name is an error naming both files** — never one overwriting the other.

**An archive is extracted and the executable inside it is shimmed**, reusing `shim:` rather
than inventing a second way onto `PATH`. The executable is guessed from the package name;
`@bin=PATH` names it when the guess is wrong, and turns the guess off rather than falling back
to it. **Finding none, or several, is an error listing what the archive held** — never a pick.

**Both keys are errors on the wrong backend.** `formats` is legal where one name resolves to
several downloadable artifacts; `channel` where one artifact ships in several version streams.
Everywhere else the ecosystem already decided, and **silently ignoring an option the user wrote
is how a config grows lines that do nothing.**

**`channel` is singular and unordered.** There is no "try edge, fall back to stable": a fallback
across version streams silently downgrades a machine, and the user asked for a stream, not a
best-effort.

## II.3 Modules

- A module is a **list of lines**.
- **The filename is the module name, lowercased.** `Editors.txt` → module `editors`. A file
  with no `module` block is one module named after the file. Anything outside a block
  belongs to the file's own module.
- **A module can `use` other modules. A module can NEVER reference a profile.** The layering
  rule. **A `use` loop is an error** (II.7).
- **`modules/*.txt`. The folder decides.** Anything else in `modules/` is silently ignored,
  so a `README.md` costs nothing.
- **LiNix only parses what the active profiles reach.** `linix check` parses everything on
  demand.
- **No `present:`.** A bare line already means present.
- `-` subtraction does not exist in modules. `absent:` does.

## II.4 Profiles

- Set math over modules and profiles: `|` union, `&` intersect, `\` difference, `-`
  subtract, parentheses. Directives `exclude` / `intersect`; **`use` is union** (V.46).
- **Set math produces packages, so a profile that uses it resolves to packages, not to
  modules** (V.46). It operates on lines, not on names, so **every surviving package still
  knows the file it came from** and `upgrade --module` still finds it.
- **Order is fixed: gather, then narrow by each `intersect`, then subtract. Subtraction
  always wins**, whatever order you wrote the lines in — otherwise `use gaming` below
  `-steam` quietly puts steam back.
- `intersect` narrows and never adds: a package only the other side has does not appear.
- **A profile MAY hold package lines directly.** Cost, accepted knowingly: a module can
  never reach them (layering rule), so they are unshareable, permanently.
- **Only profiles can be activated.** By name, in `active` — by hand or via `activate` /
  `activate -a` / `deactivate` (II.6).
- **A profile may reference profiles. A `use` loop is an error** (II.7).
- `absent:` does not exist in profiles. `-` does.

## II.5 Naming

- **Profiles are Capitalized. Modules are lowercase.** `(Work | gaming) & security` tells
  you what everything is with zero noise.
- `use` disambiguates a reference from a package. Case disambiguates profile from module.
- Filenames are lowercased into module names, so a filename can never mint a profile.
- **Error messages must teach the rule:** *"no profile named `Editors` — did you mean the
  module `editors`? Profiles are Capitalized, modules are lowercase."*

## II.6 The other files

**`active`** — a plain list of profile names, unioned. Answers exactly one question: *what
is this machine set to right now?* Nothing else goes in it.

**Names, never expressions.** The set math lives inside profiles (II.4). `active` is the one
file you read to know what is on, so it stays a list you can read at a glance. `when` gates
it like any other file (II.2).

```
Work
Gaming

when host == laptop {
  Travel
}
```

**Three commands write it. Nothing else does.**

| form | does |
|---|---|
| `activate NAME…` | `active` becomes exactly this list |
| `activate -a NAME…` | adds to the list |
| `deactivate NAME…` | takes away from the list |

All three **write the file and sync** — the same as editing it by hand, because the file is
the state. Each prints what it touched: `active is now Work, Gaming`.

- **`activate -a` and `deactivate` write names at the top level and never touch a `when`
  block.** A block is something you wrote; those two add to it and subtract from it by hand,
  or not at all. **`activate` is the exception, and it is the whole exception** — see the next
  bullet. *(This bullet used to say "the CLI" and applied to all three verbs, which
  contradicted the one below it. Owner decided 2026-07-17: the set form sets.)*
- **`activate NAME…` overwrites the file — blocks included.** It is the set form; it sets.
  **This is not a special case and gets no extra refusal** (V.44). It does not ask, because
  overwriting the list *is* the command's job — but **it is not silent** (S6): it names every
  block it removed. *"active is now Work, Gaming. Removed the `when host == laptop` block on
  line 4."* **Automatic and silent are different things, and only one of them is a decision
  the user gets to review after the fact.**
- **The asymmetry is the point, and it is the reason `-a` exists.** `activate` is the blunt
  verb: it makes the file say exactly what you typed, and a block is part of what the file
  says. If you want your blocks kept, you want `activate -a` or `deactivate` — the surgical
  pair. **Two verbs that both half-preserve blocks would be two ways to do one thing** (P1);
  one that replaces and two that edit is one way each.
- **`deactivate NAME` removes the name from the top level AND from every `when` block that
  applies to this host.** *"Deactivate" must mean it. A verb that removed the top-level line
  and left the name switched on by a block two lines down would be reporting a state it did
  not reach* — the same defect as `activate` "setting" a list that a block then contradicts.
  If that empties a block, **the block goes too, and it says so**: *"Removed Travel. Removed
  the now-empty `when host == laptop` block on line 4."*
- **A `when` block that does NOT apply to this host is never touched, and that is not an
  exception — it is the same rule.** On the desktop, `when host == laptop { Travel }` is not
  activating anything, so there is nothing there to deactivate. **`active` is a file you
  commit and share; reaching into another host's block from this one would change a machine
  you are not sitting at.** So it changes nothing and says why: *"Travel is not active on this
  host. `active` line 4 activates it when host == laptop — edit that by hand if you meant
  every machine."*
- **This is the one place `deactivate` edits a block and `activate -a` does not**, and the
  asymmetry is not arbitrary: **adding has a choice of where to put the name and removing does
  not.** `-a` appends at the top level because a block is a rule you wrote and it has no
  business joining it. `deactivate` has no such freedom — the name is where it is, and leaving
  it there would make the verb a lie.
- **`activate` with no names is an error:** *"activate needs a profile name. To turn
  everything off, edit `active` yourself."* An unset `$PROFILE` must not empty the machine.
- **`activate -a` on a name already listed, and `deactivate` on one that isn't, say so and
  change nothing.** Not errors — the end state is what was asked for.
- **A name that isn't a profile is an error, and it teaches II.5:** *"no profile named
  `editors` — profiles are Capitalized, modules are lowercase."*
- **`deactivate` removes packages, so it goes through the plan and the guard** like every
  other removal.

**`priority`** — an ordered list of backends, with `when` blocks.

```
when host == laptop {
  apt
  cargo
}

apt
dnf
cargo
snap
```

**Listed = available to LiNix, in this order. Not listed = LiNix does not use it at all** —
`snap:foo` errors with *"snap isn't in your priority list."*

**`vars`** — your own names for conditions, so `when $role == travel` reads intent where
`when host == thinkpad` reads a proxy for it. It has its own section: **II.6b**.

**`schedules`** — lines, with `when` blocks. **Being in the file means it's on.** No
active-list.

```
schedule:nightly {
  cron = 0 3 * * *
  run  = sync
}
```

`run=` is hashed and locked exactly like a hook. A `schedule:` may take `cron`, `run` and
`notify`, and its cron is validated where the line is read, so a bad expression names the file
and line rather than surfacing when the OS scheduler is handed the job.

**`linix schedule add`/`remove` edit this file and then sync**, the way `install` edits a
module and syncs (P1) — the file is the state, so the edit IS the command. `schedule list`
reads it. **There is no second store**: the `[schedules]` config table these commands used to
write is deleted (II.17), because two stores could disagree about what this machine runs.

**`locks/`** — **generated. In git. Yours.** Records:

| | | state |
|---|---|---|
| version | `apt:curl → 7.81.0` | **built** (`locks/versions.json`) |
| **hook script hash** | `fonts:after_install → sha256:a3f1…` | **built** (`locks/hooks.toml`) |
| **resolved artifact** | `sharkdp/fd → fd-…-linux-gnu.tar.gz`, its URL, format and hash | **built** (`locks/github.toml`) |
| **resolved backend for an unpinned name** | `ripgrep → cargo` | **built** (`locks/bare.HOST.toml`) |
| **regex expansion** | `re:^texlive- → [312 names]` | **built** (`locks/regex.toml`) |

`linix lock` regenerates the version pins. **It takes no arguments** — the per-name and
per-backend forms this section used to promise (`lock <name>`, `lock --backend cargo`) do not
exist. The one-file-per-backend layout now has exactly one real instance: `locks/github.toml`,
written by the backend as it installs rather than by `linix lock`, because the artifact is only
known at the moment it is chosen. `github` asks `Layout::lock_file()` for that path rather
than building it, so there is one answer to where a backend's lock lives.

**`locks/bare.HOST.toml` is one file per machine, not one per backend** (owner ruling,
2026-07-22), which is this table's one exception to the layout above. Two reasons, and they
pull in different directions:

- *Not per backend.* The fact recorded is about a *name*; per-backend files would make a name
  that moves managers two writes — a delete from one file, an insert into another — for one
  fact changing, and would make *"what did `ripgrep` resolve to?"* a search.
- *Per host.* Which manager has `ripgrep` is a fact about a **machine**, and `locks/` travels
  with the config to every machine that shares it. One file would hold whichever answer synced
  last: the Ubuntu box writes `apt`, the Fedora box overwrites with `dnf`, and the two rewrite
  each other on every sync and collide in git forever. A file per host means each machine
  writes only its own, every file commits cleanly, and each machine still reproduces exactly.
  A hostname is sanitised to `[a-z0-9-]` before it becomes a filename, so a host called
  `../etc` writes inside `locks/` like every other host.

**An unpinned name is asked once and then frozen, and deleting the entry is how you ask again**
(owner ruling, 2026-07-21). Re-deriving the answer every run against whatever is installed
*today* is how an unedited line comes to mean a different package: install a manager that sits
higher in `priority` and happens to publish the same name, and `ripgrep` silently becomes
somebody else's `ripgrep`. A name nothing declares any more is dropped from the file; a name
frozen to a manager the line no longer accepts, or that this machine does not have, is re-asked
loudly rather than honoured — the lock exists to stop a line changing meaning, never to demand
a manager that is not here.

**`linix unlock [NAME…]` is how you ask again** (owner ruling, 2026-07-22), alongside the text
editor II.15 promises for regex. With no arguments it forgets every name this host froze;
`--list` shows them and changes nothing. It is what you run when a better source appears:
`ripgrep` frozen to `cargo` because apt did not carry it yet moves to `apt` on the next sync
once it does — **and that sync uninstalls the cargo copy**, because the old one is a managed
package nothing declares any more, which is exactly what drift removal is for (V.34). Two
copies of one package is the state this avoids, not a state it tolerates.

*Both rows were once written here as though they were real, which cost the 2026-07-20 audit a
check. A target belongs in Part III or marked, not stated in the present tense.*

**`preferences.toml`** — refusals and behaviour. **Nothing writes to it but you.**

## II.6b `vars` — named conditions, typed values, and providers

**The problem.** `when` takes detected facts only (II.2): `os`, `arch`, `host`, `hostname`,
`family`. So "this is my travel box" has to be spelled `when host == thinkpad`, in every file
that cares, and a new laptop means editing all of them. **The hostname is a proxy for the
intent, repeated until it rots.** A variable names the intent once and binds it to machines in
one place — and it does **not** break "facts are detected, never configured" (II.1), because a
variable is not a new fact: it is a **name for a condition over the facts LiNix already
detected.** The `vars` source is committed and identical on every machine; each machine derives
its own values. Nothing is typed per box.

### The statement, and the sigil

A `vars` line is `NAME = VALUE`, a statement legal **only** in a `vars` source — the way
`schedule:` is legal only in `schedules`. `when` gates it like everywhere else (II.2, *one rule
everywhere*). A variable is read back with a `$`:

```
role = desktop            # a default, always present
gpu  = none

when host in [thinkpad, x220] {
  role = travel
}
```
```
when $role == travel {    # anywhere `when` is legal
  apt:mosh
}
```

**The `$` separates two namespaces that must never merge** (V.52). `$role` is something you
decided; `family` is something the machine reported, and reading the condition tells you which.
Because a variable can never be spelled like a fact, LiNix can detect one more fact — `distro`,
`init` — forever without silently changing the meaning of a file that named a variable the same
thing. Defining a variable that shadows a fact name (`os = …`) is legal and useless.

### Every variable is always defined (IX.3)

**A variable must have a top-level, unconditional definition. A `when` block overrides it; it
may never introduce a name.** Referencing a name `vars` does not define at top level is an
error. This is the rule that makes the rest work: without it, `role` set only inside
`when host == thinkpad` is *undefined* everywhere else, and `when $role == travel` on the
desktop would have to choose between erroring on every non-laptop and treating a typo as a
block that never fires and never complains. Requiring a default deletes that question. Two
matching `when` blocks that set one name to different values is an ERROR naming both lines
(II.7 rule 5), because the default is not a claim about this machine but two matching blocks
are.

### Values are typed (W2)

A value is one of the four JSON types — **string, number, boolean, list** — not text.
`gpu = true` is a boolean, `cores = 8` a number, `ver = 1.6.0` a string (a version is not a
number), `tags = [travel, work]` a list. `"quoted"` forces a string, which is the only way to
write the literal text `true` or `5`. **There is no cross-type coercion** (V.51): `"1" == 1` is
**false**, not an error and not a silent true. `==`/`!=` compare any two values; ordering
(`<`, `>`, `<=`, `>=`) is legal **only between numbers**, because `"10" > "9"` is false under
every string ordering and true under every intuition. `in` tests list membership under the same
no-coercion equality. **There is no truthiness:** a bare `when $flag` is a parse error naming
the fix (`$flag == true`), so `false`, `""`, `0` and `[]` never quietly differ (W3). One
recorded deviation: string equality is **case-insensitive**, preserving the behaviour
`os == LINUX` has always had.

A value that is exactly one reference (`alias = $tags`) inherits that variable's type; any
other value containing `$` is string interpolation and yields a string (`tier = ${role}-heavy`).
`$$` is a literal `$`; `${name}` ends a reference where a name character would otherwise
continue. Values may be **derived from other variables**, resolved in dependency order, and a
cycle is an error naming the whole loop (the same shape as a `use` loop, II.7). A `$var` may
also be expanded into a `link:` target or a `@version=` (`~/.config/$role/init.lua`); an unknown
name there is an error, never left as literal text, and a list has no text form so it is refused
by name.

### One contract, three providers

**A provider produces `name → value`. That is the whole interface**, which is why this is one
feature and not several:

| provider | filename | what it is |
|---|---|---|
| **line file** | `vars` | the `NAME = VALUE` file above, with `when` blocks |
| **embedded** | `vars.linix` | a script LiNix runs itself, in a language it ships (Rhai) — nothing to install, resolves identically across a fleet |
| **external** | `vars.py`, `vars.sh`, `vars.js`, … | any executable, run by LiNix, printing a JSON object or `name = value` lines; only works where its interpreter is installed |

**The kind is the filename, not a config key**, so what a file *is* is visible in the repo. The
external program is handed the facts as `LINIX_OS`/`LINIX_ARCH`/`LINIX_HOST`/`LINIX_FAMILY` and
its non-zero exit is an error carrying its stderr — a provider that fails must never resolve
silently to nothing (P3). The embedded script reads the facts as the constants `OS`/`ARCH`/
`HOST`/`FAMILY` and must end in a map of the four types.

**Several provider files may coexist; `[vars] source` in `preferences.toml` names the active
one.** One present and no `source` uses it; two present and no `source` is a **loud error
listing them**, never a precedence guess (V.53). A `source` naming a file that is not there, or
a name that is not a provider, is an error.

**The embedded standard library.** `vars.linix` is trusted the same as a hook (a script in your
own repo), so it gets every power an external `vars.py` already has, always on: the clock
(`now`/`today`/`weekday`/`hour`/`year`/`month`/`day`), the shell (`sh`, `sh_ok`), read-only
files (`read_file`, `path_exists`), the environment (`env`, `has_env` — **W7's escape hatch**
for a value no fact can derive, e.g. `env("LINIX_ROLE")`), the network (`http_get`), and
`parse_json`. The fail-loud split is deliberate: a function that **asks a question**
(`sh_ok`, `path_exists`, `has_env`) returns a value, so "no" is an answer; one that **fetches**
(`sh`, `read_file`, `http_get`) throws, because a fetch that silently returned nothing would
resolve a variable to the wrong value with no sign it failed.

**The ledger is what makes "trusted the same as a hook" true (V.55).** Every provider that
executes — `vars.linix` and every external `vars.<ext>` — is hashed into `locks/` and goes
through **II.12's ledger**: first sight asks, a changed hash stops. The powers listed above are
`sh` and `http_get`, and the file carrying them resolves at step 0 of II.7 — **before** the plan
exists — so `status`, `plan` and `plan --dry-run` have all already run it by the time they print
anything. "I only previewed it" is not a state in which the script has not run. Under `-y`, or
with no terminal, an unapproved or changed provider is a **refusal**, not a skipped prompt;
`linix lock` shows the file and approves it. The `vars` line file is not hashed — it declares
values and executes nothing.

### Resolved once, and frozen into a plan (W4, W13)

A provider may read the clock or the network, so **a value can move between two commands** —
and a value that moves makes `plan` a lie: the preview resolves `$x` at 11:59:58 and shows
nothing, the `sync` you confirm at 12:00:01 resolves it again and removes forty packages the
preview never showed. So **variables are resolved exactly once per invocation** (before any
`when`, including `active`'s, is evaluated), and **a saved plan carries its resolved variables**;
the `apply` that executes a plan uses the plan's values, not fresh ones (V.54). Because a
`vars` edit feeds the desired state, which feeds the plan, which feeds the guard, **a one-line
`vars` change that removes a hundred packages hits `max_removals`/`protected` like any other
change** — potentially the most destructive edit in the repo, and it goes through the guard by
construction.

### Tooling

`linix vars` prints each resolved name, its typed value, its type, and the active provider.
`vars` (and every provider file) is part of `linix diff` and the git manifest views — otherwise
the one file that explains a change would be the one the change view could not show. `when $var`
works in `active` (`when $role == travel { Travel }`).

`linix check` lists any resolved variable no model file mentions, as a note and never an error.

**`linix why` names the variable behind a package.** Under `because:` it prints every `when` that
had to hold for the package to be declared — outermost first, across `active`, the profile and the
module — and what each condition's variables are now: *"`when $role == travel` at `active:2` —
`$role` is `travel`, set at `vars:1`"*. Only conditions that test a variable are listed: a
`when host == laptop` is already its own whole answer.

**`activate` and `deactivate` name a block with its variables' values** — *"`when $role == travel`
($role is desktop)"* — because `active` holds the condition and `vars` holds the value, and a
message pointing at the first without the second cannot be checked. **The plan names the variables
that changed** since the last successful sync, above the removals (W13).

**One rule about which facts:** every reader of a `when` — resolving, editing, or explaining — is
handed the facts that carry this run's variables. There is deliberately no form that detects its
own: an empty variable set does not make `when $role == travel` a block that fails to match, it
makes `$role` an unknown key, and a file that is correct is refused.

**`priority` is the one file read twice, and it has to be.** It says which backends exist, and
resolving variables needs that vocabulary, so neither can simply go first. The bootstrap pass takes
every backend `priority` names — `when` blocks included, matching or not — and **evaluates no
predicate**, which is why a variable is usable there at all; it produces a vocabulary and never an
order. The real pass runs against the resolved facts and decides both. A superset is safe in the
first pass because a `vars` file names no backend.

## II.7 Resolution

0. **Detect facts, then resolve `vars` → the variables, exactly once** (II.6b). This is before
   everything else because `active` itself may carry `when $role`, and the once-per-invocation
   rule is what keeps `plan` honest when a provider reads the clock or the network. The resolved
   set rides on the facts for the rest of resolution and freezes into a saved plan.
1. Read `active` → the profile names, unioned.
2. Resolve profiles → the module set. Profiles may reference profiles; modules may not.
3. Parse **only** the modules reached. Apply `when`.
4. Resolve each line. A line that pins one manager is that manager. Anything else asks its
   candidates in order (II.7b), honouring this host's lock when the line still accepts what it
   names and the machine still has it.
5. **Two active declarations that contradict = ERROR.** Stop, show both, name both files.
   Not first-wins, not file order.
6. **Dated lines:**
   - **A dated line stops counting once its date passes.**
   - **While it is counting, a dated line beats an undated one.** *(The only exception to
     rule 5.)*
7. Produce the desired state.

### II.7b Which managers a line will accept

**The problem** (owner ruling, 2026-07-22). `apt:rg` says you want apt's ripgrep, and on a
machine with apt it should keep meaning apt however many other managers carry the name. But
wanting apt's here does not mean wanting *nothing* on the Fedora box, and before this a line
had only two settings: one manager forever, or a bare name whose answer got frozen to whichever
machine synced first. Neither is what someone with two machines means.

**So the prefix is a list, in preference order:**

| written | means |
|---|---|
| `apt:rg` | apt or nothing. A pin. Still apt on a machine that also has dnf. |
| `apt,dnf:rg` | apt, then dnf, and nowhere else. |
| `apt,list:rg` | apt, then every other manager in `priority`, in its order. |
| `list:rg` | every manager in `priority`, in order. |
| `rg` | the same thing — **a bare name is `list:` spelled short.** |

**A comma, not a hyphen.** Package managers have hyphens in their names (`nix-env`, `apt-get`),
so a hyphen separator stops working the day one of them becomes a backend and `apt-get:rg`
becomes a guess. A comma never can.

**`list` is reserved** (like `re:`, II.15) and **must come last**: it already means every
manager in `priority`, so anything written after it can never be reached, and syntax that
parses but cannot run is a line that lies about what it does. A manager named twice, an empty
slot (`apt,,dnf`), and a name that is not a backend are each errors — the chain is not a place
where C13's unchecked prefix gets back in.

**A pattern must still pin.** `apt,dnf:re:^fonts-` is an error: a pattern is matched against
one manager's catalogue and frozen in one regex lock, and a chain gives it neither (II.15).

**Only an unpinned line is locked.** `apt:rg` has nothing to record — the line already says
apt. A chain and a bare name record whichever manager answered, in this host's
`locks/bare.HOST.toml` (II.6), and are re-asked when that manager is gone or the line stops
accepting it.

**Two lines declaring one name with different lists is an error**, naming both. A name resolves
to one manager on one machine, so picking either list silently would make the other line a lie —
the same reasoning as rule 5.

**A manager that could not answer has not said no** (owner ruling, 2026-07-22). Asking a
candidate has three outcomes, not two: it has the name, it does not, or **it could not be
asked** — a package index that was never fetched, a registry that timed out, a command that
failed. The name still falls through to the next candidate, so one broken manager does not
fail a sync. **But nothing is written down.** The lock records only a pick that every
manager ahead of it actually refused; a pick made past silence is a guess, and the next sync
asks again. When the silent manager comes back and turns out to have the name, the package
**moves there on that sync** — installed from the manager that has it, and the copy the guess
installed removed, because nothing declares it any more (the `unlock` migration, II.6).

**And when nothing has it either, "no such package" is a lie.** The error names which
managers could not answer and what they said, because a stale index and a misspelling look
identical from the outside and only one of them is fixed by editing the line.

### Cycles

**A `use` cycle is an error, at both layers.** `Work` uses `Gaming` uses `Work`. Module `a`
uses `b` uses `a`. **Self-reference is the one-element case** and is the same error.

**`@requires` cycles are the same error** — `apt:a@requires=apt:b` and
`apt:b@requires=apt:a`. Same graph, same walk, same answer: the planner orders by the native
dependency graph plus `@requires` edges, and a loop has no order. **It owes the same error as
a `use` loop**: which packages, and the file and line each edge came from.

**The error names every file and line in the loop, in order, and stops.** It does not dedupe
and carry on (V.45):

```
ERROR: profiles reference each other in a loop

  profiles/Work.txt:3     use Gaming
  profiles/Gaming.txt:7   use Servers
  profiles/Servers.txt:2  use Work
                          ^ back to Work
```

**A diamond is not a cycle.** `Work` and `Gaming` may both `use base`. Reaching a module
twice by two routes is not an error — sharing a module is what modules are for (V.2). Only a
path that **returns to where it started** is a loop. *(So the check is a path, not a set of
everything ever visited.)*

**`linix check` catches cycles no active profile reaches** — consistent with II.3: LiNix
parses what the active profiles reach, `check` parses everything on demand. It follows `use` from
every module and profile in the folders, so a loop nobody activated is still found; it gates on
this host's facts like everything else, so a `when` arm written for another machine is parsed and
not walked.

**Ordering is the planner's job, never the file layout's.** Repos first → refresh indexes →
packages (native dependency graph + `@requires` edges) → things depending on packages
(services, shims, links).

**What LiNix may remove: what it manages and you stopped declaring. Plus `absent:`. Nothing
else, ever.**

## II.8 Commands

| command | does |
|---|---|
| `install PKG… [--into NAME]` | write the line, sync |
| `uninstall PKG… [--temp]` | remove the line from every active module, sync |
| `forget PKG…` | drop from the registry. Stays installed. LiNix never touches it again |
| `adopt [PKG]` | take over the machine, or one package |
| `sync` | make the machine match |
| `plan` | show what sync would do |
| `check` | parse everything, report errors |
| `lock [NAME]` | freeze versions / expansions, approve hooks |
| `unlock [NAME…] [--list]` | forget which manager an unpinned name resolved to, so sync asks again (II.6) |
| `purge-unmanaged` | delete everything LiNix doesn't manage |
| `remove-orphans` | the names each backend can say are orphaned — shown, guarded, removed (II.11c) |
| `clean-cache` | downloaded archives and build caches. Removes no installed package |
| `unmanaged` | what `adopt` would adopt |
| `absent` | every `absent:` line in force, and its module |
| `diff COMMIT COMMIT` | the change in **packages**, not text |
| `teleport PKG BACKEND` | move a package to another manager: rewrite the line in place, sync |
| `shell` | throwaway shell. Outside the model |
| `bundle` | git bundle + artifacts + registry |
| `restore DIR` | put a bundle back — the other half of `bundle` (V.59) |
| `export FORMAT` | Brewfile / requirements.txt / package.json |
| `activate NAME… [-a]` | write `active` — the list, or `-a` to add to it (II.6), sync |
| `deactivate NAME…` | take away from `active` (II.6), sync |
| `upgrade`, `list`, `status`, `doctor`, `profile`, `service`, `repo`, `hold` | as today, all reduced to file edits |

**`shell` must be honest about being outside the model:** it writes no module, and **stops
recording transient packages in the registry** — which is what lets a session's leftovers
look like managed drift later.

**One writer at a time (V.61).** Every command that mutates state takes an exclusive lock on the
data directory for its whole run, and a second one waits or says who holds it — LiNix is not the
only thing that starts LiNix. The package-manager hooks (`DPkg::Post-Invoke` and its siblings)
mean an ordinary `apt install`, typed by someone who has never heard of this tool, spawns a
process that rewrites `registry.json` while a `sync` or a `watch` tick may be part-way through
its own. The registry is written whole; two whole writes are last-one-wins, and the entry that
loses is a managed package nothing declares any more, which is the definition of drift and the
input to a removal.

**`bundle` writes and `restore` reads, and they are one feature (V.59).** `bundle` already
packs the config root, `locks/`, the resolved package list, the git history as `config.bundle`,
and optionally the artifacts; `restore DIR` is that in reverse, and it is **a command, not a
README** — an instruction file cannot be tested, and a backup nothing has ever restored is a
guess. **`restore` refuses to write into a config directory that is not empty** unless told
otherwise, because the machine you reach for a backup on is usually one that still has
something on it.

This is the answer to **K9**: the backup command is `bundle`, finished — not a second archive
writer, which X.5 forbids. **It is also what a git-less machine has instead of history** (X.5),
so its end-to-end proof runs without git: bundle a config, restore it into a clean directory,
and assert the model parses and resolves to the same package set.

**Destroying a file you wrote** (e.g. `module create` over an existing file) is a **plain
refusal plus `--force`**, like every other tool. It has nothing to do with packages and must
not be wired to a setting about removals.

**Every command prints the file it touched:**
`Added jq to modules/imperative.txt (used by profile Work)`

**`--into` takes a module (lowercase) or a profile (Capitalized).**

**Three landing modules, named for how the package arrived:**

| module | arrived via |
|---|---|
| `imperative` | `linix install` |
| `hooks` | `apt install`, caught by the hook |
| `adopted` | `linix adopt` |

The first time LiNix writes to one, it adds `use <name>` to the active profile and **says
so**. A normal line you can read and delete. **Never implicit.**

**`uninstall` warns about inactive declarations:** *"jq is still declared in module
`gaming`, which isn't active. It will come back if you activate Gaming."*

**`uninstall PKG --temp` on an undeclared package is an error:** *"steam isn't declared, so
there's nothing for it to come back to. Did you mean a plain uninstall?"*

**`--backend` is allowed on read-only and upgrade; REFUSED on anything that removes.**
`plan`, `list`, `upgrade` → yes. `sync`, `purge-unmanaged` → error: *"scoping a removal
isn't safe; use a profile."*

**`clean` goes through the guard** — ask the backend what it intends, check the list against
protection, refuse if it touches something protected. **Sync nudges:** *"3 packages are now
orphaned; run `linix clean`."* Want it automatic? `schedule:tidy@cron=0 3 * * *,run=clean`.

## II.9 Adopt

**Adopt takes manually-installed packages only. Never the dependency closure.**

**(measured)**

| Backend | Record | Result |
|---|---|---|
| apt | `apt-mark showmanual` | 103 of 579 |
| pacman | `-Qqe` | 11 of 173 |
| conda | `env export --from-history` | 4 of 88 |
| winget / choco / scoop | installs no dependencies — everything **is** chosen | exact |
| **pip** | **none.** No flag separates dependencies | **adopt nothing, say why** |

**Base-image packages ARE adopted** — `grub-pc`, `linux-image-generic`. They keep the
machine bootable, and `purge-unmanaged` deletes what isn't declared.

**Output:** one `modules/adopted.txt`, grouped by backend with comment headers, sorted.
Header states: this is an estimate; deleting a line uninstalls; `linix forget` is the way
out. A second section lists OS-essential packages, commented out.

**Adopt does NOT consult `protected_packages`.** This resolves **E7**, where "protected"
means two opposite things: *never remove* in the guard, *never adopt* in `migrate.rs`.
**Protection means one thing: never remove.** So adopt takes every manual package including
protected ones; protection then prevents their removal. This is a **change from what Stage 2
built** — Stage 2 routed adopt's skipping through `guard::protection_of`, which unified the
code while keeping the word ambiguous. Adopting a protected package is correct: it belongs
in your file, and deleting that line is refused (V.26).

## II.10 The guard — ten refusals, one function

| | |
|---|---|
| `protected_packages` | never remove this |
| `unprotected_packages` | …unless I say so. **Wins over everything, including OS-essential** |
| OS-essential | never remove what the OS says is load-bearing |
| undeclarable | never remove a name no package line can hold — **not even `unprotected_packages` releases this one** |
| `max_removals` (default **20**) | never remove more than this at once |
| `max_installs` (default **unset**) | never install more than this at once |
| `deny_packages` | never install this |
| `pinned_only` (default **off**) | never install anything without an explicit `@version=` |
| `require_snapshot` (default **off**) | never change anything when no snapshot can be taken |
| `deny_vulnerable` (default **off**) | never apply when `audit` reports a managed package vulnerable |

All in `[guard]` in `preferences.toml`. One decision function. **Every removal path calls
it** — sync, `absent:`, expiry, `purge-unmanaged`, `remove-orphans`, shell exit, `uninstall`.
The last three also gate *installs* and *changes*, so the install paths call it too.

> **THIS SENTENCE IS CURRENTLY FALSE, AND HAS BEEN SINCE THE JOURNAL WAS WRITTEN.** There is an
> eighth removal path and it calls nothing: `heal()` recovers an interrupted *install* by
> uninstalling the package first (`sync/mod.rs:432`, `let _ = handler.remove(…)`). It runs
> before the plan, before the counts, before `-y` is even consulted, and it removes a package
> that every file in the config declares. On 2026-07-23 it ran `winget uninstall --silent
> Google.Chrome.EXE` on the owner's machine, from a command whose argv was `install
> nimble:nimjson`. **`protected_packages`, OS-essential and `max_removals` do not apply to it,
> because it never asks.** The sibling branch ten lines above — completing an interrupted
> *removal* — does consult the guard, which is why the gap survived review: the mechanism looked
> guarded from every angle except the one that mattered. See **S24**. Until S24 lands, the row
> above reading "**nothing overrides**" is a promise this document cannot keep.

**A removal LiNix cannot show you is a removal LiNix may not make.** The guard, the plan and the
counts are one mechanism, not three, and a path that skips the first skips all of them —
whatever it removes is invisible in `plan`, invisible in `--dry-run`, and absent from the
history. That is the property S24 broke, and it is the reason S24 is filed as the worst bug in
this document rather than as one more row.

**A removal is always a list of names (V.56).** No path may hand a manager its own
bulk-removal verb — `apt autoremove`, `dnf autoremove`, and every verb like it — because the
set those verbs delete is chosen at execution time, *after* the guard has judged and after the
user has read the plan. There is nothing for the guard to hold and nothing for the plan to
show. **A backend that cannot say what it would remove does not remove.**

**`[guard]` holds three keys that are not among the ten: `confine_bin`** (default on), which
refuses a downloaded file a destination outside the backend's bin directory (SEC1),
**`require_signed_history`** (default off), which refuses a rollback to a commit git does not
vouch for (II.13), **and the list of commands that may not run unattended** (K13, ruled
2026-07-23), shipped as `rebuild` and `purge-unmanaged` and edited by taking a name out. A
`schedules` entry naming a command on that list is refused, with the list named in the message
so the way out is in the error. The default preserves the refusal exactly as it was, so a config
that says nothing changes no behaviour; what the list adds is that **the set is the user's, not a
constant in the source**, and the next dangerous verb joins it by being written down rather than
by someone remembering to add an arm. All three are refusals in kind and none is in the decision function, for the
same reason: the fact each one needs — the deploy destination, git's verdict on one commit, the
verb at the head of a `run` line — exists only at the moment its own command asks. A
`confine_bin` check anywhere but a deploy would be checking a path nobody was about to write.
They live in this table's home because they are the same kind of promise, a refusal with one
deliberate opening. **Counting any of them among the ten would make "one decision function"
false**, and a table that quietly stops describing its own function is how the last one drifted.

**A confirmation asks; a refusal says no.**

| | `-y` |
|---|---|
| sync shows the plan and asks | **skips** |
| `max_removals` exceeded | **cannot skip.** `--allow-mass-removal` |
| `max_installs` exceeded | **cannot skip.** `--allow-mass-install` |
| hook script new or changed | **cannot skip.** `linix lock` |
| protected / OS-essential | **nothing overrides** |
| `purge-unmanaged` | **cannot skip.** Typed confirmation |
| `pinned_only` / `require_snapshot` / `deny_vulnerable` | **cannot skip.** They are refusals (V.43) |

**The plan always leads with the counts** — not a threshold, not a warning, just the plan
being readable:

```
Plan: install 30,207 · remove 0 · upgrade 3
  30,102  re:^lib
      98  apt
       7  cargo
```

## II.11 `purge-unmanaged`

**`sync` is additive; `purge-unmanaged` is exclusive. This is the answer for every backend, and
no backend gets its own** (owner ruling, 2026-07-23, N1). A thing LiNix declared and then stopped
declaring is removed by `sync`, because the ledger knows LiNix put it there. A thing LiNix never
declared is left alone by `sync` and removed by `purge-unmanaged` — packages, links, services,
firewall rules, and whatever the next backend manages. **A backend that wants an exclusive mode
of its own is asking for a second `purge-unmanaged`**, which is the two-of-everything failure
wearing a new name; the opt-in already exists and is this command.

- **The guard is a RATIO, not a count:**
  ```
  LiNix manages 3 packages.
  This will remove 576, including python3, libc6, and bash.
  That looks like you haven't adopted this machine yet.
  Run `linix adopt` first, or --allow-mass-purge if you're sure.
  ```
- `max_removals` does **not** apply (it catches accidents; this is deliberate).
  `protected_packages` and OS-essential **always** apply.
- **Snapshots first**, automatically. **If none is available, say so loudly** — *"there is no
  undo for this"* is the most important sentence this command can print.
- **Shows the whole list.** 576 packages is 576 lines. The pain is the feature.
- Docs state the residual risk in these words: adopt is an estimate; if it missed something,
  this deletes it.

## II.11b `rebuild` (V.49)

**`sync` converges; `rebuild` asserts.** Convergence cannot repair state that is wrong while
the difference is empty (X.1). `rebuild` removes what is declared so it can install it again.

- **Scope is required.** A bare `linix rebuild` errors and names the three forms. `--all` is
  not a default you can reach by pressing enter.
- **Batch per backend, one backend at a time.** All of a backend's packages come down, then
  all of them go back up, then the next backend. Within a backend a dependency shared only by
  packages that all leave really does orphan, so the repair repairs; and a failure strands one
  backend's software, not the machine.
- **Foundation backends first**, then the rest, each tier in `priority` order. "Foundation" is
  `needs_root()` — a manager that must be root installs into the system. **The reason is
  dependency direction, not blast radius**: a crate can need a system compiler, and no system
  package has ever needed a crate.
- **Removal and reinstall are two transactions, not one graph.** The transaction engine runs
  independent nodes concurrently and there is no edge between removing a package and
  installing it.
- **It never touches undeclared software.** Everything it removes, it removes to put back.
  That is what separates it from `purge-unmanaged` (II.11).
- **It never removes a protected package.** Those are dropped from the scope and named, along
  with anything declared-but-not-installed (`sync`'s job) and any package nobody declared.
  **The skips are printed, never silent.**
- **A failed reinstall stops the run.** It names the packages that are gone and does not start
  the next backend.
- **`rebuild` is not a mode of `sync`** and cannot appear in `schedules` (K13), because
  `schedules` runs sync unattended and a mode of sync is a mode a schedule can reach.

## II.11c `remove-orphans`, and what "remove" means

**It removes exactly the names it showed.** Every backend's orphan set is enumerated, printed
under "Planned changes:", judged by `guard::enforce` as one total (so the ceiling and the
protected list see the whole removal, not one backend at a time), confirmed, and then removed
through each backend's ordinary `remove`.

**A manager that cannot list its orphans is asked a different question, not trusted with a
blind one (V.56).** Where a dry run can produce the list — `apt-get autoremove --dry-run`,
`dnf autoremove --assumeno` — that is how the list is produced, and those backends join the
enumerated set like any other. Where nothing can produce it, the backend **loses orphan
removal** and `remove-orphans` says so by name. It does not fall through to the native verb.

**`remove` means remove, not purge.** A package's configuration in `/etc` is not the package,
and deleting a module line means *"stop installing this"*, which is not the same sentence as
*"destroy how I had it set up"*. Debian's `purge` is available and never the default:

| how | scope |
|---|---|
| `linix uninstall --purge NAME` | this removal only |
| `[remove] purge = true` in `preferences.toml` | this machine, every removal |

Drift removal has only the second, and that is the constraint that shapes this: by the time a
deleted line is removed **the line is gone**, so there is nothing left to carry a per-package
option. A setting that can only be machine-wide must therefore be off by default, because the
alternative is a machine-wide destructive default nobody typed.

## II.12 Hooks and the supply chain

**The lock is the approval.** `locks/` records each hook script's hash. Hash mismatch →
**stop**:

```
module `fonts` (from github:x/y) changed its after_install script since you approved it.
  was: sha256:a3f1…   now: sha256:9c2e…
Run `linix lock fonts` to see the new script and approve it.
```

**Hash everything, including your own scripts.** One rule, no exceptions.

**A `vars` provider is one of those scripts.** `vars.linix` and every external `vars.<ext>` are
hashed and approved here too (II.6b, V.55). They run earlier than any hook — before the plan
exists, and on read-only commands — so for them the ledger is the only thing between a pulled
config and a shell.

**Two kinds of hook, by when they run — both go through the ledger.** Whole-sync lifecycle
hooks live in the `[hooks]` config block (`before_sync`/`after_sync`, target `*`, run once
around the entire sync). Per-package hooks are attached to a declaration
(`apt:nginx { after_install = ./setup.sh }`) and fire inside the engine for that one package,
keyed per package (`after_install:nginx` ≠ `after_install:redis`). These are **not duplicates**
— a per-package hook cannot express "before the whole sync", so `[hooks]` stays (owner ruling
2026-07-17; that is why it is not on II.17's delete list).

**`plan` shows the trust, before anything happens:**

```
module `fonts` (github:x/y)
  adds repository  ppa:fonts/testing
  runs script      after_install: ./setup.sh   [approved]
module `dev` (local)
  runs script      after_install: ./build.sh   [CHANGED — needs approval]
```

## II.12b What reaches a command line (V.62)

**A package name is data, and every backend must say so.** Each manager invocation ends its
own options before the names begin — `apt-get install -y -- ripgrep` — so a name can never be
read as a flag. This is not defence in depth behind the grammar; it is the only layer that
holds, because the set of flags belongs to the manager and changes without us.

**A name that starts with `-` is refused at parse time**, wherever it appears — not only in the
`Subtract` position at the start of a line, which is the one place it was ever checked.

**A validator with no caller is not a validator.** Every check the tree carries is called on the
path it names, or it is deleted. Two of everything is bad; one of everything, unwired, is worse
— it reads as a defence in the source and is absent at runtime.

## II.13 History

**Git is your intent. Snapshots are your machine.** Two jobs, two mechanisms, neither
pretending to be the other.

**A generation IS a git commit. LiNix commits only on a successful sync** — so every commit
in your history is a state your machine **actually reached**.

- `git log` = where your machine has been.
- **`git diff` and `linix plan` are the same question.**
- Rollback can never take you somewhere that never worked.

**Order: snapshot → apply → commit.** On failure, restore the snapshot and don't commit —
files and machine agree, because the snapshot brought `registry.json` back with it.
**Tag the snapshot with the commit hash.**

**Rollback = `git checkout` + `sync`.** The registry is always current; its history is not
stored, because declaration + convergence reproduces it. **There is no generation format.**

**Snapshots are a preference**, default on if the machine can do it (btrfs, ZFS, or
Timeshift). Retention prunes — **one engine** (`retention`), not two.

**A restore that cannot restore says so, before it is needed (V.60).** Taking a snapshot and
restoring one are different capabilities and a provider may have the first without the second:
`btrfs subvolume snapshot SRC /` does not roll back a mounted root, whatever its exit code says.
So **a provider that cannot perform a live restore must refuse the restore**, and `doctor` and
the pre-change notice must say which kind of snapshot this machine takes. **No command prints
"rolled back" on the strength of an exit code** — the sentence is a claim about the machine, and
it is the one sentence a user cannot check for themselves at the moment they read it. There is
**one restore implementation**, not one in the provider and another in `undo`.

**No commit algebra.** Git covers what's real:

| you want | git |
|---|---|
| union of commits | `merge` |
| take that one change | `cherry-pick` — "roll back but keep the jq I added" |
| undo that one thing, keep the rest | `revert` |
| chained and nested | branches |
| **intersect of commits** | **nothing. No such operation, no use case found** |

**Integrity is `git commit -S`.** LiNix checks that git says the commit is signed, and by
whom. **`locksig.rs`, `.linix-lock.key`, and the fail-open branch are deleted.**

**LiNix commits as you.** It sets no identity of its own and forces no signing flag: whatever
your git config says is what the commit records. A commit signed by your key and authored by
`linix@localhost` would attribute a verified change to a person who does not exist, and a repo
with no identity configured is git's error to report, in git's own words (owner ruling,
2026-07-21).

**Git answers; LiNix repeats the answer.** `git log` and `linix history` show each commit's
signature and signer, and a commit git will not vouch for — an untrusted, expired or revoked
key — is never shown as signed. **Nothing is refused by default:** a fresh repo signs nothing,
and a refusal that fired on every rollback would be turned off before it caught anything. With
`[guard] require_signed_history` on, `rollback` refuses to restore a commit git does not vouch
for, naming what git said about it.

## II.14 Version pins — precedence

1. **`@version=` in a module** — you wrote it. **It wins.**
2. **`locks/`** — generated; fills in everything you didn't pin.
3. **Nothing** — whatever's current.

A hand-written pin disagreeing with the lock is **not an error** (today it fails the run).
You wrote it, it wins, LiNix regenerates the lock to agree and says so.

## II.15 Regex

**`re:` prefix. Frozen the first time it is seen; delete the entry to match again**
(owner ruling, 2026-07-21 — this replaces "live by default, lockable when you want it").

**The lock file IS the switch, and it writes itself.** The first expansion records what it
matched in `locks/regex.toml`; an entry is used as-is and no manager is asked. Deleting the
entry — in your editor, since the file is yours — matches again and records the new answer.
There is no `lock` command for it and no `unlock`: declaring the machine is LiNix's job to do
automatically, and a prompt for something that is the command's own work is a prompt nobody
wanted (P1).

*Why freezing is the default and not the option:* `apt:re:^lib` was **(measured)** at 30,207
packages. Re-matched every run, that line grows the machine the day someone else's upload
happens to fit the pattern — nothing in your files changed, nothing was reviewed, and the plan
you approved is not the plan that ran. Frozen, the expansion is a file in git, so what the
pattern means is a diff.

**The pattern must name a manager.** `apt:re:^fonts-`, never a bare `re:^fonts-`: a bare name
can be probed ("who has `ripgrep`?"), but every manager has *some* match for a pattern, so the
first yes would be an accident of `priority` order. The grammar refuses it at parse time.

**Only a manager that can produce its whole catalogue** can be matched against — a new
capability, distinct from search, because a search matches descriptions and ranks results and
cannot answer "which names match this". The system managers can (`apt-cache pkgnames`,
`pacman -Ssq`); a language registry with millions of packages and no list endpoint cannot, and
a `re:` naming one is refused by name rather than expanded to nothing. **A pattern that matches
zero packages is an error**, not an empty expansion: it is a typo every time.

**`check` shows what each pattern means**, since that is the one thing not readable from the
line:
```
1 pattern(s), frozen in `locks/regex.toml`:
  apt:re:^fonts-               312 package(s)
  (delete an entry from the lock to match again.)
```
and `why` on a matched package says *"matched by `re:^fonts-` at modules/dev.txt:3"* rather than
sending the reader to a line that does not contain the package.

**(measured)** `apt:re:^python3-.*` → 4,447. `apt:re:^lib` → 30,207.

**Residual hole, accepted:** `texlive-foo` renamed to `tex-foo` silently drops one package.
One package, recoverable, snapshot has your back.

## II.16 Everything is a line

| Today | Becomes |
|---|---|
| `linix repo add` (**stores nothing**) | `repo:apt:ppa:deadsnakes/ppa` |
| `linix shim jq --source cargo:jq` (**`--source` discarded unread**) | `shim:jq@source=cargo:jq` |
| `linix hold jq` (machine-local `registry.json`) | `apt:jq@hold` |
| hooks table in config | `apt:nginx@after_install=./setup.sh` |
| `linix schedule add` (**wrote config**) | a line in `schedules` — the command survives and now writes that file |
| `@lease=2h` (**inert today**) | `apt:jq@expires=2026-07-17T14:00` |
| `remove --temp` (**loses to sync**) | `absent:apt:jq@until=…` |
| `bloatware.txt` | `absent:apt:libreoffice` in a module |

A repo and the package needing it are **one fact**:
```
module python-latest {
  repo:apt:ppa:deadsnakes/ppa
  apt:python3.12
}
```

**Expired lines linger.** LiNix must not rewrite your files. It mentions them, **naming the
exact file and line** — never vaguely. Only the dated line is dead; the undated one is doing
real work and must stay.

## II.17 Deleted

**Commands:** `prune` · `orphans` · `clone` · `migrate` (→ `adopt`) · `remove` (→
`uninstall`)

**Flags:** `-g` / `--groups-dir` · `--no-global` · `--allow-regex-expansion` ·
`--backend` on removing commands

**Syntax:** `group:` · `include:` · `host-*.txt` · `_active_profiles.txt` · `local.txt`'s
special status · `-vim` in modules

**Config:** `[groups]` · `[hostname_packages]` · `[managed_files]` ·
`[schedules]` · `backend_priority` · `enabled_backends` · `hostname_backends` ·
`default_backend` · `prune_on_sync` · `prune_scope` · `purge_orphans` · `cache_ttl` ·
`confirm_destructive` · `protect_imperative` · `remove_bloatware` · `timeshift_path` ·
`config.snapshots` · `github_token` (→ env)
*(`max_parallel` was struck from this delete list by owner ruling 2026-07-17 — it stays as an
optional concurrency cap. See II.1 and V.41.)*

**Files:** `keep.txt` (→ `forget`) · `policy.toml` (→ `[guard]`) · `bloatware.txt` (→
`absent:`) · `.linix-lock.key` · `locks.json` (→ `locks/`) · `ghosts.json`

*(`[hooks]` was struck from the config delete list by owner ruling 2026-07-17. It is **not** a
duplicate of module hooks — the two are different features by *when they run*: `[hooks]` holds
whole-sync lifecycle hooks (`before_sync`/`after_sync`, target `*`), while `before_install`/
`after_install` are per-package hooks attached to a declaration. Deleting `[hooks]` would remove
the whole-sync kind, which modules cannot express. See II.12.)*

**Code:** `locksig.rs` · the generation format · `ManifestArchive` · `quick()` ·
`ScopedFilter::None` as a spare-everything switch · every legacy branch

## II.18 The version, and the way in (V.58)

**The repo is `github.com/SYKhayyat/LiNix`.** Everything that names a source names that — the
two install scripts, the README's one-liner, the release job.

**The version is `0.1.0`, because nothing has been released.** The tree carried `6.0.0` while
the CHANGELOG called the same commit *"v7, the declarative rewrite"* under `[Unreleased]`, so
`linix --version` answered with a number no user has ever been given, for a model that had
replaced the one that number described. "v7" is the name of the rewrite and belongs in the
CHANGELOG and in Part VII; it is not a version anyone can install. A version number is a promise
about what someone already has, and the honest promise here is that this is the first one.

**The install path is a tested path.** `install.sh` and `install.ps1` end by offering to take
over the machine, and that step must name a command that exists — both called `migrate`, which
**II.17 lists as deleted** (→ `adopt`), so the documented first run installed the binary and
then failed on the only step that makes it useful. A rename sweeps the scripts and the docs in
the same change as the source, or it is not done.

---

