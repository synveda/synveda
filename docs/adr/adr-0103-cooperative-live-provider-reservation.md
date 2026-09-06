# ADR-0103: Cooperative aggregate live-provider reservation

- **Status**: Accepted
- **Date**: 2026-09-06
- **Feature(s)**: CPR-45
- **Deciders**: Synveda maintainers

## Context

The live Colima preparation contract owns one canonical private provider root.
Six fixed child namespaces beneath that root are the complete top-level
mutation surface for the pinned Colima/Lima command. The current state owner
samples all six namespaces before and after its completed start decision, but
those observations and the state journal's own hard-link CAS do not stop a
second state base from passing the same observation and racing the first one.

Synveda needs one cooperative reservation before a later process-effect slice
can exist. It must cover the exact state run and all six observed namespaces,
survive interruption, admit deterministic recovery and retirement, and remain
incapable of starting or signalling a process. POSIX provides no transaction
that atomically creates a link in seven directories. Code executing as the
same operating-system user can also unlink or replace any user-owned marker,
so this protocol cannot create a security boundary against hostile same-UID
code.

## Decision

Use one aggregate, no-replace hard-link reservation under the canonical
provider root.

1. The state owner first publishes a new-generation `provider-reservation`
   mutation slot. It then creates and fsyncs a complete immutable reservation
   witness under an unguessable private stage name in the active state run.
   The canonical witness bytes bind the slot and completed decision, the fresh
   admission, the exact state-run and provider-root identities, and the six
   ordered namespace identities and inventories. Paths and sensitive content
   are not serialized.
2. The state run, provider root and six namespace directories must be owned by
   the current UID, have their required private modes, retain their observed
   identities, and be on one device. Any mismatch or `EXDEV` fails closed
   before process authority can exist.
3. Every conforming writer contends on the same fixed provider-root basename,
   `.synveda-clean-engine-provider-reservation`. `link(2)` from the fsynced
   witness stage to that basename is the sole reservation linearization point.
   The owner revalidates the complete state and root plan after slot acquisition
   and before creating the stage. After the CAS succeeds, the stage and marker
   must be the same device/inode/UID/mode/size/content with exactly two links,
   and the provider root is fsynced.
4. Only after that proof may the same inode be linked to its fixed state-run
   witness name. The state directory is fsynced and the unguessable stage name
   is retired. The durable witness therefore distinguishes a reservation that
   existed from a pre-link attempt. Neither the marker nor the witness is a
   process, provider, receipt, environment, lifecycle or finalization record.
5. This no-spawn generation performs no provider command. While the marker is
   held it brackets a fresh observation of all six bound namespaces with exact
   two-link marker/witness checks. It then publishes the immutable
   `retirement-authorized-without-process` settlement before unlinking only the
   verified marker. The provider root is fsynced, the durable witness is proved
   to have exactly one link, and the slot closes with zero receipt/environment
   delta and the settlement digest as operation evidence.
6. Dedicated reservation recovery can abort only `not-started`, `stage-only`
   and durably `stage-retired-before-effect` states. Once a marker or witness
   exists, recovery must converge through witness publication or exact marker
   relinking, the same bracketed observation and settlement, verified marker
   retirement and completion. It may not relabel an effect-bearing state as a
   pre-effect abort. Replacement, an extra hard link, a wrong marker type or
   mode, an unknown inode, missing causal evidence or namespace drift leaves
   the state uncertain and blocks further work.
7. Receipt, mutation-slot, close and recovery/root schemas form one deliberate
   generation cut. Older state is refused with reset-and-regenerate guidance;
   it is never relabelled or extended with this reservation contract.
8. Reservation authority is deliberately narrow: create/link/inspect/fsync and
   unlink the one bound marker plus publish its state witness, settlement and
   close. The closed contract exposes `reservation_recovery_authorized: true`
   only for that physical reservation grammar, while
   `provider_recovery_authorized` remains false. Process start, spawn, signal,
   process-group ownership, adapter execution, general provider-root mutation,
   provider evidence, receipts, environment publication, lifecycle exposure
   and finalization remain false. No live provider tuple is added to the
   adapter registry.

The fixed marker coordinates only cooperating Synveda state owners running as
the same UID. The complete witness prevents two cooperating state bases from
interpreting different namespace sets as one reservation, but it does not stop
arbitrary same-UID code from ignoring the protocol.

## Options considered

1. **One aggregate provider-root hard link (chosen).** One no-replace syscall
   selects the cooperative owner, while immutable bytes bind every covered
   identity and keep crash recovery finite.
2. **One marker inside each of the six namespaces.** Rejected: POSIX cannot
   publish the six-link set atomically, so interruption creates a prefix that
   needs ownership arbitration without improving the cooperative threat model.
3. **A lock file, advisory lock or process mutex.** Rejected: process lifetime
   is not durable evidence, advisory locking is easy to omit, and neither
   coordinates independent state bases after a crash.
4. **A nonce-derived marker name.** Rejected: concurrent owners would choose
   different names and both could believe they hold the mutation surface.
5. **Treat repeated observations as the reservation.** Rejected: observation
   detects drift but has no no-replace linearization point.

## Consequences

- Positive: all conforming state owners share one durable CAS; the witness is
  content-free, path-free, bounded and recoverable; no process authority is
  introduced.
- Negative: provider and state roots must share a filesystem device, the
  protocol adds an explicit recovery/retirement grammar, and hostile same-UID
  programs remain outside the isolation claim.
- Reversal trigger: if the reference provider no longer has one private common
  root, or independent untrusted principals must share the host, replace this
  mechanism with an OS/service boundary that can provide real transactional
  exclusion. Do not extend it into a multi-host lock.

## Compliance notes

This is deployment-fixture state, not tenant or business data. It changes no
Cedar, forced-RLS, VedaFlow, audit or public API path. The marker and all state
evidence contain opaque identities and digests only. No prompt, Knowledge,
credential or token may enter the reservation bytes, logs or errors.
