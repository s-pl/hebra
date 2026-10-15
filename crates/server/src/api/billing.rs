//! Plans and billing (Cloud): the public plan list, an owner's plan and
//! usage, checkout, the customer portal, spending limits, and Stripe's
//! webhook.

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::billing::stripe;
use crate::db::Account;
use crate::error::{ApiError, ApiResult};
use crate::identity::Principal;
use crate::server::Server;

/// Plans and prices, for the pricing page.
pub(crate) async fn plans(State(s): St) -> ApiResult<axum::Json<Value>> {
    let plans = s
        .plans
        .as_ref()
        .ok_or_else(|| ApiError::not_found("this server has no plans (Hebra Community is free)"))?;
    Ok(axum::Json(json!(plans)))
}

async fn billed(s: &Server, who: &Principal, handle: &str) -> ApiResult<Account> {
    if s.plans.is_none() {
        return Err(ApiError::not_found("billing is not enabled on this server"));
    }
    let owner =
        s.db.account_by_handle(handle)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("`{handle}` not found")))?;
    if !s.manages(who, &owner, true).await? {
        return Err(ApiError::forbidden(
            "forbidden",
            format!("you do not manage billing for `{handle}`"),
        ));
    }
    Ok(owner)
}

pub(crate) async fn overview(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = billed(&s, &who, &handle).await?;
    let allowance = s.allowance(&owner).await?;
    Ok(axum::Json(json!({
        "allowance": allowance,
        "subscription": s.db.subscription(&owner.id).await?,
        "usage": s.db.usage(&owner.id).await?,
        "credits_left": s.credits_left(&owner).await?,
        "payments": s.stripe.is_some(),
    })))
}

#[derive(Deserialize)]
pub(crate) struct CheckoutBody {
    plan: String,
}

pub(crate) async fn checkout(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    axum::Json(body): axum::Json<CheckoutBody>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = billed(&s, &who, &handle).await?;
    let stripe_config = s
        .stripe
        .as_ref()
        .ok_or_else(|| ApiError::bad("payments are not configured on this server"))?;
    let plan = s
        .plans
        .as_ref()
        .and_then(|p| p.plan(&body.plan))
        .ok_or_else(|| ApiError::bad(format!("unknown plan `{}`", body.plan)))?;
    let people = stripe::people(&s.db, &owner).await?;
    let return_url = format!("{}/settings/{}/billing", s.public_url, owner.handle);
    let url = stripe::checkout(
        &s.http,
        stripe_config,
        &s.db,
        &stripe::Purchase {
            owner: &owner,
            email: who.account.email.as_deref(),
            plan,
            people,
            return_url: &return_url,
        },
    )
    .await?;
    Ok(axum::Json(json!({ "url": url })))
}

pub(crate) async fn portal(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = billed(&s, &who, &handle).await?;
    let stripe_config = s
        .stripe
        .as_ref()
        .ok_or_else(|| ApiError::bad("payments are not configured on this server"))?;
    let return_url = format!("{}/settings/{}/billing", s.public_url, owner.handle);
    let url = stripe::portal(&s.http, stripe_config, &s.db, &owner, &return_url).await?;
    Ok(axum::Json(json!({ "url": url })))
}

#[derive(Deserialize)]
pub(crate) struct LimitBody {
    /// Monthly spending above the plan, in cents (0 stops at the allowance).
    cents: i64,
}

pub(crate) async fn set_limit(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    axum::Json(body): axum::Json<LimitBody>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = billed(&s, &who, &handle).await?;
    if !(0..=100_000_000).contains(&body.cents) {
        return Err(ApiError::bad(
            "the spending limit is between $0 and $1,000,000",
        ));
    }
    if s.db.subscription(&owner.id).await?.is_none() {
        s.db.upsert_subscription(&owner.id, "free", "free", None, None, 1, None, None)
            .await?;
    }
    s.db.set_spending_limit(&owner.id, body.cents).await?;
    Ok(axum::Json(json!({ "spending_limit_cents": body.cents })))
}

/// Stripe calls this; the signature proves it.
pub(crate) async fn stripe_webhook(
    State(s): St,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<axum::Json<Value>> {
    let (Some(config), Some(plans)) = (&s.stripe, &s.plans) else {
        return Err(ApiError::not_found("billing is not enabled on this server"));
    };
    let signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !stripe::verify_signature(
        &config.webhook_secret,
        signature,
        &body,
        jiff::Timestamp::now().as_second(),
    ) {
        return Err(ApiError::unauthorized("invalid signature"));
    }
    let event: Value = serde_json::from_slice(&body)?;
    stripe::handle_event(&s.db, plans, &event).await?;
    Ok(axum::Json(json!({ "received": true })))
}
