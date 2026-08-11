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
adapters/           how to drive things LiNix does not ship
  backends.toml       a package manager (XIII.2)
  settings.toml       a settings store (K17)
  bootstrap.toml      how to obtain a manager this machine lacks (7c)
  init.toml           an init system (U36)
  snapshot.toml       a snapshot/rollback provider (U27)
  secret.toml         a secret-decryption provider (U38)
  firewall.toml       a firewall (7o)
hooks/              a script per LiNix event — on_drift, after_sync, … (7j)
locks/              what everything resolved to    one file per backend
preferences.toml    refusals and behaviour
```

**`adapters/` is one file per kind of thing LiNix does not ship** — how to *drive* a manager, a
settings store, an init system, a firewall, a snapshot provider; and how to *get* a manager. All
of them are code the repo carries, so all of them go through II.12's approval ledger: they arrive
with a pulled config, and a config that can run a command on first sync without being read is the
whole reason that ledger exists. **`linix lock` approves every `*.toml` in `adapters/`, by
reading the folder, never a list named in the source** — a named list forgot `firewall.toml`
once, leaving it unapprovable and its rows refused on every sync (V.72), which is why the
assertion "every adapter file is approvable" is made against the directory rather than a sentence.

**A definition in `adapters/backends.toml` may take a built-in's name, and only by saying so
(Q6).** Adapters register last, and a name already in use is skipped — being named `apt` is not a
way to become `apt`. `overrides = true` on the definition replaces whatever holds the name,
built-in included, because a manager can change its CLI and the person on that machine has to be
able to correct it that day. **Two deliberate acts, never one:** the sentence in the definition,
and II.12's approval of the file it is in. LiNix says so on every run that loads it, naming the
backend and the program it now runs, and `check health` answers for the replacement rather than
for the built-in it displaced. Snapshot providers, init systems and secret stores are unchanged —
they still never shadow a built-in.

**Every adapter table answers the same three questions the same way** *(Y13)*. A row says what it
is called, which OS it is restricted to (absent means any), and why LiNix will not act on it —
and everything asked *about* rows is written once: the OS filter, the floor a row clears to be
acted on, the shipped-then-user merge, the search for the row that describes this machine, and
`{placeholder}` substitution. **A row type carries its own argv and nothing else is per-table**;
folding a firewall's `allow`/`deny`, an init's `start`/`stop` and a store's `read`/`write`/`reset`
into one schema would be a struct of twenty optional fields, which is the shape that makes a
table unreadable.

- **Every table can be confined to one platform.** `[[secret]]` could not, and it is the one
  table whose rows are handed a plaintext secret.
- **A second row claiming a name is refused and said out loud**, in every table that is keyed by
  name. `[[bootstrap]]` and `[[prereq]]` are keyed by *manager* and carry several rows for one —
  asdf needs a plugin per declared tool — so they pick with their own rule and do not deduplicate.
- **The shipped rows clear the same floor a user's row clears**, which is what K17/U1 has always
  said and what the built-in snapshot rows were not doing.
- **Writing the shared machinery a second time is a build failure**, not a review comment: the
  seven copies were each written by someone doing something reasonable in the file in front of
  them, and a ledger of tables would not have caught any of them (V.145).

**A definition in `adapters/backends.toml` may declare every capability a built-in has**
(U2, extended 2026-08-05 by `Q43`/`Q44`). `outdated_args` is the manager's own "what has an
update" verb; `machine_list_args` a machine-readable listing to prefer over `list_args`, with
`machine_list_parser` beside it. **Absent means *this backend cannot answer that*, never *the
answer is none*** — a definition with no `outdated_args` is asked per package, which is slower
and still an answer, while one that reported an empty set would mark the whole backend current.
A `machine_list_args` without its own parser is refused by name rather than read with the text
parser, which would hand JSON to a column reader and report an empty machine. And a definition
that declares an outdated verb is reachable through it **even with no `search_args`** — a
manager that lists updates but has no catalogue to search is an ordinary shape, and gating the
capability on search alone made its updates silently unreportable. `search` itself still refuses
by name when it was never configured.

**Extended again 2026-08-06 by `Y11`:** `clean_cache_args` is the manager's own way of emptying
its download cache, with `clean_cache_binary` for a manager that empties it with a different
program; `repo_remove_binary` for one that adds a source with one program and drops it with
another. Absent means *this manager has no such verb*, which `linix clean-cache` reports as
nothing to clear for that backend rather than as a success. Every field a built-in row gained
in that change is here too — that is what "a first-class peer of a built-in" has to mean, and
the reason `clean_cache` was worth a field at all is that when it existed only as Rust in six
modules, no definition and no row could say it.

**Every file above may begin with a byte-order mark, and it is read anyway** (Q22). Notepad
writes one by default and no editor shows it, so before this the three bytes became part of the
first name on the first line and the refusal named two strings that render identically. The mark
is stripped where text enters a parser — never where a file is read, because `edit.rs` reads
these same files in order to rewrite them and II.16 forbids LiNix rewriting your files, encoding
included. **Only at the start**: a zero-width character anywhere else is still refused by name
(V.112).

**Where the repo is, in four answers and one rule.** `--config-dir`, then `$LINIX_CONFIG_DIR`,
then LiNix's own settings file, then the platform default; `linix path --explain` says which of
the four won. **Every one of them must be an absolute path, and a relative one is refused by
name** — the flag, the variable, the stored value, and `linix path --set` alike. A relative root
means a different directory per invocation, and the refusal is the same sentence at all four
doors (V.125).

**LiNix's data** — `$LINIX_DATA_DIR`, `--data-dir`, or the platform data dir, under the same
absoluteness rule. **Never in git. Never in a folder LiNix scans.**

**`--config-dir` and `--data-dir` are a pair, and neither isolates a run alone.** Your files and
what LiNix recorded about them are two directories; a fresh sandbox with only the first planned
seven removals against the real machine's managed state.

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
dotfiles:PATH                 a folder mirrored into place, one file at a time
firewall:PORT/PROTO           a port; `firewall:default/incoming @value=deny` a policy
exec:PATH                     run a script the config carries — the one verb
generate:PATH                 a command whose stdout is declarations (U33), off by default
use NAME                      reference a module (lowercase) or profile (Capitalized)
```

**The last three arrived after this list was first written, and it did not notice them for two
days.** `exec:` is XIII.3 (built 7b), `dotfiles:` is U22–U25 (built 7n), `firewall:` is N1–N7
(built 7o) — all shipped, all in `Statement`, none of them here. **A grammar table that omits a
shipped statement is worse than one that is short: it reads as the closed set it is not.** The
authority is `config/grammar/statement.rs`'s `Statement` enum; this table is the human copy and
must be checked against it, not against itself.

**It was four, not three, and the fourth outlived the paragraph written about the other
three.** `generate:` shipped as U33, is a `Prefix` in `KEYWORDS`, has its own rule below — and
was still missing from this table on 2026-08-04, read past by every session that read the
sentence above it. A prose instruction to check a copy against its authority is not a check;
`tests/grammar_table_matches_the_spec_tests.rs` is, and it now fails the build if this table
and `KEYWORDS` disagree in either direction (Q29).

**`exec:` is the one statement that is a verb, and it bends the model in exactly one place.** A
false `when` on every other statement means *undo*; on `exec:` it does not, because a script that
succeeds makes its own condition false and treating that as removal would flap. What removing an
`exec:` line means is `@undo=` if the line carries one, and **dropping the record and nothing
else** if it does not (U3) — LiNix does not invent an inverse for a script that has none, and
`plan` says so in those words.

`use` takes **a name. Never a path, never a URL.** A file from the internet is a fetch step
that puts a module on disk; then you `use` it by name like everything else.

### A bare word that is a keyword is not a package (Q16, ruled 2026-07-30)

Every word this grammar reserves, written alone on a line, is **a parse error** naming the two
ways to mean it. The authority is `config/grammar/statement.rs`'s `KEYWORDS` table — the same
table the parser dispatches on, so a prefix added later is refused bare without anyone
remembering — and it is, today:

```
# prefixes — written `word:`, each introduces a typed statement
absent  repo  shim  schedule  service  link  setting  exec  generate  dotfiles  firewall
# directives — this grammar's words, written bare
use  param  exclude  intersect  module  when
# the words people arrive with, refused so a typo cannot install a package
if  else  end  import  include
```

The three groups are `KeywordRole::Prefix`, `Directive` and `Foreign`, and the ratchet checks
each group against its own role rather than the twenty-two as one bag: promoting `if` to a real
keyword is a language change, and a check that only counted words would pass it.

The ruling was measured against thirteen of these; the other nine are the same sentence one
layer over — `exec`, `dotfiles`, `firewall` and `generate` are statement prefixes this document
also failed to list when they shipped, and `exclude`, `intersect`, `module`, `use` and `param`
are directives whose bare form fell through to the package parser by the identical route.
Refusing thirteen of twenty-two would have left the typo live for `exec`.

```
modules/dev.txt:4: `link` is a keyword, not a package name
  to link a file:            link:/path/to/source @target=…
  to install a package by that name:   list:link   (or pin one: cargo:link)
```

Every one of those words is a real package in a real index — `cargo:link`, `pip:absent`,
`scoop:shim`, `gem:if`, `npm:else` — so before this ruling a half-typed `link:` line declared a
package, resolved it, and `linix check` recommended the `sync` that would install it. A typo
that installs software is worse than a typo that stops.

**The escape hatch already existed and needed no new syntax.** A bare `NAME` is defined above as
short for `list:NAME`; writing `list:link` says *"a package called `link`, resolved through
`priority`"* and means exactly what the bare form used to. **No quoting is introduced** — V.10
rejected quotes because `"` needs `\"` needs `\\` needs a newline rule, and that reasoning is
untouched by this.

The rule binds the *word*, not the prefix: `link:` with its colon and nothing after it was
already a legible refusal and stays one.

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

**`os` is the kernel** (`linux`, `windows`, `macos`, `freebsd`, …); **`family` is the
distribution** (`debian`, `fedora`, `arch`, `suse`, `alpine`), read from `/etc/os-release` and
falling back to the OS name where there are no distributions to tell apart. They are two
questions and neither stands in for the other: `apt` is a `family == debian` fact, not a
`linux` one.

**A family that cannot be shown to be X makes `family == X` false — not an error** (U26). On a
BSD or any host without `/etc/os-release`, `family` is the OS name (`freebsd`), so `== debian`
is correctly false and `== freebsd` is true. The fallback is load-bearing: `family` is **never
empty**, because an empty family is exactly what would make every `when family ==` silently
take the else branch — the silent-wrongness this rule closes.

### Option keys

| key | meaning |
|---|---|
| `version` | exact or range |
| `hold` | never upgrade. **`@hold` + `@version=` is a contradiction → error** |
| `expires` | **absolute** datetime. Present now, absent after |
| `until` | **absolute** datetime, on `absent:` only. Absent now, present after |
| `requires` | `BACKEND:NAME` — install that first. **A bare name is an error** |
| `after_install`, `before_install`, … | a hook. Hashed and locked |
| `source` | on `shim:` — `BACKEND:NAME`, which provider this stand-in forwards to. **It is read when the shim runs, not when it is deployed**: a shim is the linix binary under another name and has nowhere to keep data, so the answer comes from the line itself, which the shim process has already loaded. Absent means the bare name, resolved through `priority` like any other. **V.152** |
| `cron`, `run`, `notify` | on `schedule:` |
| `target`, `content`, `template`, `decrypt`, `identity`, `backup` | on `link:` |
| `enabled`, `status` | on `service:` |
| `value` | on `setting:` (the value to write) and on `firewall:default/…` (`allow` or `deny`) |
| `target` | on `dotfiles:` — where the tree is mirrored. Absent means the home directory. **There is no per-file option**: the tree has no place to write one, which is why it never decrypts (U24) |
| `runs`, `undo` | on `exec:` — `runs` caps how many times one script *content* may run (`1` is the default, `always` opts out); `undo` is what removing the line runs (U3) |
| `scope` | `user` or `system` — on `setting:`, `link:` and `shim:`, the three statements where "for me" and "for the machine" can differ (U19). Defaults to whatever the store does anyway, so it is written only to override. **Writing the default is not an error**: a config is allowed to state a thing it also gets for free |
| `formats` | ordered artifact preference. Repeated key makes the list. Backends that offer a choice only |
| `asset` | filename or glob narrowing the choice; `all` takes every match |
| `bin` | the executable inside an archive |
| `channel` | one version stream. Backends that publish channels only |
| `sha256` | checksum the resolved artifact must match. Not with `@asset=all` — one hash cannot verify several files |
| `allow_http` | bare flag: this URL may be `http://`. Downloading backends only (SEC2) |
| `unverified` | bare flag: nothing vouches for the bytes on this line. Legal wherever something otherwise would — LiNix's own `@sha256` on a downloading backend, and a manager that verifies a signature itself (`helm`). Refused where the manager's signed index answers anyway. **Never implied by `allow_http`** — over HTTP the checksum is the only thing left (SEC2). **On a tool that does not verify at all, it is accepted in silence (Q14)** — see below |
| `health` | `port:N`, or a command that must exit 0. A failure **restores the pre-change snapshot** (XIII.5) |
| `url` | where a `helm:` plugin is installed from. **Required on every `helm:` line** (U39) |
| `shim` | bare flag: put a PATH stand-in for this tool in `[bin_dir]`. The form R3's ruling names — a shim asked for on the tool's own line, rather than as a separate `shim:` statement. **It declares the same resource that a `shim:` statement does** (V.111), so it is placed, counted, guarded and torn down like one: adding the option to an installed package creates the stand-in, and deleting the option removes it |
| `sandbox` | bare flag: the shim above, and `linix run` confines the process |

**A shim never resolves to itself.** `[bin_dir]` is on `PATH` *ahead* of the real binary — that
is the whole mechanism — so LiNix looking up the shimmed name by bare name finds the shim, which
re-enters LiNix, which looks the name up again. Every name LiNix spawns is therefore resolved
through `PATH` **skipping any file that is the linix binary under another name**. The identity
question is asked of the file and never of the directory: `web:`, `github:` and `appimage:`
deploy real executables into that same `[bin_dir]`, and excluding the directory would hide them.
**V.152.**
| `classic` | bare flag: install this snap unconfined. `snap` only. **Converges (Q20):** adding it to an installed snap runs `snap refresh --classic`. `@classic=false` on a snap that is already classic is **refused** — snapd cannot narrow confinement, and only remove-and-reinstall can, which is the guard's call. Omitting the option manages nothing |
| `size` | the size a volume is created at. **Required on `lvm:`** — `lvcreate` has no default, so a line without one describes nothing that can be made. `lvm` only |
| `quota` | a cap on what a declared storage object may use. `btrfs` and `zfs` only |
| `mount` | where a declared storage object is mounted — recorded in fstab, so it survives a reboot, and taken out again when the declaration goes. `btrfs` and `zfs` only |
| `mount_options` | what the fstab entry's option field carries. `btrfs` only — ZFS keeps its mount properties on the dataset and has no such field — and **an error without `@mount`**, since there is then no entry for it to fill |
| `allow_shrink` | bare flag: a smaller `@size` may take space back off a volume that already exists. `lvm` only — a quota is a limit and lowering one destroys nothing — and **an error without `@size`**, since it then permits nothing (Q19) |

**A name may begin with `@`, and only the first character is special** (Q23). npm's scoped
packages are named `@scope/name` and every manager that has them prints them, so a name LiNix
lists is a name LiNix accepts: `npm:@angular/cli` is that package, and
`npm:@angular/cli@version=17.3.0` is that package pinned, because every `@` after the first
character of the name still opens the options (V.113).

**Changing an option changes the machine, or the line is refused with a reason** (Q21, ruled
2026-07-31). Every key above is a declaration, and a declaration that stops applying the moment
it is first applied is II.2's line-that-does-nothing arriving one layer in. **"Nothing happens"
is not a legitimate outcome of an edit**; neither is "it changed, and `sync` reports a change
again next run". Where the change cannot be applied at all it is refused **by name**, with the
by-hand path (`@size` shrinking, `@classic` narrowing) — never ignored. **An option the line
omits manages nothing**: absence is not a declaration of the default, or every config that never
mentioned a key acquires refusals it never asked for (V.107, V.108).

**The proof is per option, not per backend.** A lifecycle is install → list → remove and never
edits a declaration, which is exactly how five options stayed dead through thousands of green
checks.

**A key one family of backends reads is legal there and refused by name everywhere else** — the
same shape as `@url`, and for the same reason: `apt:curl@quota=10G` would read as the machine
having been told something when nothing anywhere would act on it. The authority is
`backends/capability.rs`, one table read by both the grammar and the install path, and a test
across the join asserts every key a backend reads is a key this grammar accepts (Q18). That
join is the rule, not the courtesy: five keys were read by code and refused by the parser at
once, which made `lvm:` unwritable, `snap --classic` unreachable, and shims — declarative-only
since R3 — impossible to declare.

#### `@unverified` on a tool that does not verify (Q16's sibling — Q14, ruled 2026-07-30)

`@unverified` means *"I accept a source nothing vouches for."* On a manager that verifies, LiNix
emits that manager's opt-out flag. **On a manager that does not verify at all, the state the
line asks for is already the state the machine is in, so the line is accepted, no flag is built,
and nothing is said.**

Measured, and it is the ordinary case rather than the exotic one: helm 3.21.3's
`helm plugin install --help` documents exactly two flags, `--help` and `--version`. There is no
`--verify`, no `--keyring`, no provenance — helm 3 verifies *charts* (`helm install --verify`
exists) and does not verify *plugins*. Helm 4 added plugin verification, which is where
`--verify=false` comes from.

So the earlier reading — *"accepted, and does nothing"*, the class this register keeps closing —
was the wrong diagnosis. It is **accepted, and already true**, and refusing the line would
reject correct configuration and take away the only way to install a helm plugin on helm 3.

