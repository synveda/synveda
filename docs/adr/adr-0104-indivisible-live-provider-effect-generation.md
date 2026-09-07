# ADR-0104: Indivisible live-provider effect generation

- **Status**: Accepted
- **Date**: 2026-09-06
- **Feature(s)**: CPR-45
- **Deciders**: Synveda maintainers

## Context

ADR-0103 defines and the current state owner implements a cooperative
no-spawn reservation after the completed live-provider start decision. Its
immutable witness says `process_attempt: not-started` and `held-no-process`;
its settlement authorizes retirement without a process; its fixed marker is
retired before its close. That completed history is terminal evidence that no
provider effect belonged to the generation. Reopening, relinking or extending
it would turn a proved negative into process authority.

The next boundary cannot be one child process or one successful spawn. Colima
starts an outer process which delegates to a detached Lima hostagent, usernet
and an SSH ControlMaster, while those processes create sockets, a Docker
context and a bounded but dynamic filesystem tree. Durable authorization and
an operating-system process-creation syscall cannot be atomic. A crash after
authorization may therefore leave a process that started, exited or detached
before its identity was recorded. Retrying from absence would risk two
providers; inferring ownership from a PID, name, path or socket would risk
adopting foreign state.

The mutation journal's permanent slot inode also has a generic one-link
invariant after its local publication stage is retired. Using that inode as an
external provider-root marker would couple every journal parser and recovery
path to provider-specific link-count transitions. A separate witness adds
finite pre-start publication states, but those states remain safely abortable
because no start authority can exist until the complete witness topology is
durable.

## Decision

Implement the first process-capable boundary as one atomic state generation
with a sibling effect branch after the exact completed start decision:

```text
plan-abort* -> plan-complete
  -> intent-abort* -> intent-complete
  -> decision-abort* -> decision-complete
       |-> reservation-abort* -> no-spawn-complete -> END
       `-> effect-pre-attempt-abort* -> effect-reservation-complete
             -> start-authority-complete
                  |-> pre-attempt-retired -> zero-receipt-close -> END
                  `-> start-attempted
                        |-> creation-settled(live-identity)
                        |      -> common-terminal-tail -> END
                        |-> creation-settled(exact-attempted-residual)
                        |      -> common-terminal-tail -> END
                        `-> uncertain-start -> BLOCKED
