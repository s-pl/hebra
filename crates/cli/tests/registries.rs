//! The package registries over HTTP, as npm and container clients speak to
//! them: publishing, installing, tags, access, and container uploads in
//! one piece and in chunks.

mod common;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

struct Http {
    agent: ureq::Agent,
    base: String,
}

impl Http {
    fn new(base: &str) -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build()
                .into(),
            base: base.to_owned(),
        }
    }

    /// `(status, headers we care about, body)`.
    fn call(
        &self,
        method: &str,
        path: &str,
        auth: Option<&str>,
        content_type: Option<&str>,
        body: Option<Vec<u8>>,
    ) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let url = format!("{}{path}", self.base);
        let mut resp = match (method, body) {
            ("GET" | "HEAD" | "DELETE", _) => {
                let mut req = match method {
                    "GET" => self.agent.get(&url),
                    "HEAD" => self.agent.head(&url),
                    _ => self.agent.delete(&url),
                };
                if let Some(a) = auth {
                    req = req.header("Authorization", a);
                }
                req.call().unwrap()
            }
            (m, body) => {
                let mut req = match m {
                    "POST" => self.agent.post(&url),
                    "PUT" => self.agent.put(&url),
                    "PATCH" => self.agent.patch(&url),
                    other => panic!("{other}"),
                };
                if let Some(a) = auth {
                    req = req.header("Authorization", a);
                }
                if let Some(c) = content_type {
                    req = req.header("Content-Type", c);
                }
                req.send(&body.unwrap_or_default()[..]).unwrap()
            }
        };
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_owned(),
                    v.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        let bytes = resp.body_mut().read_to_vec().unwrap_or_default();
        (status, headers, bytes)
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(T[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn sha(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn npm_packages_publish_install_tag_and_stay_private() {
    let env = common::start_server();
    let http = Http::new(&env.server);
    let ana = env.ana().to_owned();
    let name = format!("@{ana}/ui");
    let bearer = format!("Bearer {}", env.human);
    let tarball = b"\x1f\x8bpretend this is a tarball".to_vec();
    let publish = |version: &str, tag: &str| {
        json!({
            "_id": name, "name": name, "description": "buttons",
            "dist-tags": { tag: version },
            "versions": { version: { "name": name, "version": version, "dist": { "shasum": "x" } } },
            "_attachments": { format!("ui-{version}.tgz"): { "content_type": "application/octet-stream", "data": b64(&tarball), "length": tarball.len() } }
        })
        .to_string()
        .into_bytes()
    };
    let path = format!("/npm/@{ana}%2fui");
    let (status, _, body) = http.call(
        "PUT",
        &path,
        Some(&bearer),
        Some("application/json"),
        Some(publish("1.0.0", "latest")),
    );
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    // A version is never replaced.
    let (status, ..) = http.call(
        "PUT",
        &path,
        Some(&bearer),
        Some("application/json"),
        Some(publish("1.0.0", "latest")),
    );
    assert_eq!(status, 403);
    let (status, ..) = http.call(
        "PUT",
        &path,
        Some(&bearer),
        Some("application/json"),
        Some(publish("1.1.0", "next")),
    );
    assert_eq!(status, 200);

    let (status, _, body) = http.call("GET", &format!("/npm/@{ana}/ui"), Some(&bearer), None, None);
    assert_eq!(status, 200);
    let doc: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(doc["dist-tags"]["latest"], "1.0.0");
    assert_eq!(doc["dist-tags"]["next"], "1.1.0");
    let url = doc["versions"]["1.0.0"]["dist"]["tarball"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        url.ends_with(&format!("/npm/@{ana}/ui/-/ui-1.0.0.tgz")),
        "{url}"
    );
    let (status, _, got) = http.call(
        "GET",
        &url[url.find("/npm/").unwrap()..],
        Some(&bearer),
        None,
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(got, tarball);

    // dist-tags move; `npm whoami` knows who.
    let (status, ..) = http.call(
        "PUT",
        &format!("/npm/-/package/@{ana}%2fui/dist-tags/latest"),
        Some(&bearer),
        Some("application/json"),
        Some(b"\"1.1.0\"".to_vec()),
    );
    assert_eq!(status, 200);
    let (_, _, who) = http.call("GET", "/npm/-/whoami", Some(&bearer), None, None);
    assert_eq!(
        serde_json::from_slice::<Value>(&who).unwrap()["username"],
        ana
    );

    // Private: nobody else sees it, until it is public.
    let (status, ..) = http.call(
        "GET",
        &format!("/npm/@{ana}/ui"),
        Some(&format!("Bearer {}", env.bob)),
        None,
        None,
    );
    assert_eq!(status, 404);
    let (status, ..) = http.call("GET", &format!("/npm/@{ana}/ui"), None, None, None);
    assert_eq!(status, 404);
    let (status, _) = env.api(
        "PATCH",
        &format!("/accounts/{ana}/package"),
        &env.human,
        Some(json!({ "kind": "npm", "name": name, "public": true })),
    );
    assert_eq!(status, 200);
    let (status, ..) = http.call("GET", &format!("/npm/@{ana}/ui"), None, None, None);
    assert_eq!(status, 200);
    // Others still cannot publish.
    let (status, ..) = http.call(
        "PUT",
        &path,
        Some(&format!("Bearer {}", env.bob)),
        Some("application/json"),
        Some(publish("2.0.0", "latest")),
    );
    assert_eq!(status, 403);

    let (_, listed) = env.api(
        "GET",
        &format!("/accounts/{ana}/packages"),
        &env.human,
        None,
    );
    assert_eq!(listed["packages"][0]["versions"], 2, "{listed}");
}

#[test]
fn container_images_push_in_chunks_and_pull_by_tag() {
    let env = common::start_server();
    let http = Http::new(&env.server);
    let ana = env.ana().to_owned();
    let image = format!("{ana}/app");
    let basic = format!("Basic {}", b64(format!("{ana}:{}", env.human).as_bytes()));

    // Docker asks first, and is told to sign in.
    let (status, headers, _) = http.call("GET", "/v2/", None, None, None);
    assert_eq!(status, 401);
    assert!(
        header(&headers, "www-authenticate")
            .unwrap()
            .starts_with("Basic")
    );
    let (status, ..) = http.call("GET", "/v2/", Some(&basic), None, None);
    assert_eq!(status, 200);

    // A layer in two chunks.
    let layer: Vec<u8> = (0..300_000u32)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let (status, headers, _) = http.call(
        "POST",
        &format!("/v2/{image}/blobs/uploads/"),
        Some(&basic),
        None,
        Some(Vec::new()),
    );
    assert_eq!(status, 202);
    let location = header(&headers, "location").unwrap().to_owned();
    let (status, headers, _) = http.call(
        "PATCH",
        &location,
        Some(&basic),
        Some("application/octet-stream"),
        Some(layer[..100_000].to_vec()),
    );
    assert_eq!(status, 202);
    assert_eq!(header(&headers, "range"), Some("0-99999"));
    let digest = sha(&layer);
    let (status, headers, _) = http.call(
        "PUT",
        &format!("{location}?digest={digest}"),
        Some(&basic),
        Some("application/octet-stream"),
        Some(layer[100_000..].to_vec()),
    );
    assert_eq!(status, 201);
    assert_eq!(
        header(&headers, "docker-content-digest"),
        Some(digest.as_str())
    );

    // The config in one piece.
    let config = br#"{"architecture":"amd64","os":"linux"}"#.to_vec();
    let config_digest = sha(&config);
    let (status, ..) = http.call(
        "POST",
        &format!("/v2/{image}/blobs/uploads/?digest={config_digest}"),
        Some(&basic),
        Some("application/octet-stream"),
        Some(config.clone()),
    );
    assert_eq!(status, 201);

    // A manifest pointing to a blob that is not there is refused.
    let media = "application/vnd.oci.image.manifest.v1+json";
    let manifest = |layer_digest: &str| {
        json!({
            "schemaVersion": 2, "mediaType": media,
            "config": { "mediaType": "application/vnd.oci.image.config.v1+json", "digest": config_digest, "size": config.len() },
            "layers": [{ "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip", "digest": layer_digest, "size": layer.len() }]
        })
        .to_string()
        .into_bytes()
    };
    let missing = sha(b"never pushed");
    let (status, _, body) = http.call(
        "PUT",
        &format!("/v2/{image}/manifests/broken"),
        Some(&basic),
        Some(media),
        Some(manifest(&missing)),
    );
    assert_eq!(status, 400);
    assert!(String::from_utf8_lossy(&body).contains("MANIFEST_BLOB_UNKNOWN"));
    let good = manifest(&digest);
    let (status, headers, _) = http.call(
        "PUT",
        &format!("/v2/{image}/manifests/1.0"),
        Some(&basic),
        Some(media),
        Some(good.clone()),
    );
    assert_eq!(status, 201);
    assert_eq!(
        header(&headers, "docker-content-digest"),
        Some(sha(&good).as_str())
    );

    // Pull it back by tag and by digest.
    let (status, headers, body) = http.call(
        "GET",
        &format!("/v2/{image}/manifests/1.0"),
        Some(&basic),
        None,
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), Some(media));
    assert_eq!(body, good);
    let (status, headers, _) = http.call(
        "HEAD",
        &format!("/v2/{image}/blobs/{digest}"),
        Some(&basic),
        None,
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(
        header(&headers, "content-length"),
        Some(layer.len().to_string().as_str())
    );
    let (_, _, got) = http.call(
        "GET",
        &format!("/v2/{image}/blobs/{digest}"),
        Some(&basic),
        None,
        None,
    );
    assert_eq!(got, layer);
    let (_, _, tags) = http.call(
        "GET",
        &format!("/v2/{image}/tags/list"),
        Some(&basic),
        None,
        None,
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&tags).unwrap()["tags"],
        json!(["1.0"])
    );

    // Another image of the same owner mounts the layer without uploading it.
    let (status, ..) = http.call(
        "POST",
        &format!("/v2/{ana}/worker/blobs/uploads/?mount={digest}&from={image}"),
        Some(&basic),
        None,
        Some(Vec::new()),
    );
    assert_eq!(status, 201);

    // Private: anonymous pulls are asked to sign in, others are told no.
    let (status, ..) = http.call(
        "GET",
        &format!("/v2/{image}/manifests/1.0"),
        None,
        None,
        None,
    );
    assert_eq!(status, 401);
    let bob = format!("Basic {}", b64(format!("bob:{}", env.bob).as_bytes()));
    let (status, ..) = http.call(
        "GET",
        &format!("/v2/{image}/manifests/1.0"),
        Some(&bob),
        None,
        None,
    );
    assert_eq!(status, 404);
    let (status, ..) = http.call(
        "POST",
        &format!("/v2/{image}/blobs/uploads/"),
        Some(&bob),
        None,
        Some(Vec::new()),
    );
    assert_eq!(status, 403);
}