**What this must not become is a warning.** Withholding a flag the tool never had is not an
event; a `warn!` there teaches people that a working run has a problem in it.

**And it is what makes the drift gate possible rather than impossible.** The capability table is
asserted against what the installed tool *does*, in both directions: where the tool verifies,
`@unverified` must put a flag in the argv; where it does not, the argv must carry none and the
run must be quiet. A gate written as "the flag is always present" is red on helm 3 for a reason
that is not drift, which is exactly what it was blind to.

### A name is what the manager will still answer to (U39)

**A declaration names the package, never the thing its install command happens to take.** Where
a manager installs by one string and lists and removes by another, the *listed* one is the name
and the install argument rides in an option:

```
helm:diff @url=https://github.com/databus23/helm-diff
```

**A `helm:` line without `@url=` is refused, and the refusal names the fix.** LiNix does not
derive the plugin name from the URL.

### `link:` and the file that was already there (T6, ruled 2026-07-23/26)

**A `link:` that replaces a file you wrote backs it up first, and removing the declaration puts
it back.** The backup is not a pile that grows: it exists to survive one replacement, and the
teardown restores it and deletes it. That is what dissolves the question of how many may
accumulate — **a backup that is put back cannot accumulate**, so there is no retention key and no
command to list orphaned backups.

**`@backup=no` opts one line out, and there is deliberately no machine-wide key.** The exception
is stated where the exception is. A setting that turned backups off everywhere would be copied
between machines and pasted from the internet like every other setting, and the file it silently
stopped preserving would be one somebody hand-wrote.

**A link is keyed by its destination, never by its source.** The teardown used to be handed the
declaration's source, so undoing a `link:` deleted the file in the user's own dotfiles repo and
left the deployed copy standing. Keying by destination also means editing `@target=` undoes the
old destination instead of orphaning it forever.

**Decrypt mode never backs up at all.** `backup_once` exists so a user is not silently robbed of
a config file they hand-wrote; a secret LiNix itself decrypted a moment ago is not that, and the
copy would sit in plaintext under the ordinary umask beside a file that got `0600` (T1).

**A `dotfiles:` tree is the `link:` lines it stands for, and gets every one of the rules above**
(2026-08-06, `Y10`). It is expanded once into those lines, and from there it is not a second
thing: the `link:` backend places each file, the extras ledger keys one row per placed file, and
the shared teardown restores the original through the removal guard. So a tree that replaces a
file you wrote backs it up, `--replace-existing` waives the refusal and never the preservation,
deleting a file from the tree removes its link and puts your file back, and deleting the line
does that for the whole tree. **A tree with its own placement loop had none of it** — it called
`remove_file`, and a file deleted from a tree left a dangling symlink for ever. **V.139.**

**A tree's removals count against `max_extra_removals` like any other resource's.** Deleting a
`dotfiles:` line with more files than the ceiling is refused by name, pointing at the setting to
change — the same refusal twenty-one `link:` lines get, out of the same budget, because a tree is
a pile of `link:` lines and nothing else. Exempting a tree would be the second teardown again.

**Ownership is what the ledger recorded, not what the filesystem looks like.** *Did LiNix put
this here?* is answered by the row, in union with "is it a symlink" for anything placed before
the row existed. Asked as `is_symlink` alone it is wrong wherever the deploy falls back to a
copy: LiNix called its own file a destination it did not create, and refused to touch the tree
containing it.

**A `link:` links, on every platform and every drive; a copy is what a missing privilege gets,
and it says so** (2026-08-06, `Q48`). Windows is not asked whether the source and destination
share a drive — a Windows symlink stores its destination as a string and resolves it on open, so
it spans volumes, and the check that claimed otherwise compared a verbatim `\\?\C:` prefix
against a plain `C:` and answered *different drive* for every path on every machine. The only
thing that genuinely varies is `SeCreateSymbolicLinkPrivilege`, so it is the only thing handled:
the symlink is attempted, and `ERROR_PRIVILEGE_NOT_HELD` — and no other error — falls back to a
copy under a warning that names Developer Mode and says edits will not propagate until the next
sync. **V.141.**

### Health checks (XIII.5, U7)

**Two scopes, one revert path.** `@health=` on a line answers *did this upgrade break this*.
A `health = [...]` list in `preferences.toml` answers *is the machine still working* — the
boot, the network, the thing two packages away, which no package can see. **They are not
alternatives**: both run after a change, and a failure of either restores the snapshot the
sync took before it started.

**Declared health checks with no snapshot provider are refused before the change starts** —
not after, when the upgrade has already happened and the check can only confirm the damage.

**Health is an open vocabulary (U31).** A check is a user-declared command (argv, exit 0 =
healthy) beside the built-in checks — a service on a port, a config that must parse, whatever the
user's machine needs. **A check that cannot run is a failed check, never a passed one** (fail
loud): if LiNix cannot execute it, the result is unhealthy and the change rolls back. A check
command is argv a shared repo can carry, so it rides the II.12 hook ledger and is approved by
`linix lock`.

> **Half built (checked 2026-07-26).** The open vocabulary and the fail-loud rule are real —
> `model/health.rs::Probe::Command` takes any command, and `probe_ok` returns false when it
> cannot run. **The ledger sentence is not:** `verify_health` runs the command directly and
> consults no ledger, so an `@health=` arriving with a pulled config runs unapproved. It is the
> one runnable thing in the tree that II.12 does not see. Phase 7p, item 8.

```
apt:nginx@health=port:80          the port must answer after this installs
apt:nginx@health=systemctl is-active nginx
```

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

**A changed `channel` is drift on every backend that has channels, and the repair is whatever
that backend's switch actually is** (D13; owner ruling 2026-08-09, `Y23`). snap refreshes in
place. flatpak has no switch at all — it installs branches side by side and keeps them — so the
declared branch is installed and `make-current` points the app at it, and **the branch that was
there is left installed**: removing it is a removal, and a channel edit did not ask for one.

**A channel LiNix cannot read is left alone, and two branches is a channel it cannot read.**
flatpak's listing has no column saying which installed branch is current, so an app on two of
them reports no channel rather than one of the two. This is D13's rule and not a shortcut around
it: a guessed value schedules the same switch on every sync for ever, which is worse than the
drift it was meant to catch. **V.151.**

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
- **`generate:PATH` — a command whose stdout is declarations (U33), off by default.** With
  `allow_generators = true`, a generator runs through the II.12 ledger and its output is spliced
  into the stream before probing and collection, so it passes the same guard and removal preview
  as a typed line; a generator that fails is a failed resolution, never an empty set (V.79). It
  takes no options and is approved by `linix lock` (scanned first, because resolving runs it).
  The `exec:` half of U33 is the U4 amendment: `exec:` may now install software — a documentation
  change, since exec's per-script ledger approval is already its gate.
- **A module may take parameters (U32).** `param user` (required) / `param gpu = none` (with a
  default) at the top of a module; `use workstation(user=shaul, gpu=nvidia)` binds them. The
  values substitute through the existing `$name` machinery one scope wider — into `when` and into
  every value the global `vars` pass reaches — and a missing required parameter, or an argument
  naming no parameter, is a loud error, never a silent empty string (V.78). The expansion is
  ordinary declarations, shown in `linix eval` and the removal preview before anything runs. A
  profile takes no arguments: only a module declares `param`.

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
| **`exec:` script content already run here** | `sha256:… → 1` | **built** (`locks/exec.toml`). Keyed by **content**, while `locks/hooks.toml` is keyed by declared path — the two answer different questions (*has this already run?* vs *is this allowed to run?*), so a script edited after approval is both unapproved and un-run, which is the pair you want |
| **applied extras** | `service:nginx`, `link:<destination>`, `repo:apt:ppa:x/y` | **built** (`locks/extras.toml`). It is what makes *removing* a `service:`/`link:`/`shim:`/`repo:` line undo it — without a record of what was applied, deleting the last extra line is a change with nothing to diff against |

**`lock` and `unlock` name the axis they act on** (owner ruling, 2026-08-03, `Z2`):

```
linix lock   [versions|backends|scripts|all] [NAME…] [--list]
linix unlock [versions|backends|scripts|all] [NAME…] [--list]
```

| axis | the ledger | `lock` | `unlock` |
|---|---|---|---|
| `versions` | `locks/versions.json` | pin what is installed | drop the pins; sync takes what the managers offer |
| `backends` | `locks/bare.HOST.toml` | record what each unpinned name resolved to | forget it, so the next sync asks again |
| `scripts` | `locks/hooks.toml` | approve everything the config can execute, at its current hash (II.12) | withdraw approval, so a sync that reaches one refuses to run it |

**A bare `lock` or `unlock` is `all` three.** `NAME` scopes any axis and matches the whole ledger
key or its tail — `unlock versions curl` and `unlock versions apt:curl` both pick out `apt:curl`.
A name that picks nothing out warns, names the ledger, and changes nothing. **A name where the
axis goes is refused**, with the three axes listed: there is no bare-name form of either verb.

Both verbs took different, unrelated ledgers until this ruling, so `linix unlock` — the obvious
undo for `linix lock` — discarded a backend resolution and the next sync uninstalled a package
(V.127). The per-name and per-backend forms this section used to promise and then denied
(`lock <name>`, `lock --backend cargo`) exist now as `NAME` on any axis.

The one-file-per-backend layout has exactly one real instance: `locks/github.toml`, written by
the backend as it installs rather than by `linix lock`, because the artifact is only known at the
moment it is chosen. `github` asks `Layout::lock_file()` for that path rather than building it,
so there is one answer to where a backend's lock lives.

**Every path that deliberately moves a version forward re-records the pins it moved** — `upgrade`
in all its modes, and `sync --upgrade`. **Only entries already in the lock are refreshed**; a
package nobody pinned gains no pin, because an upgrade is not a `lock`. Without this an upgrade
did not stick: the pin still named the version that was replaced, and the next ordinary sync —
which converges to the lock — planned the package straight back down (V.127).

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

**`linix unlock backends [NAME…]` is how you ask again** (owner ruling, 2026-07-22; the axis
named 2026-08-03), alongside the text editor II.15 promises for regex. With no names it forgets
every name this host froze; `--list` shows them and changes nothing. It is what you run when a
better source appears:
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

**A `#rhai` hook gets this same library, from the same place (II.12, V.150).** "Trusted the same
as a hook" is a definition by reference, so the two cannot be different sets — and when they
were, the narrower one was the hook.

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

**A statement declares which phase of a sync its work happens in, and the order of the phases is
a type** *(Y13)*. The phases, in the order they run:

| phase | what runs in it |
|---|---|
| **resolution** | consumed while the desired state is computed — set math, `param`, `use`, a variable, `generate:`. Not work, and nothing downstream ever sees one. |
| **repositories** | `repo:`, before the packages: a package from a PPA cannot install until the PPA is there. |
| **packages** | the package transaction — `apt:jq`, `absent:apt:nano`. |
| **dependents** | `shim:`, `service:`, `link:`, `setting:` — each leans on the package plan having run. |
| **dotfiles** | `dotfiles:`, a tree standing for the `link:` lines it holds. |
| **firewall** | `firewall:`, after the packages, because a rule usually exists to let in something just installed. |
| **schedules** | `schedule:`, provisioned onto the OS scheduler. |
| **execs** | `exec:`, after the packages and dependents a script leans on. |

**A statement kind added to the grammar does not compile until it has been given a phase**, and
"is there work after the package plan?" is a comparison against the package phase rather than a
list of kinds. It was a list of kinds four times over — the dispatch, the dry-run branch's copy
of it, the per-kind accessors, and a chain of `||` — and each new kind was missed by one of them:
extras, then `exec:`, then `dotfiles:`, then `firewall:`. **A phase that is declared but
dispatched to nothing is a build failure**, and `repo:` is excluded from "work after the packages"
because it is phase 1, not because anybody remembered to leave it out (V.144).

**A plan states what it was computed over, and that decides what it may remove** *(Y12)*. The
removal set is `managed − desired`, so it is only as good as `desired`: a caller that hands the
planner something narrower than the machine's whole declaration set gets a removal planned for
everything outside it. There are exactly three things `desired` can be, and a caller names which:

| | what `desired` is | what may be removed |
|---|---|---|
| **whole** | the machine's whole declaration set | drift, bounded by the backends `priority` names |
| **narrowed** | one profile or module, filtered from the config | nothing |
| **just these** | a list that is not the config — a transient shell's requests | nothing |

**The whole case cannot be written without the list that bounds it.** The backends `priority`
names is a value the resolver mints from the file, not a list a caller assembles, because "not
listed means LiNix does not touch it at all" (II.6) is a promise no caller can keep by
remembering to. `activate`, `deactivate`, `plan`/`apply` and `upgrade --canary` each broke it by
planning a removal with no list at all, and none of them had to hold one to do so.

**A package left alone because its backend is not in `priority` is reported, not dropped in
silence** — the same rule as a protected package (II.10).

### II.7a The setup a manager needs before it can install anything

*(Owner ruling, 2026-07-29 — Q10, Q11, Q13.)*

Phase 0 has two halves and they run in this order:

1. **A manager the configuration declares and this machine lacks** is offered, from a
   `[[bootstrap]]` row (7c).
2. **A manager that IS here and cannot install anything until something is set up** is offered,
   from a `[[prereq]]` row. `mix` needs Hex; `asdf` needs the plugin for the tool named on the
   line; `opam` needs a switch. Each of these made *every* install through that manager fail,
   with the manager's own message and nothing LiNix could do about it.

**Ask, then do — with `--yes` as the flag that forces it.** LiNix prints what is missing and the
exact command, and runs it only if you agree. `--yes` agrees in advance, because a run that has
already answered "apply the plan" has answered this too and a second yes-flag is one more thing
to pass. A non-interactive run without `--yes` says what it would have asked and changes
nothing. `--dry-run` says the same and changes nothing.

**A row must be able to tell whether it is needed.** Every row carries a `probe`; a row with
none would act on every sync. The probe's exit code is the answer, unless the row names
`probe_output`, in which case one line of the probe's output must equal it — `asdf plugin list`
exits 0 whatever it says.

**A row whose command names `{name}` is about one declared package**, and is offered once per
line rather than once per manager. That is read off the argv, not declared beside it.

Rows ship compiled in, and a repo adds its own in `adapters/prereq.toml`, which rides the II.12
ledger like every other `adapters/` file. The built-in file does not, for the reason
`snapshot_builtins.toml` does not: gating a first-party compiled-in asset would leave a fresh
machine unable to install through `mix` until `linix lock` had run.

**`check health` reports a manager whose manager-level prerequisite is unmet as degraded**, with
the reason and the command — never `[READY]`. Degraded rather than critical because its reads
genuinely work. Per-package rows are not health's question: it has no declarations to ask about.

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
`apt:b@requires=apt:a`. Same graph, same walk, same answer: the planner orders by the
`@requires` edges, and a loop has no order. **It owes the same error as a `use` loop**: which
packages, and the file and line each edge came from.

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
packages (`@requires` edges) → things depending on packages (services, shims, links).

**Within packages, the only ordering LiNix imposes is the one you wrote.** A manager resolves
and installs its own dependency closure at install time, so LiNix does not ask what a package
depends on and does not order by the answer. **V.115a.**

**What LiNix may remove: what it manages and you stopped declaring. Plus `absent:`. Nothing
else, ever.**

### II.7c A manager this machine does not have is skipped, not failed

**One config, three machines** (owner ruling, 2026-08-06, `Y15`). A line pinned to a manager
this host does not have is **not a broken config — it is the half of the config that belongs to
a different machine.** `apt:ripgrep` beside `winget:ripgrep` beside `brew:ripgrep` is what a
portable configuration looks like, and each machine does the part it can.

So: **a declaration whose backend is not on this machine is skipped, named in the run's
`skipped` list with the reason, and the command still succeeds.** That holds for an install, for
a removal LiNix had recorded through that manager, and for `absent:`.

**The two ways a manager can be missing are one answer to the user.** A backend this build never
registered (`apt` on Windows) and one registered here whose program is not installed are
different facts about the registry and the same fact about the machine: nothing to install
through, nothing installed to remove. `BackendRegistry::runs_here` is the single spelling.

**A name that is not a backend is still an error.** The typo check is the grammar's, against
`Vocab` — `brwe:ripgrep` never reaches a plan. Skipping is for names LiNix knows; a config that
silently skipped its own misspellings would describe a machine nobody has. **This is the same
line `Q9` clause 3 drew** for a backend named in a command *argument* — a typo is the user's
mistake, a missing manager is a fact about the machine — and II.7c is that rule reaching the
declarations.

**Not knowing is not the same as knowing there is nothing.** A manager that is *here* and whose
listing failed is unanswered, and II.7b's "a manager that could not answer has not said no"
governs it — its removals are still scheduled and left to report their own failure. This rule is
only about a manager whose program is not on the machine at all.

**A package that fails still fails the command.** Absence is a property of the machine; a failed
install is a property of the run, and warning past it under a summary claiming success is what
`AU1` bans. `--keep-going` is the per-run opt-in for a caller that would rather take what it can
get; there is no file key for it, because a machine-wide setting that downgrades every future
failure to a warning is the destructive default nobody typed.

**An empty plan with a non-empty `skipped` is not `already up to date`.** A machine whose whole
config is pinned to managers it does not have has converged nothing.

## II.8 Commands

