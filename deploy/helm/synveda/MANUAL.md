# Manual preparation with ordinary Helm values

The recommended preparation entry point is `examples/prepare.mjs`. This manual
local-evaluation equivalent makes the same Helm/Secret inputs explicitly,
without Docker or a new deployment model. Use it **instead of** automatic
preparation, in a separate private directory. It does not create automatic
`state.json`; use the explicit Helm/kubectl checks below rather than pointing
the state-based operator utility at this directory.

Run on the workstation with the main guide's exported `CHART`, `IMAGES`,
`CONTEXT`, `NAMESPACE`, `RELEASE`, `STORAGE_CLASS`, `APP_PORT`, `IDENTITY_PORT`.
The chart and overlay must have passed publisher/checksum verification. Native
Node 22+, OpenSSL, Helm and kubectl are the same dependencies as the main path.
For shared installations, use the HTTPS presets/worksheets and supplied TLS
and identity references instead of this loopback-only credential recipe.

## Generate private material once

Choose a new private directory; do not use an existing preparation's files.
The block refuses conflicting selectors, incomplete material and changed
credentials. On retry it validates and reuses every existing file. Its small
filesystem and UUID helpers are from that same verified chart, not downloaded
separately. No resource is applied.

```sh
export MANUAL="${MANUAL:-$HOME/.synveda-manual-$RELEASE}"
(
  set -eu
  umask 077
  test -d "$MANUAL" || mkdir -m 700 "$MANUAL"
  node --input-type=module <<'NODE'
import { existsSync, readFileSync } from 'node:fs';
import { randomBytes, createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';
const { privatePath, atomicPrivate, uuid7, json, imagePins } = await import(pathToFileURL(process.env.CHART+'/examples/prepare.mjs'));
privatePath(process.env.MANUAL, true);
const input = Object.fromEntries(['CONTEXT','NAMESPACE','RELEASE','STORAGE_CLASS','APP_PORT','IDENTITY_PORT'].map(k => [k,process.env[k]]));
if (Object.values(input).some(v => !v)) throw Error('export all selected inputs before preparation');
for (const key of ['NAMESPACE','RELEASE']) if (!/^[a-z][a-z0-9-]{0,29}$/.test(input[key])) throw Error('namespace/release must be DNS labels up to 30 characters');
for (const key of ['APP_PORT','IDENTITY_PORT']) if (!/^[0-9]+$/.test(input[key]) || +input[key]<1024 || +input[key]>65535) throw Error('ports must be 1024 through 65535');
if (input.APP_PORT===input.IDENTITY_PORT) throw Error('choose different loopback ports');
const images=readFileSync(process.env.IMAGES,'utf8');
imagePins(images,{database:'bundled',identity:'bundled'});
input.imageHash=createHash('sha256').update(images).digest('hex');
const file=n=>process.env.MANUAL+'/'+n;
const ledger=file('selected-inputs.json');
if (existsSync(ledger)) { privatePath(ledger); if (readFileSync(ledger,'utf8')!==json(input)) throw Error('selected inputs conflict; restore the original context/release/namespace/ports/storage/image evidence'); }
else atomicPrivate(ledger,json(input));
const names=['tenant-id','postgres-password','migrator-password','gateway-password','worker-password','keycloak-password','kms-key','admin-password','member-password','approver-password','viewer-password'];
const count=names.filter(n=>existsSync(file(n))).length;
if (count!==0 && count!==names.length) throw Error('incomplete private material: preserve files and restore their matching recovery copy; never replace credentials for an applied installation');
for (const n of names) {
  if (!existsSync(file(n))) atomicPrivate(file(n),(n==='tenant-id'?uuid7():randomBytes(32).toString('hex'))+'\n');
  privatePath(file(n));
  const value=readFileSync(file(n),'utf8');
  if (!(n==='tenant-id'?/^[a-f0-9-]{36}\n$/:/^[a-f0-9]{64}\n$/).test(value)) throw Error('private material incomplete: restore original '+n);
}
console.log('Private identifiers and credentials ready; no values printed.');
NODE
  cd "$MANUAL"
  if test -e ca.key || test -e ca.crt || test -e server.key || test -e server.crt; then
    test -s ca.key && test -s ca.crt && test -s server.key && test -s server.crt || {
      echo 'Incomplete TLS files: stop and restore the original matching set; do not regenerate'; exit 1;
    }
  else
    printf '[req]\ndistinguished_name=dn\n[dn]\nCN=Synveda database CA\n[ca]\nbasicConstraints=critical,CA:true\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\n' > ca-config
    openssl req -x509 -newkey rsa:3072 -nodes -days 365 -subj '/CN=Synveda database CA' -config ca-config -extensions ca -keyout ca.key -out ca.crt 2>/dev/null
    openssl req -new -newkey rsa:3072 -nodes -subj "/CN=$RELEASE-pg-rw" -keyout server.key -out server.csr 2>/dev/null
    printf 'subjectAltName=DNS:%s-pg-rw,DNS:%s-pg-rw.%s.svc,DNS:%s-pg-rw.%s.svc.cluster.local\nextendedKeyUsage=serverAuth\n' \
      "$RELEASE" "$RELEASE" "$NAMESPACE" "$RELEASE" "$NAMESPACE" > extensions
    openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -days 365 -extfile extensions -out server.crt 2>/dev/null
  fi
  openssl verify -CAfile ca.crt -purpose sslserver server.crt
  node --input-type=module <<'NODE'
import { readFileSync } from 'node:fs';
import { X509Certificate } from 'node:crypto';
import { pathToFileURL } from 'node:url';
const { privatePath }=await import(pathToFileURL(process.env.CHART+'/examples/prepare.mjs'));
for (const file of ['ca.key','ca.crt','server.key','server.crt']) privatePath(process.env.MANUAL+'/'+file);
const cert=new X509Certificate(readFileSync('server.crt'));
if (!cert.checkHost(process.env.RELEASE+'-pg-rw')) throw Error('database SAN mismatch; restore matching certificate files');
console.log('Database hostname/SAN matches.');
NODE
)
```

