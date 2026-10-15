//! Stripe: subscriptions through Checkout, cards and invoices through the
//! Customer Portal, subscription state through signed webhooks, and usage
//! above the allowance through Billing Meter events. Card data never reaches
//! Hebra.

use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;

use super::{Plan, Plans};
use crate::config::Stripe;
use crate::db::{Account, AccountKind, Db};
use crate::error::{ApiError, ApiResult};

const API: &str = "https://api.stripe.com/v1";
/// Meter that bills agent credits above a plan's allowance.
pub const CREDITS_METER: &str = "hebra_agent_credits";
/// Webhooks older than this are rejected (replay protection).
const TOLERANCE_SECS: i64 = 300;

async fn call(
    http: &reqwest::Client,
    key: &str,
    path: &str,
    form: &[(&str, String)],
) -> ApiResult<Value> {
    let resp = http
        .post(format!("{API}{path}"))
        .basic_auth(key, None::<&str>)
        .form(form)
        .send()
        .await
        .map_err(|e| ApiError::internal("stripe", e))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| ApiError::internal("stripe", e))?;
    if !status.is_success() {
        return Err(ApiError::internal(
            "stripe",
            body["error"]["message"]
                .as_str()
                .unwrap_or("request failed"),
        ));
    }
    Ok(body)
}

/// The Stripe customer of an owner, created on first use.
async fn customer(
    http: &reqwest::Client,
    stripe: &Stripe,
    db: &Db,
    owner: &Account,
    email: Option<&str>,
) -> ApiResult<String> {
    if let Some(id) = db
        .subscription(&owner.id)
        .await?
        .and_then(|s| s.stripe_customer)
    {
        return Ok(id);
    }
    let mut form = vec![
        ("name", owner.name.clone()),
        ("metadata[owner_id]", owner.id.clone()),
        ("metadata[handle]", owner.handle.clone()),
    ];
    if let Some(email) = email {
        form.push(("email", email.to_owned()));
    }
    let created = call(http, &stripe.secret_key, "/customers", &form).await?;
    let id = created["id"]
        .as_str()
        .ok_or_else(|| ApiError::internal("stripe", "customer without id"))?
        .to_owned();
    db.upsert_subscription(&owner.id, "free", "free", Some(&id), None, 1, None, None)
        .await?;
    Ok(id)
}

/// What a Checkout page sells.
pub struct Purchase<'a> {
    pub owner: &'a Account,
    pub email: Option<&'a str>,
    pub plan: &'a Plan,
    /// People to pay for (per-person plans).
    pub people: i64,
    /// Where Stripe sends the customer back.
    pub return_url: &'a str,
}

/// A Checkout page that subscribes `owner` to `plan`.
pub async fn checkout(
    http: &reqwest::Client,
    stripe: &Stripe,
    db: &Db,
    p: &Purchase<'_>,
) -> ApiResult<String> {
    let Purchase {
        owner,
        email,
        plan,
        people,
        return_url,
    } = *p;
    let price = plan.stripe_price.as_deref().ok_or_else(|| {
        ApiError::bad(format!(
            "the {} plan cannot be bought online; contact us",
            plan.name
        ))
    })?;
    let customer = customer(http, stripe, db, owner, email).await?;
    let quantity = if plan.per_person { people.max(1) } else { 1 };
    let session = call(
        http,
        &stripe.secret_key,
        "/checkout/sessions",
        &[
            ("mode", "subscription".to_owned()),
            ("customer", customer),
            ("client_reference_id", owner.id.clone()),
            ("line_items[0][price]", price.to_owned()),
            ("line_items[0][quantity]", quantity.to_string()),
            ("subscription_data[metadata][owner_id]", owner.id.clone()),
            ("subscription_data[metadata][plan]", plan.id.clone()),
            ("automatic_tax[enabled]", "true".to_owned()),
            ("customer_update[address]", "auto".to_owned()),
            ("allow_promotion_codes", "true".to_owned()),
            ("success_url", format!("{return_url}?checkout=done")),
            ("cancel_url", format!("{return_url}?checkout=cancelled")),
        ],
    )
    .await?;
    session["url"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ApiError::internal("stripe", "checkout session without url"))
}

/// The Customer Portal (cards, invoices, cancellation) for `owner`.
pub async fn portal(
    http: &reqwest::Client,
    stripe: &Stripe,
    db: &Db,
    owner: &Account,
    return_url: &str,
) -> ApiResult<String> {
    let customer = customer(http, stripe, db, owner, None).await?;
    let session = call(
        http,
        &stripe.secret_key,
        "/billing_portal/sessions",
        &[
            ("customer", customer),
            ("return_url", return_url.to_owned()),
        ],
    )
    .await?;
    session["url"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ApiError::internal("stripe", "portal session without url"))
}