| command | does |
|---|---|
| `install PKG… [--into NAME]` | write the line, sync |
| `uninstall PKG… [--temp]` | remove the line from every active module, sync |
| `forget PKG…` | drop from the registry. Stays installed. LiNix never touches it again |
| `adopt [PKG]` | take over the machine, or one package |
| `sync` | make the machine match |
| `plan` | show what sync would do |
| `check` | **the one looking command** (U9). Nine sections — `config`, `drift`, `unmanaged`, `absent`, `conflicts`, `health`, `security`, `approvals`, `adapters` — one line each by default, detail when a section is named |
| `heal` | **the one acting-on-what-check-found command.** `doctor --fix`'s repairs live here |
| `rebuild` | remove and reinstall what is declared, one backend at a time (II.11b) |
| `lock [AXIS] [NAME…] [--list]` | freeze one of the three ledgers, or all of them: `versions`, `backends`, `scripts` (II.6) |
| `unlock [AXIS] [NAME…] [--list]` | release one of the three, or all of them. `unlock backends` is the one that can move packages (II.6) |
| `purge-undeclared` | delete everything LiNix doesn't manage |
| `remove-orphans` | the names each backend can say are orphaned — shown, guarded, removed (II.11c) |
| `clean-cache` | downloaded archives and build caches. Removes no installed package |
| `add SOURCE` | vendor someone else's module into your repo (U14) — it lands as a file you can read, never as a live reference |
| `try` | rehearse this config on a clean machine in a container (U12) |
| `eval` | print the resolved desired state as versioned JSON (U17). Takes no locks, changes nothing |
| `repl` | interactive prompt over the one resolver (U34): resolve a name here, evaluate `when`, `:vars`/`:eval`. Read-only |
| `vars` | print each resolved variable, its typed value, its type, and the active provider (II.6b) |
| `why PKG` | why this is declared: the gate chain, the variables behind it, and the commit that introduced the line (7l) |
| `diff COMMIT COMMIT` | the change in **packages**, not text |
| `teleport PKG BACKEND` | move a package to another manager: rewrite the line in place, sync |
| `shell` | throwaway shell. Outside the model |
| `bundle` | git bundle + artifacts + registry |
| `restore DIR` | put a bundle back — the other half of `bundle` (V.59) |
| `export FORMAT` | Brewfile / requirements.txt / package.json |
| `activate NAME… [-a]` | write `active` — the list, or `-a` to add to it (II.6), sync |
| `deactivate NAME…` | take away from `active` (II.6), sync |
| `snapshot restore` | pick a filesystem snapshot and go back to it (U42) — **the machine half of going back.** Named for its mechanism, not `undo`, because the other half is the manifest history |
| `history`, `rollback REF` | browse the manifest history, and go to a commit in it (II.13) — **the intent half.** A TUI and a CLI over one mechanism, which is an interface pair and not two vocabularies |
| `upgrade`, `list`, `profile`, `service`, `repo`, `hold` | as today, all reduced to file edits |

**A user may name a verb (U35).** A `[verbs]` table maps a name to a *sequence* of built-in
verbs — `refresh = ["sync", "upgrade --all"]` — the `defun` over the command surface, sibling to
`command_aliases` (which renames one command). It is **composition only**: every step must be a
built-in, a step that names anything else is refused and pointed at `exec:`/U33 (off by default),
a verb takes no arguments of its own, and a verb never shadows a built-in (V.77). One data lock
covers the whole verb, taken as a writer — the safe default for a sequence that may install or
remove.

**`status`, `doctor`, `unmanaged`, `absent`, `conflicts`, `insight`, `metrics` and `audit` are
gone, and they are gone rather than aliased** (U9, ruled 2026-07-24; built 7i). They were ten
ways to ask *how is this machine doing*, each with its own output shape, and the answer to "which
one do I run" was to run several. They are sections of `check` now. **The dividing line the whole
collapse rests on is `check` looks, `heal` acts** — which is why `heal` survived the collapse and
`doctor --fix` did not: a command that repairs is not a status command with a flag.

**A verb inherits the vocabulary of its mechanism (U42, V.86).** II.13 says going back has two
mechanisms — *"Git is your intent. Snapshots are your machine"* — and the command surface says
which one it is driving before it runs, not after. That is why the snapshot gallery is
`snapshot restore` and not `undo`: `undo` is the most natural word in the program and it pointed
at the less likely of the two meanings. **`undo` is retired rather than reassigned**, because a
word that already meant the wrong thing does not improve by meaning a second one.

**The command count is not the complaint, and a consolidation is checked against the running
program before it is made.** `uninstall`, `remove-orphans`, `purge-undeclared`, `unmanage`,
`reset` and `clean-cache` read like a cluster of synonyms and are not: two of them delete
software, two delete records, one deletes downloads, and no two can be swapped. II.17's list is
the only approved deletion; anything beyond it is a question, and a question answered from a
command list that was never run is how a real capability gets removed to fix an overlap that was
not there.

**What a run says about itself (U43, V.87).** **A command's answer goes to stdout. Only narration
goes to the log, and the log is off by default** — `warn` and above. `-v` adds the narration,
`-vv` adds debug, `-q` leaves only errors, and `-q` beats `-v` when both are given. `RUST_LOG`
outranks all of it.

The two halves are one rule and the order between them is load-bearing: **a result printed at
`info!` is a result nobody sees once the default drops.** `sync` on an up-to-date machine, and
everything `lock` and `unlock` report, were on the log channel with nothing on stdout — silence
there is worse than noise, because noise is ignorable and silence looks like a crash. **The level
is read from argv, never from the parsed CLI**: it must be live before the shim hijack, and a
filter configured before its flag is parsed is a flag that silently does nothing.

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
`plan`, `list`, `upgrade` → yes. `sync`, `purge-undeclared` → error: *"scoping a removal
isn't safe; use a profile."*

**`remove-orphans` goes through the guard** — ask the backend what it intends, check the list
against protection, refuse if it touches something protected. **Sync nudges:** *"3 packages are
now orphaned; run `linix remove-orphans`."* Want it automatic?
`schedule:tidy@cron=0 3 * * *,run=clean` — `clean` there is a schedule action, not a verb.

**A failed `install` withdraws the line it just wrote when that line can never succeed** (Q1,
owner 2026-07-27; V.90). `install` writes first and syncs second (S15), so a failure leaves a
line behind — and every later command parses the model, so one impossible line breaks `sync`,
`upgrade` and every install after it.

- **Withdrawn:** a failure that says **the name is not there** — `Error::says_a_name_is_absent()`.
  Three roads reach it: `Unresolvable` (no backend claims the name), `NoSuchPackage` from a
  backend that resolves names itself and knows which one it looked up, and a `CommandFailed`
  whose output matched that manager's own `absent_markers`.
- **Kept:** everything else. A dropped network, a held lock, a failed hook — you did mean it,
  and retrying is right.
- **Absence is not permanence, and withdrawal reads absence** (N-1, 2026-07-29; V.90). It used
  to read `CommandFailed { retry: Permanent }`, which is wrong in both directions: helm's
  `plugin already exists` is permanent about a name that is plainly there, and the 36 backends
  with no `ExitPolicy` could not answer `Permanent` at all — so the same typo wedged the config
  behind `npm:` and did not behind `scoop:`. `ExitPolicy` therefore answers two questions
  separately: `permanent_markers` for *would another attempt differ?* and `absent_markers` for
  *does the name exist?* Matching an absent marker implies permanence; the reverse never holds.
- **A transient marker outranks an absent one** (2026-08-02). Managers word "I could not reach
  the index" and "the index does not have it" identically — choco answers an unreachable feed
  with `The package was not found with the source(s) listed`, apt answers un-fetched lists with
  `Unable to locate package` — so an absent verdict is only worth reading when the same output
  does not also report a transient failure. Applies to `retryability` and to the withdrawal
  question alike. `permanent_markers` still outranks both.
- **A policy that forgives a non-zero exit must be able to contradict it.** `benign_exits`
  without `failure_markers` or `failure_line_prefixes` reports every run ending on a forgiven
  code as a success, including the ones that did nothing. Derived from the registry and
  ratcheted, because the pair that had it was found by reading the table by hand.
- **Never off `Error::retryability()`**, which also calls a refusal, a cancelled prompt and a
  bad config file `Permanent`. Deleting a declaration because someone answered "no" to a prompt
  is worse than the wedge.
- **Only lines the failure can be attributed to are withdrawn.** In a batch, the rest are kept.
  Attribution is the *one* thing a message may be read for — which of the lines this command
  wrote the manager was talking about — and never whether the name exists.
- **A backend with no `absent_markers` is a bound, not a bug.** An unclassified failure keeps
  the line, which is the safe direction; what is forbidden is the bound being *unstated*, so the
  set is derived from the registry and ratcheted.
- **A line kept on purpose names its file and how to remove it, and only an unclassified
  failure may suggest that trying again could work.** Each reason a line stays earns its own
  sentence: refused (*edit the line*), exhausted (*it already repeated*), a name absent
  elsewhere (*a name does not exist*), unclassified (*`sync` will try it again*). A wedge with
  an exit is not a wedge, and a promise the program has already disproved is not an exit.

**The model is packages *and* resources, and one evaluation yields both** (N-2, 2026-07-29;
V.90b). `link:`, `service:`, `setting:`, `shim:`, `schedule:` and `repo:` are declarations, so
every command that answers "does the machine match your files?" has to read them.

- **One computation.** `check`, `check drift`, `plan`, `apply`, `sync`'s summary and the guard
  read the same value. Five code paths each answering separately is how `check` came to report a
  match over a declared file that was not on disk while `--dry-run sync` named three teardowns on
  the same tree.
- **Including the loop that does the work** (round 4, 2026-07-30). Reporting from the shared
  value and *acting* from something else is the same defect with a longer fuse: the placement
  loop asked nothing, so `sync` re-placed all three links on every run and the second run left
  `<target>.linix-backup` files in the user's directory — backups of the copies LiNix had made
  itself — while `check` called the machine converged and `plan` reported nothing to place. The
  probe decides in both places now, and it compares a `link:` by **content**, because the
  destination merely existing is what the ledger already knew.
- **Two questions, two sources.** *Has this ever been applied?* is the extras ledger's, and the
  answer is the same for all six kinds. *Is it still in effect?* is the machine's — a resource
  LiNix placed and a user deleted is drift no record can see.
- **A resource that cannot be read back is named, not assumed converged.** "The machine matches
  your files" over something nobody looked at is the failure this rule exists to prevent, so the
  count and the keys are printed beside the verdict.
- **`plan` freezes resources and `apply` executes them**, through the same phase list `sync`
  runs. A plan that omits work `sync` would do is a review that reports nothing to see — and the
  guard's own refusal text sends the user to `linix plan` to see what would be undone.
- **And the packages go through the engine that executes every other plan** (2026-08-06, `Y14`).
  `apply` walked two serial loops of its own, so the one command named after review and
  deliberation was the one with no write-ahead log, no transaction, no rollback, no snapshot, no
  health check and one manager invocation per package — and a failure was a warning under a
  summary reading `Applied plan`. A frozen plan is a `SyncChanges`, and `SyncEngine::sync`
  executes one; **the freeze survives because the engine does not plan.** The guard `apply`
  used to call for itself is the engine's first act over the same graph under the same scope,
  which is one call rather than two that can disagree. **V.148.**
- **A frozen plan keeps the ordering it froze.** `@requires` is in the plan file, in the specs'
  own `requires`, and rebuilding the graph read it back as nothing — so an ordering held on the
  run that planned it and was dropped by the command that promises *the exact plan you inspect
  is the one you later apply*. One function wires those edges now, for the planner, `apply`,
  `rebuild` and `heal` alike: of the four hand-written copies, two had no edges at all.
- **A guard preview asks about the kind the items actually are.** `plan` merged resources into
  the package removal list and asked `RemovalKind::Package`, and `protection_of` opens by asking
  whether a package line could hold the name — which no `link:` key can. So `plan` predicted a
  refusal for undeclaring three dotfiles, `apply` performed it at rc=0, and the sentence a user
  read was about package names. Each list is inspected as its own kind, counting the other
  against the same ceiling, which is what `sync` already did.

**`check health` has four states, and "not installed" is one of them** (Q2, owner 2026-07-27;
V.91). A package manager the user does not have is **absent**, not critical:

| state | means |
|---|---|
| **ok** | it is here and it answers |
| **degraded** | it is here, it answers, something needs attention |
| **critical** | it is installed, or `priority` names it, and it cannot work |
| **absent** | it is not installed here and nothing asked for it |

**Absent is never counted as a failure and never decides the verdict.** Fail-loud is about
failures, and a manager nobody asked for is not one — `25 OK, 0 degraded, 23 critical` on a
healthy Windows box was the principle applied where there was nothing to report. **The rollup
and the detail view read the same tally**, because two counts of one machine will disagree.

**A usage error exits 1** (Q3, owner 2026-07-27; V.92). A mistyped subcommand or flag is
"failed — something went wrong", which is already in the table. It must not exit 2: that means
*a read-only command found work to do*, and a CI job branching on the published table would
read a typo as a drifted machine. **Every refusal exits 3** — a refusal raised with a generic
error instead of `Error::Refused` never reaches the mapping and is a contract violation, not a
detail.

## II.8b `--dry-run` performs nothing, and the check is not a habit

**A preview writes no file the run would have written.** Not "each verb remembers to ask" —
that was the arrangement, and it produced two rounds of the same finding. Round 1 fixed
`uninstall`, `unmanage`, `module create` and `schedule add`. Round 2 measured `activate`,
`deactivate`, `lock`, `git init` and `config init` still acting, and `--dry-run activate Work`
left the machine on Work while printing nothing at all.

So the flag is a property of the run, read where the **write** happens:

- `core::dry_run` holds it, set once in `main` after the config merge and before dispatch;
- `utils::file::persist` is the writer **every file LiNix owns** goes through — `active`, the
  profile files, `preferences.toml`, the settings file, all six ledgers under `locks/`, the WAL,
  and `data/registry.json`. It reports what it would have written, at a level the default filter
  shows, because acting silently was the worse half of the defect. **There is one writer, and
  that is the rule.** Round 4 found a second — a permissive `atomic_write` beside the
  preview-aware one — and the `save()` methods had reached for it: `--dry-run adopt` recorded 112
  packages as managed while the manifest declaring them was correctly not written, which is the
  one state the model reads as *the user deleted every line*. `atomic_write` is private now, so
  the shorter name is not reachable;
- `GitManager::init` and `commit_all` carry the same check, because a repository and a commit
  are the one case where the preview's residue is a permanent artifact rather than a changed
  file.

**One deliberate exception, and it is named rather than hidden.** `profile show` points `active`
at a profile, resolves, and puts the file back. That pair is scaffolding for a read, so it
writes directly through `swap_active_for_read`: honouring the flag there would make the first
write a no-op and `--dry-run profile show Work` describe whatever was already active — the same
silent-wrong-answer defect, moved.

**Checked over every subcommand, not over the ones with a bug report.**
`tests/dry_run_every_verb_tests.rs` snapshots **the whole fixture — config, data and the working
directory** — runs the command under the flag, snapshots again and requires the bytes to match
— **and requires the same command without the flag to change something**, so a case whose
fixture made the command a no-op fails as a broken case rather than passing as a clean one.
Every name in `--help` is either driven or exempted with a reason, and every exemption has to
name a command that still exists.

**An exemption says what the fixture cannot supply, never what the instrument cannot see.** The
snapshot walked only the config directory for two rounds, and three verbs were excused *because*
they wrote to the other one — "holds live in the data dir, not the config dir". `data/registry.json`
is the managed set: the file that decides whether the next `sync` removes a package. A reason of
that shape is not a reason, it is the finding, and it is why the gate named "every verb" could
not see B-1. `hold`, `unhold`, `heal` and `adopt` are driven; a host that cannot supply the work
(`adopt` with nothing unmanaged) is **skipped by name and counted**, never quietly passed.

### A command whose product is a file (Q15, ruled 2026-07-30)

**A file the user named as the command's destination is still a file the run would have
written, so `--dry-run` writes none of it.** `bundle` and `export` produce an artifact — a
restore set, a Brewfile — that outlives the run and can be carried to another machine. A
preview that manufactures one has produced the thing it was asked to describe.

**`plan` is the exception, and it is the whole exception.** Its product *is* the preview:
`--dry-run plan` that wrote nothing would be a command with no output. The distinction is not
"did the user name the path" — they name it for `bundle` too — but **whether the file is the
preview or the result**.

So: `--dry-run bundle` and `--dry-run export` print what they would write, to where, and write
nothing; `--dry-run plan` writes `linix-plan.json` exactly as `plan` does. A preview that
declines to write says so with the `[DRY-RUN]` marker every other verb uses, and never in the
past tense — `bundle` printed *"Bundle written to X"* about nine files it had genuinely written
under the flag, which is how this was found.

## II.9 Adopt

**Adopt takes manually-installed packages only. Never the dependency closure.**

**(measured)**

| Backend | Record | Result |
|---|---|---|
| apt | `apt-mark showmanual` | 103 of 579 |
| pacman | `-Qqe` | 11 of 173 |
| conda | `env export --from-history` | 4 of 88 |
| **winget** | `winget export` | **78 of 280** |
| choco / scoop | installs no dependencies — everything **is** chosen | exact |
| **pip** | **none.** No flag separates dependencies | **adopt nothing, say why** |

**A manager adopts only what it can put back** (2026-08-05, `Q36`). "Everything installed was
chosen" and "everything installed can be reinstalled" are two questions, and a manager may pass
the first and fail the second. `winget list` reports every Add/Remove-Programs and MSIX row under
an identifier winget invents from the registry — 186 of 280 on the measured host. `winget
uninstall` takes those; **`winget install` refuses every one of them**, and a third carry their
own version, so the name moves out from under the declaration when the package updates. A
declaration that can never converge is worse than an absent one: it fails every later `install`
in the same transaction (`Q34`). So adoption reads the manager's own export where one exists,
and where entries are dropped it says how many, why, and names some.

**Base-image packages ARE adopted** — `grub-pc`, `linux-image-generic`. They keep the
machine bootable, and `purge-undeclared` deletes what isn't declared.