Success reports `server.crt: OK` and a matching database hostname/SAN. No implicit reset/rotation occurs. If an
interruption leaves incomplete TLS/credential files, stop before applying
anything; preserve the directory and recover the original set. For a never
applied evaluation only, deliberate abandonment into a new directory is an
operator choice, not an installation recovery command.

## Build the normal values and protected Secret list

The size below is an explicit bundled PostgreSQL decision. Change `20Gi` once
before the first preparation; retained claims require maintenance/expansion
review. This block writes outputs only when absent or exactly matching. It
refuses replacing modified output files.

```sh
(
  set -eu
  umask 077
  node --input-type=module <<'NODE'
import { existsSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
const { atomicPrivate, privatePath, json } = await import(pathToFileURL(process.env.CHART+'/examples/prepare.mjs'));
const d=process.env.MANUAL, r=process.env.RELEASE, ns=process.env.NAMESPACE;
const read=n=>{privatePath(d+'/'+n);return readFileSync(d+'/'+n,'utf8').trimEnd();};
const tenant=read('tenant-id'), app='http://localhost:'+process.env.APP_PORT, identity='http://localhost:'+process.env.IDENTITY_PORT;
const v=JSON.parse(readFileSync(process.env.CHART+'/examples/bundled-database.json','utf8'));
v.keycloak=JSON.parse(readFileSync(process.env.CHART+'/examples/bundled-identity.json','utf8')).keycloak;
v.fullnameOverride=r;
v.gateway={...v.gateway,publicUrl:app,insecureDevelopmentHttp:true,databaseExistingSecret:r+'-gateway-db'};
v.worker.databaseExistingSecret=r+'-worker-db'; v.oidc.existingSecret=r+'-oidc';v.kms.existingSecret=r+'-kms';
v.install={tenant:{id:tenant,slug:r,name:r}};
Object.assign(v.postgres.bundled,{administratorExistingSecret:r+'-postgres-admin',migratorExistingSecret:r+'-migrator-db',tlsExistingSecret:r+'-pg-ca',storageClass:process.env.STORAGE_CLASS,size:'20Gi'});
Object.assign(v.keycloak,{publicUrl:identity,localEvaluation:true,proxyTrustedAddresses:'127.0.0.1/32',adminExistingSecret:'',evaluationAccountsExistingSecret:r+'-evaluation-accounts',databaseCaExistingSecret:r+'-pg-ca'});
Object.assign(v.keycloak.database,{hostname:r+'-pg-rw',existingSecret:r+'-keycloak-db'});
const items=[];
const secret=(suffix,stringData)=>items.push({apiVersion:'v1',kind:'Secret',metadata:{name:r+'-'+suffix,namespace:ns},type:'Opaque',stringData});
for (const role of ['gateway','worker','migrator']) {
  const password=read(role+'-password');const url='postgresql://synveda_'+role+':'+password+'@'+r+'-pg-rw:5432/synveda?sslmode=verify-full&sslrootcert=/run/secrets/synveda-postgres/ca.crt';
  secret(role+'-db',{password,[role==='migrator'?'uri':'DATABASE_URL']:url});
}
secret('postgres-admin',{password:read('postgres-password')});secret('keycloak-db',{password:read('keycloak-password')});
secret('pg-ca',{'tls.crt':read('server.crt'),'tls.key':read('server.key'),'ca.crt':read('ca.crt')});
secret('kms',{SYNVEDA_KMS_KEY:read('kms-key'),SYNVEDA_KMS_KEY_REF:r+':primary-v1'});
secret('oidc',{SYNVEDA_OIDC_ISSUERS:JSON.stringify([{issuer:identity+'/realms/synveda',discovery_url:'http://'+r+'-keycloak-http:80/realms/synveda/.well-known/openid-configuration',client_id:'synveda',audience:'synveda-api',algorithms:['RS256'],groups_claim:'groups',tenant:{static:{tenant_id:tenant}}}])});
secret('evaluation-accounts',Object.fromEntries(['admin','member','approver','viewer'].map(k=>[k,read(k+'-password')])));
for (const [name,bytes] of Object.entries({'values.json':json(v),'secrets.json':json({apiVersion:'v1',kind:'List',items}),'release-images.yaml':readFileSync(process.env.IMAGES,'utf8')})) {
  const p=d+'/'+name;
  if (existsSync(p)) {privatePath(p);if (readFileSync(p,'utf8')!==bytes) throw Error(name+' conflicts; restore the original matching output');}
  else atomicPrivate(p,bytes);
}
console.log('Ordinary Helm values and protected Secret list ready.');
NODE
)
```