/// Checks a webhook's `Stripe-Signature` header (`t=…,v1=…`): HMAC-SHA256
/// of `t.payload`, compared in constant time, and not older than five
/// minutes.
pub fn verify_signature(secret: &str, header: &str, payload: &[u8], now: i64) -> bool {
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for part in header.split(',') {
        match part.split_once('=') {
            Some(("t", t)) => timestamp = t.parse::<i64>().ok(),
            Some(("v1", s)) => signatures.extend(hex::decode(s).ok()),
            _ => {}
        }
    }
    let Some(t) = timestamp else { return false };
    if (now - t).abs() > TOLERANCE_SECS {
        return false;
    }
    signatures.iter().any(|sig| {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
            return false;
        };
        mac.update(t.to_string().as_bytes());
        mac.update(b".");
        mac.update(payload);
        mac.verify_slice(sig).is_ok()
    })
}

/// Applies a verified webhook event. Unknown events are ignored.
pub async fn handle_event(db: &Db, plans: &Plans, event: &Value) -> ApiResult<()> {
    let id = event["id"].as_str().unwrap_or_default();
    if id.is_empty() || !db.first_time_event(id).await? {
        return Ok(());
    }
    let object = &event["data"]["object"];
    match event["type"].as_str().unwrap_or_default() {
        "customer.subscription.created"
        | "customer.subscription.updated"
        | "customer.subscription.deleted" => {
            let Some(owner_id) = object["metadata"]["owner_id"].as_str() else {
                return Ok(());
            };
            let item = &object["items"]["data"][0];
            let price = item["price"]["id"].as_str().unwrap_or_default();
            let plan = plans
                .plans
                .iter()
                .find(|p| p.stripe_price.as_deref() == Some(price))
                .map_or("free", |p| p.id.as_str());
            let status = object["status"].as_str().unwrap_or("canceled");
            let deleted = event["type"] == "customer.subscription.deleted";
            db.upsert_subscription(
                owner_id,
                if deleted { "free" } else { plan },
                if deleted { "canceled" } else { status },
                object["customer"].as_str(),
                object["id"].as_str(),
                item["quantity"].as_i64().unwrap_or(1),
                item["current_period_start"].as_i64().map(|s| s * 1000),
                item["current_period_end"].as_i64().map(|s| s * 1000),
            )
            .await?;
            tracing::info!(owner_id, plan, status, "subscription updated");
        }
        "invoice.payment_failed" => {
            tracing::warn!(
                customer = object["customer"].as_str(),
                "payment failed; Stripe retries"
            );
        }
        _ => {}
    }
    Ok(())
}

/// Reports usage above allowances to Stripe meters (idempotent by event id).
pub async fn report_usage(http: &reqwest::Client, stripe: &Stripe, db: &Db) -> ApiResult<usize> {
    let mut sent = 0;
    for (id, owner_id, _metric, quantity) in
        db.unreported_usage("agent_credits_overage", 500).await?
    {
        let Some(customer) = db
            .subscription(&owner_id)
            .await?
            .and_then(|s| s.stripe_customer)
        else {
            db.mark_usage_reported(&id).await?;
            continue;
        };
        call(
            http,
            &stripe.secret_key,
            "/billing/meter_events",
            &[
                ("event_name", CREDITS_METER.to_owned()),
                ("identifier", id.clone()),
                ("payload[stripe_customer_id]", customer),
                ("payload[value]", quantity.to_string()),
            ],
        )
        .await?;
        db.mark_usage_reported(&id).await?;
        sent += 1;
    }
    Ok(sent)
}

/// How many people an owner pays for.
pub async fn people(db: &Db, owner: &Account) -> ApiResult<i64> {
    Ok(match owner.kind {
        AccountKind::Org => db.count_members(&owner.id).await?,
        _ => 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, t: i64, payload: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{t}.").as_bytes());
        mac.update(payload);
        format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn webhook_signatures() {
        let payload = br#"{"id":"evt_1"}"#;
        let header = sign("whsec_x", 1000, payload);
        assert!(verify_signature("whsec_x", &header, payload, 1100));
        assert!(!verify_signature("whsec_y", &header, payload, 1100));
        assert!(!verify_signature("whsec_x", &header, b"{}", 1100));
        // Replayed later than the tolerance.
        assert!(!verify_signature("whsec_x", &header, payload, 1000 + 301));
        assert!(!verify_signature("whsec_x", "garbage", payload, 1000));
    }
}
