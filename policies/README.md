# Policy packs

The embedded `regulated-strict`, `standard` and `open-collaboration` Cedar
bundles form the built-in policy catalogue. A governed Configuration version
selects a catalogue entry at a scope; publishing and binding that Configuration
flows through VedaFlow, the PDP and content-free audit.

Custom policy source is separate operator bootstrap. Local `synveda policy
apply|clear` commands use tenant-scoped storage and record explicit break-glass
audit evidence; they are not a public-API or VedaFlow mutation path. Product
configuration can select a successfully compiled custom entry, but cannot
bypass Cedar, forced RLS or audit. See
[the installation contract](../docs/INSTALL.md#governed-runtime-configuration)
and [ADR-0089](../docs/adr/adr-0089-governed-runtime-configuration.md).
