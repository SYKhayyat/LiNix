# Part XI — Proposed: `firewall:`, the perimeter as a declaration

*[LiNix v7](../../SPEC.md) — the map is there; this is one part of it.*

**Asked for on 2026-07-23**, alongside two neighbours that were answered differently: a
kernel-building engine (**out of scope — see XI.7**) and hardware-backed secrets (**Part XII**,
where half of it turns out to be built already).

Nothing in the tree speaks to a firewall today. `grep -rn "nftables\|iptables\|firewalld\|ufw\|
New-NetFirewallRule" src/` is silent, and there is no `firewall` in `src/backends/`.

## XI.1 The reason this one fits, when the kernel one did not

Everything the feature needs already exists in some other statement's machinery:

- **The statement shape** — II.16 says everything is a line, and `setting:` (X.4) is already a
  `key/subkey @value=` statement with a per-store adapter behind it. A firewall rule is the
  same shape with a different adapter.
- **The drift half** — "detect and purge unauthorised out-of-band changes" is `extras_lock`
  (S20) plus `watch` (R11's single reconcile). Both are built. A firewall backend inherits
  them by being an extra, exactly as `service:` does.
- **The refusal half** — `app/sync/guard.rs` already gates every removal. Closing a port is a
  removal, and it is the removal with the largest blast radius in this document.

So this is a new *backend*, not a new mechanism. That is the test X.4 set for `setting:` and it
is the test that fails for the kernel engine.

## XI.2 What is already possible today, and why the backend still earns its place

**An nftables user can already declare their firewall, with no new code:**

```
link:./firewall/nftables.conf@target=/etc/nftables.conf
service:nftables
```

That is a file and a unit, and LiNix has statements for both. It is genuinely declarative: the
ruleset is in git, `sync` writes it, removing the line removes it.

**What it does not give you is the three things a backend would:**

1. **One spelling across five firewalls.** `ufw`, `firewalld`, raw `nft`, `pf` and Windows
   Defender Firewall have nothing in common at the file level. A config that opens port 22 on
   a Debian laptop and a Windows workstation cannot be a `link:` twice.
2. **Per-rule drift, not per-file drift.** `link:` notices the file changed. It cannot notice
   that someone ran `ufw allow 3306` at 2am, because that did not touch the file.
3. **Read-before-write.** X.4 established this as the line between a declaration and a hook. A
   `link:`-plus-`service:` pair restarts the firewall on every sync that touches the file; a
   `firewall:` line that reads the live ruleset first writes only on a difference.

**If the answer to N3 is "one adapter",** the honest recommendation is to build nothing and
document the two lines above instead. The backend is worth its cost only across firewalls.

## XI.3 The statement

Proposed shape, mirroring `setting:<schema>/<key>` deliberately so there is one thing to learn:

```
firewall:allow/22            @proto=tcp
firewall:allow/443           @proto=tcp @from=any
firewall:allow/5432          @proto=tcp @from=10.0.0.0/8
firewall:deny/23             @proto=tcp
firewall:default/incoming    @value=deny
firewall:default/outgoing    @value=allow
```

It inherits the model rather than extending it — which, per X.4, is the bar a new statement has
to clear:

- `when` wraps it, so `when host == laptop { firewall:default/incoming@value=deny }` is a
  per-machine perimeter with no new mechanism.
- Two active declarations of the same rule that disagree is II.7 rule 5's error, not a
  last-one-wins.
- `plan` shows the rule before it exists.
- It is a dependent extra (II.7's third ordering phase, S12) — a rule can name a port a package
  is about to start listening on, so it applies after packages.
- Removing the line removes the rule, through `extras_lock`'s existing undo path.

**`@from=` is one value, not a list, in the short form** — a CIDR contains no comma, and a rule
that needs several sources is the block form, per II.2's rule about commas.

## XI.4 The adapter is per-firewall, not per-distro

`ufw` and `firewalld` both ship on Fedora; neither is the one in charge unless it is running.
The detection question is *which firewall is enforcing*, not *which distro is this* — the same
distinction X.4 drew for desktops, and the same one `service.rs` draws for init systems
(`InitSystem::Systemd | OpenRc | Launchd`).

**A host with no adapter refuses, naming the gap.** `SettingStore::None` is the precedent: a
`setting:` line on an unadapted desktop errors rather than writing something nothing reads. A
`firewall:` line on an unadapted host must do the same, because the alternative — reporting
success while the port stays open — is a security claim that is false.

## XI.5 The thing that must not happen

**This feature's flagship bug is locking the owner out of a remote machine**, and it is the
exact shape of the bug this rewrite exists to prevent: `apt-get purge` ran across hundreds of
system packages because a removal path had no guard on it. A perimeter that "instantly purges
unauthorised changes" is a removal path by construction.

Minimum, and not negotiable if this is built:

- **Every rule teardown goes through `app/sync/guard.rs`.** A guard on one command is a guard
  on nothing (CLAUDE.md).
- **A change that would drop the port carrying the current session is a refusal**, not a
  confirmation — see N2. The user cannot type `yes` to a prompt they will never see, because
  the connection carrying it is what the change closes.
- **`--dry-run` never touches the live ruleset.** `link:`'s decrypt mode is the pattern
  (`link.rs:274`): dry-run logs what it would do and returns before the side effect.

## XI.6 What it is not

**Not a packet-filter language.** There is no LiNix syntax for connection tracking, NAT, rate
limits or chains. Anyone who needs those needs `nft` itself, and XI.2's `link:` pair is how they
get it. The vocabulary here is ports, protocols, sources and the two defaults — the set that
means the same thing on every firewall. Inventing a portable spelling for what is not portable
is how this document's closed `formats` vocabulary (VIII.2) got its rule.

**Not a second place rules live.** If a host declares `firewall:` lines *and* a `link:` to
`/etc/nftables.conf`, two things own the perimeter and the last one to run wins. That is the
two-of-everything failure. See N6.

## XI.7 The neighbouring request that was refused: kernel building

**Recorded because it was asked, and because the reasoning generalises.** The proposal was a
Rust engine that reads the active hardware layout, generates a minimal kernel configuration,
and compiles a custom kernel — "zero distribution bloat".

**Out of scope, and not a K-item.** Three reasons, in the order they matter:

1. **It makes LiNix a distribution builder.** LiNix drives package managers that are already on
   the host; it builds nothing. Compiling a kernel brings a toolchain, a build cache, artifact
   storage, and a boot story — what happens when the new kernel does not boot — and none of
   those exist here. It is not an extension of this design; it is a second product sharing a
   binary.
2. **It is meaningless on two of the three supported systems.** `os` is `linux | macos |
   windows` (II.2). A core feature that only exists on one is a wrong shape for the core.
3. **The mechanism it describes already exists elsewhere and is known to be fragile.**
   Generating a config from currently-present hardware is `make localmodconfig`, in the kernel
   tree for over a decade, and its documented failure is hardware that was not plugged in when
   the config was taken.

**What was kept from it — see XIII.1**, which also answers the question this refusal raises:
*LiNix already upgrades the kernel, so why not the driver that depends on it?* One thing in this
area is genuinely in scope — rebuilding the declared out-of-tree modules that a kernel change
just invalidated, because no package manager's hook fires for a module a different manager
installed. A second, a `hardware` command that printed declarations to paste, was **withdrawn
under P8**.


---

**Decisions: N1–N7.** They live in [the decision register](../decisions.md), with a status on
each — this part states the shape, the register states what is still unanswered.