**A bare `adopt` takes only what a manager can attribute to a choice** (2026-08-05, `Q39`).
The table above is the first half of that rule — a manager that cannot separate a dependency
from a decision adopts nothing. The second half is a manager that lists perfectly well, where
*being on the machine is not evidence anybody chose it*: an init reports the services that are
running and never who started them. Those are skipped, with the count, the reason, and the
command that takes them.

**`linix adopt <backend>` takes exactly that backend**, opt-out included — a skip is a default,
never a refusal. **`--enabled-only`** narrows it to what this machine starts on its own, which
is the closest thing an init keeps to a record of a decision; a backend that cannot answer that
question is skipped and named rather than quietly widened back to everything. (V.134.)

**Output:** one `modules/adopted.txt`, grouped by backend with comment headers, sorted.
Header states: this is an estimate; deleting a line uninstalls, *except where the guard
refuses*; `linix unmanage <backend>:<name>` is the way out. A second section lists what was found and left
alone, commented out, with the count per reason.

**Adopt does NOT consult the guard — not `protected_packages`, and not OS-essential**
(2026-08-05, `Q47`). This resolves **E7**, where "protected" means two opposite things:
*never remove* in the guard, *never adopt* in `migrate.rs`. **Protection means one thing:
never remove.** So adopt takes every manual package including protected and OS-essential
ones; the guard then prevents their removal. This is a **change from what Stage 2 built** —
Stage 2 routed adopt's skipping through `guard::protection_of`, which unified the code while
keeping the word ambiguous. Adopting a guarded package is correct: it belongs in your file,
deleting that line is refused (V.26), and leaving it undeclared meant nothing in LiNix had an
opinion about the packages that keep the machine running.

## II.10 The guard — ten refusals, one function

| | |
|---|---|
| `protected_packages` | never remove this |
| `unprotected_packages` | …unless I say so. **Wins over everything, including OS-essential** |
| OS-essential | never remove what the OS says is load-bearing |
| undeclarable | never remove a name no package line can hold — **not even `unprotected_packages` releases this one** |
| `max_removals` (default **20**) | never remove more than this many packages at once |
| `max_extra_removals` (default **20**) | the same for resource teardowns — its own budget (`Y20`) |
| `max_port_closures` (default **20**) | the same for ports closed because nothing declares them — its own budget (`N8`) |
| `max_installs` (default **unset**) | never install more than this at once |
| `max_total_changes` (default **unset**) | never make more changes of every kind together than this in one command (`N8`) |
| `deny_packages` | never install this |
| `pinned_only` (default **off**) | never install anything without an explicit `@version=` |
| `require_snapshot` (default **off**) | never change anything when no snapshot can be taken |
| `deny_vulnerable` (default **off**) | never apply when `audit` reports a managed package vulnerable |

All in `[guard]` in `preferences.toml`. One decision function.

**A protection that changes the plan says so, in the same run.** A `protected_packages` rule on a
managed package nothing declares does not stop a `sync`; it removes that one removal from the
plan — and the plan then *names it and says why*, in `sync`, in the preview, in `plan`/`status`
and in `check`. `already up to date` is a claim about the machine and may not be printed over a
package LiNix has just decided not to remove. The same holds for the other reason a drift removal
is dropped: a managed package whose backend has left `priority` (V.125a).

**Every removal path calls it, and the list of paths is `GuardScope`, not a sentence.** The
enumeration below is generated by reading the enum in `app/sync/guard.rs`, because that enum is
the thing a new caller has to add itself to; a prose list is a place a path can be missing from
without anything noticing, which is exactly how S24 survived thirteen sessions.

**And the enum is not enough either.** A path that never calls the guard has no scope to appear
under, so reading the enum answers "what do the callers say they are?" and never "who did not
call?". The resource teardown in `app/apply/extras.rs` and `linix repo remove` both deleted for
months without a `GuardScope` between them, under a paragraph asserting that none could.
`tests/removal_guard_enumeration_tests.rs` asks the other question: it counts the removal calls
in `src/` and fails on any that no ledger entry accounts for. **Both directions, or neither.**

**The guard covers resources, not only packages** *(owner ruling, 2026-07-28 — Q7)*. A `link:`,
`service:`, `setting:`, `shim:`, `schedule:` or `repo:` line that leaves the model is torn down
under `protected_packages` and against `max_extra_removals`, counted **once for the whole
command** rather than once per phase. That ceiling is its own (`Y20`), and since `N8` so is the
firewall's: software leaving a machine, a resource being torn down and a perimeter tightening are
three different events, and one budget for all of them would make the strictest govern all — a
server whose first `firewall:` line closes forty ports could not also remove a package. All are
answered by the same `--allow-mass-removal`, because "yes, that many, I meant it" is one
question. A `protected_packages` entry matches a resource by
its key and also by the final component of a path key, so `protected_packages = ["vimrc"]`
protects `link:/home/u/.vimrc` — a user names the thing, not the path LiNix keys it by. The two
checks that do **not** carry over are OS-essential (no resource manager publishes such a list)
and undeclarable (no extras key parses as a package line, so applying it would refuse every
teardown forever).

```
Apply  RemoveOrphans  PurgeUnmanaged  Sync  Watch  Upgrade
Canary  Remove  ShellExit  ExpirySweep  Heal  Rebuild
```

**The scope is passed explicitly, never inferred** — every caller has to declare itself, so a new
deletion path cannot quietly inherit someone else's exemption, and a refusal can name what
refused in the words the user typed (`Remove` prints as `uninstall`, `Canary` as
`upgrade --canary`). `Sync`, `Rebuild`, `Upgrade` and the install paths also gate *installs* and
*changes*, so `max_installs`, `deny_packages` and `pinned_only` are reached from them too.

**And it is passed as the enum, never as its own label** (`S49`, V.154). A scope that is turned
into a string by one module and back into a scope by another is a scope with two vocabularies,
and the two do not have to agree — the firewall teardown's producer emitted
`"an unattended watch tick"` while its consumer matched `"watch"`, so both of that consumer's
named arms were unreachable and every teardown, including `N7`'s unattended tick, was guarded
and reported as `sync`. `GuardScope` is `Copy`. **The enum carries both vocabularies itself**:
`as_str` is the command to retype with a flag on it, `during` is what a refusal calls the run it
refused — each written out per variant, because the catch-all arm is what let one label answer
`sync` for nine of the twelve.

> **This sentence was false from the day the journal was written until 2026-07-23, and the
> eighth path is why the rule below exists.** `heal()` recovered an interrupted *install* by
> uninstalling the package first, before the plan, before the counts, before `-y` was
> consulted — and it ran `winget uninstall --silent Google.Chrome.EXE` on the owner's machine
> from a command whose argv was `install nimble:nimjson`. **The line is deleted, not guarded**
> (S24, V.64): the sentence is true again because the path is gone, which is the only repair
> that also covers the ninth path nobody has found yet. The enumeration above is still a list,
> and a list is still an assertion about what is absent — count the paths from the code.

**A recovery path may not remove** (owner ruling, 2026-07-23, S24). Anything that repairs,
retries, rolls back, or completes an interrupted operation reinstates what was wanted; it does
not delete to get there. `heal()` recovering an interrupted *install* re-runs the install — every
manager LiNix drives can install over a half-installed state — and **never uninstalls first.**

**A rollback compensates by putting back what was there, and it removes only what it knows it
added** (owner ruling, 2026-07-27, U41). The transaction records, per node and before the node
runs, whether the package was already installed and at what version.

- **An `Install` node whose package was already present is compensated by reinstalling the
  previous version, never by removing.** A `@version=` or `@channel=` change schedules an
  `Install` for a package that is already there, so removing it turns a failed upgrade into an
  uninstall.
- **An `Install` node whose package was genuinely absent is removed — through the guard.** These
  removals are issued at execution time and never pass the plan-time gate, so this is the only
  place they can be checked. A guard refusal stops that one removal; the package stays, the
  rollback reports itself incomplete and names it, and the transaction is left partly applied
  rather than a protected package deleted.
- **A prior state the manager could not report is `Unknown`, and `Unknown` is never read as
  absent.** The package stays and is named. Guessing the other way deletes software this run
  never installed.
- **A rolled-back removal comes back at the version it left at**, so a restored package does not
  silently lose its pin.

**Remove-before-install is a per-backend capability, off by default.** A manager that genuinely
cannot recover without it declares so, and when it is used **the removal is an ordinary removal**:
it reaches the guard, it is counted, it appears in the plan and in `--dry-run`, and its failure
is an error rather than a discarded result. There is no removal in this system that the plan
cannot show.

**The reason it is a default and not a guard call is that the guard already failed here.** S24
was a removal on a recovery path that reached no guard for thirteen sessions while a sentence in
this file said it did. Routing it through the guard would leave a delete on the path nobody
watches and trust the check to catch it — which is exactly the arrangement that broke. Removing
the delete removes the class.

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

**A rate-limited host is named, not waited for** (owner ruling, 2026-07-23, S26).
`rate_limit_max_wait_secs` in `preferences.toml` caps how long LiNix will wait out a rate limit —
**30 seconds by default**, one retry after it, then an error naming the host and the time the
limit resets. It is settable because the right answer differs between a laptop and a CI job; the
default is short because the old behaviour was to sleep up to an hour holding the data lock, and
a command that appears hung is the one people kill, which is what arms S24.

**`[guard]` holds three keys that are not among the ten: `confine_bin`** (default on), which
refuses a downloaded file a destination outside the backend's bin directory (SEC1),
**`require_signed_history`** (default off), which refuses a rollback to a commit git does not
vouch for (II.13), **and the list of commands that may not run unattended** (K13, ruled
2026-07-23), shipped as `rebuild` and `purge-undeclared` and edited by taking a name out. A
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
| `max_extra_removals` exceeded | **cannot skip.** `--allow-mass-removal` |
| `max_port_closures` exceeded | **cannot skip.** `--allow-mass-removal` |
| `max_installs` exceeded | **cannot skip.** `--allow-mass-install` |
| `max_total_changes` exceeded | **cannot skip.** either mass flag |
| hook script new or changed | **cannot skip.** `linix lock` |
| protected / OS-essential | **nothing overrides** |
| `purge-undeclared` | **cannot skip.** Typed confirmation |
| `pinned_only` / `require_snapshot` / `deny_vulnerable` | **cannot skip.** They are refusals (V.43) |

**The plan always leads with the counts** — not a threshold, not a warning, just the plan
being readable:

```
Plan: install 30,207 · remove 0 · upgrade 3
  30,102  re:^lib
      98  apt
       7  cargo
```

## II.11 `purge-undeclared`

**Two questions, two words** (2026-08-05, `Q31`). *Unmanaged* is what `adopt` would take:
installed, the manager attributes it to a choice, nothing declares it. *Undeclared* is every
installed package nothing declares, dependency closure and all — a strictly wider set, and the
one this command deletes. `check unmanaged` answers the first, `check drift` and this command
answer the second, and **no surface may use one word for the other set.** The verb is named
after what it deletes; it was `purge-unmanaged`, naming the set it does *not* act on.

**`sync` is additive; `purge-undeclared` is exclusive. This is the answer for every backend, and
no backend gets its own** (owner ruling, 2026-07-23, N1). A thing LiNix declared and then stopped
declaring is removed by `sync`, because the ledger knows LiNix put it there. A thing LiNix never
declared is left alone by `sync` and removed by `purge-undeclared` — packages, links, services,
firewall rules, and whatever the next backend manages. **A backend that wants an exclusive mode
of its own is asking for a second `purge-undeclared`**, which is the two-of-everything failure
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

- **A bare `linix rebuild` warns loudly, then rebuilds everything** (K2, owner ruling
  2026-07-24). It does **not** refuse. *This rule reversed on 2026-07-24 and this file carried
  the old one for two days after the code changed* — it read "Scope is required. A bare `linix
  rebuild` errors and names the three forms", which is what `app/rebuild.rs` used to `bail` with.
  The owner chose warn-and-proceed: the failure mode being guarded against is **software missing
  from a machine**, and a refusal makes the repair harder to reach while doing nothing about the
  scope. The warning is the safeguard the refusal was standing in for, and it names the narrower
  forms (`linix rebuild <pkg>`, `linix rebuild --backend <name>`) in the same breath. See V.49.
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
  That is what separates it from `purge-undeclared` (II.11).
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

**Three dialects, one language (V.150).** A hook's first line picks how it runs: a **shebang**
writes it to a file and executes it as a process, **`#rhai`** runs it in-process, and anything
else is **Lua**. All three stay (owner ruling 2026-07-20), and all three get the same things:

- **The same four facts** — `PKG_NAME`, `HOOK_TYPE`, `OS`, `ARCH`; a process gets them
  `LINIX_`-prefixed as environment variables.
- **The same standard library, for the two in-process arms.** A `#rhai` hook gets exactly what
  `vars.linix` gets (the clock, `sh`/`sh_ok`, `read_file`/`path_exists`, `env`/`has_env`,
  `http_get`, `parse_json`), because II.6b defines that file's trust *as* a hook's and a hook
  may not have less than the thing defined in its terms. Lua brings its own standard library.
- **The marker is consumed by whatever it selects.** `#rhai` is LiNix's word and never reaches
  the engine — it is not Rhai syntax, and leaving it in is a syntax error on line 1. A shebang
  is the script's own first instruction and is kept.

**LiNix reads the shebang; the kernel is not asked to (Y17).** The interpreter a `#!` line names
is looked up and put on the command line, on every platform — `python3 <script>`, not `<script>`.
Windows has no shebang mechanism at all, so a script handed to it directly fails whatever its
first line says, and every language a shebang names treats that line as a comment, so nothing has
to be rewritten to run this way.

- **An absolute interpreter that exists is used as written**, which on Unix is the same binary
  the kernel would have launched.
- **Otherwise the name is looked up on PATH**, because `/bin/bash` and `/usr/bin/python3` are Unix
  spellings and only the name travels. `/usr/bin/env` is dropped rather than launched: it *is* a
  PATH search, and that search now happens here.
- **A `python3` shebang finds a Windows `python`, then `py`.** The list is the same program under
  the name an OS gives it — never something merely similar.
- **A missing interpreter is refused by name, listing every spelling tried.** `#!/bin/bash` on a
  machine without bash cannot be made to work; it can be made to say so.
- **`env`'s environment assignments are refused, not half-honoured** — `exec:` runs through an
  executor with no per-command environment, and a form two of three callers support is worse than
  one none do.

**This is one answer for all three, and `exec:` and event hooks obey it too.** They ignored the
first line on both platforms before — `sh <script>` does not consult a shebang either — so a
`#!/usr/bin/env python3` event hook was broken on Linux as well as Windows.

**A `vars.<ext>` provider still chooses by extension, not by shebang (IX.6) — but it finds the
interpreter the same way.** `vars.py` named literally `python` on Windows and literally `python3`
elsewhere, so the one-spelling-per-platform assumption had been made twice, in opposite
directions, and each was wrong on the machine that had the other name.

**None of the three is sandboxed, and the ledger is why that is not a hole.** Lua loads
`os.execute`, a shebang is a process, Rhai has `sh`. Withholding a shell from one notation stops
nobody while the one beside it opens `#!`; what gates every hook is the approval below.

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

**A binary terminates its options when someone has run it and written down what it said.** Not
every CLI has a `--`, and a `--` a manager reads as a package name turns every install into a
failure — worse than the leading-dash refusal the grammar already made. So the default is *does
not terminate*, one table holds every binary with its answer, and each row carries either the
tool's own words or an admission that nobody asked. **The admissions are counted and the count
may fall, never rise.**

**Whether the terminator survives a version pin is read off the tokens, never off a label.** A
version that is an option (`-v 1.6`) is the one thing `--` cannot precede; a version that is an
operand (`1.6`) is protected by it like any other. Which one it is comes from the token — an
option starts with `-` — because a pin's *placement* and its *option-ness* are two facts, and
asking a backend author to declare the second is asking for the two to disagree.

**A validator with no caller is not a validator.** Every check the tree carries is called on the
path it names, or it is deleted. Two of everything is bad; one of everything, unwired, is worse
— it reads as a defence in the source and is absent at runtime.

**A backend that builds argv by hand is a backend that has to remember, and the record of what
it runs is checked against the table, not merely printed beside it** (2026-08-06, `Y11`,
V.142). The argv every backend produces is driven and recorded on every platform's CI; each
recorded invocation is now cross-checked against the terminator table, so an operand handed to
a program that ends its options at `--` without one fails the build. Recording an argv is not
checking it: `dnf install -y jq` sat in a green test directly beside `apt install -y -- jq` for
as long as both existed, and the two managers missing the hardening were the two that run as
root.

## II.12c What comes back from a command line (V.84, U40)

**LiNix reads the output of every command it runs.** stdout and stderr are captured on every
path, on every platform, whether or not a terminal is attached. Capture is a property of the
call; it is never decided by what LiNix's own handles happen to be, because a parser that works
in CI and not on a person's machine is worse than one that never works — only one of those two
gets reported.

**stdin is the one stream a child may share, only a mutating command may share it, and only
where `sudo` can be inserted** (2026-08-05, `Q35`). `sudo` asks for a password on the terminal it
was started from — and `sudo` is never inserted on Windows, so a Windows mutation gets a closed
stdin like a read. The reason is what carries the rule; where the reason does not reach, the
sharing buys nothing and costs the whole idle bound the first time a manager asks something
(V.130). A read has nothing to ask and nobody to
answer it, so a read never takes the terminal. **This binds every spawn in the tree, not only the
executor's** (2026-08-02): a probe that captures both output streams and leaves stdin inherited
asks its question where nobody can see it and then waits for an answer that cannot come. Ten
sites outside the executor did exactly that, `git` among them.

