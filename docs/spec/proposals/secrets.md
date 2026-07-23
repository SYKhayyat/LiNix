# Part XII — Secrets: what is built, what is not, and what will not be

*[LiNix v7](../../SPEC.md) — the map is there; this is one part of it.*

**Asked for on 2026-07-23** as "hardware modules like TPM 2.0 or YubiKeys decrypt and inject
credentials at runtime, keeping public config repositories completely clean." **Half of that
sentence has been true since Phase 2p and was never written down here** — which is why this part
exists at all: an undocumented feature is a feature nobody uses and the next session reimplements.

## XII.1 What is built

**`link:` mode D — decrypt a file from the repo, write the plaintext to disk.**
`src/backends/link.rs:271-295`, with `decrypt_argv` at `:45` and identity resolution at `:81`.

```
link:./secrets/npmrc.age {
  target   = ~/.npmrc
  decrypt  = age
  identity = ~/.config/linix/age.key
}
```

- **Two tools, closed set:** `age` and `sops`. Anything else is an error naming both
  (`link.rs:59`) — the same closed-vocabulary rule as `formats` (VIII.2).
- **The identity resolves in three steps:** `@identity=`, then `$LINIX_AGE_IDENTITY`, then
  `~/.config/linix/age.key`. `sops` takes no identity flag; it reads its own configuration.
- **LiNix embeds no crypto.** It runs the binary the user already trusts, and captures stdout
  raw — never trimmed — so key material survives byte-for-byte.
- **Dry-run never decrypts** (`link.rs:274`). It says what it would do and returns. A dry run
  that produced a plaintext file would make `--dry-run` the leak.
- **The plaintext is 0600 on Unix** after the write (`link.rs:285`).
- **Removing the line removes the plaintext**, through the same `remove` path as every other
  `link:` mode.

**So the headline promise already holds: the repository is public-safe, and the plaintext exists
only on the machine that can decrypt it.**

## XII.2 What is not built, and what will not be

**Hardware-backed identities (TPM 2.0, YubiKey) — not built, in scope, see XII.3.**

**Runtime injection into process memory — REOPENED (owner, 2026-07-23), tracked as T7.** It was
ruled out earlier the same day, in the words below, and the owner has since said the conversation
stays open. **The refusal is not withdrawn — it is downgraded to a question**, and the reasoning
below is what any case for the feature has to answer. The sentence at the end of this section
telling you not to re-open it no longer holds; the bar it sets does.

The proposal was
that credentials never touch the disk at all: LiNix decrypts into the memory of the process that
needs them. Ruled out rather than deferred. It requires LiNix to be in the process-launch path
for every consumer of a secret, which is a supervisor's job (`systemd`'s `LoadCredential`, a
`direnv`, a secrets agent), and LiNix is not a supervisor. The half-measure — injecting only into
children of `linix run` (`app/run.rs:138`) — would protect exactly the processes LiNix starts and
none of the ones that actually read `~/.npmrc`, while reading as though it protected both. **Do
not re-open without a use case that lives entirely inside `linix run`.**

## XII.3 The hardware half, proposed

`age` already delegates identity to plugins — `age-plugin-yubikey`, `age-plugin-tpm` — and a
plugin identity is consumed through the same `age --decrypt -i <identity>` invocation
`decrypt_argv` already builds. **The likely shape of this work is therefore relaxing what
`@identity=` accepts, not new crypto and not a new mode.** What it needs on top:

- A plugin identity file is a stub, not a key. The failure when the token is absent is a
  prompt-or-hang from the plugin, not an error from `age` — so the timeout and the message are
  the work here, not the invocation (T3).
- Touch-required tokens make decryption interactive. A `sync` that silently blocks on a
  YubiKey nobody is standing next to is the unattended-`watch` failure (T4).

**This should not be started until T1 and T2 are closed,** because they are live defects in the
half that already ships, and hardware keys make a leaked plaintext no less leaked.

## XII.4 The invariant this sits next to, and does not contradict

II.1 says secrets are *"the environment only. `GITHUB_TOKEN`. Never a file."* Mode D writes a
secret to a file. **These are two different secrets and the distinction must stay explicit, or
the next reader will delete one of them as a violation of the other:**

| | II.1's rule | mode D |
|---|---|---|
| whose secret | **LiNix's own** credential, for its own API calls | **the user's** credential, for some other program |
| where it may live | environment variable, never on disk | encrypted in the repo, plaintext at `@target=` |
| why | LiNix must never be configured with a secret, so a config is always safe to hand over | the program that needs it reads a file, and always will |

X.5's backup rule holds under both: a `bundle` copies the config root, and the config root holds
only the *encrypted* file. **What a bundle must never pick up is the decrypted target** — which
is a live question, because nothing checks where `@target=` points (T2).


---

**Decisions: T1–T5.** They live in [the decision register](../decisions.md), with a status on
each — this part states the shape, the register states what is still unanswered.