```

The first terminal branch remains semantically unchanged from ADR-0103. A
selected or completed no-spawn branch MUST NOT precede the effect branch, lend
it authority, be translated into it or be extended after close. Production and
fixture branches MUST remain class-separated. The common attempted-effect tail
is cleanup plan, authenticated quiescence and retirement, outer cleanup
settlement, a terminal receipt bound to that settlement, marker retirement and
parent fsync, one-link witness proof, then close. A pre-attempt abort publishes
no receipt; if it acquired the marker, it retires and fsyncs only the exact
witness/marker topology before its zero-delta close.

### Effect reservation and start fence

1. The owner MUST publish the complete effect mutation slot before any
   provider-root or process mutation. The slot remains the only journal
   authority record and retains the generic permanent one-link topology.
2. The owner MUST create and fsync a distinct immutable effect-witness stage.
   Its canonical bytes bind one-to-one to the effect slot, completed decision,
   fresh admission, state run, provider root, all six namespace identities and
   inventories, exact command/environment/toolchain contract and planned
   start-attempt identity. The witness is physical exclusion evidence; its
   existence alone grants no process, signal, recovery, receipt or cleanup
   authority.
3. A no-replace hard link from that stage to the existing fixed basename
   `.synveda-clean-engine-provider-reservation` MUST remain the sole
   cross-state-base CAS. The same inode MUST then be linked to one deterministic
   effect-witness name in the state run, both parents fsynced, and only the
   random stage retired. It MUST never reuse, open, relink or remove an
   ADR-0103 reservation witness.
4. Start authority MUST remain impossible until the fixed marker and state
   witness are re-proved as the same device/inode/UID/mode/size/canonical bytes
   with exactly two links. Missing, extra, wrong-name, wrong-type, wrong-device
   or replacement topology blocks.
5. The owner MUST publish a distinct durable start-authority record which binds
   the exact effect slot/witness, completed decision, source, fresh namespace
   admission, command, environment, toolchain and planned attempt identity. It
   grants only the current in-memory owner permission to publish that attempt;
   a recoverer cannot invoke from serialized authority. Immediately before
   authority publication and again before attempt publication, the owner MUST
   reconstruct those inputs from current state, re-observe the namespaces and
   re-prove the exact marker/witness topology byte-for-byte.
6. The owner MUST publish and fsync a no-replace, single-use start-attempt
   record bound to the authority digest before the first process-creation
   syscall. Only the in-memory winner returned by that publication may deliver
   the one exact start. A crash after start authority but before the attempt
   record remains pre-attempt and abortable; neither recovery nor a later owner
   may use that authority to start a process.
7. After the start-attempt fence is durable, recovery MUST NOT spawn, replay,
   downgrade the generation to `not-started` or infer that no effect occurred
   from elapsed time, `ESRCH`, an absent PID, path or socket. The legal outcomes
   are exact completion, exact retirement classified as effect-possible, or a
   blocking `uncertain-start` state.

The marker/effect-witness inode MUST remain continuously linked, without an
unlink/relink gap, from before start authority through invocation, process and
endpoint settlement, complete dynamic-resource retirement, cleanup settlement
and terminal receipt. Only then may marker retirement begin.

### Causal identity and complete observation

The generation MUST authenticate four distinct closed roles:

1. the foreground outer Colima process;
2. the detached Lima hostagent;
3. the usernet process; and
4. the SSH ControlMaster.

Each role requires an operation- and role-bound unpredictable challenge, a
durable launch edge created before delivery, PID-reuse-resistant process
identity, boot identity, UID, PID/PGID/session, exact executable identity,
closed argv/environment/cwd and its causal parent or detachment evidence.
Post-hoc process scanning, a PID file, executable name or socket pathname MUST
NOT confer ownership. The four named roles are not by themselves proof of a
complete process tree: every descendant and reparenting edge must occur in a
schema-bounded causal graph with explicit node, edge, depth, byte and elapsed-
time ceilings. Overflow, an unknown edge or a transient descendant blocks
settlement.

Readiness and identity require authenticated fresh-connection proof for every
created hostagent, usernet, SSH-control and Engine endpoint. The Docker-context
bytes MUST resolve to the exact authenticated private Engine socket while the
ambient/default context grants nothing. Context activation remains disabled.
Endpoint existence or connectability alone is insufficient, and the endpoint
set must be re-proved after detachment or outer-process exit and before
settlement.

Before the final observations, every authenticated role MUST acknowledge a
durable quiescence fence which prevents any new descendant, endpoint or
filesystem resource through creation settlement. Repeated process snapshots
without that protocol fence are insufficient. While the fence, marker, source,
roles and endpoints remain unchanged, the implementation MUST take two
byte-equal complete, bounded, no-follow recursive inventories across the six
mutation namespaces. Ordered relative identities cover type, device/inode,
UID, mode, link count and size; bounded regular evidence/config files are
hashed; symlink target bytes are recorded without following; sockets are bound
to their authenticated role. Cross-device entries, unsupported types,
unexplained hard links, overflow or any intervening change fail closed. The
production path additionally requires the already-open live macOS proxy
observation and exact `/bin/sh` and `/usr/sbin/ioreg` executable evidence.

Only the acknowledged quiescence fence, complete causal graph, authenticated
endpoints and the two equal recursive inventories may produce the live-identity
variant of the immutable outer creation settlement. Its distinct
exact-attempted-residual variant requires a conclusive failed-delivery result or
previously authenticated roles plus complete proof that every possible role,
descendant, endpoint and dynamic resource is now exactly retired. Anything
less is `uncertain-start`. One of those settlement variants is the sole input
to the cleanup plan; neither an intermediate role nor provider readiness can
skip it.

### Recovery, retirement and receipts

Recovery MUST first prove the exact prior owner and newest recoverer absent,
then publish an append-only claim binding the slot/marker topology, causal
frontier, process identities, endpoint set and recursive inventory. Before the
attempt fence it may finish or retire only the exact witness publication and
abort. After the fence it may authenticate and settle existing state or execute
only the already-published cleanup plan. It has no general provider adoption,
launch, replay, signal, prune, root-cleanup or arbitrary-path authority.
Indeterminate probes, PID reuse, `EPERM`, unknown process/resource identity or
incomplete evidence preserve state and block.

Retirement MUST be reverse-causal and leaf-first:

1. publish the immutable cleanup plan from the outer creation settlement;
2. reassert source, slot, settlement, marker/witness and current recursive
   prefix before each effect;
3. withdraw Engine/context use and request only the authenticated shutdown
   named by the plan;
4. prove the corresponding process and all descendants absent before removing
   their PID, control or socket evidence;
5. remove exact non-directory leaves deepest-first, then exact directories
   deepest-first, recording append-only progress and fsyncing each parent;
6. rescan to the exact admitted baseline and publish the outer cleanup
   settlement while the marker remains held;
7. publish a terminal receipt which binds that exact cleanup-settlement digest,
   still while the marker remains held;
8. retire only the exact fixed marker, fsync the provider root and prove the
   immutable effect witness now has one link; and
9. publish the mutation close last, bound to the owner or, after recovery, the
   newest recovery authority, plus the settlement and receipt.

Both an authenticated live-identity completion and an exact attempted-residual
completion MUST traverse this creation-settlement-to-close tail. Only a
pre-attempt abort may use the separately validated zero-receipt retirement and
close path.

Unknown leaves, aliases, mounts, symlinks, inode or security-metadata drift,
unattested live processes, vanished sockets or outer-process exit MUST NOT be
treated as successful cleanup.

### One state-generation hard cut

The implementation MUST land as one production-unreachable, registry-deny-only
atomic cut until the entire fixture effect and recovery grammar passes. It
advances receipt v5 to v6, mutation slot v6 to v7, recovery and recovery-root
v5 to v6, mutation close v7 to v8, every persisted
`mutation-journal-v6-*` binding to v7, and all dependent plan, intent,
decision, process-admission and reservation envelopes and digests. It adds
distinct effect-witness, start-authority, start-attempt, role-identity,
endpoint, recursive-inventory, create-settlement, cleanup-plan,
cleanup-progress, cleanup-settlement and completion schemas.

The non-authorizing live requirements advanced from v4 to v5 because trusted
pre-effect ownership now binds the exact bounded recursive baseline descriptor
set rather than only a top-level inventory. Every dependent non-effect contract
and digest advanced with it. V1 through v4 inputs and old dependent chains are
refused with reset-and-regenerate guidance. There is no translator, dual
reader, relabelled evidence or controlled-background compatibility path. This
clean-engine hard cut does not change Postgres schema epoch 3.

This decision grants the production live effect no present process execution,
Docker/Colima/Lima effect, recovery, registry capability, supported lifecycle,
receipt/environment/runtime/provider-evidence publication, finalization or
readiness claim. The live create registry tuple remains planning-only, cleanup
remains deny-only and lifecycle remains `plan|status|verify`. The
repository-owned fixture grants only fixture effect publication and recovery;
it never grants or invokes a provider process. It MUST NOT import or relabel
the class-closed controlled-background v5 fake, and it cannot establish live
provider support without separate environment and platform evidence plus
independent review.

A pure sibling-effect contract now binds the exact deny-only post-decision
admission to this branch shape. The production
`colima-live-provider-effect-v1` contract uses
`production-deny-only-no-invoker` and is pinned to
`e57ab31606d0cf6e33a0fd45cc86335a6ca1288d9beb28839aeb45225f24df63`;
every capability is false and event publication is refused. State imports no
production effect operation tuple and exposes no production effect publisher
or recovery entry point. A separate fixture-only contract, pinned to
`2926b334f9f63a665fc3648e32e3438627bff04a9dd90d1fed9b71e3d23aeae8`,
uses a distinct operation kind, schema and evidence class; its recovery-only
capability cannot be relabelled as production.

That fixture grammar closes the distinct witness and fixed marker, one start
authority and attempt, exactly four authenticated roles and four causal edges
at depth two, four endpoint/socket identities plus the private Docker-context
identity and bytes,
quiescence, two equal paged recursive inventories over the six namespaces,
the trusted v5 baseline descriptor map, intra-namespace hard-link closure,
exact directory-scaffolding capacity, derived cleanup, paged progress,
settlement, terminal receipt and marker retirement. Its six closed histories
cover pre-attempt, authority-only, residual, uncertain, normal and rich paged
retirement. The boundary-owned module has no direct filesystem, process or
network-executor import, and a tripwire proves construction and validation
invoke no observation I/O.

The state owner consumes receipt v6, mutation slot v7, recovery/root v6 and
close v8 as one hard cut. After the exact completed fixture start decision, a
test-only publisher commits a sequence-bound physical stage, inode-derived
witness and fixed external marker before any authority event. It may either
retire the marker and publish a zero-receipt pre-attempt completion, or publish
an attempt fence and stop with the marker/witness inode held at two links. The
attempt fence invokes no process and is not recoverable automatically.

Fixture-only recovery proves the original owner and newest prior recoverer are
absent, binds every claim to the observed physical/event frontier, supports
crash-safe forward completion before attempt, and never crosses an attempt
fence. Multiple claims at one frontier are legal because a recoverer can die
immediately after its claim; only the newest claim can publish or close. The
generic receipt path refuses `provider-effect-retired` before acquiring a slot.
Receipt v6 reserves that exact state-owned terminal phase for the future
attempted-effect tail, while the implemented pre-attempt completion changes no
receipt. No process or provider is invoked, and all effect mutation/recovery
exports remain test-only. Caller-held pure bytes are replayable contract data,
not state provenance or branch selection.

The earlier non-persisted prerequisite projection remains a separate inert
review artifact. It defines no action, operation kind or operation contract,
retains its own 512-occurrence preflight, and is consumed by neither state nor
the pure effect grammar. It is not a compatibility or execution path.

The earlier standalone process fixture is separate from the pure event fixture.
It uses four repository-owned Node processes to exercise
fsynced pre-spawn causal edges, inherited-IPC-only HMAC keys, fresh private
endpoint challenges, a recursive durable quiescence fence, actual hostagent
detach/outer exit, a changed authenticated parent observation and cooperative
leaf-first shutdown under one signed end-to-end deadline. Its injected partial
start test binds a signed hostagent frontier before recursively verified
failure cleanup. Its process nonce is explicitly not an OS start identity;
it neither proves boot/PGID/session identity, arbitrary process-tree
completeness or the production recursive filesystem inventory, nor exercises
Colima, Lima, SSH, an Engine or any provider effect. It imports no deployment
state, reservation, receipt, registry or lifecycle module and implements none
of this ADR's state-generation, attempt, recovery, settlement or close grammar.
The executable fixture evidence recorded for this slice is Darwin-only. A
Linux path is implemented and admitted by the fixture but remains unexecuted,
so it is not Linux platform evidence.

## Options considered

1. **Distinct effect witness and sibling effect branch (chosen).** Preserves
   the generic journal inode contract, reuses one cooperative CAS and makes all
   extra publication gaps pre-start and explicitly recoverable.
2. **Reuse ADR-0103's witness.** Rejected because its immutable bytes and
   retired marker prove `not-started`; reuse would falsify historical evidence.
3. **Hard-link the mutation-slot inode to the marker.** Rejected because the
   permanent generic slot is journal authority with a one-link invariant.
   Provider-specific external aliases would broaden generic parsing, recovery
   and close behavior. The separate witness is bound to the slot but never
   becomes a second authority identity.
4. **Split authorization and spawn into replayable generations.** Rejected
   because the unavoidable crash gap permits duplicate process trees.
5. **Accept partial roles or infer ownership from PIDs, paths and sockets.**
   Rejected because detachment, reparenting, PID reuse and nested resources
   make those observations insufficient.

## Consequences

- Positive: one fixed cooperative CAS covers the whole effect; one durable
  attempt makes retry behavior unambiguous; causal identity, endpoint evidence
  and exact retirement can be reviewed as one closed generation.
- Negative / accepted trade-offs: the cut is large; recursive inspection adds
  bounded cost; provider and state roots must share a local filesystem device;
  and an uncertain attempt may intentionally remain blocked rather than risk a
  duplicate or destructive adoption.
- Reversal trigger: if the pinned provider cannot expose the four causal roles,
  authenticated endpoints and a bounded stable tree, keep it unregistered and
  use a stronger supervisor or service boundary. Do not weaken identity or
  recovery to make the provider fit.

The fixed marker coordinates cooperative Synveda processes under one UID. It
is not a security boundary against hostile same-UID code, root/administrator,
mount substitution, alternate PID namespaces or external signalling. ACLs,
xattrs, flags and non-local filesystem semantics remain outside the claim.

## Compliance notes

This is content-free deployment state, not tenant or business data. It changes
no Cedar, forced-RLS, VedaFlow, audit or public API path. Persisted and public
evidence contains opaque identities and digests, never credentials, command
output, prompts, Sessions or Knowledge. Exact private paths and process inputs
may be consumed only inside the private observer and MUST NOT enter receipts,
manifests, logs or diagnostics.