**Captured is not hidden.** While a mutation runs with a terminal attached, its output is
mirrored as it arrives — to stderr, never stdout, because stdout carries LiNix's own answer.

**Pagers are suppressed at the spawn.** `SYSTEMD_PAGER`, `PAGER` and `GIT_PAGER` are set on the
one environment map every spawn inherits, and every systemctl invocation carries `--no-pager`. A
pager waits for a keypress a captured child will never receive, and its escape sequences land in
the text a parser is about to read.

**There is no switch for any of this** — no config key, no environment variable, no flag. One
path. A switch here is a switch that turns the bug back on.

**A command that has gone silent is killed, and LiNix names it** (2026-08-02). The bound is on
**silence, not duration**: a child that has printed nothing on either stream and has not exited
for `command_idle_timeout_secs` is killed, and the error names the argv and the dial. A build
that prints for an hour is never touched. That distinction is the rule — no wall-clock cap can
separate a working `cargo install` from a wedged one, because there is no number above the first
and below the second, and what does separate them is that working commands say something. The
bound applies to **every** command, not only those inside the transaction DAG; the DAG's
`total_timeout` covers the DAG, and every hang on record happened outside it. The number is a
dial and `0` removes the bound; **that a bound exists is not.**

**The bound covers the read of the output, not only the wait for the exit** (2026-08-05, `Q32`).
A command whose direct child has exited while something outside LiNix's process tree still holds
its output pipe is bounded by the same clock, on the same silence: readers that have taken
nothing for `command_idle_timeout_secs` are abandoned and the command **fails by name**. A bound
that ends before the read ends is a bound a command can walk around, and the walk-around was
silent *and* returned the child's exit code — so the command reported **success** (V.131).

**A question about the whole machine is asked of each manager once, not of each package**
(2026-08-05, `Q44`). `list --outdated` asked every manager for one package's latest version at a
time — and `lookup` defaults to a whole `search`, so it ran one registry search per installed
package: **771.4s, against 2.9s for the `list` that fed it**. Nearly every manager answers the
entire question in one command, and where one does the manager's verdict stands — LiNix does not
re-compare versions it was already told about, because `> 3.13.5` is a version winget really
prints. A manager with no such verb keeps the per-package path, **concurrently**. Measured after:
25.6s. **Not knowing and finding nothing stay different answers**, here as everywhere.

**Where a manager offers a machine-readable listing, LiNix asks for it — and negotiates**
(2026-08-05, `Q43`). pixi, dotnet and scoop each print a listing drawn for a human and will hand
over JSON on request, but the flag that asks for it arrived in some version of the tool and LiNix
does not choose which one is installed. So it asks, and a manager that refuses is read from its
text listing instead, once per run. **Never assume the flag.** An unsupported flag fails with a
usage message, which every reader here hands back as an empty result — so assuming it would
report an empty machine to exactly the users on older tooling, which is `Q40` under a new name.
Each machine-format parser is its own function, never the text parser made lenient: one parser
that accepts two shapes is how a malformed answer in one is silently read as the other.

**A read has its own bound** (2026-08-05, `Q42`). `query_idle_timeout_secs`, default 120, `0`
disables, never longer than the bound above. The number above is sized for `Checkpoint-Computer`,
a mutation legitimately silent for its whole run; a read takes seconds — `winget list` 1.5s here,
2.6s under sixteen-way contention — and a question that has said nothing for two minutes is not
about to answer. One number for both jobs meant a wedged listing cost fifteen minutes to learn
what two could have told you.

**A read that exits non-zero having said nothing at all has failed; it has not answered "nothing"**
(2026-08-05, `Q40`). A non-zero exit alone is not the verdict — "no such package" and "no
results" are ordinary non-zero replies that arrive with their reason on the page, so a read that
*printed* keeps what it printed — on either stream, since a complaint is something said, and
`Get-ComputerRestorePoint` exiting 1 with `Access denied` is for its caller to weigh. But silence
on both streams beside a failure is the one case that cannot be an
answer: a manager with nothing to report says so by exiting 0, or by printing a header. Returning
an empty listing there made `linix list --backend winget` print nothing and exit 0 on a machine
with 280 packages, and made `info` report an installed package as absent. **Absence and
unavailability are different answers, and no caller may collapse them** — not `list`, which names
the manager it could not reach and marks the listing partial; not `info`, which fails rather than
claim a package is not installed; not a hook, which says it recorded nothing and why.

**Retryability is classified from the exit code as well as from the output** (2026-08-05, `Q41`),
and **a read classified transient is retried** — `read_retry_attempts`, default 3. What a manager
says outranks what it returns, so the code is consulted only where the text classified nothing;
that is precisely the silent failure, whose haystack is empty by definition. Retry is for reads
and only reads: a read is idempotent, and a mutation retried on a guess installs twice.

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

> **The three rules below are BUILT, and the hardcoded `Vec` is gone (U27 Option A — session
> 2026-07-26).** There is no hardcoded provider list any more: the built-in providers (btrfs,
> timeshift, apfs, windows-restore, then zfs) are rows in `src/core/snapshot_builtins.toml`,
> compiled in and read through the same `ConfigSnapshotProvider` loader a user's
> `adapters/snapshot.toml` row goes through — so the mechanism is proven by the providers that
> ship (K17/U1). The built-in file is *not* hook-ledger-gated (a first-party compiled-in asset,
> and gating it would leave a fresh machine with no safety net until `linix lock` ran); the user's
> file still is, and registers last. lvm is the exemplar *user* row (no universal origin volume),
> not a shipped built-in. Windows is the one row beyond plain argv — `powershell = true` with the
> id typed as a `u32` (V.82). The live restore (zfs/lvm/btrfs) is still the only part exercised on
> hardware, not here.

**Snapshot providers are declarable (U27).** A provider is a row in an `adapters/` file — the
take/list/delete/restore argv as data — read through the same loader and hook ledger as a custom
backend, and the built-in providers are rows in it too, not a hardcoded list. **The row must
declare whether it can restore a running machine; the field is required and never inferred** — a
provider that does not declare live-restore is create-only and refuses the rollback (V.60). A
custom provider registers last and never shadows a built-in.

**The built-in providers use the same door (U27, Option A, owner 2026-07-26).** btrfs, zfs,
timeshift and lvm are argv rows in `adapters/snapshot.toml`, not a hardcoded `Vec`. **Windows
System Restore is a row too, but its id and label are typed placeholders, never free text** — the
loader substitutes the restore-point id only as a validated `u32` and the label only as the fixed
`SnapshotLabel` enum, so a declared Windows row can no more reach the elevated PowerShell with a
quote than the hardcoded path could (SEC5). A snapshot technology whose interface is typed cmdlets
is expressed as typed slots, not as a shell string; that is the one shape a snapshot row may carry
beyond plain argv, and V.82 says why.

**When several providers are available, a declared priority list picks the active one (U28)** —
the `priority` shape, an ordered list of names with a shipped default the user overrides, first
available in the list wins. The list chooses which provider; V.60 still governs what LiNix
promises about it, and the pre-change notice states which kind of snapshot this machine takes, so
a create-only provider chosen first is a visible choice, not a silent weaker net. One active
provider; "snapshot with all, restore from the best" is a later question.

**macOS gets APFS as its provider (U29)** — `tmutil localsnapshot` / `diskutil apfs`, declared
**create-only** because an APFS restore needs a reboot into recovery (not a live undo, V.60). So
macOS is no longer without a net; it has a create-only one, marked as such (U6).

**Init systems are declarable the same way (U36).** `service:` speaks systemd, OpenRC, SysVinit,
launchd and Windows `sc` as rows in `init_providers.toml`, and a machine running s6/dinit/runit/
Shepherd adds a row to `adapters/init.toml` — through the same loader and ledger, registered
last, never shadowing a built-in. A row that cannot both start and stop is refused, not
half-used (V.73). This is XIII.33's one mechanism — a name, some argv, a way to read the result —
applied to the init surface rather than a new plugin system.

**Storage objects are one family, and destroying one goes through the normal guard (U30).**
`zfs:tank/data` and `lvm:vg0/data` join `btrfs:` as declared, sized, mounted objects — Rust, not
a `ManagerConfig`, because a volume has a size and a mountpoint, not a version. Because they are
ordinary backends, a deleted line becomes drift becomes a removal through the same guard as any
package: a volume is protectable (`[guard] protected_packages`), it counts against `max_removals`,
and `zfs destroy` / `lvremove` is previewed before the guard clears it (V.80). No special
escalation — normal is already the strongest gate there is.

**And a volume can be written with the size and the mountpoint this paragraph says it has**
(Q18, ruled 2026-07-31). `@size` on `lvm:`, `@quota` and `@mount` on `btrfs:` and `zfs:`,
`@mount_options` on `btrfs:` — see II.2's option table. Until that ruling this paragraph
described a declaration nobody could write: the option table permitted none of them, so
`lvm:vg0/data` was refused with a size and refused without one. A declared mount is recorded in
fstab, which is what makes it survive a reboot, and it is taken out of fstab **before** the
volume is destroyed — an entry that outlives its subvolume stops the next boot in the initramfs
(V.106).

**A declared geometry converges, and the one direction that can destroy a filesystem is written
on the line** (Q19, ruled 2026-07-31). An edited `@quota`, `@size`, `@mount` or `@mount_options`
is drift like any other: `sync` re-applies the quota, rewrites the fstab entry, and resizes the
volume. **`lvm:` grows with `lvextend --resizefs`; it shrinks only where the line carries
`@allow_shrink=true`, and refuses otherwise with both sizes named.** `--resizefs` in both
directions is the rule and not an implementation detail — it shrinks the filesystem before the
volume, so the flag permits a *resize* rather than a truncation, and a filesystem that cannot
shrink fails before the volume is touched (V.107). **`@allow_shrink` without `@size` is a parse
error**, the same one level in `@mount_options` gets.

**Every facet is compared, and comparison is by value in bytes.** The tools are asked for raw
byte counts (`zfs list -p`, `lvs --units b --nosuffix`, `btrfs qgroup show --raw`) so only the
declared side is ever parsed; and a backend reports three states, never two — a byte count, `none`
where it looked and found no limit, and **nothing at all where it could not look**, which is left
alone (D13). A geometry facet that is satisfied does not stop the others being checked: a line
carrying `@mount` and `@quota` has both read. **Dropping an option stops declaring it, and does
not lift what it declared** — deleting a word from a config file must not silently uncap a
filesystem.

**Secret decryption opens to declared providers, last and most carefully (U38).** `age` and
`sops` stay built in; any other decrypt tool (Vault, 1Password, a KMS, GPG) is a `[[secret]]`
row in `adapters/secret.toml` — a name and the argv that puts plaintext on stdout. It plugs into
the existing decrypt path, so it inherits the T-series rules unchanged (restrict-before-write T5,
no-backup T1, never-into-the-repo T2, the touch timeout T3). **A provider that does not declare
`stdout_only = true` is refused, not trusted** (V.81): LiNix will not hand a secret to a command
that has not promised to keep it off disk and out of the logs. **A `[[secret]]` row may name an
`os`, like every other adapter row** — it could not, and it was the one table whose rows are
handed a plaintext secret (`Y13`, V.145). A row that names none applies everywhere.

**A restore that cannot restore says so, before it is needed (V.60).** Taking a snapshot and
restoring one are different capabilities and a provider may have the first without the second:
`btrfs subvolume snapshot SRC /` does not roll back a mounted root, whatever its exit code says.
So **a provider that cannot perform a live restore must refuse the restore**, and `doctor` and
the pre-change notice must say which kind of snapshot this machine takes. **No command prints
"rolled back" on the strength of an exit code** — the sentence is a claim about the machine, and
it is the one sentence a user cannot check for themselves at the moment they read it. There is
**one restore implementation**, not one in the provider and another in `snapshot restore`.

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
| `linix shim jq --source cargo:jq` (**`--source` discarded unread**) | `shim:jq@source=cargo:jq` — and the option is read: the shim provisions and runs *that* provider |
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
`uninstall`) · `clean` (split in two: `remove-orphans` for what the machine no longer needs,
`clean-cache` for the downloads it kept — V.36) · `status` · `doctor` · `unmanaged` · `absent` ·
`conflicts` · `audit` (all six → `check <section>`, ruled 2026-07-24) · `undo` (→ `snapshot
restore` for the filesystem, `rollback` for the manifests) · `shim` (→ the line
`shim:jq@source=cargo:jq` — II.16, ruled 2026-08-09 under `Y18`; `--source` was discarded by the
command and `@source=` is read by the line)

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

## II.19 What LiNix does at once

*(Owner ruling, 2026-08-02 — `Y1`–`Y4`. LiNix's entire runtime is spent waiting on other
people's processes and other people's networks, so what it does at once is a rule about the
product and not a tuning detail.)*

**Nothing is built for a command that will not use it.** Registration runs for every subcommand,
so anything a backend constructs in its constructor is paid for by `linix path` as much as by
`linix sync` — and one rate limiter's clock cost **200 ms of a 210 ms fixed startup**, on every
invocation, for a GitHub API budget an offline run never spends. An expensive object is built
where it is *used*, not where it is declared. Registering all 48 backends is budgeted at
**120 ms** by `tests/startup_budget_tests.rs`, which is the only thing that measures the part of
a run that spawns no child (V.126).

**And what does not yield is polled last.** `try_join!` over four startup futures gives the
longest instead of the sum only if the ones that hand work to another thread get polled at all;
an `async fn` with no `.await` in it holds every future after it in the tuple until it returns.

**One command per manager per wave, not one per package.** Every install and every removal that
is ready at the same moment, for the same manager, with no `@requires` edge between them, goes on
**one** command line. A `@requires` edge splits the wave; an install and a removal are two
commands; the line is bounded so it fits (100 names, 6000 bytes). Rollback granularity is
unchanged: what each package looked like before is captured per package, and a batch that fails
fails every package in it — which is what a single node failure already meant, since any failure
rolls the transaction back. **V.115.**

**The telemetry says when packages shared a command.** Several packages reporting the same
duration to the millisecond is the truth about a batch and was a lie about a serialised run; the
line says which. **V.115.**

**Recovery finishes interrupted work, and runs on the same engine as everything else**
(2026-08-05, `Q33`). `heal` completes what a run that died left half-done — `InProgress` and
`Abandoned`. A **failed** attempt is not interrupted: it reached an outcome and reported it, the
package is not installed, and its declaration is still in the manifest, so the next `sync`
schedules it again. Retrying it in recovery was the same work twice. And recovery is a graph
like any other change: batched per manager, run in parallel, with the dependency edges the
journal's own specs carry. It differs from a `sync` in exactly two ways, and both follow from
what it is — it does not roll back, and one entry nobody can finish does not leave the others
unfinished. **V.135.**

**The log records what cannot be recomputed, and nothing else** (2026-08-06, `Y10`). A package,
an `exec:` script, an `@undo=` command — each written and flushed *before* the process starts.
Every other mutation a sync makes is a converge from a declaration: a `service:`, a `setting:`,
a `firewall:` rule, a placed `link:`. Killed halfway, the next sync reads the machine, sees the
line unmet and finishes the job, which is a **better** recovery than replaying a log because it
also corrects drift the log never saw. A variant for one of those would be durability theatre,
and adding one has to argue with this paragraph first.

**And the record is written wherever the mutation is issued, not only where the engine issues
it** (2026-08-06, `Y14`). Whether a package mutation is recoverable is a property of the
mutation, never of the verb that reached it: `apply`, `upgrade`, `remove-orphans`,
`purge-undeclared`, an expiring lease, a suspension restored on shell exit and `run`'s
auto-provision all reach a package manager, and each writes its entry and flushes it before the
manager is invoked. A write that fails aborts the mutation rather than letting it run
unrecorded. **A command that installs or removes and records nothing is a build failure** —
`tests/wal_enumeration_tests.rs` counts the mutation sites in `src/` on every run and requires
each to name what recovers it, so the sentence above stays a fact rather than a claim somebody
last checked. Nine paths reached a manager with no record at all while one review sentence
described one of them. **V.147.**

**Recovery replays a package and reports a script.** Reaching a state twice is reaching it once,
so an interrupted install is finished by installing. A script that got half way has no recorded
progress and no declared end state, so re-running it repeats the half that already ran: `heal`
names it, says the next sync will run it again from the top, and resolves the entry as **failed**
— not completed, because it did not complete, and not left open, because an entry that can never
be recovered but stays `InProgress` keeps `needs_recovery` true for ever. **V.140.**

**A failure names the declaration it happened for** (2026-08-05, `Q34`). Not the manager, not
the command — the `backend:name` and the file and line it was written on. `install X` converges
the whole configuration, which is the model working and stays; the consequence is that a line
nobody has looked at can stop the install someone just typed, and until this the error was that
line's manager talking about a command the user never asked for. `install` also says outright
when the failure is not about what was asked for, and never advises taking back the line that
was: the one thing worse than keeping a wedged line is deleting a good one. **V.136.**

**Nothing is fetched for a decision already made** (2026-08-05, `Q37`). A refusal that reads only
the destination is asked before the first byte, not after the artifact is on disk and unpacked.
`github:`, `web:` and `appimage:` all deploy onto the same shared bin directory and all ask the
same question through the same function. Where the answer genuinely needs the archive open — a
release whose several artifacts are each named after the program inside them — the download is
the price of the answer and is paid. **V.132.**

**A resource already in the state it declares is not work** (2026-08-05, `Q39`). Before applying
a resource, LiNix asks the machine whether it is already in effect, and a resource that is gets
skipped — whatever the applied-resources ledger remembers, because the ledger records what LiNix
placed and the question is what is *true*. What cannot be asked is applied, and named as
unverifiable rather than assumed converged. And where the machine answers by exit code instead —
`sc start` on a running service returns 1056 — that code is success for the verb that asked and
a failure for every other verb. **V.133.**

