# First human login with bundled shared Keycloak

Use this after the shared HTTPS installation Job and private readiness pass.
Keep the main guide's `CONTEXT`, `NAMESPACE`, `RELEASE`, `PREPARED` and `APP_URL`.
Run workstation commands in a private terminal without tracing, screen sharing
or captured output. Identity administration is private Pod execution; the
chart's public ingress exposes only the Synveda realm/resources, not `/admin`,
master realm or the administration console. No new public endpoint is required.

The initial Keycloak administration credential is separate from Synveda
administrator authority. Use the chosen `--admin-user` and its unique saved
password only for this provider operation. Only the selected verified human
receives `synveda-admins`. That group's first eligible login admits the initial
Synveda administrator once, with Cedar/RLS and audit; it is not a standing
self-service administrator claim.

## 1. Prepare one selected human

On the workstation, replace the four identity fields with that person's real
provider account information. Email is profile data and grants no authority.
The password below is unique and temporary; the human changes it at first login.

```sh
export FIRST_USERNAME=chosen-human
export FIRST_NAME=Given FIRST_LAST_NAME=Family FIRST_EMAIL=person@example.com
(
  set -eu
  umask 077
  node --input-type=module <<'NODE'
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { pathToFileURL } from 'node:url';
const { privatePath, parseJson } = await import(pathToFileURL(process.env.CHART+'/examples/prepare.mjs'));
privatePath(process.env.PREPARED,true);
const p = process.env.PREPARED + '/first-human.json';
const username = process.env.FIRST_USERNAME;
if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$/.test(username ?? '')) throw Error('select an ordinary exact username');
const fields = { username, firstName: process.env.FIRST_NAME, lastName: process.env.FIRST_LAST_NAME, email: process.env.FIRST_EMAIL };
if (Object.values(fields).some(v => !v)) throw Error('complete all selected human fields');
if (existsSync(p)) {
  privatePath(p);
  const saved = parseJson(readFileSync(p),'private selected human file');
  if (Object.entries(fields).some(([k,v]) => saved[k] !== v)) throw Error('existing human file conflicts; preserve it and select the original identity');
} else writeFileSync(p, JSON.stringify({ ...fields, enabled: true, requiredActions: ['UPDATE_PASSWORD'], credentials: [{ type: 'password', temporary: true, value: randomBytes(32).toString('hex') }] }) + '\n', { mode: 0o600, flag: 'wx' });
console.log('Selected human file ready; no password printed or provider mutation performed.');
NODE
  kubectl --context "$CONTEXT" -n "$NAMESPACE" exec -i "statefulset/$RELEASE-keycloak" -- \
    sh -ec 'umask 077; test ! -e /tmp/synveda-first-human.json; cat > /tmp/synveda-first-human.json' \
    < "$PREPARED/first-human.json"
)
```

Success uploads the protected JSON into the private Pod's temporary storage.
On an interrupted upload, remove only that incomplete `/tmp/synveda-first-human.json`
after confirming no administration session is using it, then repeat the upload
with the same local file. Never regenerate a password on a retry. A changed
human file or uncertain existing account requires the identity owner's review.

Read the saved **Keycloak administrator password** privately, when prompted:

```sh
node -e 'const fs=require("fs");const s=JSON.parse(fs.readFileSync(process.env.PREPARED+"/secrets.json"));process.stdout.write(s.items.find(x=>x.metadata.name===process.env.RELEASE+"-keycloak-admin").stringData.password+"\n")'
```

## 2. Admit exactly that provider identity

Open an interactive private Pod shell from the workstation:

```sh
kubectl --context "$CONTEXT" -n "$NAMESPACE" exec -it "statefulset/$RELEASE-keycloak" -- sh
```

The following commands run **inside that shell**. Enter the same selected
username when requested. Keycloak's native Admin CLI prompts for the
administrator password without putting it into arguments. Do not add
`--password` or print the token configuration. The loopback HTTP connection
stays inside the identity Pod.

