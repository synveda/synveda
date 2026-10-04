# Operator-managed PostgreSQL starter

For an operator-free installation, use the [bundled evaluation recipe](examples/README.md).

The CNPG starter instructions are consolidated into the authoritative
[Kubernetes installation guide](README.md#1-choose-a-route-and-dependency-owners).
Use [starter-values.yaml](starter-values.yaml), the
[configuration reference](CONFIGURATION.md), and the
[operations runbook](OPERATIONS.md). Provider selection is not data migration.

## First use and membership

Follow [first human login](FIRST_LOGIN.md) and [first useful workflow](FIRST_USE.md).

## Agent credentials and revocation

Follow [scoped client setup and credentials](../../../docs/CONSUMER_CLI.md).

## Maintenance, backup and retained reinstall

Follow [recovery and maintenance](OPERATIONS.md), including exact retained
uninstall semantics and the separate destructive purge warning.