**Two knobs, because processes and sockets are not the same thing.** `max_parallel` bounds
concurrent **processes** and defaults to the core count. `network_parallel` bounds concurrent
**network requests** and defaults to 16, whatever the core count. Nothing that fans out reads a
number that is not one of these two. **V.116.**

**Every wait states its bound, or it has none on purpose.** A `search` backend that has not
answered within twice the configured network timeout (floor 30s) contributes nothing and says
so. A `@health=` port probe gives up after 5s. A download has no whole-request timeout, because
a release asset can legitimately take an hour. **V.117.**

**`upgrade` serialises what shares a package database and overlaps the rest.** The managers that
`needs_root()` run one at a time; `cargo`, `npm`, `pipx`, `uv`, `yarn`, `pnpm`, `vscode`,
`emacs`, `krew` and `go` contend with nothing and run together. **V.116.**

**The pre-sync restore point starts first and is joined last.** It is a safety net rather than a
precondition, so it runs alongside the read-only pre-flight and is joined immediately before the
first mutating command — never after it. A refused sync stops it. **It announces itself**,
because a silent fifty-second pause reads as a hang. **V.118.**

**A manager is asked what it has installed once per run.** The answer cannot change while
nothing is being installed; a mutating command is what forgets it. **V.115.**

**Variables resolve exactly once per invocation**, which II.6b has always said and which is now
what the code does. A vars provider is a program the user wrote, and running it three times runs
its side effects three times. **V.116.**

**A manager's answer may outlive its run, only if the machine's owner says so.**
`installed_cache_secs` is `0` — never — until set. When set, a listing is reused for that many
seconds; **any mutation drops it, in memory and on disk**, `--no-cache` bypasses it for one run,
and `clean-cache` forgets it outright. Every read failure is a cache miss and never an error: a
cache that cannot be read must not report an empty machine. **V.120.**

**And it answers a command that reports, never one that writes its answer down.** `list`,
`search`, `check`, `outdated`, `info`, `why` — those and no others. A stale reading is corrected
by the next run; a plan, an adoption or a saved plan built on one is a mistake with a life of its
own. An allowlist, so a command added later has to say it is a reader. **V.120a.**

**Every manager a run will ask is asked at once, not when a section gets to it.** A command that
crawls the whole machine warms every manager's listing before its first section runs; a plan asks
each manager it consults once, before it asks anything about a package. Neither adds a question —
the once-per-run memo already collapsed the duplicates — so what changes is when. A command that
consults three managers still wakes three. **V.122.**

**The registry comes out in an order a reader can predict.** Every walk of it — `available()`,
`all()`, and so every listing, every health table and every fan-out's first slots — is in the
same sequence on every run. A listing you cannot compare to yesterday's is not a listing.
**V.123.**

**A name a manager reports is a name that can be declared, and one with a space is quoted.**
`winget:"ARP\Machine\X64\Mozilla Firefox"`. The quotes are syntax and the name is what is inside
them; an `@` within them belongs to the name, and the options still open at the first `@` after
the closing quote. Prose is still not a package name, which is what quoting protects. One
function decides both whether a name can be written and how it is spelled, so a check and a
writer can never disagree. **V.121.**

**A running service is adopted as a line, and as the state it was found in.**
`service:AppMgmt@status=running`, uncommented, alongside the packages. The line carries only what
was observed: an init reports which services are **running**, never which you chose or how they
start, so no adopted line declares a start type — a bare `service:` line means *enable and start*,
and enabling rewrites the start type. The manifest header says what deleting a line does in the
words it does it in: a package is uninstalled, a service is stopped and disabled. **V.124.**

**A resource nobody declared is not something `purge-undeclared` may sweep.** That command deletes
installed packages the model does not name; a `service:`, `link:` or `setting:` is a state rather
than an artifact, and turning off a service you never named is not the promise it made. Refused by
that rule, asked of the backend and not of the name — and never again by a test about package
lines that happened to return the right answer. **V.124.**

**A user can ask where the time went, and the answer names the managers.** `--timings` reports
the wall clock, the summed child time and the ratio between them, then every command LiNix ran,
slowest first. It is off unless asked for and goes to stderr, so nothing parsing stdout sees it.
The ratio is part of the rule and not decoration: it is the only place the rest of II.19's
claims about overlapping are stated as a number the user can check on their own machine.
**V.119.**

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


## II.20 Legibility (V.128)

**A command that reports success while leaving the user with a false picture of their machine
has failed.** Not crashed, not exited non-zero — failed. This is a defect class with the same
standing as fail-loud, and it is measured by one question: *did the person understand their
machine more accurately after the command than before?*

Three rules carry it.

**1. "Nothing to do" is a claim about the world, and has to be earned.** `already up to date`,
`the machine matches your files`, `no changes` — these are the most confident sentences the tool
says, and silence is the only output nobody writes a test for. A path that drops something from
consideration appends to a reported list; **an empty plan with a non-empty skip list is not
convergence.** `Declined::reported` in `app/sync/planner.rs` is this rule for removals — the type
exists so "does the user hear about this?" cannot be answered by omission, and a variant added
later does not compile until it says its sentence. Every reporting path owes the same.

**2. Every mutation states where, not just what.** `created` and `kept` are true about a file and
say nothing about which directory received it. A path a command acted on is part of what the
command did.

**3. Absence is reported like presence.** Skipped, not found, declined, and not applicable are
output. A `--json` consumer that can only see the actions cannot tell a converged machine from
one holding a package nothing will ever remove.

The error messages already meet this standard — file, line, what is wrong, what to do, and what
the concept means. **This rule is that standard applied to success, to absence, and to history.**

---

## II.21 A command LiNix names is a command LiNix has (`F-2`, built 2026-08-05, never ruled)

**Every `linix <verb>` written anywhere a person reads or a machine runs names a live path
through the CLI.** Source strings, doc comments, backend data tables, the install scripts, the
container harnesses, the examples, and `readme.md`. Checked on every run against clap's own
command tree — names, aliases and nested subcommands — by
`tests/named_commands_exist_tests.rs`.

**The convention that makes it checkable: prose calls the product `LiNix`.** A lowercase `linix`
that opens a line or follows a quote, a backtick or a shell operator is an invocation and is
checked; `the LiNix binary` is a sentence and is not. Without that distinction the gate needs a
list of English words to ignore, and a list of words to ignore is one more thing that rots.

**`readme.md`'s verb tables are checked too**, by the same tree — a table whose first column is
backticked command paths is found by *what it contains*, not by where it sits, and the number of
such tables is pinned so one cannot rot past recognition and leave the gate in silence.

**`docs/` is checked against the Deleted register, not against the live surface.** It is a record
— a changelog, a bug tracker and a decision register — and a record has to be free to write
`linix doctor` when it is describing the day `linix doctor` was deleted. So the property there is
weaker by one clause: a dead command named in `docs/` must be one II.17 records as dead. A name
that is neither live nor registered is the finding. `readme.md` keeps the strict rule, because it
is the one document a user reads as instructions.

**The rule this generalises: a gate is drawn around the property, not around the artifact that
was under review.** See `why.md` for the six live defects that were sitting outside three
working gates when this was written.

## II.22 One path per backend, and a capability the machinery lacks is a field (`Y11`, V.142)

**A backend is a `ManagerConfig` row, or it is a module named in
`tests/backend_is_data_not_code_tests.rs` with what the generic machinery cannot express — and
with the line in that module which makes the claim true.** A reason is a claim (E29), and a
claim is checked against the code it describes. Length is not evidence: three exemptions
described code that was not in the module they excused, and the only assertion on them was that
they ran past sixty characters.

**Where a conversion is blocked, the machinery gains a field. It never gains a compromise.**
Eleven backends have come off the hand-written path this way, and every one of them cost a
field that is now available to all sixty-two: `extra_probes`, `upgrade_reinstall_args`,
`property_probes`, `SearchSource`, `VersionPin`, `CacheClean`, `DependsProbe`,
`OutdatedProbe::silence_is_none`, and the `{name_component}` placeholder. That is the
difference between converting a backend and deleting one.

**Two paths for one job is one path where a property is enforced and one where it is
remembered, and the second one loses.** It lost in both directions before this rule existed:

- the hand-written path lost the `--` terminator, on `dnf` and `pacman` — the two managers that
  run as root, and the only two backends in the tree that built install and remove argv without
  `core::argv::push_names`;
- the data path lost `clean_cache` entirely, because `ManagerConfig` had no field for it. All
  forty rows answered `Unsupported`, and `linix clean-cache` on a Debian machine printed *"No
  backend on this machine has a cache to clear"* over a full `/var/cache/apt/archives`;
- the data path also lost the exclusive lock, keying it on the *program* rather than the
  manager — so OpenBSD's `pkg_add` and `pkg_delete` took two different locks over one package
  database. Every hand-written module had keyed on the manager all along.

**A manager's exclusive lock names the manager, never the program.** A manager with two
binaries has one database and two names for it.

**A repository name that becomes a path segment is validated as one.** `{name}` is an argument
the manager parses; `{name_component}` is a segment LiNix builds a path out of, and the
difference is that `../../etc/cron.d/x` is an ordinary argument and a directory escape. Both
managers that put a name in a path validated it by hand; the shared path did not, because until
they became rows no row did.

**Every verb that builds argv is driven by the drift gate, not only the ones it was written
for.** `clean_cache` and `list_orphans` produce argv from the same rows `install` does, and
neither was walked against a manager's own `--help` until 2026-08-06 — a subcommand only one
verb reaches is a subcommand upstream can delete unnoticed.

## II.23 A gate is not a gate until it has been watched to fail (`S48`, V.153)

**A check that scans the source carries an oracle, and the oracle drives the *same predicate the
check drives* — over an input it must catch and over every input it must not.** Not a
restatement of the predicate in a string literal beside it; not an assertion that a sample line
contains the substring the scan looks for. The question an oracle answers is *does the scan
still see*, and only running the scan answers it.

**So the predicate is a named function, not a block inside the walk.** A rule spelled as four
`continue`s inside a `for` loop can be checked only by reading it, and reading it is what every
one of these gates was already relying on. Three of them were nested one layer deeper still —
inside the test function — where nothing outside could reach them at all.

**The controls are the shapes that look like a finding and are not**, each one written because a
scan got it wrong: a commented-out arm, an empty body, the last alternative of an or-pattern, a
`stderr` read where the rule is about `stdout`, a definition line where the rule is about calls,
a wrap that sits *below* the call rather than above it, and the exemption list itself.

**And a scan that reaches nothing reports nothing, which reads exactly like a clean tree.** So a
source scan asserts a floor on what it visited — files walked, sites matched, ledgers found —
and the floor fails before the emptiness can be read as health.

**Ratchets are bidirectional.** A count that may only shrink is asserted `<= n` *and* `>= n`, so
slack that was lowered and never re-pinned is as red as growth.

## II.24 A command says whether it writes; a list beside the enum does not (`S50`, V.155)

**Whether a run takes the exclusive lock on `data/` is answered by `Commands::writes()`, an
exhaustive match on the enum itself.** Not by a list of names sitting near it. A new subcommand
does not compile until it says which it is, which is the strongest form of "locked by default"
available — a runtime default that treats an unrecognised name as a writer is a guess, and it is
a guess nobody is ever prompted to revisit.

**The lock is over the DATA directory, and that is the whole of the question.** `path --set`,
`config init` and `edit` write into the CONFIG repo through `utils::file::persist`, which is
atomic; they are readers here. `edit` is the reason the distinction is worth keeping: it blocks
on `$EDITOR`, and locking it once stopped every other LiNix on the machine for as long as
somebody read a manifest in vim (AU6).

**`--dry-run` never exempts anything** (S25). A preview of a writer reads the state a concurrent
writer is rewriting.

**A command can be a reader that contains a writer, and the lock is taken where the writing
starts.** `history` opens a browser a person reads at their own pace; its rollback action
reaches the whole install/remove path. Locking the command would be the `$EDITOR` mistake;
leaving the action unlocked was the live one. So the exemption is at the command and the
acquisition is at the mutation, and neither is inferred from the other.

**A user verb takes the lock when any of its steps writes** — not when the first does, and not
unconditionally. One lock spans the whole verb, because two steps that have to agree about the
same registry must not have it released between them.

## II.25 A query answers about the machine; the catalogue answers a different question (`S53`, V.156)

**`Queryable::info` means *is this installed here, and at what version*.** `Some` is present;
`None` is absent; `Err` is *could not ask*, which is a third answer and never a synonym for the
second. `Searchable` is where *could this be installed* is asked, and nothing else may answer it
in `info`'s place.

**A backend whose query tool answers from a catalogue must find the installed record inside that
answer.** `brew info`, `snap info` and a marketplace endpoint all describe things that are merely
publishable. Each of those reports carries the local fact somewhere — `installed: []`, the
`installed:` line, the local CLI's own listing — and that is the part `info` is about. **The
version comes from the installed record too, never from `latest` or `stable`**: a drift check
compares the declaration against what is on disk, and comparing it against upstream's newest
build re-installs for ever the moment upstream publishes.

**The name a listing returns is the name an install was given.** State keyed by a URL is listed
by that URL; a subvolume created at `/mnt/data/vol` is listed as `/mnt/data/vol`. A name `list`
does not return is a package `sync` believes is absent, and it re-creates it on every run.

## II.26 A `--json` flag buys the shape of the answer, not the whole stream (`S51`, V.157)

**Manager output is read for the document inside it, never parsed from byte zero.** One reader,
`parsers::json_document`. It finds the first document and stops at the end of it, so a banner
above and a summary below are both survivable — and both are real: composer prints
`Changed current directory to …` ahead of every global command, and it is not the only one.

**A syntax error is not an empty machine.** A reader that answers `unwrap_or_default()` to
unparseable bytes reports nothing installed, which plans every declaration as a fresh install and
drops every removal — the `LX-1` failure again, one layer up. `json_document` returns `Option`
precisely so the caller has to say which of the two it means.

## II.27 A structured answer is judged by its container, not by its line count (`S54`, V.158)

**Every `--json` reader answers through `parsers::or_unrecognised_json`, and hands it the
container it went looking for:**

- **`Some(0)`** — the shape was found and holds nothing. That is a machine with none of these,
  which is true and common. `Ok(vec![])`.
- **`Some(n)`, nothing read** — `n` entries the reader could not get a name out of. A schema
  change. Refuse.
- **`None`** — the shape was not there at all, which includes the output not being a document.
  A schema change. Refuse.

**`None` and `Some(0)` are different answers and must be spelled differently.** A reader that
reports "no entries" when the key it wanted was renamed has turned a format change into an empty
machine, and `sync` answers an empty machine by installing every declaration and dropping every
removal.

**`or_unrecognised` is for text listings only.** Counting lines of a document asks a question
about a shape that has no lines, and the arm added to paper over that made the answer
unconditionally `Ok` for anything containing parseable JSON — which disabled `LX-1` for every
backend that reached it.

**One implementation, and it is the shared one.** Five files carried a private `unreadable`
helper building the same `Unrecognised` by hand, and seven sites carried the
`found.is_empty() && !container.is_empty()` literal. The correct rule living in six copies while
the shared helper held the weak one is how the five backends using the shared helper became the
unprotected ones.

## II.28 A ceiling is a budget for the command: one per kind, and one over all of them (`S55`, `Y20`, `N8`, V.159)

**One value per command counts what it has changed, and the `enforce*` family is the only thing
that reads or writes it.** Not a `usize` each caller assembles: a sync changes things in six
places, and a number three callers compute is a number one of them computes wrong.

**A count per kind, a ceiling per kind** (`Y20`, `N8`):

- **`max_removals`** (default 20) — packages. Software leaving the machine.
- **`max_extra_removals`** (default 20) — every resource teardown: `link:`, `service:`,
  `setting:`, `shim:`, `schedule:`, `repo:`, and a `dotfiles:` tree's files.
- **`max_port_closures`** (default 20) — a port closed because no `firewall:` line declares it.
  Reachability is its own axis: the run that first declares a perimeter closes more ports than a
  settled machine ever tears down resources, and it must not spend a teardown allowance to do it.

**And one ceiling over everything the command changes** (`N8`):

- **`max_total_changes`** (default **0**, off) — installs and upgrades, every removal of every
  kind, resources written, ports opened and closed. Nineteen packages, nineteen resources and
  nineteen ports pass all three per-kind ceilings and are fifty-seven changes; this is the number
  that objects. **Off by default**, because a machine that has not asked for a total must not
  start refusing the sync it ran yesterday.

**No kind spends another kind's budget**, and every budget is spent across the whole command, not
per phase — the failure that made this a rule was two phases each passing a limit the run
exceeded once.

**Every gate answers the total, including the ones that only add.** Installs go through
`enforce_installs`; resources placed and ports opened go through `enforce_additions`, which has
no ceiling of its own and exists so that a total is a total.

**A refusal names every ceiling it hit.** `[guard] max_removals` printed over a port closure
sends the reader to the wrong line; naming one of two sends them to raise a number and meet the
other on the next run.

**A refused set is not recorded.** A removal that was never allowed must not raise the total the
next phase is measured against.

**`--allow-mass-removal` answers every removal count and the total; `--allow-mass-install`
answers the install count and the total, and no removal count.** Both say "yes, that many, I
meant it", and a total is made of both — but the flag that means *install* that many must never
also mean *remove* that many. Protection remains a refusal (V.26): nothing overrides it, on any
kind.

## II.29 A kind is a type, and every dispatch over it is exhaustive (`S56`, V.160)

**`ResourceKind` is what a declaration declares.** `Statement::kind()` returns it; the extras
ledger's keys parse back into it; `Display` and `FromStr` are the only conversions to and from
the keyword a user writes.

**No dispatch over a kind may have a catch-all.** Not `_ =>`, not `other =>`. A keyword added to
the grammar must not compile until every path that acts on kinds says what it means — which
paths those are is the compiler's business, not a reviewer's.