```sh
set -eu
umask 077
ADMIN_DIR=$(mktemp -d /tmp/synveda-kcadm.XXXXXX)
CFG="$ADMIN_DIR/kcadm.config"
trap 'rm -f "$CFG" /tmp/synveda-first-human.json; rmdir "$ADMIN_DIR"' EXIT
kcadm() { /opt/keycloak/bin/kcadm.sh "$@" --config "$CFG"; }
kcadm config credentials --server http://127.0.0.1:8080 --realm master --user "$KC_BOOTSTRAP_ADMIN_USERNAME"
printf 'Selected human username: '
read -r HUMAN
HUMAN_ID=$(kcadm get users -r synveda -q "username=$HUMAN" -q exact=true --fields id --format csv --noquotes)
if [ -z "$HUMAN_ID" ]; then
  HUMAN_ID=$(kcadm create users -r synveda -f /tmp/synveda-first-human.json -i)
fi
# A repeated command never updates an existing person's password.
case "$HUMAN_ID" in ''|*[!a-f0-9-]*) echo 'Stop: user lookup must identify one UUID'; exit 1;; esac
test "${#HUMAN_ID}" = 36
GROUP_ID=$(kcadm get groups -r synveda -q search=synveda-admins --fields id --format csv --noquotes)
case "$GROUP_ID" in ''|*[!a-f0-9-]*) echo 'Stop: bootstrap group lookup must identify one UUID'; exit 1;; esac
test "${#GROUP_ID}" = 36
kcadm get "users/$HUMAN_ID" -r synveda --fields id,username,enabled
kcadm get "groups/$GROUP_ID" -r synveda --fields id,name,path
kcadm get "groups/$GROUP_ID/members" -r synveda --fields id,username
```

Verify the username/UUID is the chosen person, the group is exactly
`/synveda-admins`, and its members are empty or only that same UUID from an
interrupted first-login attempt. **Stop if another identity is present.** The
identity owner must resolve that mismatch; never add a second bootstrap user.
Once these checks match, run inside the same private shell:

```sh
kcadm update "users/$HUMAN_ID/groups/$GROUP_ID" -r synveda -n
printf 'Selected stable provider subject: %s\n' "$HUMAN_ID"
```

Keep this shell open for step 4. The subject is non-secret; record it in the
operator handoff so the Synveda identity can be compared exactly. Do not delete
or recreate that provider account during recovery.

## 3. Sign in and verify Synveda authority

On the workstation, retrieve only the selected human's temporary password:

```sh
node -e 'const fs=require("fs");process.stdout.write(JSON.parse(fs.readFileSync(process.env.PREPARED+"/first-human.json")).credentials[0].value+"\n")'
```

Open `$APP_URL/console/` in a fresh private browser session. Sign in as the
selected username and change that temporary password as requested. Getting
started should permit creation of the first workspace. Install a CLI matching
the server, then verify through the authenticated public API:

```sh
synveda login --gateway "$APP_URL" --profile first-human --no-browser
synveda whoami --profile first-human --capabilities --json
```

Success names the prepared tenant, the exact recorded provider subject and
allowed tenant administration capabilities. Check People/Access for the
initial tenant administrator grant and Advanced Audit for bootstrap admission.
Readiness and a Keycloak group claim alone are insufficient. If login succeeds
without those capabilities, stop and diagnose the tenant/issuer/group binding;
do not create an extra administrator or rebind identity by email.

## 4. Remove temporary bootstrap membership

After the subject and durable Synveda authority match, run in the still-open
private Pod shell:

```sh
kcadm delete "users/$HUMAN_ID/groups/$GROUP_ID" -r synveda
kcadm get "groups/$GROUP_ID/members" -r synveda --fields id,username
exit
```

Success shows no bootstrap group members and deletes the temporary token/JSON
files on exit. Sign out of Synveda, close provider SSO/private browser state,
then sign in freshly and repeat `whoami --capabilities`. The existing durable
Synveda grant should remain. Removing this one-time provider membership does
not revoke a Synveda grant; subsequent access changes use governed People/Access.
Invite other people through the product, without adding them to this group.

The Keycloak bootstrap administrator is also temporary provider access. Before
retiring it, the identity owner must establish and test a separately protected
named provider administration/recovery identity using the
[Keycloak administration procedure](https://www.keycloak.org/docs/latest/server_admin/#admin-cli).
Retirement of that provider administrator is separate from the completed
Synveda group removal. Preserve the full identity database, stable subjects,
signing keys, configuration and chosen credential custody in the recovery set.

For TLS/redirect/group failures, follow
[installation-stage recovery](OPERATIONS.md#installation-stage-recovery).
External OIDC administrators perform the equivalent identity/group assignment
in their existing provider using the [worksheet](CONFIGURATION.md#external-oidc-worksheet).
This candidate procedure still needs a fresh-identity live cluster qualification;
no measured first-sign-in time is claimed.