Inspect only `values.json` for normal review. Keep every private file, including
CA key/credentials, with database and identity recovery custody. On repeated
Secret creation, compare all keys first and create missing Secrets only:

```sh
node --input-type=module <<'NODE'
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
const { privatePath, parseJson }=await import(pathToFileURL(process.env.CHART+'/examples/prepare.mjs'));
privatePath(process.env.MANUAL,true); privatePath(process.env.MANUAL+'/secrets.json');
const items=parseJson(readFileSync(process.env.MANUAL+'/secrets.json'),'protected Secret list').items;
const args=['--context',process.env.CONTEXT,'-n',process.env.NAMESPACE,'--request-timeout=15s'];
const missing=[];
for (const item of items) {
  if (item.metadata.namespace!==process.env.NAMESPACE) throw Error('namespace mismatch; stop');
  const r=spawnSync('kubectl',[...args,'get','secret',item.metadata.name,'--ignore-not-found=true','-o','json'],{encoding:'utf8',timeout:20000});
  if (r.status!==0) throw Error('cannot inspect Secret; correct namespaced RBAC and retry');
  if (!r.stdout.trim()) {missing.push(item);continue;}
  const existing=parseJson(r.stdout,'Secret API response');
  if (Object.entries(item.stringData).some(([k,v])=>existing.data?.[k]!==Buffer.from(v).toString('base64'))) throw Error('existing Secret conflicts with recovery material; no changes applied');
}
if (missing.length) {
  const r=spawnSync('kubectl',[...args,'create','-f','-'],{input:JSON.stringify({apiVersion:'v1',kind:'List',items:missing}),encoding:'utf8',timeout:20000});
  if (r.status!==0) throw Error('partial/refused creation; preserve credentials and retry this same block');
}
console.log('Original Secrets match; no existing value replaced.');
NODE
helm lint "$CHART" --strict -f "$MANUAL/values.json" -f "$MANUAL/release-images.yaml"
helm template "$RELEASE" "$CHART" -n "$NAMESPACE" -f "$MANUAL/values.json" -f "$MANUAL/release-images.yaml" > "$MANUAL/rendered.yaml"
```

Before installation perform the main guide's context/RBAC/quota/limits,
StorageClass and TLS/issuer checks with your administrator; the manual path
has no extra automatic preflight. Then use its exact Helm command with
`MANUAL` instead of `PREPARED`. Success and failing install stages are identical.
Local forwards and evaluation login use the same release-derived names/ports.
Retained reinstall reuses the original Secret files and existing bundled claim;
never rerun random-password or CA generation against retained data.