**Where a kind genuinely has no work, the arm says so and says why.** An arm reading
`K::Firewall => Ok(())` beside a sentence explaining that the perimeter is reconciled as a whole
is a decision. The same `Ok(())` reached through `other =>` is an accident, and the two are
indistinguishable at the call site — which is the entire failure this rule prevents:

- **In a teardown, `Ok(())` means *done*.** The caller drops the key from the ledger, so a
  resource nobody knew how to remove is forgotten while still in effect, and no later sync looks
  at it again. The warning that accompanied it was below the default log filter.
- **In a probe, `None` means *unverifiable*, and unverifiable places.** A kind that falls through
  is re-applied on every sync, for ever.

**A ledger key whose kind this build does not have is kept, not dropped.** It is left in place,
reported, and re-offered next run. Forgetting a row is the one outcome that cannot be undone.

## II.30 `<kind>:<subject>` has one producer and one reader, and it is a type (`S57`, V.161)

**`ExtraKey { kind: ResourceKind, subject: String }` is the extras-ledger key.** `Display` writes
it, `FromStr` reads it, and the ledger on disk is a set of exactly those strings. Nothing else
formats one and nothing else splits one.

**The split is at the FIRST colon and the subject is not split again.** A `repo:` subject is
itself `backend:spec`, and that inner structure belongs to the repo backend. One type splitting
one string twice is how the second reader gets it wrong.

**`Statement::key()` is the display form of a line, not a wire format.** It produces two key
spaces — `backend:name` for a package, `kind:subject` for a keyword — and its type does not say
which. A reader that splits it on `:` and trusts the prefix reads `apt:jq` as the kind `apt`.
The extras ledger therefore builds its key from `kind()` and `subject()`, not from `key()`: that
the two agree today is true, and is not a promise anybody made.

**And the package half of that hazard already has its one parser** — the grammar
(`config/grammar/`, reached through `split_removal_target`). Anything that splits a package spec
on `:` by hand is a bug, including a `rsplit` that takes the tail: `web:https://x/y.deb` has
three colons and the last one is inside the URL.

**A row this build cannot parse is kept, reported, and re-offered.** The ledger deserialises as
strings and parses per row where it is used, so one unreadable row does not fail the file — and
the guard still counts and protects it, because only its *kind* is unknown and the guard does
not dispatch on kind.

## II.31 A capability the config describes is a capability the registry hands out (`S58`, V.162)

**If a manager's `ManagerConfig` says how to do something, its `BackendCapabilities` says it can
do it.** A config carrying `upgrade_args` and a builder without `.with_upgradable(…)` is a
manager that silently sits out `linix upgrade`: `as_upgradable()` answers `None`, and every
caller reads that as *this manager has no such concept* — the same answer a `link:` gives.

**Where the two must differ, the difference is listed with its reason.** Some `upgrade_args` are
not an upgrade-everything verb: `pip install --upgrade` needs names and fails without them,
`bun upgrade` replaces the runtime rather than the packages. Those are correct omissions and
they are indistinguishable from a loss until somebody writes down which is which — so they live
in a named exemption list, one sentence each, checked to still be needed.

**A capability matrix cannot be the only check, because it is written from the code.**
`assert_caps` pins what the registry *does* hand out, so an omission is pinned as correct on the
day it is made. The second test asks whether the config and the registration agree, which is a
question the matrix cannot express.

## II.32 One durable write, two preview policies, and nothing else reaches the disk (`S59`, V.163)

**A rename is atomic against a reader and says nothing about power loss.** The directory entry
can reach the disk before the bytes it points at, leaving a file of the right name and zero
length. So every write LiNix performs is: bytes to a temporary file beside the destination,
`flush`, any permission change, `sync_all`, then rename. That sequence exists once, as
`utils::file::durable_write`.

**Two front doors, because there are two preview policies and both are correct:**

- **`utils::file::persist`** — the config repo (`active`, `preferences.toml`, a manifest, a lock,
  the WAL, the state registry). A dry run prints *would write …* and stops.
