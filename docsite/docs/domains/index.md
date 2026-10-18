# Domain management (fork feature)

A step-by-step Chinese acceptance and cleanup guide is available at
`example/domains/README.zh-CN.md`.

This feature is disabled by default. It manages exact HTTP(S) hostnames using
Cloudflare DNS and an existing dedicated Traefik file provider or a remotely
managed Cloudflare Tunnel. It does not change authoritative nameservers, install
an ingress, deploy an application, or create a Cloudflare account.

## Enable deliberately

Set `domain_management_enabled = true` and a stable, unique
`domain_instance_id` in the Core configuration. Keep the same identity only when
intentionally restoring ownership on the same installation. Never run two copies
of a restored installation against the same external objects. Core uses a database
controller lease and resource fencing; an external API without compare-and-swap
still cannot provide exactly-once writes.

Credentials are references to administrator-configured Core `secrets`. Configure
these out of band, preferably using the existing secret-file facility. Do not put
Cloudflare API tokens in resource files, CLI arguments, Git, or frontend state.
Cloudflare API tokens and cloudflared connector tokens are different credentials.
Variables are not an encrypted credential vault.

For proxied Traefik HTTPS, full TLS observation also requires read access to the
Cloudflare zone SSL setting and direct reachability to the configured origin.
Only strict origin validation plus a verified origin certificate establishes this
condition; missing read permission or network access produces Unknown.

Create a DNS provider and an ingress connection through the Domains UI or
`km domain provider create --file provider.json` and
`km domain ingress create --file ingress.json`. Both require administrator
permission. Validate each connection before planning a binding. Scope connections
to explicit hostname suffixes and approved backend origins. In this version,
`allowed_networks` contains exact HTTP(S) origins (scheme, host and port), not CIDR
ranges. Administrator-approved DNS names remain a trust boundary; this is not a
general DNS rebinding firewall. Container targets require a verified shared custom
Docker network; Tunnel targets use an explicitly approved URL. See
`example/domains/` for credential-free file shapes.

For Traefik, separately enable `domain_enabled` on Periphery and map each **Ingress
resource ID** to an administrator-owned absolute directory in
`domain_ingress_directories`. Mount this directory into Traefik as a directory,
with file-provider `watch` enabled. Do not place manual YAML/TOML files in it.
Pre-create the directory without symlinks or group/world write permission.
Persist Periphery's hidden ownership and fencing files together with the routes.
Use a dedicated ingress instance: existing Docker label routers cannot be reliably
observed through the file provider. Periphery accepts structured routes, never a
client-specified path or shell command. A successful atomic file write does not
prove that Traefik loaded it.

A Cloudflare Tunnel must be remotely managed, already connected and dedicated to
Komodo configuration writes. Unknown configuration fields and the final catch-all
are preserved. External edits cause a drift conflict instead of being overwritten.
There is no conditional-update guarantee from Cloudflare: do not configure another
writer for that tunnel.

## Plan, apply, inspect

```sh
km --profile home domain create --file example/domains/domain.json --yes
km --profile home domain plan my-app --out app.plan.json
km --profile home domain apply --plan app.plan.json --yes --format json
km --profile home domain status my-app --format json
km --profile home domain wait my-app --generation 1 --timeout 120s
```

Alternatively, `plan --file domain.json` saves a creation intent without saving a
Domain until apply. Plan files contain a server plan reference, expiration,
generation, request ID and safe summary. Editing a plan file does not alter the
server's steps. Reuse the same file/request ID to retry an interrupted apply.
An expired or stale plan requires a new preview.
Automatic reconciliation revisits previously applied Ready/Degraded bindings only.
Blocked drift is not overwritten automatically. Retryable or interrupted operations
can be resumed using the same server plan and request ID. Shared Tunnel configuration
drift requires restoring the last known configuration; single-object adoption does
not waive whole-configuration drift protection.

Saving a configuration does not publish it. Execute permission only reconciles
unchanged saved state. Creating/updating a binding also requires explicit
`AttachDomain` on a Deployment/Stack and `Attach` on both connections; ordinary
Read/Execute/Write does not implicitly grant exposure permission.

