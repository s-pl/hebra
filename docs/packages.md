# Packages, releases and webhooks

## npm packages

The server is an npm registry at `/npm/`, for packages scoped by an account
(`@acme/ui` belongs to `acme`). Publishing needs a mandate of someone who can
publish for that account; installing a public package needs nothing.

```sh
npm config set @acme:registry https://hebra.example.com/npm/
npm config set //hebra.example.com/npm/:_authToken $HB_TOKEN
npm publish
npm install @acme/ui
npm dist-tag add @acme/ui@1.4.0 stable
npm unpublish @acme/ui@1.3.0
```

Or in `.npmrc`:

```ini
@acme:registry=https://hebra.example.com/npm/
//hebra.example.com/npm/:_authToken=${HB_TOKEN}
```

## Container images

The server speaks the OCI distribution API at `/v2/`: Docker, Podman, crane
and the rest. Log in with any user name and a mandate as the password.

```sh
docker login hebra.example.com -u acme -p $HB_TOKEN
docker tag app hebra.example.com/acme/app:1.0
docker push hebra.example.com/acme/app:1.0
docker pull hebra.example.com/acme/app:1.0
```

Chunked and monolithic uploads, layers mounted from other images, image
indexes (multi-platform images), tags and `_catalog` work. Layers are stored
once, by digest.

Docker only talks to registries over HTTPS. For a server on plain HTTP (a
test on your network), add it to `insecure-registries` in Docker's
`daemon.json`.

## Who can install

New packages are private: people and agents with access to the owner can
install them. In the hub (the owner → Packages → a package) an administrator
makes one public, describes it, or removes versions.

## Releases

A release is a pushed tag with notes and files.

```sh
hb tag -a v1.4.0 -m "Faster clones" && hb push --tags
```

Then **Releases → New release** in the hub, or the API:

```sh
curl -X POST -H "Authorization: Bearer $HB_TOKEN" -H "Content-Type: application/json" \
  -d '{"tag": "v1.4.0", "title": "Faster clones", "notes": "…"}' \
  https://hebra.example.com/api/v1/repos/acme/app/releases

# Files, from a check or a script (up to 4 GB each):
curl -T hb-x86_64-linux.tar.gz -H "Authorization: Bearer $HB_TOKEN" \
  https://hebra.example.com/api/v1/repos/acme/app/releases/v1.4.0/assets/hb-x86_64-linux.tar.gz
```

Drafts are only visible to people who can write. Downloads are counted.

## Webhooks

Repository settings → Webhooks. Each event is a `POST` with JSON:

```json
{ "event": "push", "repo": "acme/app", "data": { … }, "at": "2026-11-04T17:30:00Z" }
```

Events: `push` (a bookmark moved), `review`, `task`, `check`, `release`, and
`ping` when you press Ping.

Headers: `X-Hebra-Event`, `X-Hebra-Delivery` (an id) and
`X-Hebra-Signature-256: sha256=<hex>`, the HMAC-SHA256 of the raw body with
the webhook's secret. Check it before trusting a delivery:

```js
const expected = "sha256=" + crypto.createHmac("sha256", secret).update(rawBody).digest("hex");
const ok = crypto.timingSafeEqual(Buffer.from(expected), Buffer.from(req.headers["x-hebra-signature-256"]));
```

A delivery that fails is tried three times, waiting longer each time. Every
try is kept with its status and time, and shown under the webhook.