- **`CommandExecutor::write_atomic` / `write_secret`** — the machine (a systemd unit, a `link:`
  target, a backend's state). A dry run diverts the bytes into the VFS, so a previewed command
  can read back what a previewed command would have written.

**A permission change happens on the temporary file, before the rename.** A `chmod` afterwards
means the target path holds readable plaintext for however long that takes, and for a secret
"however short" is not an argument (T5).

**Anything else that renames a file into place is a fourth writer and is refused by a scan**,
unless it is listed with the reason it must be its own. There is one such entry: the
installed-listing cache, which is deliberately neither durable nor preview-aware, because a torn
cache file is a cache miss and an fsync per listing would be a disk barrier on the read path.

## II.33 Rollback does not undo work that moved the machine toward the declared state (`S60`, `U41`, V.164)

**One rule, both directions, and one function that answers it.** `plan_intends_present(backend,
name)` reads the plan being executed; the install arm skips its compensating removal on
`Some(true)`, the removal arm skips its compensating reinstate on `Some(false)`, and neither acts
on `None`.

- **An install that succeeded, of something still declared,** is not failed work. It is the goal,
  reached early, and removing it hands the next sync the same work.
- **A removal that succeeded, of something still undeclared,** is the same event from the other
  side. The fact that authorised the removal is still true when the rollback fires.

**The set is the plan's own `Install` nodes**, not a copy of the desired state assembled
elsewhere. `apply` and `heal` rebuild a plan from a file rather than from a model, so the graph
is the only source all three paths share — and a rollback deciding from a second copy of the
desired state is how the two drift apart.

**The removal half applies only to a run that is reconciling against the manifest.**
`GuardScope::reconciles()` is exhaustive over every scope; a scope added later must say which it
is, because inheriting `true` means inheriting *a failed run may leave your software deleted*.
Two are exempt: a `rebuild`'s removal phase is the first half of a reinstall of declared
packages, and an `uninstall` was typed by a person.

**What it costs is stated, not buried:** a package the user had, that a reconciling run removed,
stays removed after a failed transaction. **Generations and snapshots are the durable
put-it-back** — a restore point is taken before every sync — and a durable `Prior` in the WAL is
deferred, not rejected (`U41`).

## II.34 The shipping surface is checked like the program is (`S61`, `S62`, V.165)

**Every environment variable a bootstrap script documents is one it reads, and every one it
reads is one it documents.** `install.sh` and `install.ps1` are piped from the internet: their
header list is the whole interface, and a name in it that nothing reads is a promise, not a
comment.

**An installer installs a binary; compiling is the fallback** (`S79`). Both scripts promised a
*30-second first run* and both did `cargo install --git`, which resolves 448 crates and builds
them under fat LTO on a stranger's machine — so the promise in the header was contradicted by
the next twenty lines, and a toolchain was a precondition for using a package manager. The
published asset comes first; the source build runs when there is no asset for the platform, no
downloader, or no network, and only that path requires Rust. **A release asset is named for the
target it was built for.** Four build targets producing one filename upload as one asset that
three of them overwrite, which is a release page that looks complete and serves the wrong
binary to three platforms out of four.

**A gate whose input is missing must fail, not pass.** `grep -q "^$MSRV"` with an empty `$MSRV`
matches every line. Any check built from an interpolated value tests the value first.

**One number, one place.** A count of anything — crates, backends, refusals — is derived where
it can be and corrected everywhere when it cannot. Six files claiming three different dependency
counts is the failure a 226-line script was already written to prevent for a different number.

**A suppression is addressed to a tool that runs.** A `# shellcheck disable=` in a repository
with no shellcheck is a comment shaped like a gate. Either the linter runs — in CI, and softly
in both release scripts, so a developer meets it before a red push — or the directive goes.

**A release path is rehearsed before the release.** The job that publishes runs on
`workflow_dispatch` too, downloading the same artifacts and asserting the same files exist,
stopping before it publishes. A path whose first execution is the thing it exists for is
untested by construction.

**LiNix is `MIT OR Apache-2.0`** (`Z1`). Both files are at the root, the SPDX expression is in
`Cargo.toml`, and LiNix's own crate answers the same `cargo deny` licence gate as every
dependency.

## II.35 A rewrite leaves everything it was not asked to change (`S63`, `S64`, V.166)

**A file LiNix edits keeps its line endings.** `str::lines()` drops the carriage return, so
rejoining with a bare newline converts a CRLF file to LF in full — one added package becomes a
whole-file diff, and the grammar accepts a BOM precisely because Notepad writes one, and Notepad
writes CRLF too. The ending is read from the file being rewritten, never from the platform.

**A teardown undoes what the declaration did, at the place it did it.** Where a declaration's
effect depends on an option — `setting:x@scope=system` — the ledger key carries that option,
because by teardown time the line is gone and the ledger is the only record left. Resetting the
default scope instead is not a smaller version of the right answer; it changes a different key
and reports success.

**A read command resolves the configuration once.** `linix why` is answering *from* your files;
resolving them again per match makes the cost of an answer scale with the answer's length.

**A comment justifying a check must be true, or the check dies with it.** Three source scans were
justified by "`verbs/` is private to the binary" — it is `pub mod verbs;` — and a scan resting on
a false reason is one the next reader deletes.

## II.36 A listing is shared, and a lookup keeps its matching rules (`S65`, V.167)

**The once-per-run installed listing is handed out as `Arc<Vec<Package>>`.** `Queryable::info` is
asked once per declared package and is list-then-find in thirteen backends; taking an owned `Vec`
there clones the whole listing to read one row.

**A cheaper answer that skips a matching rule is a wrong answer.** `info` is where each manager's
name rules live — choco and winget are case-insensitive, winget accepts a bare moniker for a
vendor-qualified id, go matches a module path by its trailing segment — and where the properties
`@channel` and `@classic` drift on come from. Replacing the call with a map lookup deletes all of
that; sharing the listing deletes only the copying.

**A whole-collection clone that filters nothing is not a filter.** The unscoped plan is the
common case and it borrowed nothing.

## II.37 The integration suite is one target, and the list is checked (`S67`, V.168)

**`autotests = false`, and `tests/main.rs` names every file as a module.** Cargo claims each
`tests/*.rs` as its own binary otherwise, and each is linked against the whole crate — 101 of
them, 36 of which never call the library API at all.

**Nothing moves to earn that.** Every file keeps its path, its name and its doc comment; only the
declaration site changes. A file gated by an inner `#![cfg(...)]` carries the gate on its `mod`
line, where it also stops the module being compiled off-platform at all.

**A file that is not in the list does not run**, which is the one real cost — so
`every_test_file_is_in_the_suite` compares the directory against the module list and fails when
they disagree. Without that check this arrangement is a way to lose a test silently, which is
worse than any link time it saves.

**Run one file with `cargo test --test suite <module>::`.** The module path is the filename.

## II.38 A test process sees the fixture and nothing else (`S68`, V.169)

**One `Fixture`, in `tests/harness/`.** It sets `current_dir`, `LINIX_CONFIG_DIR`,
`LINIX_DATA_DIR`, `HOME` and `USERPROFILE` into the fixture root, and closes stdin.

Every one of those is there because a copy that omitted it was measuring the developer's machine:
without `HOME`, a `link:` target under `~` writes to their real dotfiles; without `current_dir`,
the binary runs in the repository root where a stray `linix.txt` is a manifest the product reads;
without a null stdin, a prompt hangs CI instead of failing it.

**Per-file setup stays per-file.** A test that needs a `tree/` directory, or `use extras` in the
profile, keeps a free `setup(name)` built on `Fixture::new` — and a bespoke helper keeps its own
`impl Fixture` block beside the tests that use it, which an inherent impl in any module of the
crate may do. What is shared is only what was identical.

## II.39 One audit for every exemption table (`S69`, V.170)

**A scanning gate's exemption table is audited by `Ledger::audit` in `tests/ledger/`, not by
four assertions written out beside it.** The four are: the walk read at least its floor; the
predicate still matches something; every site found is in the table; every entry in the table is
still found, and still carries a reason of at least the stated length.

**The floor is required.** `Ledger::audit` panics on a floor of zero. A gate with no floor is a
gate that passes when its predicate stops matching, which is the failure this rule exists to
prevent and the one that got written three times when the four assertions were nine copies.

**The exemptions are subtracted by the ledger, never by the walk.** A finding set that arrives
with the excused sites already removed cannot tell a live exemption from a dead one, because the
subtraction only runs in one direction. The walk reports what is there; the ledger decides what
is excused.

**A site's own knowledge stays at the site.** *This exemption contradicts a row three lines
below it*, *this reason is long and is still a schedule*, *this excused name is not a backend at
all* — the ledger cannot check any of those, and each stays written out with a line saying
which failure the shared check would otherwise misreport. Where the order matters, the local
assertion runs first.

## II.40 A backend whose only Rust was a parser name is a row (`S71`, V.171)

**A built-in backend is a row in `src/backends/builtin_backends.toml` unless it needs something
a row cannot hold.** The five things a row cannot hold are listed in that file's own header;
everything else — argv, flags, root, exclusivity, version pins, repos, OS, capabilities — is
data, and was always data.

**A row names its readers; it does not describe them twice.** `reads`, `searches`,
`outdated_reads`, `machine_list_reads`, `essential_reads` and `depends_reads` name a function in
`src/parsers/named.rs`. Each resolver answers to one field — `outdated_reads` goes through
`probe`, `essential_reads` through `names` — and a name that resolves under the wrong one is a
row that loads and reads nothing. A named reader wins over a described `parser`; both is a row
that said the same thing twice, and the named one has a fixture behind it.

**A reader name that resolves to nothing is a defect, not a default.** The fields are `Option`
and `None` legally means *fall back to the described parser*, so nothing about a typo is
detectable at load. `every_row_can_read_what_it_asks_for` is the only thing between a misspelling
and a backend that reports an empty machine.

**A row with `search_args` and a named `reads` names a `searches`.** `NamedParser` substitutes
an empty-vector closure otherwise, and the capability is still advertised.

**Rows register before hand-written registrations**, so a name held by both is decided in favour
of the Rust, and `no_backend_is_both_a_row_and_a_registrar` fails rather than letting the order
of two calls decide it silently.

**Becoming a row does not end argv coverage.** Every row has a case in `argv_cases()`, checked
by `every_row_has_an_argv_row` — the other half of
`every_registrar_has_an_argv_row_or_a_written_reason`, which scans for registrars and cannot see
a row.

## II.41 A reader is registered with its manager's own bytes (`S72`, V.172)

**A row that can list carries a `[backend.fixture]`**: the stdout its manager printed, what the
row's reader must produce from it, and a `source` naming where the bytes came from. The suite
runs one against the other through `parser_for` — the same function that resolves the live
parser, so a fixture cannot pass against a second resolution of the same fields.

**`source` is part of the fixture, not a note on it.** Bytes typed from a README look exactly
like bytes captured from a tool. One that was not captured says `UNVERIFIED` and is counted;
the count is a ratchet that may fall and never rise.

**An empty listing is not an unreadable one.** A manager that prints only a table header, or
only `No archives currently installed.`, has answered *none* — and a reader that refuses it
stops every verb that needs the installed set. The header check belongs in the shared
noise-line rule, not in each reader's `filter_map`, because a header dropped from the packages
while left in the candidates is what makes the refusal.

**One reader per manager where the output differs.** A shape shared by eight managers is a
claim about eight tools; the fixture column is what tests it, and where it fails the answer is
a reader of that manager's own, not a wider guess in the shared one.

## II.42 A downloaded artifact is torn down in one place and named by its key (`S73`, V.173)

**`github:`, `web:` and `appimage:` remove through `artifact::teardown`.** The four steps —
hand the owning system manager its package back (D5), delete the deployed paths, drop the
cached download, put the record back on failure — are one function, and a backend that grows a
fifth downloader inherits them rather than retyping them.

**The cache is dropped only after everything else came off.** A cache emptied beside a file
that would not delete costs a re-download and buys nothing.

**The failure sentence says the thing is still there**, in both the words the three used before
they shared one: *still installed and still on disk*.

**Whatever a downloader's `fetch_installed` calls a package, its `remove` finds under that
name.** These three answer from a JSON file LiNix wrote, so the identity is theirs to keep, and
getting it wrong is not a cosmetic bug: a reported name the state is not keyed by makes every
declaration read as absent and re-downloads it on every run for ever.

**A search that is an HTTP call is a `SearchSource`, not a bespoke capability.** npm's registry
and PyPI are both variants a row can name; neither is reachable only from Rust.

## II.43 One confirm, and it takes the unattended answer as an argument (`S74`, V.174)

**Every yes/no question goes through `core::prompt::confirm`.** There is one, and
`only_one_place_asks_for_a_yes_or_no` fails when there are two.

**A confirm has three outcomes.** Yes, no, and *nobody is there to answer*. The third is named
at the call site, never defaulted:

- `Unattended::Refuse(sentence)` for anything that changes the machine. The sentence names the
  verb, the flag that proceeds without a human, and the safe way to look first.
- `Unattended::Decline(sentence)` for an offer the run survives without. It prints and answers
  no; it does not fail the run.

**The default is no.** A confirm that defaults to yes fires on a stray newline.

**`Unattended::Refuse` builds an `Error::Refused`**, so it exits 3 and fires
`on_guard_refusal` like every other refusal. That conversion has a test of its own, matching on
the variant rather than on the message.

## II.44 A verb is handed the reader, and an absent flag is not a flag set to false (`S75`, V.175)

**`--json` becomes `core::Output` once, in `main`'s dispatch.** A handler takes `Output`, never
`json: bool`. `a_verb_is_handed_a_reader_not_a_flag` fails on any other signature; only
`args.rs` (where clap parses the flag) and `output.rs` (the one conversion) may name it.

**A printing site asks `out.is_human()`, not `!json`.** The affirmative is the point: the
question is whether a person is there, and two `--json` defects were written by answering the
negative one instead.

**More than one boolean in a signature gets a struct with named fields.** `SyncMode { locked,
upgrade }` rather than two adjacent `bool`s; `CliOverrides { .. }` rather than six positional
`Option<bool>`s, one of which was `allow_mass_removal`.

**A command-line flag with no `--no-` form only ever turns a setting on.** `dry_run`, `yes`,
`verbose`, `allow_mass_removal` and `allow_mass_install` exist in `preferences.toml` and on the
command line; the merge is `|=`, because an absent flag means *the file decides* and there is no
way for a user to spell *off* on the command line. Any new flag in that pair joins the table in
`config::tests::flag_pairs` — all five are asserted together, not one of them chosen.

## II.45 One predicate, one remover, one block reader (`S76`, V.176)

**A policy has one predicate for "does this delete anything".** `RetentionPolicy::prunes` is
it. A pin list (`keep`) is a veto inside a rule that is already on; it never turns deletion on
by itself, because pinning is how a user says *keep this*.

**A path is deleted by `utils::file::force_remove` or its async twin `remove_deployed_path`,
and neither follows a symlink.** They share `remove_by_kind`: a symlink is removed as itself
(file form, then directory form, because Windows needs both and reports a link as neither), a
directory recursively, anything else as a file. An already-absent path is a completed removal.

**A directory is created by `utils::file::ensure_dir` / `ensure_dir_async`**, which name the
directory in the error. `create_dir_all(p)?` on its own reports `Access is denied` and no path.

**`grammar::block_header` and `grammar::when_predicate` decide what opens a block**, everywhere
— reader, writer, REPL. `only_the_grammar_decides_what_opens_a_block` fails on a sixth copy.

**One `Writes`.** `model::Writes` is whether writes reach the disk, for every subsystem that
previews. A second enum answering that question is a second answer to it.

## II.46 A rule about this repo's files is a Rust gate (`S77`, V.177)

**A predicate that reads repo files as text belongs in `tests/`, not in a shell script.** It runs
in `cargo test`, where the other gates that read this repo run, and it fails on a developer
machine before a release script is reached.

**What stays in shell is what shell alone can answer**: lifting a harness function body and
running it under the interpreter CI uses. Reimplementing that in Rust would be testing a copy.

**Before writing a gate, look for the one that exists.** Gate parity had a Rust successor
already, stronger than the shell predicate being replaced; a third copy would have been the
defect the change was made to remove.

**An exemption list is audited by the gate that reads it.** A name in `NOT_GATES` that matches no
file is a claim about nothing, and it is how an exemption outlives the thing it excused.

## II.47 A way to extend LiNix is a row in a table the program can read (`S78`, V.178)

**`app::adapters::SURFACES` is the list of extension surfaces**, and there is no other. A reader
that opens a file under `adapters/` without a row there is invisible to `linix adapters` and
absent from the docs; `every_adapter_surface_is_in_the_table` fails on it.

**`Layout::adapter_file` is the only way to name one.** No caller joins the path by hand — that
is how `firewall:` came to be the surface with no accessor, and a list built from the accessors
would have been a list with a hole in it.

**A user can ask what this machine has extended.** `linix adapters` reports, per surface, the
file, whether the approval ledger cleared it, and **how many rows are actually in force**. The
last is not the same question as *does the file parse*: a `[[backends]]` for a `[[backend]]`
reader parses perfectly and is read by nothing.

**A surface that cannot be used is reported by `adapters::cannot_use`**, which names the file,
what a row there teaches, how a row opens, and the command that lists all eight. A reader does
not write its own sentence.

**A malformed adapter file warns and is skipped; `check adapters` is where it is loud** *(owner
ruling, 2026-08-10)*. Refusing the whole `sync` was the alternative and it was rejected: the file
is optional, the failure fires mid-sync on a working machine, and a typo in an extension must not
stop you installing a package. So the sync degrades — and because a warning inside a sync is a
warning nobody reads twice, the same fact is a **non-zero exit** from `linix check`, where being
loud costs nothing because looking changes nothing. Note what a skip actually costs: the built-in
adapters still ship, so an unusable file does not switch a surface off — it silently returns you
to stock behaviour, which is worse to miss than an outage.

**The readme names every surface.** A plugin surface nobody can find is not one, and the docs
were the other place the list lived only by hand.

## II.49 An environment the OS owns is written into only when a line says so (`Q49`, V.179)

**A manager that refuses to write into a distro-owned environment is right, and LiNix does not
argue with it.** Debian, Ubuntu, Alpine, openSUSE and Fedora mark their Python `EXTERNALLY-
MANAGED` (PEP 668); pip then refuses every install, `--user` included. Two package managers
writing one site-packages is how a system python stops booting.

**The refusal names what a declaration can do about it.** pip's own text is addressed to a
person typing `pip install` and offers venvs, `pipx` and a flag; LiNix adds the two answers a
*manifest line* can hold — `pipx:<name>`, a backend it already drives, and `@system=true`.

**`@system=true` is per line and splits the batch.** It says *write into the OS-owned
environment*, and one line's permission is never handed to the packages beside it. A batch
containing both forms becomes two commands.

**The flag is asked of the tool before it is sent.** `--break-system-packages` arrived in pip
23.0.1; an older pip answers `no such option`. A flag emitted blind trades a refusal the user
can act on for an argv defect they cannot.

**`@system` is legal on the backends that have such a notion and refused by name elsewhere.**
`capability::OS_OWNED_ENV` is the one table.

## II.50 `heal` clears a manager lock it can prove nothing holds (`Q50`, V.180)

**A killed run leaves the manager's lock behind, and every later run fails.** `heal` is the
command whose subject is *a run was interrupted*; clearing that lock is its work.

**`heal` only, never `sync`.** Deleting another package manager's file is a repair asked for by
name, not something a converge does on the way past.

**Only locks whose existence IS the lock.** pacman's `db.lck`, dnf's `metadata_lock.pid`,
zypper's `/run/zypp.pid` are created around a transaction and removed at its end. **apt's and
dpkg's are not**, and they are rows of the same `stale_lock::MANAGER_LOCKS` table carrying a
`never_remove_because` rather than being left out of it: those files exist permanently, are held
with `flock(2)` — which the kernel releases when the holder dies — and deleting one deletes what
the next `apt` expects to lock. A reason that travels with the paths it is about cannot be
re-admitted by someone extending a table who never read the other one.

**Staleness is proved, not assumed.** A lock naming a pid is stale when that pid is not running;
a lock naming nothing is stale when no process of that manager is running at all. Anything else,
including a pid file with nothing readable in it, is left alone.

**Every removal is reported by name and with its reason**, and a removal that fails is reported
too — a lock LiNix could not clear is one the user can now clear themselves.

## II.51 LiNix waits for another package manager rather than failing (`Q51`, V.181)

**Two package managers on one machine is not an error, it is a Tuesday.** An `apt upgrade` in
another terminal, an unattended-upgrade timer, a GUI updater, or a manager orphaned by a run that
was killed and still finishing its transaction. The manager says so plainly, and LiNix used to
answer with four retries over three and a half seconds and the sentence *"this is not the
transient failure its output looks like"* — which was false in precisely the case that printed it.

**The manager's words say a lock is taken; the machine says which kind.** Three states, three
answers, and collapsing any two of them is the original defect:

- **held by something live** — wait for it, announcing the holder;
- **on disk with nothing holding it** — fail at once, naming `linix heal`, because waiting on a
  lock nothing holds never ends;
- **free** — the holder let go between the failure and the question, which is an ordinary race
  and gets the ordinary backoff.

**Bounded, and the bound is one budget across the whole retry loop.** `manager_lock_wait_secs`,
default 300 seconds, `0` to opt out. Sized for the *other* manager's transaction rather than for
LiNix's patience: a `dnf upgrade` of a hundred packages legitimately runs that long, and a wait
that expires before the ordinary case finishes is the same failure with delay in front of it.

**The wait announces itself when it starts.** A wait with no reason given is indistinguishable
from a hang, and a hang is what people kill — which is the interruption that leaves the machine
II.50 exists to repair.

**Nothing is scanned unless the manager already said the word.** The `/proc` question is asked
only after a failure whose text matched that manager's own phrasing for a taken lock. A
successful install never pays for this, and a missing package never waits on it.

**No lock file on disk, no lock.** A running `pacman` with no `db.lck` is a `pacman` doing
something that is not a transaction. Asking the process list before the filesystem inverted this,
and the cost was not theoretical: the `/proc` scan answers *yes, something is running* on a
machine that has no `/proc`, deliberately — for **clearing** a lock that is the safe direction —
so every row read as held on Windows, where none of these files exists.

**And `heal` settles the locks before it judges them (II.50).** A survey is a snapshot, and `heal`
was acting on one: it looked once, correctly left a lock alone because a manager was alive, and
then that manager — an orphan of the very run `heal` was recovering from — exited during the
recovery. By the time the lock was stale, the only step that could clear it had run, and `heal`
finished by telling the user to run `heal`. So a live holder is waited out first, under the same
budget and announced the same way. An orphan finishing the interrupted run's transaction is the
most interesting thing on that machine; waiting for it *is* the repair.

**Backends that drive one manager take one lock.** `pacman` and `yay` both write
`/var/lib/pacman/`; `apt` and `apt-get` share dpkg's; `dnf`, `yum` and `microdnf` share dnf's.
Keyed by their own names they were several locks over one database, and LiNix contended with
itself. The families live in `stale_lock::MANAGER_LOCKS`, because *which backends share a manager
lock* and *which lock is left behind when one is killed* are the same fact.

## II.52 Every process LiNix starts belongs to LiNix (`Q52`, V.182)

**A child is asked to stop before it is killed.** SIGTERM, a grace period, then SIGKILL only for
one that will not go. SIGKILL cannot be caught, so a package manager stopped with it never rolls
its transaction back and never unlinks its lock — LiNix was manufacturing the wedged machine
II.50 repairs. And LiNix's child is usually `sudo`, which forwards a SIGTERM and dies alone under
a SIGKILL, leaving the real manager running as root with its parent gone.

**Three doors, and no fourth.**

- `executor::supervised_output` — captured, bounded by `command_idle_timeout_secs`, stopped on
  every exit including an abandoned future. For a tool nobody is watching.
- `executor::supervised_status` — streams inherited and no idle bound, because a program waiting
  for someone to type is not a hung one. Still owned: an editor holding the terminal after LiNix
  has gone is nobody's idea of finished.
- `blocking::command_output` / `command_status` — for a `std::process::Command`, whose hazard is
  the opposite one. It cannot be abandoned, so it holds a runtime worker until the child exits.

**Blocking waits do not sit on a runtime worker.** A confirm at a prompt, a TUI event loop, the
data-directory lock's two-minute poll: each of them parked a tokio worker for its whole duration.
`core::blocking` is where that is decided — `on_the_terminal` where the call cannot move,
`off_the_runtime` where the work can.

**A gate, not a sweep.** `tests/a_spawned_child_has_an_owner_tests.rs` fails on a `Command` that
reaches `spawn`/`output`/`status` outside the executor unless it goes through a door or sits in
an exemption table with a sentence. Fixing seventeen sites fixes seventeen sites; the gate is
what stops the eighteenth — and it found ten of the seventeen on its first run.

## II.53 A version is recorded everywhere and replayed only where it can be (`Q53`, V.183)

**A lockfile does two jobs and only one of them works on every manager.** *Reproduce* needs the
manager to accept a version as an install argument; *detect drift* only needs it to report one.
So `linix lock` records what it observes on every backend, and a recorded version becomes an
install argument only for a backend that can take one.

**The report that reads those records is `check`'s own, not the planner's.** The planner answers
*what would a sync change*, and a sync changes nothing on a manager it cannot tell which version
to install — so leaving version drift to the planner would report none at all for Homebrew,
pacman, snap and the rest, which are exactly the managers whose record is the only place the
movement is visible. `check` compares the lockfile against what is installed and **names** what
moved: a count sends the reader off to diff two files by hand.

**A version LiNix recorded is never fed back to a manager that cannot replay it.** Doing so is
what killed a release: brew's observed `tokei 14.0.0` was read back as `@version=14.0.0`, built
into `tokei@14.0.0`, and Homebrew answered *No available formula with that name* — because
`name@version` there is a different formula's **name**, not a version selector. The sync failed
permanently on a pin nobody typed.

**A pin somebody typed that cannot be honoured is refused by name, before anything runs.** Not
dropped, not attempted. `sync` names the manager, the pin and the reason, skips that package and
continues; `sync --locked` treats the same fact as fatal, because a run whose purpose is to
reproduce a machine must not report success over a package it resolved freely. Silently
installing a different version is the one outcome worse than either honouring the pin or refusing
it.

**Whether and why are separate, and are checked against each other.**
`Installable::pins_version` answers whether, and defaults to **false** — a backend that says
nothing refuses a pin it might have honoured, which is a message, where the opposite default
installs the wrong version and reports success, which is not.
`capability::CANNOT_PIN_VERSION` answers why, in the words the refusal prints. Neither is derived
from the other: `a_version_pin_is_honoured_or_explained_tests` fails on a backend that cannot pin
and has no reason recorded, **and** on a reason left behind by a backend that has since learned
to pin. A backend may be unable to pin. It may not be silently unable.

## II.54 Nothing outside LiNix may ask a question LiNix cannot answer (`S88`, V.184)

**A password prompt is not read from stdin.** `sudo`, `git` and their credential helpers open
`/dev/tty` directly, so closing a child's stdin does not stop one waiting — it only guarantees
nobody sees the question. A mistyped sudo password, and a terminal with nobody in front of it,
each cost LiNix the full command idle bound: fifteen silent minutes, indistinguishable from a
slow package manager, every night for six nights.

**So the asking happens in exactly one place, or not at all.** Credentials are primed once per
run with a bounded, interactive `sudo -v`, and **every escalated command then runs `sudo -n`** —
which can refuse, and cannot wait. With no terminal to ask on, LiNix says so immediately instead
of waiting to be told what it already knows. git is given `GIT_TERMINAL_PROMPT=0` and
`GCM_INTERACTIVE=never` for the same reason, and ssh is deliberately left alone: silencing it
means overriding `core.sshCommand`, which would break every working custom transport to fix a
misconfigured one.

**A password bound is not a command bound.** A package manager may legitimately work in silence
for minutes; a prompt cannot — either somebody is typing or nobody is there.
`sudo_password_timeout_secs` is its own number for that reason.

## II.55 A download is bounded before it fills the disk (`V.185`)

**A response body is streamed to disk, never buffered whole.** All three download backends read
the entire body into memory before writing it, so a URL answering with something enormous
exhausted RAM before the disk was ever touched. Chunked writing bounds the memory to one chunk
whatever the server sends.

**And it is counted against a ceiling.** `max_download_bytes` is generous — AppImages are
legitimately large — and movable, and `0` removes it. A declared `Content-Length` over the
ceiling refuses before a byte moves; a server that declares nothing, or lies, is caught by the
running count. A body that goes over takes its partial file with it, because a half-downloaded
artifact left on disk is one a later run can find and treat as complete.

## II.56 The log owns what the registry forgot, and a removal that removed nothing says so (`S87`, `Q54`, V.186)

**Two files record what LiNix has installed, and they are not equally durable.** The write-ahead
log is written per operation, before and after each one. The ownership registry is held in
memory through a run and serialised once, at the end, and only when the whole transaction
succeeded. Every crash between those two points leaves a package installed, `Completed` in the
log, and owned by nobody — and nothing downstream notices, because the package is present so no
sync reinstalls it and its entry is closed so no recovery replays it.

**So the registry is a view of the log, and recovery is what makes them agree.** Before every
sync, and on `linix heal`, every install the log completed that the registry does not carry and
the manager still holds is recorded as LiNix's. Three limits are part of the rule: a completed
removal cancels an earlier install (a package LiNix gave up and somebody put back by hand is
theirs), a manager that cannot be asked leaves its packages unclaimed (assuming they are there
would have LiNix issue removals for packages the machine does not have), and a preview records
nothing.

**And this repair is not gated on anything being interrupted.** `needs_recovery` asks about
entries that are still open; the records lost here belong to entries the log already calls
`Completed`.

**A package the user told LiNix to forget stays forgotten.** `unmanage` drops the registry entry
and the manifest line and leaves the package installed — which, read from the registry alone, is
indistinguishable from an ownership record a killed run never wrote. So `unmanage` clears the
package's *finished* log entries too: the log is the third record of the same relationship, and a
repair that read only two of the three would take the package back, find it declared nowhere, and
remove it. Entries that are still open survive the forgetting, because a package being forgotten
is not a reason to lose the evidence that its install never completed.

**Second: a removal that removed nothing does not report success.** `uninstall` deletes the
declaration and lets the sync take the package away as drift — and drift removal only removes
what LiNix manages, so a name LiNix does not own plans no change. When the command ends with a
package it named still installed, and LiNix has no record of installing it, it says exactly
that, names `adopt` and `--absent`, and fails. The question is asked only of names the registry
did not carry when the command started, so an ordinary uninstall costs nothing extra.

**Third: `--absent` removes what LiNix does not own, by declaring it.** `uninstall PKG --absent`
undeclares the package, writes an `absent:` line, and lets the ordinary converge do the removal
— the same guard, the same plan, the same counts, because `absent:` is already the declaration
that reaches outside what LiNix manages (II.2). The line stays: ownership is the record an
unowned removal lacks, and a declaration is a record. It conflicts with `--temp`, which says the
opposite about the same package. A bare name resolves to the manager that *holds* it and is
refused when no manager does — resolving it the way `install` does would write a permanent line
naming a manager that never had the package. A survivor after the sync is still reported, since
a package that outlived an `absent:` line is a failed removal, not a refused one.