`Accepted` and an Update ID mean the request was accepted, not that the website
is reachable. Check the target generation and all five conditions separately.
`Unknown` is intentional when Core cannot prove DNS, TLS or public reachability.
A timeout or Ctrl-C stops the client wait; the server may continue.

Use `--format json` for a `domain.v1` envelope. Logs and prompts go to stderr.
Noninteractive writes require `--yes`; JSON output never implies consent.
Stdin requires `--file - --input-format json|toml`. Exit codes are 0 success or
accepted, 1 execution/confirmation failure, 2 invalid input/capability, 3 conflict,
4 authorization, and 5 client wait timeout. Legacy commands retain their contract.

## Migration, ownership and removal

```sh
km domain migrate my-app --ingress tunnel-home --out migrate.plan.json
km domain apply --plan migrate.plan.json --yes --wait
km domain finalize-migration my-app --yes --wait
km domain remove my-app --out remove.plan.json
km domain apply --plan remove.plan.json --yes --wait
```

Migration retains old handles until finalization. Certificate bootstrap that needs
DNS cutover may require downtime; the plan must expose that risk. External DNS
requires manual DNS changes and observation. Do not delete a connection while
it is referenced by an active or unfinished migration.

Only created or explicitly adopted objects can be removed. Matching a hostname is
not ownership. Adopt plans name precise object IDs; abandon is an explicit transfer
to manual management. There is no `--force` switch. External drift blocks writes
and deletion. Removal keeps a tombstone while owned DNS and then drained routes
are cleaned up. It never removes a parent wildcard DNS record.

## ResourceSync and recovery

Resource TOML accepts `[[dns_provider]]`, `[[ingress]]`, and `[[domain]]`.
References export as names. Connections save before domains, and target deployments
finish before domain reconciliation. `deploy = false` saves only desired state;
`deploy = true` invokes the same server planner and reconciler using the initiating
user's authority. Missing connection declarations are retained. Managed Domain
removal goes through the owned-object cleanup workflow before deleting dependent
applications. Failed cleanup retains its tombstone and blocks those deletions;
successful sync deletion retains operation audit records.

Database backups enumerate collections, including plans, operation IDs, leases
and ownership. Restore with domain management disabled, review instance identity
and ownership, expire/replan old plans, and observe incomplete operations before
allowing writes. Do not copy only the public Domain collection: internal ownership
and operation records are part of recovery.

## Verification boundary

Unit/contract tests cover local validation and safe failure paths. They do not
prove actual Cloudflare permissions, authoritative DNS, a running Traefik load,
certificate issuance, cloudflared connectivity, or public reachability. Those
checks require a separately authorized test deployment and scoped credentials.

Provider references: [Cloudflare DNS records](https://developers.cloudflare.com/api/resources/dns/subresources/records/methods/create/),
[remote Tunnel API](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/get-started/create-remote-tunnel-api/),
[Traefik file provider](https://doc.traefik.io/traefik/reference/install-configuration/providers/others/file/).

The optional `example/domains/traefik.compose.yaml` is a separate dedicated ingress
Stack template; creating an Ingress resource does not deploy it. For independent
load verification, configure `domain_ingress_observation_urls` on Periphery with
an administrator-selected, protected Traefik API address. The code only reads
`/api/rawdata`, matches the precise generated router/service, and checks their
status. Do not expose that API publicly. A rejected load can restore a previously
verified route only while ownership, fingerprint and fencing remain current.

Adoption is intentionally explicit and currently handles one object per plan:
Cloudflare DNS uses an exact record ID; Tunnel uses the exact hostname of one
observed ingress rule. Arbitrary manual Traefik files cannot be adopted through
this RPC. Unsupported multi-record DNS sets are rejected before a write. These
limits are capability errors, not an implicit takeover or a force override.

Existing resource wire formats remain unchanged. Older clients can continue using
those resources, but clients with strict resource/update enums must upgrade before
reading Domain, DnsProvider, Ingress, or their new update events.
